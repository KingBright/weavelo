use super::*;

#[test]
fn first_selection_needs_no_viewer_and_late_results_cannot_replace_current_intent() {
    let mut state = State::default();
    let first = state.reduce(Action::Select(ShareKind::Window)).unwrap();
    assert!(matches!(
        first.call,
        SharingCall::Select {
            kind: ShareKind::Window,
            remember: true
        }
    ));
    assert!(state.reduce(Action::Select(ShareKind::Display)).is_none());
    state.completed(
        first.receipt,
        Ok(SharingSnapshot {
            selecting: Some(9),
            ..Default::default()
        }),
    );
    let cancel = state.reduce(Action::Cancel).unwrap();
    assert!(matches!(cancel.call, SharingCall::Cancel { attempt: 9 }));
    assert!(!state.completed(first.receipt, Err("late failure".into())));
    assert!(state.completed(cancel.receipt, Ok(SharingSnapshot::default())));
    assert!(!state.pending());
}
#[test]
fn failed_owner_check_removes_stale_stop_actions_without_replaying_selection() {
    let mut state = State::default();
    let source = CaptureSource::Window(7);
    let receipt = state.reduce(Action::Refresh).unwrap().receipt;
    state.completed(
        receipt,
        Ok(SharingSnapshot {
            sources: vec![crate::sharing_permissions::SharedSource {
                source,
                label: "Window 1".into(),
                kind: ShareKind::Window,
                supports_input: false,
                remembered: true,
            }],
            ..Default::default()
        }),
    );
    let stop = state.reduce(Action::Stop(source)).unwrap();
    state.completed(stop.receipt, Err("Owner changed".into()));
    assert!(state.reduce(Action::Stop(source)).is_none());
    assert!(state.snapshot.sources.is_empty());
}
#[test]
fn remembering_is_explicit_and_old_status_cannot_overwrite_a_command() {
    let mut state = State::default();
    state.reduce(Action::Remember(false));
    let select = state.reduce(Action::Select(ShareKind::Display)).unwrap();
    assert!(matches!(
        select.call,
        SharingCall::Select {
            remember: false,
            ..
        }
    ));
    assert!(!state.polled(select.receipt, Ok(SharingSnapshot::default())));
    let mut other = State::default();
    let refresh = other.reduce(Action::Refresh).unwrap();
    assert!(other.completed(refresh.receipt, Ok(SharingSnapshot::default())));
    assert!(state.pending() && !other.pending());
}

use gpui::{Context, FocusHandle, Focusable, Modifiers, Render, TestAppContext, WindowAppearance};
use std::cell::RefCell;
struct Fixture {
    state: State,
    root: FocusHandle,
    output: Rc<RefCell<Vec<Action>>>,
}
impl Focusable for Fixture {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.root.clone()
    }
}
impl Render for Fixture {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let output = self.output.clone();
        Controls::new(self.state.clone(), move |action, _| {
            output.borrow_mut().push(action)
        })
    }
}
#[gpui::test]
fn source_buttons_open_system_selection_without_a_viewer_row(cx: &mut TestAppContext) {
    cx.update(|cx| {
        crate::product_components::component::init(cx).unwrap();
        crate::product_components::theme::install(
            WindowAppearance::Dark,
            crate::design_system::weavelo_themes(),
            cx,
        );
    });
    let output = Rc::new(RefCell::new(Vec::new()));
    let (view, cx) = cx.add_window_view({
        let output = output.clone();
        move |_, cx| Fixture {
            state: State::default(),
            root: cx.focus_handle(),
            output,
        }
    });
    cx.run_until_parked();
    let center = cx.debug_bounds("share-control").unwrap().center();
    cx.simulate_click(center, Modifiers::none());
    assert_eq!(
        *output.borrow(),
        [Action::Select(ShareKind::ControlledDisplay)]
    );
    output.borrow_mut().clear();
    view.update(cx, |view, cx| {
        view.state.snapshot.selecting = Some(3);
        cx.notify();
    });
    cx.run_until_parked();
    let center = cx.debug_bounds("share-window").unwrap().center();
    cx.simulate_click(center, Modifiers::none());
    assert!(output.borrow().is_empty());
    let center = cx.debug_bounds("share-cancel").unwrap().center();
    cx.simulate_click(center, Modifiers::none());
    assert_eq!(*output.borrow(), [Action::Cancel]);
}
