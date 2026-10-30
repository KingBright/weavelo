use super::{Action, Error};
use std::{sync::{Arc, atomic::{AtomicUsize, Ordering}}, thread};
use tokio::sync::mpsc::UnboundedSender;
use windows_sys::Win32::{Foundation::*, System::LibraryLoader::GetModuleHandleW,
    UI::{Shell::*, WindowsAndMessaging::*}};

const CALLBACK: u32 = WM_APP + 71;
struct State { actions: UnboundedSender<Action>, window: Arc<AtomicUsize>, icon: AtomicUsize, taskbar_created: u32 }
pub(super) struct Tray { window: Arc<AtomicUsize>, thread: Option<thread::JoinHandle<()>> }

fn wide(text: &str) -> Vec<u16> { text.encode_utf16().chain(Some(0)).collect() }
fn icon() -> HICON {
    let bytes = include_bytes!("../../../assets/brand/weavelo/favicon.ico");
    // The existing brand ICO includes a 32px PNG entry. No generic app icon.
    let row=&bytes[22..38];
    let size=u32::from_le_bytes(row[8..12].try_into().expect("ICO size"));
    let offset=u32::from_le_bytes(row[12..16].try_into().expect("ICO offset")) as usize;
    unsafe { CreateIconFromResourceEx(bytes[offset..].as_ptr(),size,1,0x0003_0000,32,32,0) }
}
unsafe extern "system" fn message(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        let state=GetWindowLongPtrW(window,GWLP_USERDATA) as *const State;
        if !state.is_null() && (*state).taskbar_created!=0 && message==(*state).taskbar_created {
            let image=(*state).icon.load(Ordering::Acquire) as HICON;
            if !image.is_null() {let _=register_icon(window,image);}
            return 0;
        }
        if message==CALLBACK && !state.is_null() {
            let event=lparam as u32 & 0xffff;
            if event==NIN_SELECT || event==(NIN_SELECT|NINF_KEY) {
                let _=(*state).actions.send(Action::Open);
            } else if event==WM_CONTEXTMENU {
                let menu=CreatePopupMenu();
                let open=wide("Open Weavelo");let quit=wide("Quit Weavelo");
                AppendMenuW(menu,MF_STRING,1,open.as_ptr());AppendMenuW(menu,MF_STRING,2,quit.as_ptr());
                let mut point=POINT{x:0,y:0};GetCursorPos(&mut point);
                SetForegroundWindow(window);
                let chosen=TrackPopupMenu(menu,TPM_RETURNCMD|TPM_NONOTIFY|TPM_RIGHTBUTTON,
                    point.x,point.y,0,window,std::ptr::null());
                DestroyMenu(menu);PostMessageW(window,WM_NULL,0,0);
                match chosen {1=>{let _=(*state).actions.send(Action::Open);},2=>{let _=(*state).actions.send(Action::Quit);},_=>{}}
            }
            return 0;
        }
        match message {
            WM_CLOSE => {
                let mut item:NOTIFYICONDATAW=std::mem::zeroed();item.cbSize=std::mem::size_of_val(&item) as u32;
                item.hWnd=window;item.uID=1;Shell_NotifyIconW(NIM_DELETE,&item);
                DestroyWindow(window);0
            }
            WM_DESTROY => {
                if !state.is_null() {(*state).window.store(0,Ordering::Release);}
                PostQuitMessage(0);0
            }
            WM_NCDESTROY => {SetWindowLongPtrW(window,GWLP_USERDATA,0);DefWindowProcW(window,message,wparam,lparam)}
            _ => DefWindowProcW(window,message,wparam,lparam),
        }
    }
}

unsafe fn register_icon(window: HWND, image: HICON) -> bool {
    unsafe {
        let mut item:NOTIFYICONDATAW=std::mem::zeroed();item.cbSize=std::mem::size_of_val(&item) as u32;
        item.hWnd=window;item.uID=1;item.uFlags=NIF_MESSAGE|NIF_ICON|NIF_TIP|NIF_SHOWTIP;
        item.uCallbackMessage=CALLBACK;item.hIcon=image;
        for (to,from) in item.szTip.iter_mut().zip(wide("Weavelo")) {*to=from;}
        if Shell_NotifyIconW(NIM_ADD,&item)==0 {return false;}
        item.Anonymous.uVersion=NOTIFYICON_VERSION_4;
        if Shell_NotifyIconW(NIM_SETVERSION,&item)==0 {Shell_NotifyIconW(NIM_DELETE,&item);return false;}
        true
    }
}

impl Tray {
    pub fn start(actions: UnboundedSender<Action>) -> Result<Self, Error> {
        let window=Arc::new(AtomicUsize::new(0));let stored=window.clone();
        let (started,ready)=std::sync::mpsc::sync_channel(1);
        let thread=thread::Builder::new().name("weavelo-tray".into()).spawn(move || unsafe {
            let module=GetModuleHandleW(std::ptr::null());let name=wide("WeaveloBackgroundTray");
            let mut class:WNDCLASSW=std::mem::zeroed();class.lpfnWndProc=Some(message);
            class.hInstance=module;class.lpszClassName=name.as_ptr();
            if RegisterClassW(&class)==0 {let _=started.send(Err(std::io::Error::last_os_error().to_string()));return;}
            let handle=CreateWindowExW(0,name.as_ptr(),name.as_ptr(),0,0,0,0,0,
                std::ptr::null_mut(),std::ptr::null_mut(),module,std::ptr::null());
            if handle.is_null() {
                let _=started.send(Err(std::io::Error::last_os_error().to_string()));
                UnregisterClassW(name.as_ptr(),module);return;
            }
            let state=Box::new(State{actions,window:stored.clone(),icon:AtomicUsize::new(0),
                taskbar_created:RegisterWindowMessageW(wide("TaskbarCreated").as_ptr())});
            SetWindowLongPtrW(handle,GWLP_USERDATA,state.as_ref() as *const State as isize);
            stored.store(handle as usize,Ordering::Release);
            let image=icon();
            state.icon.store(image as usize,Ordering::Release);
            if image.is_null() || !register_icon(handle,image) {
                let _=started.send(Err(format!("Shell tray registration failed: {}",std::io::Error::last_os_error())));
                DestroyWindow(handle);if !image.is_null(){DestroyIcon(image);}
                UnregisterClassW(name.as_ptr(),module);return;
            }
            let _=started.send(Ok(()));
            let mut event:MSG=std::mem::zeroed();
            while GetMessageW(&mut event,std::ptr::null_mut(),0,0)>0 {TranslateMessage(&event);DispatchMessageW(&event);}
            // WM_CLOSE removed the item and destroyed this exact owned window.
            DestroyIcon(image);UnregisterClassW(name.as_ptr(),module);drop(state);
        })?;
        ready.recv().map_err(|_| "Windows tray startup was interrupted")?
            .map_err(|error|format!("Windows tray unavailable: {error}"))?;
        Ok(Self{window,thread:Some(thread)})
    }
}
impl Drop for Tray {
    fn drop(&mut self) {
        let window=self.window.load(Ordering::Acquire);
        if window!=0 {unsafe {PostMessageW(window as HWND,WM_CLOSE,0,0);}}
        if let Some(thread)=self.thread.take() {let _=thread.join();}
    }
}
