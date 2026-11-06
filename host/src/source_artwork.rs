//! Artwork is enumerated with the source directory, never on a video/input tick.
use protocol::{
    interaction::{ApplicationIcon, SourceIconBinding},
    session::{CaptureSource, CaptureSourceInfo, SessionCommand},
};
use std::collections::{HashMap, HashSet};

#[derive(Default)]
pub(crate) struct Requests {
    // One latest query per live subscription; UDP retries reuse its result.
    queries: HashMap<u32, Query>,
}
struct Query {
    request_id: u32,
    source: CaptureSource,
    task: Option<tokio::task::JoinHandle<Result<protocol::appearance::SourceAppearance, String>>>,
    result: Option<Result<protocol::appearance::SourceAppearance, String>>,
    dirty: bool,
}
impl Requests {
    pub fn needs_poll(&self)->bool {self.queries.values().any(|query|query.dirty || query.task.is_some())}
    pub fn request(&mut self, request_id: u32, id: u32, source: CaptureSource) {
        if let Some(query) = self.queries.get_mut(&id) {
            if query.request_id == request_id && query.source == source { query.dirty=true; return; }
            if let Some(task)=query.task.take(){task.abort();}
        }
        let task=tokio::spawn(async move {
            // Share the existing native enumeration permit, including cancelled work.
            let directory=crate::source_catalog::enumerate_native(move|| {
                let icon=if matches!(source,CaptureSource::Window(_)) {
                    let sources=crate::capture_sources::list().map_err(|e|e.to_string())?;
                    let info=sources.into_iter().find(|info|info.source==source)
                        .ok_or("The selected source is no longer available")?;
                    icon_for_source(source,info.process_id,256)
                } else {None};
                Ok(Directory {sources:vec![],bindings:vec![],icons:icon.into_iter().collect()})
            }).await?;
            let device=tokio::task::spawn_blocking(remote_core::device_artwork::desktop_badge)
                .await.map_err(|e|e.to_string())??;
            Ok(protocol::appearance::SourceAppearance {application:directory.icons.into_iter().next(),device})
        });
        self.queries.insert(id,Query {request_id,source,task:Some(task),result:None,dirty:true});
    }
    pub async fn ready(&mut self, live: &HashMap<u32,CaptureSource>) -> Vec<SessionCommand> {
        self.queries.retain(|id,query|live.get(id)==Some(&query.source));
        let mut replies=Vec::new();
        for (id,query) in &mut self.queries {
            if query.task.as_ref().is_some_and(|task|task.is_finished()) {
                query.result=Some(query.task.take().unwrap().await.unwrap_or_else(|e|Err(e.to_string())));
            }
            if query.dirty { if let Some(result)=&query.result {
                replies.push(SessionCommand::SourceAppearance {request_id:query.request_id,id:*id,source:query.source,appearance:Box::new(result.clone())});
                query.dirty=false;
            }}
        }
        replies
    }
}
impl Drop for Query {fn drop(&mut self){if let Some(task)=self.task.take(){task.abort();}}}

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
        #[cfg(not(target_os = "linux"))]
        let mut apps = HashMap::<i32, Option<ApplicationIcon>>::new();
        let mut included = HashSet::new();
        for source in &directory.sources {
            if !matches!(source.source, CaptureSource::Window(_)) {
                continue;
            }
            #[cfg(target_os = "linux")]
            let icon = icon_for_source(source.source, source.process_id, 64);
            #[cfg(not(target_os = "linux"))]
            let icon = {
                let Some(pid) = source.process_id.filter(|pid| *pid > 0) else {
                    continue;
                };
                let Some(icon) = apps.entry(pid).or_insert_with(|| native_icon(pid, 64)) else {
                    continue;
                };
                Some(icon.clone())
            };
            let Some(icon) = icon else { continue; };
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
    pub fn reply(mut self, request_id: u32, mut sources: Vec<CaptureSourceInfo>) -> SessionCommand {
        remote_core::source_picker::sort(&mut sources);
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

fn icon_for_source(source: CaptureSource, pid: Option<i32>, edge: u32) -> Option<ApplicationIcon> {
    #[cfg(target_os = "linux")]
    {
        let _ = pid;
        crate::linux_kwin::icon(source, edge)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = source;
        pid.filter(|pid| *pid > 0).and_then(|pid| native_icon(pid, edge))
    }
}

#[cfg(not(any(target_os = "macos",target_os="windows")))]
fn native_icon(_pid: i32, _edge: u32) -> Option<ApplicationIcon> {
    None
}

#[cfg(target_os="windows")]
fn native_icon(pid:i32, edge:u32)->Option<ApplicationIcon> {
    use windows_sys::Win32::{Graphics::Gdi::*,UI::{Shell::*,WindowsAndMessaging::{DrawIconEx,DestroyIcon,DI_NORMAL}}};
    use windows_sys::Win32::System::Com::{CoInitializeEx,CoUninitialize,COINIT_APARTMENTTHREADED,COINIT_DISABLE_OLE1DDE};
    // Shell icon handlers require COM on this exact worker thread. Balance both
    // S_OK and S_FALSE without changing an existing apartment's concurrency model.
    if unsafe{CoInitializeEx(std::ptr::null(),(COINIT_APARTMENTTHREADED|COINIT_DISABLE_OLE1DDE) as u32)}<0{return None;}
    struct Apartment;
    impl Drop for Apartment {fn drop(&mut self){unsafe{CoUninitialize();}}}
    let _apartment=Apartment;
    let path=crate::windows_sources::application_path(pid.try_into().ok()?)?;
    let name=path.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    let mut info:SHFILEINFOW=unsafe{std::mem::zeroed()};
    let flags=if edge>64 {SHGFI_SYSICONINDEX} else {SHGFI_ICON|SHGFI_LARGEICON};
    if unsafe{SHGetFileInfoW(name.as_ptr(),0,&mut info,std::mem::size_of_val(&info) as u32,flags)}==0{return None;}
    let icon=if edge>64 {
        use windows::Win32::UI::{Controls::{IImageList,ILD_TRANSPARENT},Shell::{SHGetImageList,SHIL_JUMBO}};
        let list:IImageList=unsafe{SHGetImageList(SHIL_JUMBO as i32)}.ok()?;
        unsafe{list.GetIcon(info.iIcon,ILD_TRANSPARENT.0)}.ok()?.0
    } else {info.hIcon};
    if icon.is_null(){return None;}
    let result=(||unsafe{
        let dc=CreateCompatibleDC(std::ptr::null_mut());if dc.is_null(){return None;}
        let mut bitmap_info:BITMAPINFO=std::mem::zeroed();
        bitmap_info.bmiHeader.biSize=std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bitmap_info.bmiHeader.biWidth=edge as i32;bitmap_info.bmiHeader.biHeight=-(edge as i32);
        bitmap_info.bmiHeader.biPlanes=1;bitmap_info.bmiHeader.biBitCount=32;bitmap_info.bmiHeader.biCompression=BI_RGB;
        let mut bits=std::ptr::null_mut();
        let bitmap=CreateDIBSection(dc,&bitmap_info,DIB_RGB_COLORS,&mut bits,std::ptr::null_mut(),0);
        if bitmap.is_null()||bits.is_null(){DeleteDC(dc);return None;}
        let previous=SelectObject(dc,bitmap);std::ptr::write_bytes(bits as *mut u8,0,(edge*edge*4) as usize);
        let okay=DrawIconEx(dc,0,0,icon,edge as i32,edge as i32,0,std::ptr::null_mut(),DI_NORMAL)!=0;GdiFlush();
        let mut rgba=okay.then(||std::slice::from_raw_parts(bits as *const u8,(edge*edge*4) as usize).to_vec());
        SelectObject(dc,previous);DeleteObject(bitmap);DeleteDC(dc);
        let rgba=rgba.as_mut()?;
        let alpha_present=rgba.chunks_exact(4).any(|pixel|pixel[3]!=0);
        for pixel in rgba.chunks_exact_mut(4){
            pixel.swap(0,2);
            if !alpha_present{pixel[3]=255;}else if pixel[3]>0{
                for channel in 0..3{pixel[channel]=(u32::from(pixel[channel])*255/u32::from(pixel[3])).min(255) as u8;}
            }
        }
        (if edge == 64 {remote_core::application_icons::encode_rgba(edge,edge,rgba)} else {remote_core::application_icons::encode_artwork(edge,edge,rgba)}).ok()
    })();
    unsafe{DestroyIcon(icon);}result
}

#[cfg(target_os = "macos")]
fn native_icon(pid: i32, edge: u32) -> Option<ApplicationIcon> {
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
        let mut proposed=CGRect::new(&CGPoint::new(0.0,0.0),&CGSize::new(f64::from(edge),f64::from(edge)));
        let cg = get_image(
            image,
            sel_registerName(c"CGImageForProposedRect:context:hints:".as_ptr()),
            &mut proposed,
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
        let scale = f64::from(edge) / w.max(h) as f64;
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
        (if edge == 64 {remote_core::application_icons::encode_rgba(width as u32,height as u32,&pixels)} else {remote_core::application_icons::encode_artwork(width as u32,height as u32,&pixels)}).ok()
    }
}
