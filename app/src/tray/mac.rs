use super::{Action, Error};
use cocoa::{appkit::{NSApp, NSApplication, NSApplicationActivationPolicy, NSEventType},
    base::{id, nil, BOOL, YES}, foundation::{NSAutoreleasePool, NSPoint, NSSize, NSString}};
use objc::{class, msg_send, sel, sel_impl, declare::ClassDecl, runtime::{Object, Sel}};
use std::future::Future;
use tokio::sync::mpsc::UnboundedSender;

fn string(value: &str) -> id {
    unsafe {
        let string = NSString::alloc(nil).init_str(value);
        let _: id = msg_send![string, autorelease];
        string
    }
}
extern "C" fn open(this: &Object, _: Sel, _: id) { send(this, Action::Open); }
extern "C" fn quit(this: &Object, _: Sel, _: id) { send(this, Action::Quit); }
extern "C" fn reopen(this: &Object, _: Sel, _: id, _: BOOL) -> BOOL { send(this, Action::Open);YES }
extern "C" fn terminate(this: &Object, _: Sel, _: id) -> usize { send(this, Action::Quit);0 }
fn send(this: &Object, action: Action) {
    unsafe {
        let sender = (*this.get_ivar::<*const std::ffi::c_void>("actions")).cast::<UnboundedSender<Action>>();
        if let Some(sender) = sender.as_ref() { let _ = sender.send(action); }
    }
}

/// AppKit owns only a status item and menu, with no renderer or video buffers.
/// Its event loop stays on the process's main thread; service work stays on Tokio.
pub(super) async fn run(sender: UnboundedSender<Action>,
    future: impl Future<Output = Result<(), Error>> + Send + 'static) -> Result<(), Error> {
    let runtime = tokio::runtime::Handle::current();
    let job = crate::desktop::foreground_runtime::run(move || unsafe {
        let main: BOOL = msg_send![class!(NSThread), isMainThread];
        if main != YES { return Err::<_, Error>("The macOS tray requires the main thread".into()); }
        let pool = NSAutoreleasePool::new(nil);
        let app = NSApplication::sharedApplication(nil);
        app.setActivationPolicy_(NSApplicationActivationPolicy::NSApplicationActivationPolicyAccessory);
        let mut delegate = ClassDecl::new("WeaveloTrayActions", class!(NSObject))
            .ok_or("Weavelo tray actions were already registered")?;
        delegate.add_ivar::<*const std::ffi::c_void>("actions");
        delegate.add_method(sel!(openWeavelo:), open as extern "C" fn(&Object, Sel, id));
        delegate.add_method(sel!(quitWeavelo:), quit as extern "C" fn(&Object, Sel, id));
        delegate.add_method(sel!(applicationShouldHandleReopen:hasVisibleWindows:),
            reopen as extern "C" fn(&Object, Sel, id, BOOL) -> BOOL);
        delegate.add_method(sel!(applicationShouldTerminate:),
            terminate as extern "C" fn(&Object, Sel, id) -> usize);
        let target: id = msg_send![delegate.register(), new];
        let sender = Box::new(sender);
        (*target).set_ivar("actions", (sender.as_ref() as *const UnboundedSender<Action>).cast::<std::ffi::c_void>());
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
        for (label, action) in [("Open Weavelo", sel!(openWeavelo:)), ("Quit Weavelo", sel!(quitWeavelo:))] {
            let entry: id = msg_send![class!(NSMenuItem), alloc];
            let entry: id = msg_send![entry, initWithTitle:string(label) action:action keyEquivalent:string("")];
            let _: () = msg_send![entry, setTarget: target];
            let _: () = msg_send![menu, addItem: entry];
            let _: () = msg_send![entry, release];
        }
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
        let _: () = msg_send![menu, release];
        let _: () = msg_send![app, setDelegate: nil];
        let _: () = msg_send![target, release];
        drop(sender);
        pool.drain();
        Ok(job)
    })??;
    job.await??
}
