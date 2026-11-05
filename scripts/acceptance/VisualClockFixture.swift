// A disposable visual latency marker, never a product capture backend.
import AppKit
import Foundation

guard CommandLine.arguments.count == 2 else { fatalError("Expected an owned state directory") }
let directory = URL(fileURLWithPath: CommandLine.arguments[1])
try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
let app = NSApplication.shared
app.setActivationPolicy(.regular)
var draws: UInt64 = 0
var lastSave: TimeInterval = 0
let clockWindow = NSWindow(contentRect: NSRect(x: 160, y: 180, width: 640, height: 240),
                      styleMask: [.titled, .closable], backing: .buffered, defer: false)
clockWindow.title = "Weavelo visual clock · owned latency fixture"
clockWindow.isReleasedWhenClosed = false
final class ClockView: NSView {
    override var isFlipped: Bool { true }
    override func draw(_ dirtyRect: NSRect) {
        let now = Date().timeIntervalSince1970
        let milliseconds = UInt64(now * 1000)
        draws += 1
        NSColor(calibratedWhite: 0.035, alpha: 1).setFill(); bounds.fill()
        func text(_ value: String, _ y: CGFloat, _ size: CGFloat, _ color: NSColor) {
            (value as NSString).draw(at: NSPoint(x: 20, y: y), withAttributes: [
                .font: NSFont.monospacedDigitSystemFont(ofSize: size, weight: .medium),
                .foregroundColor: color])
        }
        text("UTC milliseconds · frame \(draws)", 12, 18, .lightGray)
        text(String(milliseconds), 42, 48, .white)
        text("Software visual marker · physical presentation unmeasured", 110, 15, .lightGray)
        for row in 0..<2 {
            let y = CGFloat(160 + row * 18)
            NSColor(calibratedRed: 1, green: 0, blue: 1, alpha: 1).setFill()
            NSRect(x: 20, y: y, width: 10, height: 12).fill()
            NSColor(calibratedRed: 0, green: 1, blue: 1, alpha: 1).setFill()
            NSRect(x: 432, y: y, width: 10, height: 12).fill()
            for bit in 0..<64 {
                let on = ((milliseconds >> bit) & 1) != 0
                (on != (row == 1) ? NSColor.white : NSColor.black).setFill()
                NSRect(x: 36 + bit * 6, y: Int(y), width: 6, height: 12).fill()
            }
        }
        if now - lastSave >= 1 {
            lastSave = now
            let status: [String: Any] = ["pid": ProcessInfo.processInfo.processIdentifier,
                "window_id": clockWindow.windowNumber, "draws": draws, "draw_utc_ms": milliseconds,
                "title": clockWindow.title, "auto_stop_seconds": 600]
            if let data = try? JSONSerialization.data(withJSONObject: status, options: [.sortedKeys]) {
                try? data.write(to: directory.appendingPathComponent("state.json"), options: .atomic)
            }
        }
    }
}
final class Delegate: NSObject, NSApplicationDelegate {
    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { true }
}
let delegate = Delegate(); app.delegate = delegate
clockWindow.contentView = ClockView(frame: NSRect(x: 0, y: 0, width: 640, height: 240))
clockWindow.makeKeyAndOrderFront(nil); app.activate(ignoringOtherApps: true)
let timer = Timer(timeInterval: 1.0 / 60, repeats: true) { _ in clockWindow.contentView?.needsDisplay = true }
RunLoop.main.add(timer, forMode: .common)
Timer.scheduledTimer(withTimeInterval: 600, repeats: false) { _ in app.terminate(nil) }
app.run()
