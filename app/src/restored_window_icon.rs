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
    dock_proxy: Option<macos_dock::Proxy>,
}
impl Artwork {
    pub fn sync(&mut self,window:&mut Window,cx:&App,bitmap:Option<&IconBitmap>) {
        let next=bitmap.map(|bitmap|bitmap.id);
        if self.current==next {return;}
        let result=(||->Result<(),String> {
            if let Some(bitmap)=bitmap {
                let (edge,pixels)=remote_application_pixels(bitmap, computer_badge(cx)?)?;
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

    pub fn sync_stream(&mut self, window: &mut Window, cx: &App, bitmap: Option<&IconBitmap>, title: &str) {
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
            let title_changed = self.dock_title != title || self.dock_proxy.is_none();
            if !icon_changed && !title_changed {
                return;
            }
            let pixels = if icon_changed {
                let raw = if let Some(bitmap) = bitmap {
                    computer_badge(cx).and_then(|badge| remote_application_pixels(bitmap, badge))
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
                if proxy.send(title, pixels.as_ref().map(|(edge, rgba)| (*edge, rgba.as_slice()))).is_ok() {
                    self.dock_icon = next_icon;
                    self.dock_title = title.to_owned();
                } else {
                    self.dock_proxy = None;
                }
            }
        }
        #[cfg(not(target_os = "macos"))]
        let _ = title;
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
        }
    }
}

fn remote_application_pixels(
    bitmap: &IconBitmap,
    device_badge: &image::RgbaImage,
) -> Result<(u32, Vec<u8>), String> {
    let (edge, pixels) = native_pixels(bitmap);
    let mut image = image::RgbaImage::from_raw(edge, edge, pixels).ok_or("Invalid application icon")?;
    if edge < 64 {
        image = image::imageops::resize(&image, 64, 64, image::imageops::FilterType::Triangle);
    }
    let edge = image.width();
    let badge_edge = edge * 3 / 8;
    let offset = edge - badge_edge - edge / 32;
    let badge = image::imageops::resize(device_badge, badge_edge, badge_edge, image::imageops::FilterType::Triangle);
    // The application and SVG badge both use premultiplied RGBA here.
    for (x, y, foreground) in badge.enumerate_pixels() {
        let background = image.get_pixel_mut(x + offset, y + offset);
        let remaining = 255 - u16::from(foreground[3]);
        for channel in 0..4 {
            background[channel] = (u16::from(foreground[channel])
                + (u16::from(background[channel]) * remaining + 127) / 255).min(255) as u8;
        }
    }
    Ok((edge, image.into_raw()))
}

fn computer_badge(cx: &App) -> Result<&'static image::RgbaImage, String> {
    static BADGE: OnceLock<image::RgbaImage> = OnceLock::new();
    if let Some(badge) = BADGE.get() {
        return Ok(badge);
    }
    let image = cx.svg_renderer().render_single_frame(
        include_bytes!("product_components/icons/device-badge-computer.svg"), 1., false,
    ).map_err(|error| error.to_string())?;
    let size = image.size(0);
    let badge = image::RgbaImage::from_raw(
        size.width.0 as u32,
        size.height.0 as u32,
        image.as_bytes(0).ok_or("Missing device badge pixels")?.to_vec(),
    ).ok_or("Invalid device badge dimensions")?;
    Ok(BADGE.get_or_init(|| badge))
}

#[cfg(target_os = "macos")]
fn default_remote_dock_pixels(cx: &App) -> Result<(u32, Vec<u8>), String> {
    static DOCK_FALLBACK: OnceLock<Vec<u8>> = OnceLock::new();
    if let Some(pixels) = DOCK_FALLBACK.get() {
        return Ok((64, pixels.clone()));
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
    let raw = image::imageops::resize(&rgba, 64, 64, image::imageops::FilterType::Triangle).into_raw();
    Ok((64, DOCK_FALLBACK.get_or_init(|| raw).clone()))
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

    const MAX_TITLE_BYTES: usize = 512;
    const MAX_ICON_EDGE: u32 = 256;

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

        pub fn send(&mut self, title: &str, icon: Option<(u32, &[u8])>) -> Result<(), String> {
            let stdin = self.stdin.as_mut().ok_or("Dock proxy closed")?;
            let frame = encode_frame(title, icon)?;
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

    fn icon_bytes(edge: u32) -> Option<usize> {
        (64..=MAX_ICON_EDGE).contains(&edge).then(|| edge as usize * edge as usize * 4)
    }

    pub(super) fn encode_frame(title: &str, icon: Option<(u32, &[u8])>) -> Result<Vec<u8>, String> {
        let title_bytes = title.as_bytes();
        let bounded_title = &title_bytes[..title.floor_char_boundary(MAX_TITLE_BYTES.min(title_bytes.len()))];
        let (edge, pixels) = match icon {
            Some((edge, bytes)) if icon_bytes(edge) == Some(bytes.len()) => (edge, bytes),
            Some(_) => return Err("Invalid Dock icon dimensions".into()),
            None => (0, &[][..]),
        };
        let mut out = Vec::with_capacity(8 + bounded_title.len() + pixels.len());
        out.extend_from_slice(&(bounded_title.len() as u32).to_be_bytes());
        out.extend_from_slice(bounded_title);
        out.extend_from_slice(&edge.to_be_bytes());
        out.extend_from_slice(pixels);
        Ok(out)
    }

    pub(super) fn decode_frame(mut reader: impl Read) -> std::io::Result<(String, Option<(u32, Vec<u8>)>)> {
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
        let edge = u32::from_be_bytes(header);
        let pixels = if edge == 0 {
            None
        } else if let Some(bytes) = icon_bytes(edge) {
            let mut buf = vec![0u8; bytes];
            reader.read_exact(&mut buf)?;
            Some((edge, buf))
        } else {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "Invalid Dock icon dimensions"));
        };
        Ok((title, pixels))
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

    unsafe fn update_presentation(title: &str) {
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
        }
    }

    unsafe fn set_dock_icon_rgba(edge: u32, rgba: &[u8]) {
        if icon_bytes(edge) != Some(rgba.len()) {
            return;
        }
        unsafe {
            let rep: id = msg_send![class!(NSBitmapImageRep), alloc];
            let rep: id = msg_send![
                rep,
                initWithBitmapDataPlanes: std::ptr::null_mut::<*mut u8>()
                pixelsWide: edge as isize
                pixelsHigh: edge as isize
                bitsPerSample: 8isize
                samplesPerPixel: 4isize
                hasAlpha: YES
                isPlanar: NO
                colorSpaceName: ns_string("NSCalibratedRGBColorSpace")
                bitmapFormat: 0usize
                bytesPerRow: edge as isize * 4
                bitsPerPixel: 32isize
            ];
            if rep == nil {
                return;
            }
            let data: *mut u8 = msg_send![rep, bitmapData];
            if !data.is_null() {
                std::ptr::copy_nonoverlapping(rgba.as_ptr(), data, rgba.len());
                let _: () = msg_send![rep, setSize: NSSize::new(64.0, 64.0)];
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
                while let Ok((title, pixels)) = decode_frame(&mut stdin) {
                    dispatch::Queue::main().exec_async(move || unsafe {
                        update_presentation(&title);
                        if let Some((edge, rgba)) = pixels {
                            set_dock_icon_rgba(edge, &rgba);
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
        let bitmap = IconBitmap {
            id: [2; 32], width: 256, height: 256,
            rgba: [240, 80, 20, 255].repeat(256 * 256).into(), png: vec![].into(),
        };
        let badge = image::RgbaImage::from_pixel(32, 32, image::Rgba([10, 20, 30, 128]));
        let (edge, pixels) = remote_application_pixels(&bitmap, &badge).unwrap();
        assert_eq!(edge, 256);
        assert_eq!(&pixels[..4], &[240, 80, 20, 255]);
        assert_ne!(&pixels[(200 * 256 + 200) * 4..(200 * 256 + 200) * 4 + 4], &[240, 80, 20, 255]);
        assert!(pixels.chunks_exact(4).all(|pixel| pixel[..3].iter().all(|channel| *channel <= pixel[3])));
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn macos_dock_frame_round_trips_title_and_optional_icon() {
        let pixels = vec![42u8; 256 * 256 * 4];
        let encoded = macos_dock::encode_frame("Safari · MacStudio", Some((256, &pixels))).unwrap();
        let (title, decoded_pixels) = macos_dock::decode_frame(&encoded[..]).unwrap();
        assert_eq!(title, "Safari · MacStudio");
        assert_eq!(decoded_pixels, Some((256, pixels)));

        let title_only = macos_dock::encode_frame("Terminal · Mini", None).unwrap();
        let (title2, decoded_none) = macos_dock::decode_frame(&title_only[..]).unwrap();
        assert_eq!(title2, "Terminal · Mini");
        assert!(decoded_none.is_none());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn dock_title_preserves_utf8_at_the_length_boundary_and_rejects_bad_pixels() {
        let title = format!("{}远程📱", "a".repeat(511));
        let encoded = macos_dock::encode_frame(&title, None).unwrap();
        let (decoded, pixels) = macos_dock::decode_frame(&encoded[..]).unwrap();
        assert_eq!(decoded, "a".repeat(511));
        assert!(pixels.is_none());
        assert!(macos_dock::encode_frame("title", Some((64, &[0; 4]))).is_err());
        let mut invalid = 513u32.to_be_bytes().to_vec();
        invalid.extend_from_slice(&[0; 513]);
        assert!(macos_dock::decode_frame(&invalid[..]).is_err());
    }

}
