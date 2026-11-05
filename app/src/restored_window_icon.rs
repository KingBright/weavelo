use gpui::{App,Window,WindowIcon};
use std::sync::OnceLock;
use remote_core::application_icons::IconBitmap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DockAction {
    Activate,
    End,
}

#[derive(Default)]
pub(super) struct Artwork {
    current: Option<[u8; 32]>,
    #[cfg(target_os = "macos")]
    dock_icon: Option<[u8; 32]>,
    #[cfg(target_os = "macos")]
    dock_title: String,
    #[cfg(target_os = "macos")]
    dock_device: String,
    #[cfg(target_os = "macos")]
    dock_proxy: Option<macos_dock::Proxy>,
}
impl Artwork {
    pub fn sync(&mut self,window:&mut Window,cx:&App,bitmap:Option<&IconBitmap>) {
        let next=bitmap.map(|bitmap|bitmap.id);
        if self.current==next {return;}
        let result=(||->Result<(),String> {
            if let Some(bitmap)=bitmap {
                let (edge,pixels)=remote_application_pixels(bitmap)?;
                let icon=WindowIcon::new(edge,pixels).map_err(|e|e.to_string())?;
                window.set_window_icon(&icon).map_err(|e|e.to_string())?;
            } else {apply(window,cx);}
            Ok(())
        })();
        match result {
            Ok(())=>self.current=next,
            Err(error)=>eprintln!("Weavelo application window icon: {error}"),
        }
    }

    pub fn sync_stream(&mut self, window: &mut Window, cx: &App, bitmap: Option<&IconBitmap>, title: &str, device: &str) {
        self.sync(window, cx, bitmap);
        #[cfg(target_os = "macos")]
        {
            if cfg!(test)
                || std::env::var_os("WEAVELO_RESTORED_TEST_OUTPUT").is_some()
                || std::env::var_os("WEAVELO_DISABLE_DOCK_TILES").is_some()
            {
                return;
            }
            let next_icon = Some(bitmap.map_or([0u8; 32], |b| b.id));
            let icon_changed = self.dock_icon != next_icon || self.dock_proxy.is_none();
            let title_changed = self.dock_title != title || self.dock_device != device || self.dock_proxy.is_none();
            if !icon_changed && !title_changed {
                return;
            }
            let pixels = if icon_changed {
                let raw = if let Some(bitmap) = bitmap {
                    remote_application_pixels(bitmap).map(|(_, rgba)| rgba)
                } else {
                    default_remote_dock_pixels(cx)
                };
                match raw {
                    Ok(rgba) => Some(rgba),
                    Err(error) => {
                        eprintln!("Weavelo stream Dock icon pixels: {error}");
                        None
                    }
                }
            } else {
                None
            };
            if self.dock_proxy.is_none() {
                match macos_dock::Proxy::spawn() {
                    Ok(proxy) => self.dock_proxy = Some(proxy),
                    Err(error) => {
                        eprintln!("Weavelo stream Dock proxy: {error}");
                        return;
                    }
                }
            }
            if let Some(proxy) = &mut self.dock_proxy {
                if proxy.send(title, device, pixels.as_deref()).is_ok() {
                    self.dock_icon = next_icon;
                    self.dock_title = title.to_owned();
                    self.dock_device = device.to_owned();
                } else {
                    self.dock_proxy = None;
                }
            }
        }
        #[cfg(not(target_os = "macos"))]
        let _ = (title, device);
    }

    pub fn poll_dock_action(&mut self) -> Option<DockAction> {
        #[cfg(target_os = "macos")]
        {
            if let Some(proxy) = &mut self.dock_proxy {
                return proxy.poll();
            }
        }
        None
    }

    pub fn close(&mut self) {
        #[cfg(target_os = "macos")]
        {
            self.dock_proxy = None;
            self.dock_icon = None;
            self.dock_title.clear();
            self.dock_device.clear();
        }
    }
}

fn remote_application_pixels(bitmap:&IconBitmap)->Result<(u32,Vec<u8>),String> {
    let (edge,pixels)=native_pixels(bitmap);
    let image=image::RgbaImage::from_raw(edge,edge,pixels).ok_or("Invalid application icon")?;
    let mut image=image::imageops::resize(&image,64,64,image::imageops::FilterType::Triangle);
    composite_weavelo_badge(&mut image)?;
    Ok((64,image.into_raw()))
}

fn composite_weavelo_badge(image: &mut image::RgbaImage) -> Result<(), String> {
    static BADGE:OnceLock<image::RgbaImage>=OnceLock::new();
    let badge=if let Some(badge)=BADGE.get() {badge}else {
        let mut badge=image::load_from_memory(include_bytes!("../../assets/brand/weavelo/icon-24.png"))
            .map_err(|error|error.to_string())?.into_rgba8();
        for pixel in badge.pixels_mut() {
            for channel in 0..3 {pixel[channel]=((u16::from(pixel[channel])*u16::from(pixel[3])+127)/255) as u8;}
        }
        BADGE.get_or_init(||badge)
    };
    // Source-over in premultiplied space: keep the application's identity and
    // add the product mark once per asset change, never in the video path.
    for (x,y,foreground) in badge.enumerate_pixels() {
        let background=image.get_pixel_mut(x+40,y+40);
        let remaining=255-u16::from(foreground[3]);
        for channel in 0..4 {
            background[channel]=(u16::from(foreground[channel])+(u16::from(background[channel])*remaining+127)/255).min(255) as u8;
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn default_remote_dock_pixels(cx: &App) -> Result<Vec<u8>, String> {
    static DOCK_FALLBACK: OnceLock<Vec<u8>> = OnceLock::new();
    if let Some(pixels) = DOCK_FALLBACK.get() {
        return Ok(pixels.clone());
    }
    let image = cx
        .svg_renderer()
        .render_single_frame(include_bytes!("product_components/icons/remote-window.svg"), 1., false)
        .map_err(|e| e.to_string())?;
    let size = image.size(0);
    let rgba = image::RgbaImage::from_raw(
        size.width.0 as u32,
        size.height.0 as u32,
        image.as_bytes(0).ok_or("Missing native window icon pixels")?.to_vec(),
    )
    .ok_or("Invalid window icon dimensions")?;
    let mut resized = image::imageops::resize(&rgba, 64, 64, image::imageops::FilterType::Triangle);
    composite_weavelo_badge(&mut resized)?;
    let raw = resized.into_raw();
    Ok(DOCK_FALLBACK.get_or_init(|| raw).clone())
}

fn native_pixels(bitmap:&IconBitmap)->(u32,Vec<u8>) {
    // Catalog PNGs contain straight RGBA; native window icons require square,
    // premultiplied RGBA. Preserve aspect and pad instead of stretching artwork.
    let edge=bitmap.width.max(bitmap.height);
    let mut pixels=vec![0;edge as usize*edge as usize*4];
    let left=(edge-bitmap.width)/2;
    let top=(edge-bitmap.height)/2;
    for y in 0..bitmap.height {
        for x in 0..bitmap.width {
            let from=((y*bitmap.width+x)*4) as usize;
            let to=(((y+top)*edge+x+left)*4) as usize;
            let alpha=u16::from(bitmap.rgba[from+3]);
            for channel in 0..3 {pixels[to+channel]=((u16::from(bitmap.rgba[from+channel])*alpha+127)/255) as u8;}
            pixels[to+3]=alpha as u8;
        }
    }
    (edge,pixels)
}

pub(super) fn apply(window:&mut Window,cx:&App) {
    let result=(||->Result<bool,String> {
        static ICON:OnceLock<WindowIcon>=OnceLock::new();
        let icon=if let Some(icon)=ICON.get() {icon.clone()}else{
            let image=cx.svg_renderer().render_single_frame(
                include_bytes!("product_components/icons/remote-window.svg"),1.,false).map_err(|e|e.to_string())?;
            let size=image.size(0);
            let rgba=image::RgbaImage::from_raw(size.width.0 as u32,size.height.0 as u32,
                image.as_bytes(0).ok_or("Missing native window icon pixels")?.to_vec()).ok_or("Invalid window icon dimensions")?;
            let rgba=image::imageops::resize(&rgba,64,64,image::imageops::FilterType::Triangle).into_raw();
            let icon=WindowIcon::new(64,rgba).map_err(|e|e.to_string())?;
            ICON.get_or_init(||icon).clone()
        };
        window.set_window_icon(&icon).map_err(|e|e.to_string())
    })();
    if let Err(error)=result {eprintln!("Weavelo native window icon: {error}");}
}

#[cfg(target_os = "macos")]
pub(crate) mod macos_dock {
    use super::DockAction;
    use cocoa::{
        appkit::{NSApplication, NSApplicationActivationPolicy},
        base::{BOOL, NO, YES, id, nil},
        foundation::{NSAutoreleasePool, NSSize, NSString},
    };
    use objc::{
        class,
        declare::ClassDecl,
        msg_send,
        runtime::{Object, Sel},
        sel, sel_impl,
    };
    use std::{
        io::{BufRead, BufReader, Read, Write},
        process::{Child, ChildStdin, Command, Stdio},
        sync::mpsc::{self, Receiver},
    };

    const ICON_BYTES: usize = 64 * 64 * 4;
    const MAX_TITLE_BYTES: usize = 512;

    pub(super) struct Proxy {
        child: Child,
        stdin: Option<ChildStdin>,
        actions: Receiver<DockAction>,
    }

    impl Proxy {
        pub fn spawn() -> Result<Self, String> {
            let exe = std::env::current_exe().map_err(|e| e.to_string())?;
            let mut child = Command::new(exe)
                .arg("--macos-dock-tile")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|e| e.to_string())?;
            let stdin = child.stdin.take().ok_or("Missing Dock proxy stdin")?;
            let stdout = child.stdout.take().ok_or("Missing Dock proxy stdout")?;
            let (tx, actions) = mpsc::channel();
            std::thread::spawn(move || {
                let reader = BufReader::new(stdout);
                for line in reader.lines().map_while(Result::ok) {
                    let action = match line.trim() {
                        "activate" => Some(DockAction::Activate),
                        "end" => Some(DockAction::End),
                        _ => None,
                    };
                    if let Some(action) = action && tx.send(action).is_err() {
                        break;
                    }
                }
            });
            Ok(Self {
                child,
                stdin: Some(stdin),
                actions,
            })
        }

        pub fn send(&mut self, title: &str, device: &str, rgba: Option<&[u8]>) -> Result<(), String> {
            let stdin = self.stdin.as_mut().ok_or("Dock proxy closed")?;
            let frame = encode_frame(title, device, rgba)?;
            stdin.write_all(&frame).map_err(|e| e.to_string())?;
            stdin.flush().map_err(|e| e.to_string())
        }

        pub fn poll(&mut self) -> Option<DockAction> {
            let mut latest = None;
            while let Ok(action) = self.actions.try_recv() {
                latest = Some(action);
                if action == DockAction::End {
                    break;
                }
            }
            latest
        }
    }

    impl Drop for Proxy {
        fn drop(&mut self) {
            self.stdin.take();
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    pub(super) fn encode_frame(title: &str, device: &str, rgba: Option<&[u8]>) -> Result<Vec<u8>, String> {
        let title_bytes = title.as_bytes();
        let bounded_title = &title_bytes[..title.floor_char_boundary(MAX_TITLE_BYTES.min(title_bytes.len()))];
        let pixels = match rgba {
            Some(bytes) if bytes.len() == ICON_BYTES => bytes,
            Some(_) => return Err("Dock icon frame must be 64x64 RGBA".into()),
            None => &[][..],
        };
        let bounded_device = &device.as_bytes()[..device.floor_char_boundary(MAX_TITLE_BYTES.min(device.len()))];
        let mut out = Vec::with_capacity(12 + bounded_title.len() + bounded_device.len() + pixels.len());
        out.extend_from_slice(&(bounded_title.len() as u32).to_be_bytes());
        out.extend_from_slice(bounded_title);
        out.extend_from_slice(&(bounded_device.len() as u32).to_be_bytes());
        out.extend_from_slice(bounded_device);
        out.extend_from_slice(&(pixels.len() as u32).to_be_bytes());
        out.extend_from_slice(pixels);
        Ok(out)
    }

    pub(super) fn decode_frame(mut reader: impl Read) -> std::io::Result<(String, String, Option<Vec<u8>>)> {
        let mut header = [0u8; 4];
        reader.read_exact(&mut header)?;
        let title_len = u32::from_be_bytes(header) as usize;
        if title_len > MAX_TITLE_BYTES {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "Dock title too long"));
        }
        let mut title_bytes = vec![0u8; title_len];
        reader.read_exact(&mut title_bytes)?;
        let title = String::from_utf8_lossy(&title_bytes).into_owned();
        reader.read_exact(&mut header)?;
        let device_len = u32::from_be_bytes(header) as usize;
        if device_len > MAX_TITLE_BYTES {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "Dock device name too long"));
        }
        let mut device_bytes = vec![0u8; device_len];
        reader.read_exact(&mut device_bytes)?;
        let device = String::from_utf8_lossy(&device_bytes).into_owned();
        reader.read_exact(&mut header)?;
        let pixels_len = u32::from_be_bytes(header) as usize;
        let pixels = if pixels_len == 0 {
            None
        } else if pixels_len == ICON_BYTES {
            let mut buf = vec![0u8; ICON_BYTES];
            reader.read_exact(&mut buf)?;
            Some(buf)
        } else {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "Invalid Dock icon length"));
        };
        Ok((title, device, pixels))
    }

    fn ns_string(value: &str) -> id {
        unsafe {
            let s = NSString::alloc(nil).init_str(value);
            let _: id = msg_send![s, autorelease];
            s
        }
    }

    fn emit_action(command: &str) {
        let mut stdout = std::io::stdout().lock();
        let _ = writeln!(stdout, "{command}");
        let _ = stdout.flush();
    }

    extern "C" fn did_become_active(_: &Object, _: Sel, _: id) {
        emit_action("activate");
        unsafe {
            let app = NSApplication::sharedApplication(nil);
            let _: () = msg_send![app, deactivate];
        }
    }

    extern "C" fn should_reopen(_: &Object, _: Sel, _: id, _: BOOL) -> BOOL {
        emit_action("activate");
        YES
    }

    extern "C" fn activate_menu(_: &Object, _: Sel, _: id) {
        emit_action("activate");
    }

    extern "C" fn end_menu(_: &Object, _: Sel, _: id) {
        emit_action("end");
    }

    extern "C" fn should_terminate(_: &Object, _: Sel, _: id) -> usize {
        emit_action("end");
        0
    }

    extern "C" fn dock_menu(this: &Object, _: Sel, _: id) -> id {
        unsafe {
            let title: id = *this.get_ivar("streamTitle");
            let label = if title == nil { ns_string("Show remote window") } else { title };
            let menu: id = msg_send![class!(NSMenu), new];
            let _: id = msg_send![menu, autorelease];
            let _: () = msg_send![menu, setAutoenablesItems: NO];
            let show: id = msg_send![class!(NSMenuItem), alloc];
            let show: id = msg_send![
                show,
                initWithTitle: label
                action: sel!(activateWeaveloStream:)
                keyEquivalent: ns_string("")
            ];
            let _: () = msg_send![show, setTarget: this as *const Object as id];
            let _: () = msg_send![show, setEnabled: YES];
            let _: () = msg_send![menu, addItem: show];
            let _: () = msg_send![show, release];

            let end: id = msg_send![class!(NSMenuItem), alloc];
            let end: id = msg_send![
                end,
                initWithTitle: ns_string("Close Remote Window")
                action: sel!(endWeaveloStream:)
                keyEquivalent: ns_string("")
            ];
            let _: () = msg_send![end, setTarget: this as *const Object as id];
            let _: () = msg_send![end, setEnabled: YES];
            let _: () = msg_send![menu, addItem: end];
            let _: () = msg_send![end, release];
            menu
        }
    }

    pub(super) fn device_badge(device: &str) -> String {
        let name = device.trim();
        let chars: Vec<_> = name.chars().collect();
        if chars.len() <= 7 { return name.to_owned(); }
        format!("{}…{}", chars[..3].iter().collect::<String>(),
            chars[chars.len()-2..].iter().collect::<String>())
    }

    unsafe fn update_presentation(title: &str, device: &str) {
        unsafe {
            let app = NSApplication::sharedApplication(nil);
            let delegate: id = msg_send![app, delegate];
            if delegate != nil {
                let old: id = *(*delegate).get_ivar("streamTitle");
                let next = ns_string(&format!("Show {title}"));
                let _: id = msg_send![next, retain];
                (*delegate).set_ivar("streamTitle", next);
                if old != nil { let _: () = msg_send![old, release]; }
            }
            let tile: id = msg_send![app, dockTile];
            if tile != nil {
                let _: () = msg_send![tile, setBadgeLabel: ns_string(&device_badge(device))];
                let _: () = msg_send![tile, display];
            }
        }
    }

    unsafe fn set_dock_icon_rgba(rgba: &[u8]) {
        if rgba.len() != ICON_BYTES {
            return;
        }
        unsafe {
            let rep: id = msg_send![class!(NSBitmapImageRep), alloc];
            let rep: id = msg_send![
                rep,
                initWithBitmapDataPlanes: std::ptr::null_mut::<*mut u8>()
                pixelsWide: 64isize
                pixelsHigh: 64isize
                bitsPerSample: 8isize
                samplesPerPixel: 4isize
                hasAlpha: YES
                isPlanar: NO
                colorSpaceName: ns_string("NSCalibratedRGBColorSpace")
                bitmapFormat: 0usize
                bytesPerRow: 256isize
                bitsPerPixel: 32isize
            ];
            if rep == nil {
                return;
            }
            let data: *mut u8 = msg_send![rep, bitmapData];
            if !data.is_null() {
                std::ptr::copy_nonoverlapping(rgba.as_ptr(), data, ICON_BYTES);
                let image: id = msg_send![class!(NSImage), alloc];
                let image: id = msg_send![image, initWithSize: NSSize::new(64.0, 64.0)];
                let _: () = msg_send![image, addRepresentation: rep];
                let app = NSApplication::sharedApplication(nil);
                let _: () = msg_send![app, setApplicationIconImage: image];
                let tile: id = msg_send![app, dockTile];
                if tile != nil {
                    let _: () = msg_send![tile, display];
                }
                let _: () = msg_send![image, release];
            }
            let _: () = msg_send![rep, release];
        }
    }

    pub fn run() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // This helper starts directly on main, before Tokio initialization. It
        // owns no async media/network tasks and must not enter the GUI runtime
        // adapter, which intentionally requires a multi-thread Tokio executor.
        unsafe {
            let pool = NSAutoreleasePool::new(nil);
            let app = NSApplication::sharedApplication(nil);
            app.setActivationPolicy_(
                NSApplicationActivationPolicy::NSApplicationActivationPolicyRegular,
            );
            if let Some(mut decl) = ClassDecl::new("WeaveloDockTileDelegate", class!(NSObject)) {
                decl.add_ivar::<id>("streamTitle");
                decl.add_method(
                    sel!(applicationDidBecomeActive:),
                    did_become_active as extern "C" fn(&Object, Sel, id),
                );
                decl.add_method(
                    sel!(applicationShouldHandleReopen:hasVisibleWindows:),
                    should_reopen as extern "C" fn(&Object, Sel, id, BOOL) -> BOOL,
                );
                decl.add_method(
                    sel!(activateWeaveloStream:),
                    activate_menu as extern "C" fn(&Object, Sel, id),
                );
                decl.add_method(
                    sel!(endWeaveloStream:),
                    end_menu as extern "C" fn(&Object, Sel, id),
                );
                decl.add_method(
                    sel!(applicationShouldTerminate:),
                    should_terminate as extern "C" fn(&Object, Sel, id) -> usize,
                );
                decl.add_method(
                    sel!(applicationDockMenu:),
                    dock_menu as extern "C" fn(&Object, Sel, id) -> id,
                );
                let delegate: id = msg_send![decl.register(), new];
                (*delegate).set_ivar("streamTitle", nil);
                let _: () = msg_send![app, setDelegate: delegate];
            }
            std::thread::spawn(|| {
                let mut stdin = std::io::stdin().lock();
                while let Ok((title, device, pixels)) = decode_frame(&mut stdin) {
                    dispatch::Queue::main().exec_async(move || unsafe {
                        update_presentation(&title, &device);
                        if let Some(rgba) = pixels {
                            set_dock_icon_rgba(&rgba);
                        }
                    });
                }
                std::process::exit(0);
            });
            app.run();
            let _: () = msg_send![pool, drain];
            Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn application_artwork_keeps_aspect_and_premultiplies_transparent_pixels() {
        let bitmap=IconBitmap{id:[1;32],width:2,height:1,
            rgba:vec![255,128,64,128, 255,255,255,0].into(),png:vec![].into()};
        let (edge,pixels)=native_pixels(&bitmap);
        assert_eq!(edge,2);
        assert_eq!(pixels,vec![128,64,32,128, 0,0,0,0, 0,0,0,0, 0,0,0,0]);
        assert!(WindowIcon::new(edge,pixels).is_ok());
    }
    #[test]
    fn remote_badge_keeps_the_application_and_has_valid_premultiplied_alpha() {
        let bitmap=IconBitmap{id:[2;32],width:64,height:64,
            rgba:[240,80,20,255].repeat(64*64).into(),png:vec![].into()};
        let (edge,pixels)=remote_application_pixels(&bitmap).unwrap();
        assert_eq!(edge,64);
        assert_eq!(&pixels[..4],&[240,80,20,255]);
        assert_ne!(&pixels[(54*64+54)*4..(54*64+54)*4+4],&[240,80,20,255]);
        assert!(pixels.chunks_exact(4).all(|pixel|pixel[..3].iter().all(|channel|*channel<=pixel[3])));
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn macos_dock_frame_round_trips_title_and_optional_icon() {
        let pixels = vec![42u8; 64 * 64 * 4];
        let encoded = macos_dock::encode_frame("Safari · MacStudio", "MacStudio", Some(&pixels)).unwrap();
        let (title, device, decoded_pixels) = macos_dock::decode_frame(&encoded[..]).unwrap();
        assert_eq!(title, "Safari · MacStudio");
        assert_eq!(device, "MacStudio");
        assert_eq!(decoded_pixels.as_deref(), Some(pixels.as_slice()));

        let title_only = macos_dock::encode_frame("Terminal · Mini", "Mini", None).unwrap();
        let (title2, device2, decoded_none) = macos_dock::decode_frame(&title_only[..]).unwrap();
        assert_eq!(title2, "Terminal · Mini");
        assert_eq!(device2, "Mini");
        assert!(decoded_none.is_none());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn dock_title_preserves_utf8_at_the_length_boundary_and_rejects_bad_pixels() {
        let title = format!("{}远程📱", "a".repeat(511));
        let encoded = macos_dock::encode_frame(&title, "工作室", None).unwrap();
        let (decoded, device, pixels) = macos_dock::decode_frame(&encoded[..]).unwrap();
        assert_eq!(decoded, "a".repeat(511));
        assert_eq!(device, "工作室");
        assert!(pixels.is_none());
        assert!(macos_dock::encode_frame("title", "Device", Some(&[0; 4])).is_err());
        let mut invalid = 513u32.to_be_bytes().to_vec();
        invalid.extend_from_slice(&[0; 513]);
        assert!(macos_dock::decode_frame(&invalid[..]).is_err());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn device_badge_keeps_short_aliases_and_both_ends_of_long_names() {
        assert_eq!(macos_dock::device_badge("  Studio  "), "Studio");
        assert_eq!(macos_dock::device_badge("MBPM2MAX"), "MBP…AX");
        assert_eq!(macos_dock::device_badge("工作室电脑二号"), "工作室电脑二号");
        assert_eq!(macos_dock::device_badge("A very long device 42"), "A v…42");
    }
}
