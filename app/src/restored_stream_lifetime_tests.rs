//! Real GPUI window events and application owners. No capture or OS input.
use super::{
    FrontendExitGroup, RestoredDashboard, StreamPanel, UnifiedRuntimeHandle, component,
    open_restored_window, product_window_appearance, weavelo_themes,
};
use gpui::{Modifiers, TestAppContext, VisualTestContext};
use protocol::session::CaptureSource;
use std::{sync::Arc, time::Duration};

#[gpui::test]
fn close_and_end_stream_remove_only_their_windows_and_preserve_other_work(cx: &mut TestAppContext) {
    let executor = tokio::runtime::Runtime::new().unwrap();
    let _entered = executor.enter();
    let backend = executor
        .block_on(crate::UnifiedServiceOwner::start(
            crate::UnifiedServiceOwnerConfig::default(),
        ))
        .unwrap();
    let runtime = Arc::new(UnifiedRuntimeHandle::from_workspace_owner(Arc::new(
        backend,
    )));
    let group = Arc::new(FrontendExitGroup::default());
    let first = cx.update(|cx| {
        component::init(cx).unwrap();
        crate::product_components::theme::install(
            product_window_appearance(),
            weavelo_themes(),
            cx,
        );
        open_restored_window(
            runtime.clone(),
            None,
            Default::default(),
            None,
            None,
            group.clone(),
            cx,
        )
        .unwrap()
    });
    let second = cx.update(|cx| {
        open_restored_window(
            runtime,
            None,
            Default::default(),
            None,
            None,
            group.clone(),
            cx,
        )
        .unwrap()
    });
    let first_view = first.root(cx).unwrap();
    let second_view = second.root(cx).unwrap();
    let first_owner = first_view.read_with(cx, |this, _| this.owner.clone());
    let second_owner = second_view.read_with(cx, |this, _| this.owner.clone());
    let files = first_view.read_with(cx, |this, _| this.device_files("peer-b"));
    let first_watch = first_owner
        .begin_directed_receive(
            "receive-a",
            "peer-a",
            CaptureSource::Window(7),
            Duration::from_secs(30),
        )
        .unwrap();
    let second_watch = second_owner
        .begin_directed_receive(
            "receive-b",
            "peer-b",
            CaptureSource::Display(4),
            Duration::from_secs(30),
        )
        .unwrap();
    first_view.update(cx, |this, cx| {
        this.stream_device = Some("peer-a".into());
        this.stream_source = Some(CaptureSource::Window(7));
        this.open_popout_pip_window(cx);
    });
    second_view.update(cx, |this, _| {
        this.stream_device = Some("peer-b".into());
        this.stream_source = Some(CaptureSource::Display(4));
    });
    cx.run_until_parked();
    let pip = first_view.read_with(cx, |this, _| this.pip_windows[0]);
    {
        let mut visual = VisualTestContext::from_window(first.into(), cx);
        assert!(visual.simulate_close());
    }
    first
        .update(cx, |_, window, _| window.remove_window())
        .unwrap();
    cx.run_until_parked();
    assert!(first.update(cx, |_, _, _| ()).is_err());
    assert!(
        pip.update(cx, |_, _, _| ()).is_err(),
        "the ended stream's PiP also closes"
    );
    assert!(!first_owner.directed_receive_current("receive-a"));
    assert!(second_owner.directed_receive_current("receive-b"));
    assert!(second.update(cx, |_, _, _| ()).is_ok());
    second_view.update(cx, |this, cx| {
        this.drawer_open = true;
        this.stream_panel = StreamPanel::More;
        cx.notify();
    });
    {
        let mut visual = VisualTestContext::from_window(second.into(), cx);
        visual.run_until_parked();
        let end = visual
            .debug_bounds("end-current-stream")
            .expect("current stream end button");
        visual.simulate_click(end.center(), Modifiers::none());
    }
    cx.run_until_parked();
    assert!(
        second.update(cx, |_, _, _| ()).is_err(),
        "End stream exits instead of leaving an idle window"
    );
    assert!(!second_owner.directed_receive_current("receive-b"));
    assert!(Arc::ptr_eq(
        &group
            .file_contexts
            .lock()
            .unwrap()
            .get("peer-b")
            .unwrap()
            .owner,
        &files.owner
    ));
    assert!(!group.stopping.load(std::sync::atomic::Ordering::Acquire));
    first_watch.abort();
    second_watch.abort();
}
