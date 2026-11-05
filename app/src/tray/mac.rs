use super::{
    Action, Error,
    menu::{Entry, Menu, Source},
};
use cocoa::{
    appkit::{NSApp, NSApplication, NSApplicationActivationPolicy, NSEventType},
    base::{BOOL, NO, YES, id, nil},
    foundation::{NSAutoreleasePool, NSPoint, NSSize, NSString},
};
use objc::{
    class,
    declare::ClassDecl,
    msg_send,
    runtime::{Object, Sel},
    sel, sel_impl,
};
use std::future::Future;
use std::sync::Mutex;
use tokio::sync::mpsc::UnboundedSender;

fn string(value: &str) -> id {
    unsafe {
        let string = NSString::alloc(nil).init_str(value);
        let _: id = msg_send![string, autorelease];
        string
    }
}
extern "C" fn open(this: &Object, _: Sel, _: id) {
    send(this, Action::Open);
}
extern "C" fn quit(this: &Object, _: Sel, _: id) {
    send(this, Action::Quit);
}
extern "C" fn reopen(this: &Object, _: Sel, app: id, _: BOOL) -> BOOL {
    // Launch Services can promote the tray owner when opening the bundle.
    // Return it to the menu bar before forwarding the request to the frontend.
    unsafe {
        app.setActivationPolicy_(
            NSApplicationActivationPolicy::NSApplicationActivationPolicyAccessory,
        );
    }
    send(this, Action::Open);
    NO
}
extern "C" fn terminate(this: &Object, _: Sel, _: id) -> usize {
    send(this, Action::Quit);
    0
}
struct State {
    actions: UnboundedSender<Action>,
    source: Source,
    menu: Mutex<Menu>,
}
fn state(this: &Object) -> Option<&State> {
    unsafe {
        (*this.get_ivar::<*const std::ffi::c_void>("actions"))
            .cast::<State>()
            .as_ref()
    }
}
fn send(this: &Object, action: Action) {
    if let Some(state) = state(this) {
        let _ = state.actions.send(action);
    }
}
extern "C" fn perform_menu(this: &Object, _: Sel, item: id) {
    if let Some(state) = state(this) {
        let tag: isize = unsafe { msg_send![item, tag] };
        if let Some(action) = state.menu.lock().unwrap().action(tag as i32) {
            let _ = state.actions.send(action);
        }
    }
}
unsafe fn populate(menu: id, entries: &[Entry], target: id) {
    unsafe {
        let _: () = msg_send![menu,setAutoenablesItems: false];
        for entry in entries {
            let item: id = msg_send![class!(NSMenuItem), alloc];
            let item: id = msg_send![item,initWithTitle:string(&entry.label) action:sel!(performWeaveloMenu:) keyEquivalent:string("")];
            let _: () = msg_send![item,setTarget:target];
            let _: () = msg_send![item,setTag:entry.id as isize];
            let _: () = msg_send![item,setEnabled:entry.enabled];
            if !entry.children.is_empty() {
                let submenu: id = msg_send![class!(NSMenu), new];
                populate(submenu, &entry.children, target);
                let _: () = msg_send![item,setSubmenu:submenu];
                let _: () = msg_send![submenu, release];
            }
            let _: () = msg_send![menu,addItem:item];
            let _: () = msg_send![item, release];
        }
    }
}
extern "C" fn update_menu(this: &Object, _: Sel, menu: id) {
    let Some(state) = state(this) else {
        return;
    };
    state.source.refresh();
    let mut tree = state.menu.lock().unwrap();
    tree.rebuild(&state.source.current());
    unsafe {
        let _: () = msg_send![menu, removeAllItems];
        populate(menu, &tree.roots, this as *const Object as id);
    }
}

/// AppKit owns only a status item and menu, with no renderer or video buffers.
/// Its event loop stays on the process's main thread; service work stays on Tokio.
pub(super) async fn run(
    sender: UnboundedSender<Action>,
    source: Source,
    future: impl Future<Output = Result<(), Error>> + Send + 'static,
) -> Result<(), Error> {
    let runtime = tokio::runtime::Handle::current();
    let job = crate::desktop::foreground_runtime::run(move || unsafe {
        let main: BOOL = msg_send![class!(NSThread), isMainThread];
        if main != YES {
            return Err::<_, Error>("The macOS tray requires the main thread".into());
        }
        let pool = NSAutoreleasePool::new(nil);
        let app = NSApplication::sharedApplication(nil);
        app.setActivationPolicy_(
            NSApplicationActivationPolicy::NSApplicationActivationPolicyAccessory,
        );
        let mut delegate = ClassDecl::new("WeaveloTrayActions", class!(NSObject))
            .ok_or("Weavelo tray actions were already registered")?;
        delegate.add_ivar::<*const std::ffi::c_void>("actions");
        delegate.add_method(sel!(openWeavelo:), open as extern "C" fn(&Object, Sel, id));
        delegate.add_method(sel!(quitWeavelo:), quit as extern "C" fn(&Object, Sel, id));
        delegate.add_method(
            sel!(applicationShouldHandleReopen:hasVisibleWindows:),
            reopen as extern "C" fn(&Object, Sel, id, BOOL) -> BOOL,
        );
        delegate.add_method(
            sel!(applicationShouldTerminate:),
            terminate as extern "C" fn(&Object, Sel, id) -> usize,
        );
        delegate.add_method(
            sel!(performWeaveloMenu:),
            perform_menu as extern "C" fn(&Object, Sel, id),
        );
        delegate.add_method(
            sel!(menuNeedsUpdate:),
            update_menu as extern "C" fn(&Object, Sel, id),
        );
        let target: id = msg_send![delegate.register(), new];
        let state = Box::new(State {
            actions: sender,
            source,
            menu: Mutex::new(Menu::default()),
        });
        (*target).set_ivar(
            "actions",
            (state.as_ref() as *const State).cast::<std::ffi::c_void>(),
        );
        // Finder/Dock activation can target the running accessory instance.
        // Route it to the same single-instance GUI launcher as the tray menu.
        let _: () = msg_send![app, setDelegate: target];
        let status_bar: id = msg_send![class!(NSStatusBar), systemStatusBar];
        let item: id = msg_send![status_bar, statusItemWithLength: 24.0f64];
        let item: id = msg_send![item, retain];
        let button: id = msg_send![item, button];
        let original: id = msg_send![app, applicationIconImage];
        let image: id = msg_send![original, copy];
        let _: () = msg_send![image, setSize: NSSize::new(18.0, 18.0)];
        let _: () = msg_send![button, setImage: image];
        let _: () = msg_send![image, release];
        let _: () = msg_send![button, setToolTip: string("Weavelo")];
        let menu: id = msg_send![class!(NSMenu), new];
        let _: () = msg_send![menu,setDelegate:target];
        let _: () = msg_send![item, setMenu: menu];
        let menu_address = menu as usize;
        let worker = runtime.spawn(future);
        let job = runtime.spawn(async move {
            let result = worker.await;
            dispatch::Queue::main().exec_async(move || {
                // The menu is retained until this main-loop callback stops run().
                let menu = menu_address as id;
                let _: () = msg_send![menu, cancelTracking];
                let app = NSApp();
                app.stop_(nil);
                let wake: id = msg_send![class!(NSEvent),
                    otherEventWithType:NSEventType::NSApplicationDefined
                    location:NSPoint::new(0.0,0.0) modifierFlags:0u64 timestamp:0.0f64
                    windowNumber:0isize context:nil subtype:0i16 data1:0isize data2:0isize];
                app.postEvent_atStart_(wake, YES);
            });
            result
        });
        app.run();
        let _: () = msg_send![status_bar, removeStatusItem: item];
        let _: () = msg_send![item, release];
        let _: () = msg_send![menu,setDelegate:nil];
        let _: () = msg_send![menu, release];
        let _: () = msg_send![app, setDelegate: nil];
        let _: () = msg_send![target, release];
        drop(state);
        pool.drain();
        Ok(job)
    })??;
    job.await??
}
