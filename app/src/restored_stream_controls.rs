//! Device-bound controls and file contexts. These reuse the existing session and
//! transfer owners; a window is only a presentation of the retained work.
use super::*;
use crate::design_system::color_accent_rose;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum StreamPanel {
    #[default]
    Control,
    Files,
    Sound,
    Display,
    More,
}

pub(super) struct DeviceFiles {
    pub(super) device: String,
    pub(super) owner: Arc<OriginalOwner>,
    connecting: AtomicBool,
    error: Mutex<Option<String>>,
}

impl DeviceFiles {
    fn connect(self: &Arc<Self>) {
        self.owner.poll();
        if self
            .owner
            .active_connection()
            .is_some_and(|conn| conn.peer_is_responsive())
            || self.connecting.swap(true, Ordering::AcqRel)
        {
            return;
        }
        *self.error.lock().unwrap() = None;
        let context = self.clone();
        tokio::spawn(async move {
            let result = context.owner.connect_files(&context.device).await;
            *context.error.lock().unwrap() = result.err().map(|error| error.to_string());
            context.connecting.store(false, Ordering::Release);
        });
    }

    fn disconnect(self: &Arc<Self>) {
        if self.connecting.swap(true, Ordering::AcqRel) {
            return;
        }
        *self.error.lock().unwrap() = None;
        let context = self.clone();
        tokio::spawn(async move {
            let result = context.owner.close_all_verified().await;
            *context.error.lock().unwrap() = result.err();
            context.connecting.store(false, Ordering::Release);
        });
    }
}

impl RestoredDashboard {
    pub(super) fn device_files(&self, device: &str) -> Arc<DeviceFiles> {
        let mut contexts = self.exit_group.file_contexts.lock().unwrap();
        contexts
            .entry(device.to_owned())
            .or_insert_with(|| {
                let owner = OriginalOwner::new(self.runtime.owner.clone());
                self.exit_group
                    .owners
                    .lock()
                    .unwrap()
                    .push(Arc::downgrade(&owner));
                Arc::new(DeviceFiles {
                    device: device.to_owned(),
                    owner,
                    connecting: AtomicBool::new(false),
                    error: Mutex::new(None),
                })
            })
            .clone()
    }

    pub(super) fn open_device_files(&mut self, device: String, cx: &mut Context<Self>) {
        if self.purpose != WindowPurpose::Management && self.stream_device.as_ref() != Some(&device) {
            self.status = "This window belongs to another device".into();
            return;
        }
        self.device_files(&device).connect();
        self.files_device = Some(device);
        self.active_tab = DrawerTab::Files;
        self.stream_panel = StreamPanel::Files;
        self.drawer_open = true;
        self.show_apps_menu = false;
        self.owner.release_input();
        self.pointer_input = PointerInputTracker::default();
        self.remote_text = None;
        self.toolbar_revealed = true;
        self.last_toolbar_activity = Instant::now();
        cx.notify();
    }

    pub(super) fn poll_device_files(&self) {
        for context in self.exit_group.file_contexts.lock().unwrap().values() {
            context.owner.poll();
        }
    }

    pub(super) fn device_label(&self, device: &str) -> String {
        self.owner
            .snapshot()
            .devices
            .iter()
            .find(|item| item.device_id == device)
            .map(|item| item.display_name.clone())
            .unwrap_or_else(|| device.into())
    }

    pub(super) fn transfers_panel(&self, cx: &mut Context<Self>) -> Div {
        let selected = self.files_device.as_deref();
        let mut panel = div().flex().flex_col().gap_3();
        if let Some(device) = selected {
            let context = self.device_files(device);
            panel = panel.child(self.device_files_panel(&context, cx));
        } else {
            panel = panel.child(file_transfer_panel(self.owner.clone(), cx));
        }
        let contexts: Vec<_> = self
            .exit_group
            .file_contexts
            .lock()
            .unwrap()
            .values()
            .cloned()
            .collect();
        for context in contexts {
            if selected == Some(context.device.as_str())
                || context.owner.file_transfer_snapshot().is_empty()
            {
                continue;
            }
            let device = context.device.clone();
            panel = panel.child(
                div()
                    .id(format!("device-transfers-{device}"))
                    .child(
                        command_button(
                            format!("browse-transfers-{device}"),
                            ActionVariantKind::Neutral,
                            cx,
                        )
                        .child(self.device_label(&device))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.open_device_files(device.clone(), cx)
                        })),
                    )
                    .child(file_transfer_rows(context.owner.clone(), cx)),
            );
        }
        panel
    }

    fn device_files_panel(&self, context: &DeviceFiles, cx: &mut Context<Self>) -> Div {
        let disconnect = self.device_files(&context.device);
        let busy = context.connecting.load(Ordering::Acquire)
            || context
                .owner
                .file_transfer_snapshot()
                .iter()
                .any(|transfer| transfer.is_running());
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .text_size(px(12.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(format!("Files · {}", self.device_label(&context.device))),
            )
            .when(context.connecting.load(Ordering::Acquire), |panel| {
                panel.child("Updating file connection…")
            })
            .when_some(context.error.lock().unwrap().clone(), |panel, error| {
                let device = context.device.clone();
                panel
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(color_accent_rose())
                            .child(error),
                    )
                    .child(
                        command_button("retry-device-files", ActionVariantKind::Neutral, cx)
                            .child("Retry")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.open_device_files(device.clone(), cx)
                            })),
                    )
            })
            .child(file_transfer_panel(context.owner.clone(), cx))
            .child(
                command_button("disconnect-device-files", ActionVariantKind::Neutral, cx)
                    .disabled(busy)
                    .child("Disconnect files")
                    .on_click(move |_, _, _| disconnect.disconnect()),
            )
    }

    pub(super) fn open_stream_panel(&mut self, panel: StreamPanel, cx: &mut Context<Self>) {
        if panel == StreamPanel::Files {
            if let Some(device) = self.stream_device.clone() {
                self.open_device_files(device, cx);
            }
            return;
        }
        self.owner.release_input();
        self.pointer_input = PointerInputTracker::default();
        self.remote_text = None;
        self.drawer_open = !self.drawer_open || self.stream_panel != panel;
        self.stream_panel = panel;
        self.show_apps_menu = false;
        self.toolbar_revealed = true;
        self.last_toolbar_activity = Instant::now();
        cx.notify();
    }

    fn toggle_source_menu(&mut self, cx: &mut Context<Self>) {
        let Some(binding) = self.owner.source_view_binding() else { return; };
        if !self.owner.source_binding_is_current(&binding) { return; }
        self.reset_pointer_scroll();
        self.owner.release_input();
        self.pointer_input = PointerInputTracker::default();
        self.show_apps_menu = !self.show_apps_menu;
        self.toolbar_revealed = true;
        self.toolbar_hovered = false;
        self.last_toolbar_activity = Instant::now();
        if self.show_apps_menu {
            self.request_bound_capture_sources(binding, cx);
        }
        cx.notify();
    }

    fn android_navigation_target(
        &self,
        snapshot: &DashboardSnapshot,
    ) -> Option<crate::desktop::original_owner::ViewportTarget> {
        if self.input_locked
            || !self.capture_supports_input
            || !snapshot.input_eligible
            || self.drawer_open
            || self.show_apps_menu
            || self.source_selection.is_some()
            || self.stream_source != Some(snapshot.active_capture_source)
        {
            return None;
        }
        self.scroll_target()
    }

    fn send_android_navigation(&mut self, action: &'static str, cx: &mut Context<Self>) {
        let snapshot = self.snapshot();
        let Some(button) = protocol::session::android_navigation_button(
            &snapshot.capture_sources,
            snapshot.active_capture_source,
            action,
        ) else {
            self.status = format!("Android {action} is unavailable for this source.");
            cx.notify();
            return;
        };
        let Some(target) = self.android_navigation_target(&snapshot) else {
            self.status = "Remote navigation is waiting for an unlocked, current painted source frame.".into();
            cx.notify();
            return;
        };
        if let Err(reason) = self.owner.queue_viewport_button_tap(&target, button) {
            self.status = format!("Android {action} was not sent: {reason}.");
        }
        self.last_toolbar_activity = Instant::now();
        cx.notify();
    }

    pub(super) fn android_companion_controls(
        &self,
        snapshot: &DashboardSnapshot,
        fullscreen: bool,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let back = protocol::session::android_navigation_button(
            &snapshot.capture_sources,
            snapshot.active_capture_source,
            "back",
        );
        let home = protocol::session::android_navigation_button(
            &snapshot.capture_sources,
            snapshot.active_capture_source,
            "home",
        );
        let current_target = self.android_navigation_target(snapshot).is_some();
        let disabled_reason = if !snapshot.active_capture_supports_input {
            Some("This source is view-only.")
        } else if !current_target {
            Some("Waiting for an unlocked, current painted source frame.")
        } else {
            None
        };
        let back_tooltip = if back.is_none() {
            "Back is unavailable for this source."
        } else if let Some(reason) = disabled_reason {
            reason
        } else {
            "Android Back"
        };
        let home_tooltip = if home.is_none()
            && matches!(snapshot.active_capture_source, protocol::session::CaptureSource::Window(_))
        {
            "Home is unavailable for a managed app window."
        } else if home.is_none() {
            "Home is unavailable for this source."
        } else if let Some(reason) = disabled_reason {
            reason
        } else {
            "Android Home"
        };
        let back_view = cx.weak_entity();
        let home_view = cx.weak_entity();
        let fit_view = cx.weak_entity();
        let theme = cx.theme().clone();
        div()
            .id("android-navigation-gutter")
            .occlude()
            .w(px(window_fit::ANDROID_COMPANION_WIDTH_PX))
            .py_1().rounded_full()
            .flex_none()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_1()
            .bg(if fullscreen { rgb(0x000000).into() } else { theme.surface.canvas })
            .on_any_mouse_down(cx.listener(|_, _, _, cx| cx.stop_propagation()))
            .child(
                command_button("android-navigation-back", ActionVariantKind::Neutral, cx)
                    .w(px(window_fit::ANDROID_COMPANION_WIDTH_PX))
                    .h(px(40.))
                    .px_0()
                    .disabled(back.is_none() || !current_target)
                    .tooltip(tooltip(back_tooltip).build())
                    .on_click(move |_, _, cx| {
                        let _ = back_view.update(cx, |this, cx| this.send_android_navigation("back", cx));
                    })
                    .child(icon(IconName::Back).size(px(18.))),
            )
            .child(
                command_button("android-navigation-home", ActionVariantKind::Neutral, cx)
                    .w(px(window_fit::ANDROID_COMPANION_WIDTH_PX))
                    .h(px(40.))
                    .px_0()
                    .disabled(home.is_none() || !current_target)
                    .tooltip(tooltip(home_tooltip).build())
                    .on_click(move |_, _, cx| {
                        let _ = home_view.update(cx, |this, cx| this.send_android_navigation("home", cx));
                    })
                    .child(icon(IconName::Home).size(px(18.))),
            )
            .child(
                command_button("android-navigation-fit", ActionVariantKind::Neutral, cx)
                    .w(px(window_fit::ANDROID_COMPANION_WIDTH_PX))
                    .h(px(40.))
                    .px_0()
                    .tooltip(tooltip("Fit the remote picture to the viewer").build())
                    .on_click(move |_, _, cx| {
                        let _ = fit_view.update(cx, |this, cx| {
                            this.reset_pointer_scroll();
                            this.scale_mode = ViewportScaleMode::AspectFit;
                            this.viewport.lock().unwrap().reset();
                            this.persist_preferences();
                            cx.notify();
                        });
                    })
                    .child(icon(IconName::Window).size(px(18.))),
            )
    }

    pub(super) fn fullscreen_micro_rail(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        div()
            .id("fullscreen-toolbar-rail-position")
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .flex()
            .justify_center()
            .child(
                div()
                    .id("fullscreen-toolbar-rail-hit-target")
                    .occlude()
                    .w(px(window_fit::FULLSCREEN_RAIL_WIDTH_PX + 16.0))
                    .h(px(window_fit::FULLSCREEN_RAIL_HIT_HEIGHT_PX))
                    .flex()
                    .justify_center()
                    .pt(px(8.))
                    .tooltip(tooltip("Show controls").build())
                    .on_any_mouse_down(cx.listener(|_, _, _, cx| cx.stop_propagation()))
                    .on_click(cx.listener(|this, _, _, cx| {
                        if should_reveal_toolbar(ToolbarRevealTrigger::MicroRailClick) {
                            this.toolbar_revealed = true;
                            this.toolbar_hovered = false;
                            this.last_toolbar_activity = Instant::now();
                            cx.notify();
                        }
                    }))
                    .child(
                        div()
                            .id("fullscreen-toolbar-rail")
                            .w(px(window_fit::FULLSCREEN_RAIL_WIDTH_PX))
                            .h(px(window_fit::FULLSCREEN_RAIL_HEIGHT_PX))
                            .rounded_full()
                            .bg(cx.theme().action.primary.bg),
                    ),
            )
    }

    pub(super) fn stream_toolbar(
        &self,
        snapshot: &DashboardSnapshot,
        fullscreen: bool,
        compact: bool,
        tiny: bool,
        input_hint: Option<&str>,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let visibility = control_island_visibility(snapshot.role.kind());
        let theme = cx.theme().clone();
        let source = confirmed_source(&snapshot.capture_sources, snapshot.active_capture_source);
        let device = self
            .stream_device
            .as_deref()
            .map(|id| self.device_label(id))
            .unwrap_or_else(|| "Weavelo".into());
        let source_label = source.map(capture_source_label).unwrap_or_else(|| "Waiting for source".into());
        let context_title = format!("{device} · {source_label}");
        let button_size = if tiny { 20.0 } else { 28.0 };
        let mut row = div()
            .id(if fullscreen { "fullscreen_toolbar_pill" } else { "stream_header" })
            .occlude()
            .flex()
            .items_center()
            .gap_0p5()
            .when(fullscreen, |row| row.px_2().py_1().rounded_full().bg(color_glass_card()).border_1()
                .border_color(color_border_fine()).shadow_lg())
            .when(!fullscreen, |row| row.w_full().h(px(window_fit::STREAM_HEADER_HEIGHT_PX))
                .px_1().rounded(px(14.)).bg(theme.surface.base).border_1().border_color(theme.border.muted))
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                this.toolbar_hovered = *hovered;
                this.last_toolbar_activity = Instant::now();
                cx.notify();
            }))
            .on_mouse_move(cx.listener(|this, _: &MouseMoveEvent, _, _| {
                this.last_toolbar_activity = Instant::now()
            }))
            .on_any_mouse_down(cx.listener(|_, _, _, cx| cx.stop_propagation()));
        row = row.when(!tiny, |row| row.child(
            div()
                .id("stream-header-context")
                .flex()
                .flex_1()
                .min_w_0()
                .items_center()
                .gap_1()
                .px_1()
                .when_some(source, |row, source| row.child(capture_source_badge(source, &snapshot.source_icons, &theme)))
                .child(div().size(px(6.)).flex_none().rounded_full().bg(if visibility.show_disconnect {
                    Hsla::from(color_accent_emerald())
                } else { theme.content.disabled }))
                .child(
                    div()
                        .id("stream-device-title")
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(px(12.))
                        .text_color(theme.content.primary)
                        .tooltip(tooltip(context_title.clone()).build())
                        .child(context_title.clone()),
                ),
        ));
        let source_binding = snapshot.source_binding.clone();
        let source_button = command_button("stream-primary-source", ActionVariantKind::Neutral, cx)
            .size(px(button_size)).p_0()
            .tooltip(tooltip("Switch source or open a new view").build())
            .child(icon(IconName::NewView).size(px(if tiny { 12. } else if compact { 13. } else { 14. })))
            .disabled(source_binding.is_none())
            .on_click(cx.listener(|this, _, _, cx| this.toggle_source_menu(cx)));
        let files_button = command_button("stream-primary-files", ActionVariantKind::Neutral, cx)
            .size(px(button_size)).p_0()
            .tooltip(tooltip("Files").build())
            .child(icon(IconName::Files).size(px(if tiny { 12. } else if compact { 13. } else { 14. })))
            .disabled(self.stream_device.is_none())
            .on_click(cx.listener(|this, _, _, cx| this.open_stream_panel(StreamPanel::Files, cx)));
        let more_button = command_button(
            "stream-primary-more",
            if self.drawer_open { ActionVariantKind::Primary } else { ActionVariantKind::Neutral },
            cx,
        )
        .size(px(button_size)).p_0()
        .tooltip(tooltip("Control, sound, display and session options").build())
        .child(icon(IconName::More).size(px(if tiny { 12. } else if compact { 13. } else { 14. })))
        .on_click(cx.listener(|this, _, _, cx| this.open_stream_panel(StreamPanel::More, cx)));
        let fullscreen_button = command_button("stream-primary-fullscreen", ActionVariantKind::Neutral, cx)
            .size(px(button_size)).p_0()
            .tooltip(tooltip(if fullscreen { "Exit fullscreen · F11" } else { "Fullscreen · F11" }).build())
            .child(icon(IconName::Maximize(fullscreen)).size(px(if tiny { 12. } else { 14. })))
            .on_click(cx.listener(|this, _, window, cx| {
                this.reset_pointer_scroll();
                this.owner.release_input();
                this.pointer_input = PointerInputTracker::default();
                this.toolbar_revealed = true;
                this.last_toolbar_activity = Instant::now();
                window.toggle_fullscreen();
                cx.refresh_windows();
            }));
        row = row.child(source_button).child(files_button).child(more_button).child(fullscreen_button);
        let cua_mode = snapshot.computer_use_status.mode;
        let cua_label = snapshot
            .computer_use_status
            .status_label
            .clone()
            .unwrap_or_else(|| match cua_mode {
                Some(crate::desktop::original_owner::ComputerUseMode::Observing) => {
                    "AI Observing screen".into()
                }
                Some(crate::desktop::original_owner::ComputerUseMode::Controlling) => {
                    "AI Controlling — Move mouse or press Esc to take over".into()
                }
                Some(crate::desktop::original_owner::ComputerUseMode::PausedByHuman) => {
                    "AI Paused — Human took over input".into()
                }
                None => String::new(),
            });
        let mut container = div()
            .id(if fullscreen { "fullscreen-controls-container" } else { "stream-header-container" })
            .relative()
            .flex()
            .flex_col()
            .gap_1()
            .when(fullscreen, |tree| tree.absolute().top(px(8.)).left_0().right_0().items_center())
            .when(!fullscreen, |tree| tree.mx(px(window_fit::STREAM_WINDOW_GUTTER_PX)).mt(px(window_fit::STREAM_HEADER_TOP_PX)));
        container = container.child(row)
            .when_some(cua_mode, |tree, mode| {
                let badge_color = match mode {
                    crate::desktop::original_owner::ComputerUseMode::Observing => {
                        Hsla::from(color_accent_cyan())
                    }
                    crate::desktop::original_owner::ComputerUseMode::Controlling => {
                        Hsla::from(color_accent_emerald())
                    }
                    crate::desktop::original_owner::ComputerUseMode::PausedByHuman => {
                        cx.theme().content.secondary
                    }
                };
                tree.child(
                    div()
                        .id("computer-use-status-capsule")
                        .occlude()
                        .h(px(24.))
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_3()
                        .py_1()
                        .rounded_full()
                        .bg(if fullscreen { color_glass_card() } else { theme.surface.base })
                        .border_1()
                        .border_color(badge_color)
                        .shadow_md()
                        .child(div().size(px(8.)).rounded_full().bg(badge_color))
                        .child(div().text_size(px(11.)).child(cua_label))
                        .child(
                            command_button("computer-use-dismiss-pill", ActionVariantKind::Neutral, cx)
                                .h(px(20.))
                                .px_2()
                                .child(if mode == crate::desktop::original_owner::ComputerUseMode::PausedByHuman {
                                    "Dismiss"
                                } else {
                                    "Pause AI"
                                })
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.owner.release_input();
                                    this.owner.set_computer_use_active(None, None);
                                    cx.notify();
                                })),
                        ),
                )
            })
            .when(self.show_apps_menu, |tree| {
                tree.child(div().absolute().top(px(window_fit::STREAM_HEADER_HEIGHT_PX + 4.0))
                    .left_0().right_0().flex().justify_center()
                    .child(capture_source_menu(
                        &snapshot.capture_sources,
                        &snapshot.source_icons,
                        snapshot.source_binding.as_ref(),
                        snapshot.active_capture_source,
                        snapshot.pending_capture_source,
                        snapshot.capture_source_error.as_deref(),
                        snapshot.role.session(),
                        cx,
                    )))
            });
        container
    }

    pub(super) fn stream_controls_panel(
        &self,
        snapshot: &DashboardSnapshot,
        host_stats: Option<&UiHostStats>,
        viewport_width: f32,
        viewport_height: f32,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let title = match self.stream_panel {
            StreamPanel::Control => "Control",
            StreamPanel::Files => "Files",
            StreamPanel::Sound => "Sound",
            StreamPanel::Display => "Display",
            StreamPanel::More => "More",
        };
        let body = match self.stream_panel {
            StreamPanel::Control => self.stream_input_panel(snapshot, cx),
            StreamPanel::Files => self
                .stream_device
                .as_deref()
                .map(|device| self.device_files_panel(&self.device_files(device), cx))
                .unwrap_or_else(|| div().child("Choose a device")),
            StreamPanel::Sound => self.stream_sound_panel(cx),
            StreamPanel::Display => div()
                .flex()
                .flex_col()
                .gap_3()
                .child(scale_mode_control(self.scale_mode, cx))
                .when_some(self.viewport_controls(cx),|panel,controls|panel.child(controls))
                .child(display_switcher_control(
                    &snapshot.capture_sources,
                    snapshot.source_binding.as_ref(),
                    snapshot.active_capture_source,
                    snapshot.pending_capture_source,
                    self.show_apps_menu,
                    cx,
                ))
                .child(
                    command_button("fit-stream-window", ActionVariantKind::Neutral, cx)
                        .child("Fit window")
                        .tooltip(tooltip("Resize this window to fit the source without leaving the display").build())
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.window_fit.request_fit();
                            cx.notify();
                        })),
                )
                .child(
                    command_button("island_popout_pip", ActionVariantKind::Neutral, cx)
                        .child("Open picture in picture")
                        .on_click(cx.listener(|this, _, _, cx| this.open_popout_pip_window(cx))),
                )
                .child(drawer_network_telemetry_tab(
                    self.stream_settings.values().resolution(),
                    self.stream_settings.values().fps,
                    self.stream_settings.values().bitrate_kbps,
                    self.media_controls(cx),
                    host_stats,
                    self.owner.clone(),
                    false,
                    cx,
                )),
            StreamPanel::More => self.stream_more_panel(snapshot, cx),
        };
        let compact_panel = viewport_width < 420.0 || viewport_height < 280.0;
        let top = if viewport_height < 280.0 {
            8.0
        } else {
            window_fit::STREAM_HEADER_TOP_PX + window_fit::STREAM_HEADER_HEIGHT_PX
                + if snapshot.computer_use_status.mode.is_some() { 36.0 } else { 8.0 }
        };
        let bottom = if viewport_height < 280.0 { 8.0 } else { 12.0 };
        let mut tabs = div().flex().flex_wrap().items_center().gap_1().px_2().pb_2();
        for (panel, label, symbol) in [
            (StreamPanel::Control, "Control", IconName::Control),
            (StreamPanel::Sound, "Sound", IconName::Sound),
            (StreamPanel::Display, "Display", IconName::Display),
            (StreamPanel::More, "More", IconName::More),
        ] {
            let hint = if panel == StreamPanel::Control {
                snapshot.input_status.as_str()
            } else {
                label
            };
            tabs = tabs.child(
                command_button(
                    format!("stream-panel-tab-{panel:?}"),
                    if self.stream_panel == panel { ActionVariantKind::Primary } else { ActionVariantKind::Neutral },
                    cx,
                )
                .when(compact_panel, |button| button.size(px(28.)).p_0())
                .when(!compact_panel, |button| button.h(px(28.)).px_2().gap_1())
                .tooltip(tooltip(hint.to_owned()).build())
                .child(icon(symbol).size(px(13.)))
                .when(!compact_panel, |button| button.child(label))
                .on_click(cx.listener(move |this, _, _, cx| this.open_stream_panel(panel, cx))),
            );
        }
        div()
            .id("stream-local-panel")
            .occlude()
            .absolute()
            .top(px(top))
            .right(px(12.))
            .bottom(px(bottom))
            .w(px((viewport_width - 24.0).max(1.0).min(370.0)))
            .flex()
            .flex_col()
            .rounded_lg()
            .bg(color_glass_card())
            .border_1()
            .border_color(color_border_fine())
            .shadow_lg()
            .on_any_mouse_down(cx.listener(|_, _, _, cx| cx.stop_propagation()))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .when(compact_panel, |header| header.px_2().py_1())
                    .when(!compact_panel, |header| header.p_3())
                    .child(
                        div()
                            .text_size(px(13.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(title),
                    )
                    .child(
                        command_button("close-stream-panel", ActionVariantKind::Neutral, cx)
                            .size(px(24.))
                            .p_0()
                            .child("×")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.drawer_open = false;
                                this.owner.release_input();
                                this.input_focus.focus(window);
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .id("stream-panel-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .when(compact_panel, |scroll| scroll.px_1().py_1())
                    .when(!compact_panel, |scroll| scroll.p_3())
                    .child(div().flex().flex_col().gap_2().child(tabs).child(body)),
            )
    }

    fn stream_input_panel(&self, snapshot: &DashboardSnapshot, cx: &mut Context<Self>) -> Div {
        div().flex().flex_col().gap_3()
            .child(command_button("island_toggle_input_lock", if self.input_locked { ActionVariantKind::Neutral } else { ActionVariantKind::Primary }, cx)
                .disabled(!snapshot.active_capture_supports_input)
                .child(input_control_caption(self.input_locked, snapshot.active_capture_supports_input, snapshot.input_eligible, false))
                .on_click(cx.listener(|this, _, _, cx| { this.set_input_locked(!this.input_locked); cx.notify(); })))
            .child(div().text_size(px(11.)).child(snapshot.input_status.clone()))
            .child(command_button("island_keyboard_mode", ActionVariantKind::Neutral, cx)
                .child(if self.text_mode { "Text · local input method" } else { "Keys · physical keyboard" })
                .on_click(cx.listener(|this, _, _, cx| {
                    this.owner.release_input(); this.remote_text = None; this.text_mode = !this.text_mode; cx.notify();
                })))
            .child(div().text_size(px(11.)).text_color(cx.theme().content.secondary)
                .child("Text uses your local input method. Keys forwards physical keys. Ctrl+Alt+Esc releases control."))
    }

    fn stream_sound_panel(&self, cx: &mut Context<Self>) -> Div {
        let (volume, muted) = self.owner.audio();
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(div().child(format!("Playback · {volume}%")))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        command_button("volume_down", ActionVariantKind::Neutral, cx)
                            .child("−")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.owner.set_audio(volume.saturating_sub(5), muted);
                                cx.notify();
                            })),
                    )
                    .child(
                        command_button("volume_up", ActionVariantKind::Neutral, cx)
                            .child("+")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.owner
                                    .set_audio(volume.saturating_add(5).min(100), muted);
                                cx.notify();
                            })),
                    )
                    .child(
                        command_button("remote_audio_mute", ActionVariantKind::Neutral, cx)
                            .child(if muted { "Unmute" } else { "Mute" })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.owner.set_audio(volume, !muted);
                                cx.notify();
                            })),
                    ),
            )
            .child(security_toggle_row(
                "Microphone to this connection",
                "Shared by this device connection",
                self.owner.talkback_enabled(),
                self.owner.supports_talkback(),
                {
                    let owner = self.owner.clone();
                    let enabled = owner.talkback_enabled();
                    move |_| {
                        owner.set_talkback_enabled(!enabled);
                    }
                },
                cx,
            ))
    }

    fn stream_more_panel(&self, snapshot: &DashboardSnapshot, cx: &mut Context<Self>) -> Div {
        let binding = snapshot.source_binding.clone();
        let mut body = div()
            .flex()
            .flex_col()
            .gap_3()
            .child(security_toggle_row(
                "Clipboard",
                "Shared by this device connection",
                self.owner.clipboard_sync_enabled(),
                self.owner.supports_clipboard_sync(),
                {
                    let owner = self.owner.clone();
                    let enabled = owner.clipboard_sync_enabled();
                    move |_| {
                        owner.set_clipboard_sync_enabled(!enabled);
                    }
                },
                cx,
            ))
            .child(security_toggle_row(
                "Background AI Computer Use",
                "Allow authorized AI agents to control this window while unfocused",
                snapshot.computer_use_status.background_allowed,
                snapshot.active_capture_supports_input,
                {
                    let owner = self.owner.clone();
                    let enabled = snapshot.computer_use_status.background_allowed;
                    move |_| {
                        owner.set_computer_use_background_allowed(!enabled);
                    }
                },
                cx,
            ))
            .child(
                command_button("replace-current-source", ActionVariantKind::Neutral, cx)
                    .disabled(binding.is_none())
                    .child("Replace current source…")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.drawer_open = false;
                        this.begin_remote_source_replacement(binding.clone(), cx);
                    })),
            )
            .child(display_switcher_control(
                &snapshot.capture_sources,
                snapshot.source_binding.as_ref(),
                snapshot.active_capture_source,
                snapshot.pending_capture_source,
                self.show_apps_menu,
                cx,
            ))
            .child(
                crate::desktop::session_recovery::ConnectionRecoveryControls::new(
                    self.owner.session_tabs(),
                    {
                        let view = cx.weak_entity();
                        move |action, cx| {
                            let _ = view
                                .update(cx, |this, cx| this.dispatch_session_action(action, cx));
                        }
                    },
                ),
            )
            .child(div().text_size(px(11.)).child(self.status.clone()))
            .child(
                command_button("island_disconnect", ActionVariantKind::Danger, cx)
                    .debug_selector(||"end-current-stream".into())
                    .child("End this stream")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.end_stream_window(cx)
                    })),
            );
        #[cfg(any(unix, windows))]
        if let Some(device) = self.stream_device.clone() {
            body = body.child(
                command_button("send-back-to-device", ActionVariantKind::Neutral, cx)
                    .child("Send a local screen to this device…")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.begin_delivery_selection(device.clone(), cx)
                    })),
            );
        }
        body
    }
}
