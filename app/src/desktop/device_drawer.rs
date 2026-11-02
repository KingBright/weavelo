use super::device_list::{
    DeviceConnectionStatus, DeviceFilterKind, DeviceListAction, DeviceListState,
    DeviceListViewModel,
};
use crate::design_system::{color_accent_cyan, color_accent_emerald, color_border_fine};
use crate::product_components::component::tooltip;
use crate::product_components::theme::ActiveTheme;
use ely_gpui_component::{
    buttons::{Button, ButtonVariant},
    primitives::IconName,
    theme::ControlSize,
};
use gpui::{App, FontWeight, IntoElement, RenderOnce, Window, div, prelude::*, px, rgb};
use remote_core::discovery::DiscoveryScope;
use std::rc::Rc;

type ActionSink = Rc<dyn Fn(DeviceListAction, &mut App)>;

#[derive(IntoElement)]
pub(crate) struct DeviceDrawer {
    model: DeviceListViewModel,
    state: DeviceListState,
    send: ActionSink,
}

impl DeviceDrawer {
    pub(crate) fn new(
        model: DeviceListViewModel,
        state: DeviceListState,
        send: impl Fn(DeviceListAction, &mut App) + 'static,
    ) -> Self {
        Self {
            model,
            state,
            send: Rc::new(send),
        }
    }
}

impl RenderOnce for DeviceDrawer {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        let mut filters = div().flex().gap_1();
        for (label, filter) in [
            ("All", DeviceFilterKind::All),
            ("LAN", DeviceFilterKind::Lan),
            ("P2P", DeviceFilterKind::P2p),
            ("Relay", DeviceFilterKind::Relay),
        ] {
            let send = self.send.clone();
            filters = filters.child(
                Button::new(format!("filter_{label}"), label)
                    .debug_selector(move || format!("filter-{label}"))
                    .variant(if filter == self.state.filter {
                        ButtonVariant::Primary
                    } else {
                        ButtonVariant::Secondary
                    })
                    .control_size(ControlSize::Sm)
                    .h(px(24.0))
                    .flex_1()
                    .text_size(px(9.0))
                    .on_click(move |_, _, cx| send(DeviceListAction::SelectFilter(filter), cx)),
            );
        }
        let mut content = div().flex().flex_col().gap_2().child(filters);
        if self.model.rows.is_empty() {
            return content.child(
                div()
                    .p_4()
                    .rounded_md()
                    .bg(theme.surface.sunken)
                    .text_color(theme.content.tertiary)
                    .text_size(px(11.0))
                    .child("No devices in this filter. Join a device group or choose All."),
            );
        }
        for row in self.model.rows {
            let id = row.device_id.clone();
            let scope = match row.scope {
                DiscoveryScope::Lan => "LAN",
                DiscoveryScope::P2p => "P2P Direct",
                DiscoveryScope::Relay => "Relay",
                DiscoveryScope::Mesh => "Mesh",
            };
            let details = self.send.clone();
            let detail_id = id.clone();
            let view = self.send.clone();
            let view_id = id.clone();
            let files = self.send.clone();
            let files_id = id.clone();
            let delivery = self.send.clone();
            let delivery_id = id.clone();
            let more = self.send.clone();
            let more_id = id.clone();
            let mut card = div()
                .flex()
                .flex_col()
                .gap_2()
                .p_2()
                .rounded(px(8.))
                .shadow_xs()
                .bg(if row.is_active() {
                    rgb(0x192830).into()
                } else {
                    theme.surface.raised
                })
                .border_1()
                .border_color(if row.is_active() {
                    color_accent_cyan()
                } else {
                    color_border_fine()
                })
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .size(px(7.))
                                .flex_none()
                                .rounded_full()
                                .bg(if row.online {
                                    color_accent_emerald().into()
                                } else {
                                    theme.content.disabled
                                }),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .flex_1()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .child(
                                    Button::new(
                                        format!("device-name-{id}"),
                                        row.display_name.clone(),
                                    )
                                    .debug_selector({
                                        let id = id.clone();
                                        move || format!("device-name-{id}")
                                    })
                                    .variant(ButtonVariant::Ghost)
                                    .h(px(23.))
                                    .p_0()
                                    .justify_start()
                                    .text_size(px(13.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .tooltip(tooltip("Device details").build())
                                    .on_click(
                                        move |_, _, cx| {
                                            details(
                                                DeviceListAction::Details(detail_id.clone()),
                                                cx,
                                            )
                                        },
                                    ),
                                )
                                .child(
                                    div()
                                        .text_size(px(10.))
                                        .text_color(theme.content.secondary)
                                        .child(format!(
                                            "{scope} · {}",
                                            if row.online {
                                                row.connection.label()
                                            } else {
                                                "Offline"
                                            }
                                        )),
                                ),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_none()
                                .items_center()
                                .gap_1()
                                .child(
                                    Button::new(format!("drawer_connect_{id}"), "")
                                        .debug_selector({
                                            let id = id.clone();
                                            move || format!("connect-{id}")
                                        })
                                        .icon(IconName::Monitor)
                                        .tooltip(tooltip("View screens and app windows").build())
                                        .control_size(ControlSize::Sm)
                                        .size(px(28.))
                                        .p_0()
                                        .variant(
                                            if row.connection == DeviceConnectionStatus::Viewing {
                                                ButtonVariant::Primary
                                            } else {
                                                ButtonVariant::Ghost
                                            },
                                        )
                                        .disabled(!row.can_connect)
                                        .on_click(move |_, _, cx| {
                                            view(DeviceListAction::Connect(view_id.clone()), cx)
                                        }),
                                )
                                .child(
                                    Button::new(format!("send_screen_{id}"), "")
                                        .icon(IconName::ScreenShare)
                                        .tooltip(tooltip("Send screens or app windows").build())
                                        .control_size(ControlSize::Sm)
                                        .variant(ButtonVariant::Ghost)
                                        .size(px(28.))
                                        .p_0()
                                        .disabled(!row.online)
                                        .on_click(move |_, _, cx| {
                                            delivery(
                                                DeviceListAction::SendScreen(delivery_id.clone()),
                                                cx,
                                            )
                                        }),
                                )
                                .child(
                                    Button::new(format!("drawer_files_{id}"), "")
                                        .debug_selector({
                                            let id = id.clone();
                                            move || format!("files-{id}")
                                        })
                                        .icon(IconName::FolderOpen)
                                        .tooltip(tooltip("Files").build())
                                        .control_size(ControlSize::Sm)
                                        .variant(ButtonVariant::Ghost)
                                        .size(px(28.))
                                        .p_0()
                                        .disabled(!row.can_open_files)
                                        .on_click(move |_, _, cx| {
                                            files(DeviceListAction::OpenFiles(files_id.clone()), cx)
                                        }),
                                )
                                .child(
                                    Button::new(format!("device-more-{id}"), "")
                                        .debug_selector({
                                            let id = id.clone();
                                            move || format!("more-{id}")
                                        })
                                        .icon(IconName::Ellipsis)
                                        .tooltip(tooltip("More device actions").build())
                                        .control_size(ControlSize::Sm)
                                        .variant(ButtonVariant::Ghost)
                                        .size(px(28.))
                                        .p_0()
                                        .on_click(move |_, _, cx| {
                                            more(DeviceListAction::More(more_id.clone()), cx)
                                        }),
                                ),
                        ),
                );
            if self.state.more.as_ref() == Some(&id) {
                let workspace = self.send.clone();
                let workspace_id = id.clone();
                let receive = self.send.clone();
                let receive_id = id.clone();
                card = card.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_2()
                        .child(
                            Button::new(format!("workspace-{id}"), "Open several sources…")
                                .debug_selector({
                                    let id = id.clone();
                                    move || format!("workspace-{id}")
                                })
                                .icon(IconName::LayoutGrid)
                                .control_size(ControlSize::Sm)
                                .variant(ButtonVariant::Secondary)
                                .disabled(!row.can_open_workspace)
                                .on_click(move |_, _, cx| {
                                    workspace(
                                        DeviceListAction::OpenWorkspace(workspace_id.clone()),
                                        cx,
                                    )
                                }),
                        )
                        .child(
                            Button::new(format!("copy_receive_{id}"), "Copy macOS receive command")
                                .icon(IconName::Terminal)
                                .control_size(ControlSize::Sm)
                                .variant(ButtonVariant::Secondary)
                                .disabled(!row.online)
                                .on_click(move |_, _, cx| {
                                    receive(
                                        DeviceListAction::CopyReceiveCommand(receive_id.clone()),
                                        cx,
                                    )
                                }),
                        ),
                );
            }
            if self.state.details.as_ref() == Some(&id) {
                card = card.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .text_size(px(11.))
                        .text_color(theme.content.secondary)
                        .child(format!("Device ID · {id}"))
                        .child(format!("Address · {}", row.endpoint)),
                );
            }
            content = content.child(card);
        }
        content
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AppDevice;
    use ely_gpui_component::primitives::FocusScope;
    use gpui::{Context, FocusHandle, Modifiers, TestAppContext, WindowAppearance};
    use remote_core::role::{RoleSession, RoleState};
    use std::cell::RefCell;

    struct DrawerFixture {
        root: FocusHandle,
        model: DeviceListViewModel,
        output: Rc<RefCell<Vec<DeviceListAction>>>,
        state: DeviceListState,
    }
    impl Render for DrawerFixture {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let output = self.output.clone();
            let view = cx.weak_entity();
            FocusScope::new(&self.root)
                .root()
                .w(px(360.0))
                .child(DeviceDrawer::new(
                    self.model.clone(),
                    self.state.clone(),
                    move |action, cx| {
                        output.borrow_mut().push(action.clone());
                        let _ = view.update(cx, |this, cx| {
                            this.state.reduce(action);
                            cx.notify();
                        });
                    },
                ))
        }
    }
    fn device(id: &str, online: bool) -> AppDevice {
        AppDevice {
            device_id: id.into(),
            display_name: id.into(),
            endpoint: "127.0.0.1:9000".parse().unwrap(),
            scope: DiscoveryScope::Lan,
            can_stream: true,
            can_view: true,
            online,
            last_seen_ms: 0,
        }
    }

    #[gpui::test]
    fn drawer_disabled_and_reordered_controls_keep_device_targets(cx: &mut TestAppContext) {
        cx.update(|cx| {
            crate::product_components::component::init(cx).unwrap();
            crate::product_components::theme::install(
                WindowAppearance::Dark,
                crate::design_system::weavelo_themes(),
                cx,
            );
        });
        let devices = [
            device("a", true),
            device("b", true),
            device("offline", false),
        ];
        let role = RoleState::Viewing(RoleSession::new(devices[0].role_peer(), 7, 0));
        let model = DeviceListViewModel::project(&devices, &role, DeviceFilterKind::All);
        let output = Rc::new(RefCell::new(Vec::new()));
        let (view, cx) = cx.add_window_view({
            let output = output.clone();
            move |_, cx| DrawerFixture {
                root: cx.focus_handle(),
                model,
                output,
                state: DeviceListState::default(),
            }
        });
        cx.run_until_parked();
        for selector in ["connect-offline", "files-offline"] {
            let bounds = cx.debug_bounds(selector).expect("device control rendered");
            cx.simulate_click(bounds.center(), Modifiers::none());
        }
        assert!(
            output.borrow().is_empty(),
            "disabled controls must not dispatch effects"
        );
        let bounds = cx.debug_bounds("connect-a").unwrap();
        cx.simulate_click(bounds.center(), Modifiers::none());
        assert_eq!(
            *output.borrow(),
            [DeviceListAction::Connect("a".into())],
            "an existing view still allows choosing another source on that device"
        );
        output.borrow_mut().clear();
        let bounds = cx.debug_bounds("connect-b").unwrap();
        cx.simulate_click(bounds.center(), Modifiers::none());
        assert_eq!(*output.borrow(), [DeviceListAction::Connect("b".into())]);
        output.borrow_mut().clear();
        let role = RoleState::Connecting(RoleSession::new(devices[1].role_peer(), 8, 1));
        let reversed = [devices[1].clone(), devices[0].clone(), devices[2].clone()];
        view.update(cx, |view, cx| {
            view.model = DeviceListViewModel::project(&reversed, &role, DeviceFilterKind::All);
            cx.notify();
        });
        cx.run_until_parked();
        let bounds = cx.debug_bounds("connect-b").unwrap();
        cx.simulate_click(bounds.center(), Modifiers::none());
        assert!(output.borrow().is_empty());
        let bounds = cx.debug_bounds("connect-a").unwrap();
        cx.simulate_click(bounds.center(), Modifiers::none());
        let bounds = cx.debug_bounds("files-b").unwrap();
        cx.simulate_click(bounds.center(), Modifiers::none());
        assert_eq!(
            *output.borrow(),
            [
                DeviceListAction::Connect("a".into()),
                DeviceListAction::OpenFiles("b".into())
            ]
        );
        output.borrow_mut().clear();
        let bounds = cx.debug_bounds("more-a").unwrap();
        cx.simulate_click(bounds.center(), Modifiers::none());
        cx.run_until_parked();
        let bounds = cx.debug_bounds("workspace-a").unwrap();
        cx.simulate_click(bounds.center(), Modifiers::none());
        assert_eq!(
            *output.borrow(),
            [
                DeviceListAction::More("a".into()),
                DeviceListAction::OpenWorkspace("a".into())
            ],
            "More retains its captured device after reordering"
        );
    }

    struct FocusFixture {
        root: FocusHandle,
        first: FocusHandle,
        disabled: FocusHandle,
        last: FocusHandle,
    }
    impl Render for FocusFixture {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            FocusScope::new(&self.root).root().child(
                div()
                    .flex()
                    .flex_col()
                    .child(Button::new("first", "First").focus_handle(&self.first))
                    .child(
                        Button::new("disabled", "Disabled")
                            .focus_handle(&self.disabled)
                            .disabled(true),
                    )
                    .child(Button::new("last", "Last").focus_handle(&self.last)),
            )
        }
    }
    #[gpui::test]
    fn ely_tab_scope_skips_disabled_controls_in_both_directions(cx: &mut TestAppContext) {
        cx.update(|cx| crate::product_components::component::init(cx).unwrap());
        let (view, cx) = cx.add_window_view(|window, cx| {
            let first = cx.focus_handle().tab_stop(true);
            let root = cx.focus_handle();
            window.focus(&root);
            FocusFixture {
                root,
                first,
                disabled: cx.focus_handle().tab_stop(true),
                last: cx.focus_handle().tab_stop(true),
            }
        });
        cx.run_until_parked();
        cx.simulate_keystrokes("tab");
        cx.update(|window, cx| assert!(view.read(cx).first.is_focused(window)));
        cx.simulate_keystrokes("tab");
        cx.update(|window, cx| assert!(view.read(cx).last.is_focused(window)));
        cx.simulate_keystrokes("shift-tab");
        cx.update(|window, cx| assert!(view.read(cx).first.is_focused(window)));
        cx.update(|window, cx| assert!(!view.read(cx).disabled.is_focused(window)));
    }
}
