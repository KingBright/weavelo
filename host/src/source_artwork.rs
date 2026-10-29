//! Artwork is enumerated with the source directory, never on a video/input tick.
use protocol::{
    interaction::{ApplicationIcon, SourceIconBinding},
    session::{CaptureSource, CaptureSourceInfo, SessionCommand},
};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug, Default)]
pub(crate) struct Directory {
    pub sources: Vec<CaptureSourceInfo>,
    pub bindings: Vec<SourceIconBinding>,
    pub icons: Vec<ApplicationIcon>,
}
impl Directory {
    pub fn new(mut sources: Vec<CaptureSourceInfo>, known: &[[u8; 32]]) -> Self {
        sources.truncate(protocol::interaction::MAX_APPLICATION_ICONS);
        let mut directory = Self {
            sources,
            ..Self::default()
        };
        let known: HashSet<_> = known.iter().copied().collect();
        let mut apps = HashMap::<i32, Option<ApplicationIcon>>::new();
        let mut included = HashSet::new();
        for source in &directory.sources {
            if !matches!(source.source, CaptureSource::Window(_)) {
                continue;
            }
            let Some(pid) = source.process_id.filter(|pid| *pid > 0) else {
                continue;
            };
            let Some(icon) = apps.entry(pid).or_insert_with(|| native_icon(pid)) else {
                continue;
            };
            directory.bindings.push(SourceIconBinding {
                source: source.source,
                icon_id: icon.id,
            });
            if !known.contains(&icon.id) && included.insert(icon.id) {
                directory.icons.push(icon.clone());
            }
        }
        directory
    }
    pub fn reply(mut self, request_id: u32, sources: Vec<CaptureSourceInfo>) -> SessionCommand {
        let advertised: HashSet<_> = sources.iter().map(|s| s.source).collect();
        self.bindings.retain(|b| advertised.contains(&b.source));
        let referenced: HashSet<_> = self.bindings.iter().map(|b| b.icon_id).collect();
        self.icons.retain(|i| referenced.contains(&i.id));
        SessionCommand::Sources {
            request_id,
            sources,
            source_icons: self.bindings,
            icons: self.icons,
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn native_icon(_pid: i32) -> Option<ApplicationIcon> {
    None
}

#[cfg(target_os = "macos")]
fn native_icon(pid: i32) -> Option<ApplicationIcon> {
    use core_graphics::geometry::{CGPoint, CGRect, CGSize};
    use std::ffi::{c_char, c_void};
    type Obj = *mut c_void;
    #[link(name = "objc")]
    unsafe extern "C" {
        fn objc_getClass(name: *const c_char) -> Obj;
        fn sel_registerName(name: *const c_char) -> Obj;
        fn objc_msgSend();
    }
    #[link(name = "AppKit", kind = "framework")]
    unsafe extern "C" {}
    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGColorSpaceCreateDeviceRGB() -> Obj;
        fn CGColorSpaceRelease(space: Obj);
        fn CGImageGetWidth(image: Obj) -> usize;
        fn CGImageGetHeight(image: Obj) -> usize;
        fn CGBitmapContextCreate(
            data: *mut c_void,
            width: usize,
            height: usize,
            bits: usize,
            stride: usize,
            space: Obj,
            info: u32,
        ) -> Obj;
        fn CGContextDrawImage(context: Obj, rect: CGRect, image: Obj);
        fn CGContextRelease(context: Obj);
    }
    // All borrowed AppKit objects are consumed before this autorelease pool ends.
    unsafe fn send(object: Obj, selector: &std::ffi::CStr) -> Obj {
        let call: unsafe extern "C" fn(Obj, Obj) -> Obj =
            unsafe { std::mem::transmute(objc_msgSend as *const ()) };
        unsafe { call(object, sel_registerName(selector.as_ptr())) }
    }
    unsafe {
        let pool = send(
            send(objc_getClass(c"NSAutoreleasePool".as_ptr()), c"alloc"),
            c"init",
        );
        struct Pool(Obj);
        impl Drop for Pool {
            fn drop(&mut self) {
                unsafe {
                    send(self.0, c"drain");
                }
            }
        }
        let _pool = Pool(pool);
        let call: unsafe extern "C" fn(Obj, Obj, i32) -> Obj =
            std::mem::transmute(objc_msgSend as *const ());
        let app = call(
            objc_getClass(c"NSRunningApplication".as_ptr()),
            sel_registerName(c"runningApplicationWithProcessIdentifier:".as_ptr()),
            pid,
        );
        if app.is_null() {
            return None;
        }
        let image = send(app, c"icon");
        if image.is_null() {
            return None;
        }
        let get_image: unsafe extern "C" fn(Obj, Obj, *mut CGRect, Obj, Obj) -> Obj =
            std::mem::transmute(objc_msgSend as *const ());
        let cg = get_image(
            image,
            sel_registerName(c"CGImageForProposedRect:context:hints:".as_ptr()),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        );
        if cg.is_null() {
            return None;
        }
        let (w, h) = (CGImageGetWidth(cg), CGImageGetHeight(cg));
        if w == 0 || h == 0 {
            return None;
        }
        let scale = 64.0 / w.max(h) as f64;
        let (width, height) = (
            (w as f64 * scale).round().max(1.0) as usize,
            (h as f64 * scale).round().max(1.0) as usize,
        );
        let mut pixels = vec![0u8; width * height * 4];
        let space = CGColorSpaceCreateDeviceRGB();
        if space.is_null() {
            return None;
        }
        // Big endian byte order + premultiplied-last means RGBA bytes on all Macs.
        let context = CGBitmapContextCreate(
            pixels.as_mut_ptr().cast(),
            width,
            height,
            8,
            width * 4,
            space,
            0x4001,
        );
        CGColorSpaceRelease(space);
        if context.is_null() {
            return None;
        }
        CGContextDrawImage(
            context,
            CGRect::new(
                &CGPoint::new(0.0, 0.0),
                &CGSize::new(width as f64, height as f64),
            ),
            cg,
        );
        CGContextRelease(context);
        for pixel in pixels.chunks_exact_mut(4) {
            let alpha = u32::from(pixel[3]);
            if alpha > 0 {
                for channel in &mut pixel[..3] {
                    *channel = ((u32::from(*channel) * 255 + alpha / 2) / alpha).min(255) as u8;
                }
            }
        }
        remote_core::application_icons::encode_rgba(width as u32, height as u32, &pixels).ok()
    }
}
