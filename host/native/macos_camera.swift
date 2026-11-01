import AVFoundation
import CoreMedia
import CoreVideo
import Foundation

typealias FrameCallback = @convention(c) (UnsafeMutableRawPointer?, UnsafeMutableRawPointer?, UInt32, UInt32) -> Void

private func devices() -> [AVCaptureDevice] {
    var types: [AVCaptureDevice.DeviceType] = [.builtInWideAngleCamera]
    if #available(macOS 14.0, *) { types += [.external, .continuityCamera] }
    else { types.append(.externalUnknown) }
    return AVCaptureDevice.DiscoverySession(deviceTypes: types, mediaType: .video, position: .unspecified).devices
}

@_cdecl("weavelo_camera_catalog")
func cameraCatalog() -> UnsafeMutablePointer<CChar>? {
    let cameras: [[String: Any]] = devices().map { device in
        let size = CMVideoFormatDescriptionGetDimensions(device.activeFormat.formatDescription)
        let facing = device.position == .front ? "Front" : device.position == .back ? "Back" : "Unspecified"
        return ["key": device.uniqueID, "name": device.localizedName, "facing": facing,
                "switch_group": NSNull(), "width": max(0, size.width), "height": max(0, size.height), "rotation": 0]
    }
    guard let data = try? JSONSerialization.data(withJSONObject: cameras),
          let text = String(data: data, encoding: .utf8) else { return nil }
    return strdup(text)
}

@_cdecl("weavelo_camera_free_string")
func cameraFreeString(_ text: UnsafeMutablePointer<CChar>?) { free(text) }

@_cdecl("weavelo_camera_access")
func cameraAccess() -> Int32 { Int32(AVCaptureDevice.authorizationStatus(for: .video).rawValue) }

@_cdecl("weavelo_camera_request_access")
func cameraRequestAccess(_ context: UnsafeMutableRawPointer?,
                         _ done: @escaping @convention(c) (UnsafeMutableRawPointer?, Int32) -> Void) {
    AVCaptureDevice.requestAccess(for: .video) { granted in done(context, granted ? 3 : 2) }
}

private final class CameraOutput: NSObject, AVCaptureVideoDataOutputSampleBufferDelegate {
    let context: UnsafeMutableRawPointer?
    let callback: FrameCallback
    init(_ context: UnsafeMutableRawPointer?, _ callback: @escaping FrameCallback) {
        self.context = context; self.callback = callback
    }
    func captureOutput(_ output: AVCaptureOutput, didOutput sample: CMSampleBuffer, from connection: AVCaptureConnection) {
        guard CMSampleBufferDataIsReady(sample), let image = CMSampleBufferGetImageBuffer(sample) else { return }
        // Transfer one retained CMSampleBuffer; the CVPixelBuffer never becomes CPU pixels.
        callback(context, Unmanaged.passRetained(sample).toOpaque(),
                 UInt32(CVPixelBufferGetWidth(image)), UInt32(CVPixelBufferGetHeight(image)))
    }
}

private final class CameraSession {
    var session: AVCaptureSession?
    var output: AVCaptureVideoDataOutput?
    var delegate: CameraOutput?
    let frames = DispatchQueue(label: "fun.hackerlife.weavelo.camera.frames", qos: .userInteractive)

    init(_ key: String, _ width: UInt32, _ height: UInt32, _ fps: UInt32,
         _ context: UnsafeMutableRawPointer?, _ callback: @escaping FrameCallback) throws {
        guard AVCaptureDevice.authorizationStatus(for: .video) == .authorized else {
            throw NSError(domain: "WeaveloCamera", code: 1, userInfo: [NSLocalizedDescriptionKey:
                "Camera permission is required on this Mac. Open Weavelo Sharing & permissions."])
        }
        guard let device = devices().first(where: { $0.uniqueID == key }) else {
            throw NSError(domain: "WeaveloCamera", code: 2, userInfo: [NSLocalizedDescriptionKey: "The selected camera is no longer available"])
        }
        let wantedArea = UInt64(width) * UInt64(height)
        let formats = device.formats.filter { !$0.videoSupportedFrameRateRanges.isEmpty }
        func area(_ format: AVCaptureDevice.Format) -> UInt64 {
            let size = CMVideoFormatDescriptionGetDimensions(format.formatDescription)
            return UInt64(max(0, size.width)) * UInt64(max(0, size.height))
        }
        guard let format = formats.filter({ area($0) <= wantedArea }).max(by: { area($0) < area($1) })
                ?? formats.min(by: { area($0) < area($1) }) else {
            throw NSError(domain: "WeaveloCamera", code: 3, userInfo: [NSLocalizedDescriptionKey: "Camera has no supported video format"])
        }
        let rate = format.videoSupportedFrameRateRanges.min(by: {
            abs(min(max(Double(fps), $0.minFrameRate), $0.maxFrameRate)-Double(fps)) <
            abs(min(max(Double(fps), $1.minFrameRate), $1.maxFrameRate)-Double(fps))
        })!
        let interval = Double(fps) >= rate.maxFrameRate ? rate.minFrameDuration :
            Double(fps) <= rate.minFrameRate ? rate.maxFrameDuration : CMTime(value: 1, timescale: CMTimeScale(fps))
        let session = AVCaptureSession(); let output = AVCaptureVideoDataOutput()
        let delegate = CameraOutput(context, callback)
        self.session = session; self.output = output; self.delegate = delegate
        var complete = false
        defer {
            if !complete {
                if session.isRunning { session.stopRunning() }
                output.setSampleBufferDelegate(nil, queue: nil)
                frames.sync {}
            }
        }
        session.beginConfiguration()
        let input = try AVCaptureDeviceInput(device: device)
        guard session.canAddInput(input) else {
            throw NSError(domain: "WeaveloCamera", code: 4, userInfo: [NSLocalizedDescriptionKey: "Camera input is unavailable or occupied"])
        }
        session.addInput(input)
        let selectedSize = CMVideoFormatDescriptionGetDimensions(format.formatDescription)
        let preset: AVCaptureSession.Preset?
        switch (selectedSize.width, selectedSize.height) {
        case (3840, 2160): preset = .hd4K3840x2160
        case (1920, 1080): preset = .hd1920x1080
        case (1280, 720): preset = .hd1280x720
        case (640, 480): preset = .vga640x480
        default: preset = nil
        }
        // macOS has no inputPriority preset. Its default .high preset can
        // replace activeFormat at start, so align the session with the native
        // mode instead of letting it silently promote 720p to 1080p.
        if let preset = preset {
            guard session.canSetSessionPreset(preset) else {
                throw NSError(domain: "WeaveloCamera", code: 8, userInfo: [NSLocalizedDescriptionKey:
                    "Camera cannot retain its selected native video mode"])
            }
            session.sessionPreset = preset
        }
        try device.lockForConfiguration()
        device.activeFormat = format
        device.activeVideoMinFrameDuration = interval; device.activeVideoMaxFrameDuration = interval
        device.unlockForConfiguration()
        let available = output.availableVideoPixelFormatTypes
        guard let pixel = [kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange, kCVPixelFormatType_420YpCbCr8BiPlanarFullRange]
            .first(where: { available.contains($0) }) else {
            throw NSError(domain: "WeaveloCamera", code: 5, userInfo: [NSLocalizedDescriptionKey: "Camera cannot produce native NV12 buffers"])
        }
        output.videoSettings = [kCVPixelBufferPixelFormatTypeKey as String: pixel]
        output.alwaysDiscardsLateVideoFrames = true
        output.setSampleBufferDelegate(delegate, queue: frames)
        guard session.canAddOutput(output) else {
            throw NSError(domain: "WeaveloCamera", code: 6, userInfo: [NSLocalizedDescriptionKey: "Camera output cannot be configured"])
        }
        session.addOutput(output)
        if let connection = output.connection(with: .video), connection.isVideoMirroringSupported {
            connection.automaticallyAdjustsVideoMirroring = false; connection.isVideoMirrored = false
        }
        session.commitConfiguration()
        session.startRunning()
        guard session.isRunning else {
            throw NSError(domain: "WeaveloCamera", code: 7, userInfo: [NSLocalizedDescriptionKey: "Camera did not start; check permission and availability"])
        }
        let actualSize = CMVideoFormatDescriptionGetDimensions(device.activeFormat.formatDescription)
        guard actualSize.width == selectedSize.width && actualSize.height == selectedSize.height else {
            throw NSError(domain: "WeaveloCamera", code: 9, userInfo: [NSLocalizedDescriptionKey:
                "Camera changed its selected native video mode"])
        }
        complete = true
    }
    func stop() {
        session?.stopRunning()
        output?.setSampleBufferDelegate(nil, queue: nil)
        frames.sync {}
        // Release native resources before acknowledging teardown to Rust.
        delegate = nil; output = nil; session = nil
    }
}

@_cdecl("weavelo_camera_start")
func cameraStart(_ key: UnsafePointer<CChar>?, _ width: UInt32, _ height: UInt32, _ fps: UInt32,
                 _ context: UnsafeMutableRawPointer?, _ callback: @escaping FrameCallback,
                 _ error: UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>?) -> UnsafeMutableRawPointer? {
    guard let key = key, width >= 2, height >= 2, fps > 0, fps <= 240 else {
        error?.pointee = strdup("Invalid camera configuration"); return nil
    }
    do {
        return Unmanaged.passRetained(try CameraSession(String(cString: key), width, height, fps, context, callback)).toOpaque()
    } catch let reason { error?.pointee = strdup(reason.localizedDescription); return nil }
}

@_cdecl("weavelo_camera_stop")
func cameraStop(_ handle: UnsafeMutableRawPointer, _ context: UnsafeMutableRawPointer?,
                _ done: @escaping @convention(c) (UnsafeMutableRawPointer?) -> Void) {
    DispatchQueue.global(qos: .userInitiated).async {
        let owner = Unmanaged<CameraSession>.fromOpaque(handle)
        autoreleasepool { owner.takeUnretainedValue().stop() }
        owner.release()
        done(context)
    }
}

@_cdecl("weavelo_camera_running")
func cameraRunning(_ handle: UnsafeMutableRawPointer) -> Bool {
    let owner = Unmanaged<CameraSession>.fromOpaque(handle).takeUnretainedValue()
    return AVCaptureDevice.authorizationStatus(for: .video) == .authorized && owner.session?.isRunning == true
}
