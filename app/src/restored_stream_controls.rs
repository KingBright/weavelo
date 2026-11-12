//! Device-bound controls and file contexts. These reuse the existing session and
//! transfer owners; a window is only a presentation of the retained work.
use super::*;

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
        if self.purpose != WindowPurpose::Management && self.stream_device.as_ref() != Some(&device)
        {
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
            if self.owner.supports_clipboard_sync() {
                panel = panel.child(clipboard_quick_panel(self.owner.clone(), cx));
            }
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
                        .rounded_full()
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
        let rose_ink = crate::design_system::theme_accent_rose(cx);
        let busy = context.connecting.load(Ordering::Acquire)
            || context
                .owner
                .file_transfer_snapshot()
                .iter()
                .any(|transfer| transfer.is_running());
        div()
            .flex()
            .flex_col()
            .gap_2p5()
            .child(
                div()
                    .text_size(px(12.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(crate::i18n::tr_format!(
                        "Files · {}",
                        self.device_label(&context.device)
                    )),
            )
            .when(context.connecting.load(Ordering::Acquire), |panel| {
                panel.child(crate::i18n::tr("Updating file connection…"))
            })
            .when_some(context.error.lock().unwrap().clone(), |panel, error| {
                let device = context.device.clone();
                panel
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rose_ink)
                            .child(crate::i18n::message(&error)),
                    )
                    .child(
                        command_button("retry-device-files", ActionVariantKind::Neutral, cx)
                            .rounded_full()
                            .child(crate::i18n::tr("Retry"))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.open_device_files(device.clone(), cx)
                            })),
                    )
            })
            .child(file_transfer_panel(context.owner.clone(), cx))
            .child(
                command_button("disconnect-device-files", ActionVariantKind::Neutral, cx)
                    .rounded_full()
                    .disabled(busy)
                    .child(crate::i18n::tr("Disconnect files"))
                    .on_click(move |_, _, _| disconnect.disconnect()),
            )
    }

    pub(super) fn open_stream_panel(&mut self, panel: StreamPanel, cx: &mut Context<Self>) {
        self.network_details_open = false;
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

    pub(super) fn toggle_source_menu(&mut self, cx: &mut Context<Self>) {
        let Some(binding) = self.owner.source_view_binding() else {
            return;
        };
        if !self.owner.source_binding_is_current(&binding) {
            return;
        }
        self.reset_pointer_scroll();
        self.owner.release_input();
        self.pointer_input = PointerInputTracker::default();
        self.show_apps_menu = !self.show_apps_menu;
        if self.show_apps_menu {
            self.network_details_open = false;
            self.drawer_open = false;
        }
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
            || self.network_details_open
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
            self.status =
                "Remote navigation is waiting for an unlocked, current painted source frame."
                    .into();
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
            Some(crate::i18n::tr("This source is view-only."))
        } else if !current_target {
            Some(crate::i18n::tr(
                "Waiting for an unlocked, current painted source frame.",
            ))
        } else {
            None
        };
        let back_tooltip = if back.is_none() {
            crate::i18n::tr("Back is unavailable for this source.")
        } else if let Some(reason) = disabled_reason {
            reason
        } else {
            crate::i18n::tr("Android Back")
        };
        let home_tooltip = if home.is_none()
            && matches!(
                snapshot.active_capture_source,
                protocol::session::CaptureSource::Window(_)
            ) {
            crate::i18n::tr("Home is unavailable for a managed app window.")
        } else if home.is_none() {
            crate::i18n::tr("Home is unavailable for this source.")
        } else if let Some(reason) = disabled_reason {
            reason
        } else {
            crate::i18n::tr("Android Home")
        };
        let back_view = cx.weak_entity();
        let home_view = cx.weak_entity();
        let fit_view = cx.weak_entity();
        let theme = cx.theme().clone();
        let companion_bg = if fullscreen {
            crate::design_system::theme_glass_floating(cx)
        } else {
            theme.surface.canvas
        };
        div()
            .id("android-navigation-gutter")
            .occlude()
            .w(px(window_fit::ANDROID_COMPANION_WIDTH_PX))
            .py_1()
            .rounded_full()
            .flex_none()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_1()
            .bg(companion_bg)
            .when(fullscreen, |gutter| {
                gutter
                    .border_1()
                    .border_color(crate::design_system::theme_border_fine(cx))
            })
            .on_any_mouse_down(cx.listener(|_, _, _, cx| cx.stop_propagation()))
            .child(
                command_button("android-navigation-back", ActionVariantKind::Neutral, cx)
                    .w(px(window_fit::ANDROID_COMPANION_WIDTH_PX))
                    .h(px(40.))
                    .px_0()
                    .rounded_full()
                    .disabled(back.is_none() || !current_target)
                    .tooltip(tooltip(back_tooltip).build())
                    .on_click(move |_, _, cx| {
                        let _ = back_view
                            .update(cx, |this, cx| this.send_android_navigation("back", cx));
                    })
                    .child(icon(IconName::Back).size(px(18.))),
            )
            .child(
                command_button("android-navigation-home", ActionVariantKind::Neutral, cx)
                    .w(px(window_fit::ANDROID_COMPANION_WIDTH_PX))
                    .h(px(40.))
                    .px_0()
                    .rounded_full()
                    .disabled(home.is_none() || !current_target)
                    .tooltip(tooltip(home_tooltip).build())
                    .on_click(move |_, _, cx| {
                        let _ = home_view
                            .update(cx, |this, cx| this.send_android_navigation("home", cx));
                    })
                    .child(icon(IconName::Home).size(px(18.))),
            )
            .child(
                command_button("android-navigation-fit", ActionVariantKind::Neutral, cx)
                    .w(px(window_fit::ANDROID_COMPANION_WIDTH_PX))
                    .h(px(40.))
                    .px_0()
                    .rounded_full()
                    .tooltip(
                        tooltip(crate::i18n::tr("Fit the remote picture to the viewer")).build(),
                    )
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
        let cua_mode = self.owner.snapshot().computer_use_status.mode;
        let rail_color = match cua_mode {
            Some(crate::desktop::original_owner::ComputerUseMode::Controlling) => {
                crate::design_system::theme_online_dot(cx)
            }
            Some(crate::desktop::original_owner::ComputerUseMode::Observing) => {
                crate::design_system::theme_accent_cyan(cx)
            }
            _ => cx.theme().action.primary.bg,
        };
        let rail_border = crate::design_system::theme_border_fine(cx);
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
                    .w(px(window_fit::FULLSCREEN_RAIL_WIDTH_PX + 24.0))
                    .h(px(window_fit::FULLSCREEN_RAIL_HIT_HEIGHT_PX))
                    .flex()
                    .justify_center()
                    .pt(px(6.))
                    .cursor_pointer()
                    .tooltip(tooltip(crate::i18n::tr("Show controls")).build())
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
                            .bg(rail_color)
                            .border_1()
                            .border_color(rail_border)
                            .shadow_sm(),
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
        let floating_bg = crate::design_system::theme_glass_floating(cx);
        let floating_border = crate::design_system::theme_border_fine(cx);
        let live_dot = crate::design_system::theme_online_dot(cx);
        let source = confirmed_source(&snapshot.capture_sources, snapshot.active_capture_source);
        let device = self
            .stream_device
            .as_deref()
            .map(|id| self.device_label(id))
            .unwrap_or_else(|| "Weavelo".into());
        let source_label = source
            .map(capture_source_label)
            .unwrap_or_else(|| crate::i18n::tr("Waiting for source").into());
        let context_title = crate::i18n::tr_format!("{} · {}", device, source_label);
        let button_size = if tiny { 20.0 } else { 26.0 };
        let source_binding = snapshot.source_binding.clone();
        let can_switch_source = source_binding.is_some();
        let toolbar_stats = self.host_stats_snapshot(&snapshot.role);
        let cua_mode = snapshot.computer_use_status.mode;
        let cua_label = snapshot
            .computer_use_status
            .status_label
            .clone()
            .unwrap_or_else(|| match cua_mode {
                Some(crate::desktop::original_owner::ComputerUseMode::Observing) => {
                    crate::i18n::tr("AI Observing screen").into()
                }
                Some(crate::desktop::original_owner::ComputerUseMode::Controlling) => {
                    crate::i18n::tr("AI Controlling — Move mouse or press Esc to take over").into()
                }
                Some(crate::desktop::original_owner::ComputerUseMode::PausedByHuman) => {
                    crate::i18n::tr("AI Paused — Human took over input").into()
                }
                None => String::new(),
            });
        let mut row = div()
            .id(if fullscreen {
                "fullscreen_toolbar_pill"
            } else {
                "stream_header"
            })
            .occlude()
            .flex()
            .items_center()
            .gap_1()
            .when(fullscreen, |row| {
                row.px_2p5()
                    .py_1()
                    .rounded_full()
                    .bg(floating_bg)
                    .border_1()
                    .border_color(floating_border)
                    .shadow_lg()
            })
            .when(!fullscreen, |row| {
                row.w_full()
                    .h(px(window_fit::STREAM_HEADER_HEIGHT_PX))
                    .px_2()
                    .bg(theme.surface.base)
                    .border_b_1()
                    .border_color(theme.border.divider)
            })
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                this.toolbar_hovered = *hovered;
                this.last_toolbar_activity = Instant::now();
                cx.notify();
            }))
            .on_mouse_move(cx.listener(|this, _: &MouseMoveEvent, _, _| {
                this.last_toolbar_activity = Instant::now()
            }))
            .on_any_mouse_down(cx.listener(|_, _, _, cx| cx.stop_propagation()));
        if cfg!(target_os = "macos") && !fullscreen && !tiny {
            row = row.child(
                div()
                    .id("macos-traffic-lights-spacer")
                    .w(px(64.))
                    .h_full()
                    .flex_none()
                    .on_mouse_down(MouseButton::Left, |_, window, _| {
                        if !cfg!(test) {
                            window.start_window_move();
                        }
                    }),
            );
        }
        row = row.when(!tiny, |row| {
            row.child(
                div()
                    .id("stream-header-context")
                    .flex()
                    .flex_initial()
                    .max_w(px(if compact { 240. } else { 360. }))
                    .min_w_0()
                    .items_center()
                    .gap_1p5()
                    .px_1p5()
                    .py_0p5()
                    .rounded_full()
                    .when(can_switch_source, |ctx| {
                        ctx.cursor_pointer()
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_source_menu(cx)))
                    })
                    .when_some(source, |row, source| {
                        row.child(capture_source_badge(source, &snapshot.source_icons, &theme))
                    })
                    .child(div().size(px(6.)).flex_none().rounded_full().bg(
                        if visibility.show_disconnect {
                            live_dot
                        } else {
                            theme.content.disabled
                        },
                    ))
                    .child(
                        div()
                            .id("stream-device-title")
                            .min_w_0()
                            .truncate()
                            .text_size(px(12.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme.content.primary)
                            .tooltip(
                                tooltip({
                                    let mut full = context_title.clone();
                                    if let Some(route) = snapshot.route.as_deref() {
                                        full = format!("{full} · {}", crate::i18n::message(route));
                                    }
                                    if let Some(hint) = input_hint {
                                        full = format!("{full} · {hint}");
                                    }
                                    full
                                })
                                .build(),
                            )
                            .child(context_title.clone()),
                    ),
            )
        });
        if !fullscreen {
            row = row.child(
                div()
                    .id("stream-titlebar-drag-region")
                    .flex_1()
                    .h_full()
                    .min_w(px(12.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .on_mouse_down(MouseButton::Left, |event, window, _| {
                        if event.click_count >= 2 {
                            window.titlebar_double_click();
                        } else if !cfg!(test) {
                            window.start_window_move();
                        }
                    })
                    .when_some((!tiny).then_some(cua_mode).flatten(), |drag, mode| {
                        let badge_color = match mode {
                            crate::desktop::original_owner::ComputerUseMode::Observing => {
                                crate::design_system::theme_accent_cyan(cx)
                            }
                            crate::desktop::original_owner::ComputerUseMode::Controlling => {
                                crate::design_system::theme_online_dot(cx)
                            }
                            crate::desktop::original_owner::ComputerUseMode::PausedByHuman => {
                                cx.theme().content.secondary
                            }
                        };
                        drag.child(
                            div()
                                .id("computer-use-status-capsule")
                                .occlude()
                                .h(px(22.))
                                .flex()
                                .items_center()
                                .gap_1p5()
                                .px_2()
                                .rounded_full()
                                .bg(theme.surface.sunken)
                                .border_1()
                                .border_color(badge_color)
                                .tooltip(tooltip(crate::i18n::message(&cua_label)).build())
                                .on_any_mouse_down(
                                    cx.listener(|_, _, _, cx| cx.stop_propagation()),
                                )
                                .child(div().size(px(6.)).rounded_full().bg(badge_color))
                                .child(
                                    div()
                                        .text_size(px(10.5))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(crate::i18n::tr("AI")),
                                )
                                .child(
                                    command_button(
                                        "computer-use-dismiss-pill",
                                        ActionVariantKind::Neutral,
                                        cx,
                                    )
                                    .h(px(18.))
                                    .px_1p5()
                                    .rounded_full()
                                    .text_size(px(10.))
                                    .child(
                                        if mode
                                            == crate::desktop::original_owner::ComputerUseMode::PausedByHuman
                                        {
                                            crate::i18n::tr("Dismiss")
                                        } else {
                                            crate::i18n::tr("Pause AI")
                                        },
                                    )
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.owner.release_input();
                                        this.owner.set_computer_use_active(None, None);
                                        cx.notify();
                                    })),
                                ),
                        )
                    }),
            );
        }
        let signal_button = self.network_status_chip(
            &snapshot.role,
            snapshot.route.as_deref(),
            toolbar_stats.as_ref(),
            tiny,
            cx,
        );
        let android_inline =
            !fullscreen && !tiny && has_android_mediaprojection_source(&snapshot.capture_sources);
        let control_caption = input_control_caption(
            self.input_locked,
            snapshot.active_capture_supports_input,
            snapshot.input_eligible,
            compact,
        );
        let control_button = command_button(
            "stream-primary-control",
            if !self.input_locked && snapshot.active_capture_supports_input {
                ActionVariantKind::Primary
            } else {
                ActionVariantKind::Neutral
            },
            cx,
        )
        .size(px(button_size))
        .rounded_full()
        .p_0()
        .tooltip(tooltip(crate::i18n::tr(control_caption)).build())
        .child(icon(IconName::Control).size(px(if tiny {
            12.
        } else if compact {
            13.
        } else {
            14.
        })))
        .disabled(!snapshot.active_capture_supports_input)
        .on_click(cx.listener(|this, _, window, cx| {
            this.set_input_locked(!this.input_locked);
            if !this.input_locked {
                this.input_focus.focus(window);
            }
            this.last_toolbar_activity = Instant::now();
            cx.notify();
        }));
        let source_button = command_button(
            "stream-primary-source",
            if self.show_apps_menu {
                ActionVariantKind::Primary
            } else {
                ActionVariantKind::Neutral
            },
            cx,
        )
        .size(px(button_size))
        .rounded_full()
        .p_0()
        .tooltip(tooltip(crate::i18n::tr("Switch source or open a new view")).build())
        .child(icon(IconName::NewView).size(px(if tiny {
            12.
        } else if compact {
            13.
        } else {
            14.
        })))
        .disabled(source_binding.is_none())
        .on_click(cx.listener(|this, _, _, cx| this.toggle_source_menu(cx)));
        let (volume, muted) = self.owner.audio();
        let mute_button = command_button(
            "stream-primary-mute",
            if muted {
                ActionVariantKind::Danger
            } else {
                ActionVariantKind::Neutral
            },
            cx,
        )
        .size(px(button_size))
        .rounded_full()
        .p_0()
        .tooltip(tooltip(crate::i18n::tr(if muted { "Unmute" } else { "Mute" })).build())
        .child(
            icon(if muted {
                IconName::SoundMuted
            } else {
                IconName::Sound
            })
            .size(px(if tiny {
                12.
            } else if compact {
                13.
            } else {
                14.
            })),
        )
        .on_click(cx.listener(move |this, _, _, cx| {
            this.owner.set_audio(volume, !muted);
            this.last_toolbar_activity = Instant::now();
            cx.notify();
        }));
        let fit_button = command_button("stream-primary-fit", ActionVariantKind::Neutral, cx)
            .size(px(button_size))
            .rounded_full()
            .p_0()
            .tooltip(tooltip(crate::i18n::tr("Fit window")).build())
            .child(icon(IconName::Window).size(px(if tiny {
                12.
            } else if compact {
                13.
            } else {
                14.
            })))
            .on_click(cx.listener(|this, _, _, cx| {
                this.window_fit.request_fit();
                this.last_toolbar_activity = Instant::now();
                cx.notify();
            }));
        let files_button = command_button("stream-primary-files", ActionVariantKind::Neutral, cx)
            .size(px(button_size))
            .rounded_full()
            .p_0()
            .tooltip(tooltip(crate::i18n::tr("Files")).build())
            .child(icon(IconName::Files).size(px(if tiny {
                12.
            } else if compact {
                13.
            } else {
                14.
            })))
            .disabled(self.stream_device.is_none())
            .on_click(cx.listener(|this, _, _, cx| this.open_stream_panel(StreamPanel::Files, cx)));
        let more_button = command_button(
            "stream-primary-more",
            if self.drawer_open {
                ActionVariantKind::Primary
            } else {
                ActionVariantKind::Neutral
            },
            cx,
        )
        .size(px(button_size))
        .rounded_full()
        .p_0()
        .tooltip(
            tooltip(crate::i18n::tr(
                "Control, sound, display and session options",
            ))
            .build(),
        )
        .child(icon(IconName::More).size(px(if tiny {
            12.
        } else if compact {
            13.
        } else {
            14.
        })))
        .on_click(cx.listener(|this, _, _, cx| this.open_stream_panel(StreamPanel::More, cx)));
        let fullscreen_button =
            command_button("stream-primary-fullscreen", ActionVariantKind::Neutral, cx)
                .size(px(button_size))
                .rounded_full()
                .p_0()
                .tooltip(
                    tooltip(crate::i18n::tr(if fullscreen {
                        "Exit fullscreen · F11"
                    } else {
                        "Fullscreen · F11"
                    }))
                    .build(),
                )
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
        row = row.child(signal_button);
        if android_inline {
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
            let back_view = cx.weak_entity();
            let home_view = cx.weak_entity();
            row = row
                .child(
                    command_button("titlebar-android-back", ActionVariantKind::Neutral, cx)
                        .size(px(button_size))
                        .rounded_full()
                        .p_0()
                        .disabled(back.is_none() || !current_target)
                        .tooltip(tooltip(crate::i18n::tr("Android Back")).build())
                        .on_click(move |_, _, cx| {
                            let _ = back_view
                                .update(cx, |this, cx| this.send_android_navigation("back", cx));
                        })
                        .child(icon(IconName::Back).size(px(13.))),
                )
                .child(
                    command_button("titlebar-android-home", ActionVariantKind::Neutral, cx)
                        .size(px(button_size))
                        .rounded_full()
                        .p_0()
                        .disabled(home.is_none() || !current_target)
                        .tooltip(tooltip(crate::i18n::tr("Android Home")).build())
                        .on_click(move |_, _, cx| {
                            let _ = home_view
                                .update(cx, |this, cx| this.send_android_navigation("home", cx));
                        })
                        .child(icon(IconName::Home).size(px(13.))),
                );
        }
        row = row
            .child(control_button)
            .child(source_button)
            .when(!tiny, |r| r.child(mute_button).child(fit_button))
            .child(files_button)
            .child(more_button)
            .child(fullscreen_button);
        let mut container = div()
            .id(if fullscreen {
                "fullscreen-controls-container"
            } else {
                "stream-header-container"
            })
            .flex()
            .flex_col()
            .gap_1()
            .when(fullscreen, |tree| {
                tree.absolute()
                    .top(px(8.))
                    .left_0()
                    .right_0()
                    .items_center()
            })
            .when(!fullscreen, |tree| tree.w_full().flex_none());
        container = container.child(row).when_some(
            fullscreen.then_some(cua_mode).flatten(),
            |tree, mode| {
                let badge_color = match mode {
                    crate::desktop::original_owner::ComputerUseMode::Observing => {
                        crate::design_system::theme_accent_cyan(cx)
                    }
                    crate::desktop::original_owner::ComputerUseMode::Controlling => {
                        crate::design_system::theme_online_dot(cx)
                    }
                    crate::desktop::original_owner::ComputerUseMode::PausedByHuman => {
                        cx.theme().content.secondary
                    }
                };
                tree.child(
                    div()
                        .id("computer-use-status-capsule")
                        .occlude()
                        .h(px(26.))
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_3()
                        .py_1()
                        .rounded_full()
                        .bg(floating_bg)
                        .border_1()
                        .border_color(badge_color)
                        .shadow_md()
                        .child(div().size(px(8.)).rounded_full().bg(badge_color))
                        .child(div().text_size(px(11.)).child(crate::i18n::message(&cua_label)))
                        .child(
                            command_button(
                                "computer-use-dismiss-pill",
                                ActionVariantKind::Neutral,
                                cx,
                            )
                            .h(px(20.))
                            .px_2()
                            .rounded_full()
                            .child(
                                if mode
                                    == crate::desktop::original_owner::ComputerUseMode::PausedByHuman
                                {
                                    crate::i18n::tr("Dismiss")
                                } else {
                                    crate::i18n::tr("Pause AI")
                                },
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.owner.release_input();
                                this.owner.set_computer_use_active(None, None);
                                cx.notify();
                            })),
                        ),
                )
            },
        );
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
            StreamPanel::Control => crate::i18n::tr("Control"),
            StreamPanel::Files => crate::i18n::tr("Files"),
            StreamPanel::Sound => crate::i18n::tr("Sound"),
            StreamPanel::Display => crate::i18n::tr("Display"),
            StreamPanel::More => crate::i18n::tr("More"),
        };
        let body = match self.stream_panel {
            StreamPanel::Control => self.stream_input_panel(snapshot, cx),
            StreamPanel::Files => self
                .stream_device
                .as_deref()
                .map(|device| self.device_files_panel(&self.device_files(device), cx))
                .unwrap_or_else(|| div().child(crate::i18n::tr("Choose a device"))),
            StreamPanel::Sound => self.stream_sound_panel(cx),
            StreamPanel::Display => div()
                .flex()
                .flex_col()
                .gap_3()
                .child(scale_mode_control(self.scale_mode, cx))
                .when_some(self.viewport_controls(cx), |panel, controls| {
                    panel.child(controls)
                })
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
                        .rounded_full()
                        .child(crate::i18n::tr("Fit window"))
                        .tooltip(
                            tooltip(crate::i18n::tr(
                                "Resize this window to fit the source without leaving the display",
                            ))
                            .build(),
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.window_fit.request_fit();
                            cx.notify();
                        })),
                )
                .child(
                    command_button("island_popout_pip", ActionVariantKind::Neutral, cx)
                        .rounded_full()
                        .child(crate::i18n::tr("Open picture in picture"))
                        .on_click(cx.listener(|this, _, _, cx| this.open_popout_pip_window(cx))),
                )
                .child(
                    if matches!(
                        snapshot.active_capture_source,
                        protocol::session::CaptureSource::Window(_)
                    ) {
                        self.window_stream_quality_panel(
                            snapshot,
                            host_stats,
                            viewport_width,
                            viewport_height,
                            cx,
                        )
                    } else {
                        drawer_network_telemetry_tab(
                            self.stream_settings.values().resolution(),
                            self.stream_settings.values().fps,
                            self.stream_settings.values().bitrate_kbps,
                            self.media_controls(cx),
                            host_stats,
                            self.owner.clone(),
                            false,
                            self.pipeline_diagnostics_open,
                            cx,
                        )
                    },
                ),
            StreamPanel::More => self.stream_more_panel(snapshot, cx),
        };
        let compact_panel = viewport_width < 420.0 || viewport_height < 280.0;
        let top = if viewport_height < 280.0 {
            8.0
        } else {
            window_fit::STREAM_HEADER_HEIGHT_PX + 6.0
        };
        let bottom = if viewport_height < 280.0 { 8.0 } else { 12.0 };
        let panel_theme = cx.theme().clone();
        let mut tabs = crate::product_components::segmented_track(&panel_theme).flex_wrap();
        for (panel, label, symbol) in [
            (StreamPanel::Control, "Control", IconName::Control),
            (StreamPanel::Sound, "Sound", IconName::Sound),
            (StreamPanel::Display, "Display", IconName::Display),
            (StreamPanel::More, "More", IconName::More),
        ] {
            let hint = if panel == StreamPanel::Control {
                crate::i18n::message(&snapshot.input_status)
            } else {
                crate::i18n::tr(label).to_owned()
            };
            let is_active = self.stream_panel == panel;
            let btn_variant = if is_active {
                ely_gpui_component::buttons::ButtonVariant::Primary
            } else {
                ely_gpui_component::buttons::ButtonVariant::Ghost
            };
            tabs = tabs.child(
                ely_gpui_component::buttons::Button::new(format!("stream-panel-tab-{panel:?}"), "")
                    .variant(btn_variant)
                    .control_size(ely_gpui_component::theme::ControlSize::Sm)
                    .rounded(px(7.))
                    .when(compact_panel, |button| button.size(px(26.)).p_0())
                    .when(!compact_panel, |button| button.h(px(26.)).px_2p5().gap_1())
                    .tooltip(tooltip(hint.to_owned()).build())
                    .child(icon(symbol).size(px(13.)))
                    .when(!compact_panel, |button| {
                        button.child(crate::i18n::tr(label))
                    })
                    .on_click(cx.listener(move |this, _, _, cx| this.open_stream_panel(panel, cx))),
            );
        }
        let panel_bg = crate::design_system::theme_glass_card(cx);
        let panel_border = crate::design_system::theme_border_fine(cx);
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
            .rounded(px(14.))
            .bg(panel_bg)
            .border_1()
            .border_color(panel_border)
            .shadow_lg()
            .on_any_mouse_down(cx.listener(|_, _, _, cx| cx.stop_propagation()))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .border_b_1()
                    .border_color(panel_theme.border.divider)
                    .when(compact_panel, |header| header.px_2p5().py_1p5())
                    .when(!compact_panel, |header| header.px_3p5().py_2p5())
                    .child(
                        div()
                            .text_size(px(13.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(title),
                    )
                    .child(
                        command_button("close-stream-panel", ActionVariantKind::Neutral, cx)
                            .size(px(24.))
                            .rounded(px(7.))
                            .p_0()
                            .child(icon(IconName::Close).size(px(13.)))
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
                    .child(div().flex().flex_col().gap_2p5().child(tabs).child(body)),
            )
    }

    fn stream_input_panel(&self, snapshot: &DashboardSnapshot, cx: &mut Context<Self>) -> Div {
        div().flex().flex_col().gap_3()
            .child(command_button("island_toggle_input_lock", if self.input_locked { ActionVariantKind::Neutral } else { ActionVariantKind::Primary }, cx)
                .rounded_full()
                .disabled(!snapshot.active_capture_supports_input)
                .child(input_control_caption(self.input_locked, snapshot.active_capture_supports_input, snapshot.input_eligible, false))
                .on_click(cx.listener(|this, _, _, cx| { this.set_input_locked(!this.input_locked); cx.notify(); })))
                .child(div().text_size(px(11.)).child(crate::i18n::message(&snapshot.input_status)))
            .child(command_button("island_keyboard_mode", ActionVariantKind::Neutral, cx)
                .rounded_full()
                .child(crate::i18n::tr(if self.text_mode { "Text · local input method" } else { "Keys · physical keyboard" }))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.owner.release_input(); this.remote_text = None; this.text_mode = !this.text_mode; cx.notify();
                })))
            .child(div().text_size(px(11.)).text_color(cx.theme().content.secondary)
                .child(crate::i18n::tr("Text uses your local input method. Keys forwards physical keys. Ctrl+Alt+Esc releases control.")))
    }

    fn stream_sound_panel(&self, cx: &mut Context<Self>) -> Div {
        use protocol::audio_output::{AudioOutputMode, LocalPlayback};
        let (volume, muted) = self.owner.audio();
        let host_audio = self.owner.host_audio();
        let sound_theme = cx.theme().clone();
        let header = if muted {
            crate::i18n::tr_format!("Playback · {volume}% (Muted)", volume = volume)
        } else {
            crate::i18n::tr_format!("Playback · {volume}%", volume = volume)
        };
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(div().font_weight(FontWeight::MEDIUM).child(header))
            .child(
                crate::product_components::segmented_track(&sound_theme)
                    .flex_wrap()
                    .children([0u8, 25, 50, 75, 100].into_iter().map(|preset| {
                        let active = (!muted && volume == preset) || (muted && preset == 0);
                        let variant = if active {
                            ely_gpui_component::buttons::ButtonVariant::Primary
                        } else {
                            ely_gpui_component::buttons::ButtonVariant::Ghost
                        };
                        ely_gpui_component::buttons::Button::new(
                            format!("volume_preset_{preset}"),
                            format!("{preset}%"),
                        )
                        .variant(variant)
                        .control_size(ely_gpui_component::theme::ControlSize::Sm)
                        .h(px(24.))
                        .px_2p5()
                        .rounded(px(7.))
                        .text_size(px(10.5))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.owner.set_audio(preset, preset == 0);
                            cx.notify();
                        }))
                    })),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        command_button("volume_down", ActionVariantKind::Neutral, cx)
                            .rounded_full()
                            .child("−")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.owner.set_audio(volume.saturating_sub(5), muted);
                                cx.notify();
                            })),
                    )
                    .child(
                        command_button("volume_up", ActionVariantKind::Neutral, cx)
                            .rounded_full()
                            .child("+")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.owner
                                    .set_audio(volume.saturating_add(5).min(100), false);
                                cx.notify();
                            })),
                    )
                    .child(
                        command_button(
                            "remote_audio_mute",
                            if muted {
                                ActionVariantKind::Danger
                            } else {
                                ActionVariantKind::Neutral
                            },
                            cx,
                        )
                        .rounded_full()
                        .child(crate::i18n::tr(if muted { "Unmute" } else { "Mute" }))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.owner.set_audio(volume, !muted);
                            cx.notify();
                        })),
                    ),
            )
            .when_some(host_audio, |panel, (pending, status)| {
                let playback = status.as_ref().map(|status| status.local_playback);
                let supported = status
                    .as_ref()
                    .is_none_or(|status| status.private_supported);
                let mirror_selected = matches!(playback, None | Some(LocalPlayback::Enabled));
                let private_selected = playback == Some(LocalPlayback::Disabled);
                let caption = if pending {
                    "Changing host audio output…"
                } else {
                    match playback {
                        None | Some(LocalPlayback::Enabled) => "Play on both",
                        Some(LocalPlayback::Disabled) => "Remote only",
                        _ => "Host audio output is unconfirmed",
                    }
                };
                panel.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(
                            div()
                                .text_size(px(11.))
                                .font_weight(FontWeight::MEDIUM)
                                .child(crate::i18n::tr("Host audio output")),
                        )
                        .child(
                            crate::product_components::segmented_track(&sound_theme)
                                .child(
                                    ely_gpui_component::buttons::Button::new(
                                        "host_audio_mirror",
                                        crate::i18n::tr("Play on both"),
                                    )
                                    .variant(if mirror_selected {
                                        ely_gpui_component::buttons::ButtonVariant::Primary
                                    } else {
                                        ely_gpui_component::buttons::ButtonVariant::Ghost
                                    })
                                    .control_size(ely_gpui_component::theme::ControlSize::Sm)
                                    .h(px(26.))
                                    .rounded(px(7.))
                                    .disabled(pending || !supported || mirror_selected)
                                    .on_click(cx.listener(
                                        |this, _, _, cx| {
                                            this.owner.set_host_audio(AudioOutputMode::Mirror);
                                            cx.notify();
                                        },
                                    )),
                                )
                                .child(
                                    ely_gpui_component::buttons::Button::new(
                                        "host_audio_private",
                                        crate::i18n::tr("Remote only"),
                                    )
                                    .variant(if private_selected {
                                        ely_gpui_component::buttons::ButtonVariant::Primary
                                    } else {
                                        ely_gpui_component::buttons::ButtonVariant::Ghost
                                    })
                                    .control_size(ely_gpui_component::theme::ControlSize::Sm)
                                    .h(px(26.))
                                    .rounded(px(7.))
                                    .disabled(pending || !supported || private_selected)
                                    .on_click(cx.listener(
                                        |this, _, _, cx| {
                                            this.owner.set_host_audio(AudioOutputMode::Private);
                                            cx.notify();
                                        },
                                    )),
                                ),
                        )
                        .child(
                            div()
                                .text_size(px(10.5))
                                .text_color(sound_theme.content.secondary)
                                .child(crate::i18n::tr(caption)),
                        )
                        .when_some(status.and_then(|status| status.message), |row, message| {
                            row.child(
                                div()
                                    .text_size(px(10.5))
                                    .text_color(sound_theme.content.secondary)
                                    .child(crate::i18n::message(&message)),
                            )
                        }),
                )
            })
            .child(security_toggle_row(
                crate::i18n::tr("Microphone to this connection"),
                crate::i18n::tr("Shared by this device connection"),
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
                crate::i18n::tr("Clipboard"),
                crate::i18n::tr("Shared by this device connection"),
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
                crate::i18n::tr("Background AI Computer Use"),
                crate::i18n::tr(
                    "Allow authorized AI agents to control this window while unfocused",
                ),
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
                    .rounded_full()
                    .disabled(binding.is_none())
                    .child(crate::i18n::tr("Replace current source…"))
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
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(cx.theme().content.secondary)
                    .child(crate::i18n::message(&self.status)),
            )
            .child(
                command_button("island_disconnect", ActionVariantKind::Danger, cx)
                    .rounded_full()
                    .debug_selector(|| "end-current-stream".into())
                    .child(crate::i18n::tr("End this stream"))
                    .on_click(cx.listener(|this, _, _, cx| this.end_stream_window(cx))),
            );
        #[cfg(any(unix, windows))]
        if let Some(device) = self.stream_device.clone() {
            body = body.child(
                command_button("send-back-to-device", ActionVariantKind::Neutral, cx)
                    .rounded_full()
                    .child(crate::i18n::tr("Send a local screen to this device…"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.begin_delivery_selection(device.clone(), cx)
                    })),
            );
        }
        body
    }

    fn window_stream_quality_panel(
        &self,
        snapshot: &DashboardSnapshot,
        host_stats: Option<&UiHostStats>,
        viewport_width: f32,
        viewport_height: f32,
        cx: &mut Context<Self>,
    ) -> Div {
        let theme = cx.theme().clone();
        let view = cx.weak_entity();
        let ov = self.window_stream_override;
        let baseline = self.stream_settings.values();
        let live_source = self
            .active_window_app_identity(snapshot)
            .map(|(_, _, _, sz)| sz)
            .unwrap_or((1920, 1080));
        let encoded_cap = ov.clarity.compute_cap(live_source);
        let local_vp = {
            let w = viewport_width.round() as i64;
            let h = (viewport_height - window_fit::STREAM_HEADER_HEIGHT_PX).round() as i64;
            (w >= 120 && h >= 120).then_some((w as u32, h as u32))
        };
        let can_reset = !ov.is_default(baseline) || snapshot.window_resize_notice.is_some();

        let mut clarity_track = crate::product_components::segmented_track(&theme).flex_wrap();
        for (scale, label) in [
            (WindowClarityScale::Native100, "Native 100%"),
            (WindowClarityScale::High75, "HD 75%"),
            (WindowClarityScale::Standard50, "Std 50%"),
            (WindowClarityScale::Saver35, "Saver 35%"),
        ] {
            let active = ov.clarity == scale;
            let variant = if active {
                ely_gpui_component::buttons::ButtonVariant::Primary
            } else {
                ely_gpui_component::buttons::ButtonVariant::Ghost
            };
            clarity_track = clarity_track.child(
                ely_gpui_component::buttons::Button::new(
                    format!("win-clarity-{}", scale.percent()),
                    "",
                )
                .variant(variant)
                .control_size(ely_gpui_component::theme::ControlSize::Sm)
                .rounded(px(7.0))
                .h(px(25.0))
                .px_2p5()
                .child(crate::i18n::tr(label))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.select_window_clarity(scale, cx);
                })),
            );
        }

        let layout_options = [
            (WindowLayoutPreset::KeepOriginal, "Original size"),
            (WindowLayoutPreset::FitLocalViewport, "Fit viewport"),
            (WindowLayoutPreset::Fixed1280x800, "1280×800"),
            (WindowLayoutPreset::Fixed1440x900, "1440×900"),
            (WindowLayoutPreset::Fixed1600x1000, "1600×1000"),
            (WindowLayoutPreset::Portrait900x1440, "900×1440 Portrait"),
        ];

        let resize_feedback = match &snapshot.window_resize_notice {
            Some(Ok((w, h))) => Some((
                format!("{} {w} × {h}", crate::i18n::tr("Remote window adapted to")),
                crate::design_system::theme_accent_cyan(cx),
            )),
            Some(Err(reason)) => Some((crate::i18n::message(reason), theme.status.warning.fg)),
            None => None,
        };

        let mut content = div()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_0p5()
                            .child(
                                div()
                                    .text_size(px(12.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme.content.primary)
                                    .child(crate::i18n::tr("Window Stream & Layout")),
                            )
                            .child(
                                div()
                                    .text_size(px(10.0))
                                    .text_color(crate::design_system::theme_accent_cyan(cx))
                                    .child(crate::i18n::tr("Isolated · Global defaults untouched")),
                            ),
                    )
                    .when(can_reset, |row| {
                        row.child(
                            command_button("reset-window-stream", ActionVariantKind::Neutral, cx)
                                .h(px(23.0))
                                .px_2p5()
                                .rounded_full()
                                .text_size(px(10.0))
                                .child(crate::i18n::tr("Reset default"))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.reset_window_stream_override(cx);
                                })),
                        )
                    }),
            )
            .child(
                div()
                    .p_3p5()
                    .bg(theme.surface.base)
                    .border_1()
                    .border_color(crate::design_system::theme_border_fine(cx))
                    .rounded(px(12.0))
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1p5()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .child(
                                        div()
                                            .text_size(px(10.5))
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(theme.content.secondary)
                                            .child(crate::i18n::tr("Stream Clarity Scale")),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(10.0))
                                            .text_color(crate::design_system::theme_accent_cyan(cx))
                                            .child(format!(
                                                "{}×{} → {}×{}",
                                                live_source.0,
                                                live_source.1,
                                                encoded_cap.0,
                                                encoded_cap.1
                                            )),
                                    ),
                            )
                            .child(clarity_track),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1p5()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .child(
                                        div()
                                            .text_size(px(10.5))
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(theme.content.secondary)
                                            .child(crate::i18n::tr("Remote Window Size")),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(9.5))
                                            .text_color(theme.content.tertiary)
                                            .child(crate::i18n::tr("Auto-restores on exit")),
                                    ),
                            )
                            .child(div().flex().flex_wrap().gap_1p5().children(
                                layout_options.into_iter().map(|(preset, label)| {
                                    let active = ov.layout == preset;
                                    let variant = if active {
                                        ActionVariantKind::Primary
                                    } else {
                                        ActionVariantKind::Neutral
                                    };
                                    command_button(
                                        format!("win-layout-{}", preset.key()),
                                        variant,
                                        cx,
                                    )
                                    .h(px(24.0))
                                    .text_size(px(10.0))
                                    .child(crate::i18n::tr(label).to_owned())
                                    .on_click(cx.listener(
                                        move |this, _, _, cx| {
                                            this.select_window_layout(preset, local_vp, cx);
                                        },
                                    ))
                                }),
                            ))
                            .when_some(resize_feedback, |col, (msg, color)| {
                                col.child(div().text_size(px(10.0)).text_color(color).child(msg))
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_size(px(10.0))
                                    .text_color(theme.content.secondary)
                                    .child(crate::i18n::tr("Window Frame Rate:")),
                            )
                            .child(
                                div().flex().flex_wrap().gap_1p5().children(
                                    [
                                        (30u32, "30 FPS (Power Save)"),
                                        (60, "60 FPS (Standard)"),
                                        (120, "120 FPS (High-Hz)"),
                                    ]
                                    .into_iter()
                                    .map(|(fps, label)| {
                                        let variant = if ov.fps == fps {
                                            ActionVariantKind::Primary
                                        } else {
                                            ActionVariantKind::Neutral
                                        };
                                        command_button(format!("win_fps_{fps}"), variant, cx)
                                            .h(px(24.0))
                                            .text_size(px(10.0))
                                            .child(crate::i18n::tr(label).to_owned())
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.select_window_fps(fps, cx);
                                            }))
                                    }),
                                ),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_size(px(10.0))
                                    .text_color(theme.content.secondary)
                                    .child(crate::i18n::tr("Window Bitrate:")),
                            )
                            .child(
                                div().flex().flex_wrap().gap_1p5().children(
                                    [
                                        (2_000u32, "2 Mbps"),
                                        (5_000, "5 Mbps"),
                                        (10_000, "10 Mbps"),
                                        (20_000, "20 Mbps"),
                                        (40_000, "40 Mbps"),
                                    ]
                                    .into_iter()
                                    .map(|(kbps, label)| {
                                        let variant = if ov.bitrate_kbps == kbps {
                                            ActionVariantKind::Primary
                                        } else {
                                            ActionVariantKind::Neutral
                                        };
                                        command_button(format!("win_bitrate_{kbps}"), variant, cx)
                                            .h(px(24.0))
                                            .text_size(px(10.0))
                                            .child(crate::i18n::tr(label).to_owned())
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.select_window_bitrate(kbps, cx);
                                            }))
                                    }),
                                ),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .text_size(px(11.0))
                            .child(crate::i18n::tr("Custom window FPS and bitrate (kbps)")),
                    )
                    .child(
                        component::text_input("win_custom_fps")
                            .content(self.window_fps_draft.clone())
                            .on_change({
                                let view = view.clone();
                                move |text, _, cx| {
                                    let _ = view.update(cx, |this, cx| {
                                        this.window_fps_draft = text.to_string();
                                        cx.notify();
                                    });
                                }
                            }),
                    )
                    .child(
                        component::text_input("win_custom_bitrate")
                            .content(self.window_bitrate_draft.clone())
                            .on_change({
                                let view = view.clone();
                                move |text, _, cx| {
                                    let _ = view.update(cx, |this, cx| {
                                        this.window_bitrate_draft = text.to_string();
                                        cx.notify();
                                    });
                                }
                            }),
                    )
                    .child(
                        command_button("apply_win_custom_stream", ActionVariantKind::Primary, cx)
                            .child(crate::i18n::tr("Apply custom values"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.apply_window_custom_rates(cx);
                            })),
                    )
                    .when_some(self.window_settings_error.as_ref(), |col, err| {
                        col.child(
                            div()
                                .text_size(px(10.5))
                                .text_color(theme.status.error.fg)
                                .child(err.clone()),
                        )
                    })
                    .child(
                        command_button("toggle_win_media_pause", ActionVariantKind::Neutral, cx)
                            .child(if self.manual_media_paused {
                                crate::i18n::tr("Resume media")
                            } else {
                                crate::i18n::tr("Pause media · keep connected")
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.set_stream_paused(!this.manual_media_paused, cx);
                            })),
                    ),
            );

        if let Some(stats) = host_stats {
            content = content
                .child(
                    div()
                        .pt_1()
                        .text_size(px(11.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme.content.primary)
                        .child(crate::i18n::tr("Live Transport & Latency Telemetry")),
                )
                .child(
                    telemetry_metric_row(
                        "Capture → receive (est.)",
                        stats.capture_receive.value(),
                        crate::design_system::theme_accent_cyan(cx),
                        cx,
                    )
                    .id("win_capture_receive_metric")
                    .tooltip(tooltip(stats.capture_receive.detail()).build()),
                )
                .child(telemetry_metric_row(
                    "Video frame rate",
                    format!("{:.1} fps", stats.fps),
                    crate::design_system::theme_online_dot(cx),
                    cx,
                ));
        }
        content
    }
}
