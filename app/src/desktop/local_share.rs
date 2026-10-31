//! Frontend state contains labels and receipts, never an OS grant or descriptor.
pub(crate) use crate::sharing_permissions::ShareKind;
use crate::{
    product_components::theme::ActiveTheme,
    sharing_permissions::{SharingCall, SharingSnapshot},
};
use ely_gpui_component::{
    buttons::{Button, ButtonVariant},
    theme::ControlSize,
};
use gpui::{App, IntoElement, RenderOnce, Window, div, prelude::*, px};
use protocol::session::CaptureSource;
use std::rc::Rc;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    Refresh,
    Select(ShareKind),
    Remember(bool),
    Cancel,
    Stop(CaptureSource),
    StopAll,
}
pub(crate) struct Effect {
    pub receipt: u64,
    pub call: SharingCall,
}
#[derive(Clone)]
pub(crate) struct State {
    serial: u64,
    busy: bool,
    pub snapshot: SharingSnapshot,
    remember: bool,
    error: Option<String>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            serial: 0,
            busy: false,
            snapshot: SharingSnapshot::default(),
            remember: true,
            error: None,
        }
    }
}
impl State {
    pub fn reduce(&mut self, action: Action) -> Option<Effect> {
        let pending = self.pending();
        let call = match action {
            Action::Remember(value) if !pending => {
                self.remember = value;
                return None;
            }
            Action::Refresh if !self.busy => SharingCall::Status,
            Action::Select(kind) if !pending => SharingCall::Select {
                kind,
                remember: self.remember,
            },
            Action::Cancel => SharingCall::Cancel {
                attempt: self.snapshot.selecting?,
            },
            Action::Stop(source)
                if self
                    .snapshot
                    .sources
                    .iter()
                    .any(|item| item.source == source) =>
            {
                SharingCall::Stop { source }
            }
            Action::StopAll => SharingCall::StopAll,
            _ => return None,
        };
        self.serial = self.serial.checked_add(1)?;
        self.busy = true;
        self.error = None;
        Some(Effect {
            receipt: self.serial,
            call,
        })
    }
    pub fn receipt(&self) -> u64 {
        self.serial
    }
    pub fn completed(&mut self, receipt: u64, result: Result<SharingSnapshot, String>) -> bool {
        if receipt != self.serial {
            return false;
        }
        self.busy = false;
        self.apply(result);
        true
    }
    pub fn polled(&mut self, receipt: u64, result: Result<SharingSnapshot, String>) -> bool {
        if receipt != self.serial || self.busy {
            return false;
        }
        self.apply(result);
        true
    }
    fn apply(&mut self, result: Result<SharingSnapshot, String>) {
        match result {
            Ok(snapshot) => {
                self.snapshot = snapshot;
                self.error = None;
            }
            Err(error) => {
                self.snapshot = SharingSnapshot::default();
                self.error = Some(error);
            }
        }
    }
    pub fn pending(&self) -> bool {
        self.busy || self.snapshot.selecting.is_some() || self.snapshot.stopping
    }
}

#[derive(IntoElement)]
pub(crate) struct Controls {
    state: State,
    send: Rc<dyn Fn(Action, &mut App)>,
}
impl Controls {
    pub fn new(state: State, send: impl Fn(Action, &mut App) + 'static) -> Self {
        Self {
            state,
            send: Rc::new(send),
        }
    }
}
impl RenderOnce for Controls {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let pending = self.state.pending();
        let mut panel = div()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .rounded(px(8.0))
            .bg(cx.theme().surface.sunken)
            .text_size(px(11.0))
            .text_color(cx.theme().content.primary)
            .child(div().child("Share this computer"));
        let mut choices = div().flex().flex_wrap().gap_2();
        for (id, label, kind) in [
            ("share-window", "App window", ShareKind::Window),
            ("share-display", "Screen", ShareKind::Display),
            (
                "share-control",
                "Screen + control",
                ShareKind::ControlledDisplay,
            ),
        ] {
            let send = self.send.clone();
            choices = choices.child(
                Button::new(id, label)
                    .debug_selector(move || id.into())
                    .control_size(ControlSize::Sm)
                    .rounded_full()
                    .h(px(28.0))
                    .disabled(pending)
                    .on_click(move |_, _, cx| send(Action::Select(kind), cx)),
            );
        }
        let remember = self.send.clone();
        let selected = self.state.remember;
        panel = panel.child(choices).child(
            Button::new(
                "share_remember",
                if selected {
                    "Remember selection · On"
                } else {
                    "Remember selection · Off"
                },
            )
            .control_size(ControlSize::Sm)
            .variant(ButtonVariant::Secondary)
            .rounded_full()
            .h(px(26.0))
            .disabled(pending)
            .on_click(move |_, _, cx| remember(Action::Remember(!selected), cx)),
        );
        if self.state.snapshot.selecting.is_some() {
            let cancel = self.send.clone();
            panel = panel.child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child("Confirm the system selection…")
                    .child(
                        Button::new("share_cancel", "Cancel")
                            .debug_selector(|| "share-cancel".into())
                            .control_size(ControlSize::Sm)
                            .rounded_full()
                            .h(px(26.0))
                            .on_click(move |_, _, cx| cancel(Action::Cancel, cx)),
                    ),
            );
        } else if self.state.busy {
            panel = panel.child("Updating sharing…");
        }
        if self.state.snapshot.stopping {
            panel = panel.child("Stopping sharing…");
        }
        for source in &self.state.snapshot.sources {
            let send = self.send.clone();
            let id = source.source;
            panel = panel.child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        div().min_w_0().flex_1().child(source.label.clone()).child(
                            div()
                                .text_size(px(10.0))
                                .text_color(cx.theme().content.secondary)
                                .child(format!(
                                    "{}{}",
                                    if source.supports_input {
                                        "Control"
                                    } else {
                                        "Video only"
                                    },
                                    if source.remembered {
                                        " · remembered"
                                    } else {
                                        ""
                                    }
                                )),
                        ),
                    )
                    .child(
                        Button::new(format!("share_stop_{id:?}"), "Stop")
                            .control_size(ControlSize::Sm)
                            .rounded_full()
                            .h(px(26.0))
                            .on_click(move |_, _, cx| send(Action::Stop(id), cx)),
                    ),
            );
        }
        if !self.state.snapshot.sources.is_empty() {
            let send = self.send.clone();
            panel = panel.child(
                Button::new("share_stop_all", "Stop sharing")
                    .control_size(ControlSize::Sm)
                    .variant(ButtonVariant::Secondary)
                    .rounded_full()
                    .h(px(26.0))
                    .on_click(move |_, _, cx| send(Action::StopAll, cx)),
            );
        } else if !pending {
            panel = panel.child("No source shared");
        }
        if let Some(message) = self.state.error.or(self.state.snapshot.message) {
            panel = panel.child(message);
        }
        panel
    }
}
#[cfg(target_os = "linux")]
pub(crate) mod linux;
#[cfg(test)]
mod tests;
