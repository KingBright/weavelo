use super::device_list::{
    DeviceActivity, DeviceFilterKind, DeviceListAction, DeviceListState,
    DeviceListViewModel,
};
use crate::design_system::color_accent_emerald;
use crate::product_components::{component::tooltip,icon,IconName};
use crate::product_components::theme::ActiveTheme;
use crate::product_components::{device_badge_element, device_badge_image};
use ely_gpui_component::{
    buttons::{Button, ButtonVariant},
    theme::ControlSize,
};
use gpui::{App, FontWeight, IntoElement, RenderOnce, Window, div, prelude::*, px};
use remote_core::discovery::DiscoveryScope;
use std::{rc::Rc,collections::BTreeMap};

type ActionSink = Rc<dyn Fn(DeviceListAction, &mut App)>;

#[derive(IntoElement)]
pub(crate) struct DeviceDrawer {
    model: DeviceListViewModel,
    state: DeviceListState,
    send: ActionSink,
    activity:BTreeMap<String,DeviceActivity>,
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
            activity:BTreeMap::new(),
        }
    }
}

impl DeviceDrawer {
    pub(crate) fn with_activity(mut self,activity:BTreeMap<String,DeviceActivity>)->Self {self.activity=activity;self}
}

impl RenderOnce for DeviceDrawer {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme=cx.theme();
        let mut filters=div().flex().gap_1();
        for (label,filter) in [("All",DeviceFilterKind::All),("LAN",DeviceFilterKind::Lan),("P2P",DeviceFilterKind::P2p),("Relay",DeviceFilterKind::Relay)] {
            let send=self.send.clone();
            filters=filters.child(Button::new(format!("filter_{label}"),crate::i18n::tr(label))
                .debug_selector(move||format!("filter-{label}"))
                .variant(if filter==self.state.filter {ButtonVariant::Primary}else{ButtonVariant::Ghost})
                .control_size(ControlSize::Sm).h(px(24.)).px_2().text_size(px(10.))
                .on_click(move|_,_,cx|send(DeviceListAction::SelectFilter(filter),cx)));
        }
        let mut content=div().flex().flex_col().gap_2().child(filters);
        if self.model.rows.is_empty() {
            return content.child(div().py_3().text_color(theme.content.tertiary).text_size(px(12.))
                .child(crate::i18n::tr("No devices. Add a device or choose All.")));
        }
        for row in self.model.rows {
            let id=row.device_id.clone();
            let activity=self.activity.get(&id).cloned().unwrap_or_default();
            let status=activity.label();
            let scope=crate::i18n::tr(match row.scope {DiscoveryScope::Lan=>"LAN",DiscoveryScope::P2p=>"Direct",DiscoveryScope::Relay=>"Relay",DiscoveryScope::Mesh=>"Mesh"});
            let active=activity.receiving>0||activity.sharing>0||activity.connecting;
            let send=self.send.clone();let detail_id=id.clone();
            let mut card=div().flex().flex_col().gap_1().px_3().py_2().rounded(px(10.))
                .bg(theme.surface.base).border_1()
                .border_color(if active {theme.border.focus}else{theme.border.default})
                .child(div().min_h(px(42.)).flex().items_center().gap_2()
                    .child(div().size(px(34.)).flex_none().flex().items_center().justify_center()
                        .rounded(px(8.)).bg(theme.surface.sunken)
                        .child(device_badge_element(device_badge_image(row.badge, cx))))
                    .child(div().min_w_0().flex_1().flex().flex_col()
                        .child(Button::new(format!("device-name-{id}"),row.display_name.clone())
                            .debug_selector({let id=id.clone();move||format!("device-name-{id}")})
                            .variant(ButtonVariant::Ghost).h(px(22.)).p_0().min_w_0().w_full().justify_start()
                            .text_size(px(13.)).font_weight(FontWeight::SEMIBOLD)
                            .tooltip(tooltip(row.display_name.clone()).build())
                            .on_click(move|_,_,cx|send(DeviceListAction::Details(detail_id.clone()),cx)))
                        .child(div().flex().items_center().gap_1().text_size(px(10.))
                            .text_color(theme.content.secondary)
                            .child(div().size(px(6.)).flex_none().rounded_full().bg(if row.online {color_accent_emerald().into()}else{theme.content.disabled}))
                            .child(if !status.is_empty(){status}else{crate::i18n::tr_format!("{} · {}",scope,crate::i18n::tr(if row.online {"Available"}else{"Offline"}))})))
                    .child({
                        let send=self.send.clone();let target=id.clone();let selector=format!("connect-{id}");
                        Button::new(format!("device-connect-{id}"),crate::i18n::tr("View"))
                            .debug_selector(move||selector.clone()).variant(ButtonVariant::Primary)
                            .control_size(ControlSize::Sm).h(px(28.)).px_2().text_size(px(11.))
                            .disabled(!row.can_connect).on_click(move|_,_,cx|send(DeviceListAction::Connect(target.clone()),cx))
                    })
                    .child({
                        let send=self.send.clone();let target=id.clone();let selector=format!("more-{id}");
                        Button::new(format!("device-more-{id}"),"")
                            .debug_selector(move||selector.clone()).child(icon(IconName::More).size(px(18.)))
                            .tooltip(tooltip(crate::i18n::tr("More device actions")).build()).control_size(ControlSize::Sm)
                            .variant(ButtonVariant::Ghost).size(px(28.)).p_0()
                            .on_click(move|_,_,cx|send(DeviceListAction::More(target.clone()),cx))
                    }));
            if let Some(error)=activity.error.as_ref() {
                card=card.child(div().text_size(px(11.)).text_color(theme.content.primary)
                    .child(crate::i18n::tr_format!("Needs attention: {error}", error=crate::i18n::message(error))));
            }
            if self.state.more.as_ref()==Some(&id) || self.state.details.as_ref()==Some(&id) {
                let mut more=div().flex().flex_col().gap_2().py_2();
                let mut commands=div().flex().flex_wrap().gap_2();
                for (key,label,enabled,action) in [
                    ("send","Send screen or app",row.online,DeviceListAction::SendScreen(id.clone())),
                    ("files","Files",row.can_open_files,DeviceListAction::OpenFiles(id.clone())),
                    ("workspace","Open another source",row.can_open_workspace,DeviceListAction::OpenWorkspace(id.clone())),
                    ("show","Show stream",activity.receiving>0||activity.connecting||activity.error.is_some(),DeviceListAction::ShowStreams(id.clone())),
                    ("end","End streams",activity.receiving>0||activity.connecting||activity.error.is_some(),DeviceListAction::EndStreams(id.clone())),
                    ("stop-send","Stop sending",activity.can_stop_sending,DeviceListAction::StopSending(id.clone())),
                ] {
                    if !enabled && key!="send" && key!="files" {continue;}
                    let send=self.send.clone();let selector=format!("{key}-{id}");
                    commands=commands.child(Button::new(format!("device-{key}-{id}"),crate::i18n::tr(label))
                        .debug_selector(move||selector.clone()).variant(ButtonVariant::Secondary)
                        .control_size(ControlSize::Sm).h(px(26.)).px_2().text_size(px(11.)).disabled(!enabled)
                        .on_click(move|_,_,cx|send(action.clone(),cx)));
                }
                more=more.child(commands).child(div().text_size(px(10.)).text_color(theme.content.tertiary)
                    .child(format!("{scope} · {} · {}",row.endpoint,row.device_id)));
                card=card.child(more);
            }
            content=content.child(card);
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
            badge: Default::default(),
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
        let bounds = cx.debug_bounds("more-offline").unwrap();
        cx.simulate_click(bounds.center(), Modifiers::none());
        cx.run_until_parked();
        output.borrow_mut().clear();
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
        let bounds = cx.debug_bounds("more-b").unwrap();
        cx.simulate_click(bounds.center(), Modifiers::none());
        cx.run_until_parked();
        let bounds = cx.debug_bounds("files-b").unwrap();
        cx.simulate_click(bounds.center(), Modifiers::none());
        assert_eq!(
            *output.borrow(),
            [
                DeviceListAction::Connect("a".into()),
                DeviceListAction::More("b".into()),
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
