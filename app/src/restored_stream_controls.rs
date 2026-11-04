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

    pub(super) fn stream_toolbar(
        &self,
        snapshot: &DashboardSnapshot,
        fullscreen: bool,
        compact: bool,
        input_hint: Option<&str>,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let visibility = control_island_visibility(snapshot.role.kind());
        let mut row = div()
            .id("floating_toolbar_pill")
            .occlude()
            .flex()
            .items_center()
            .gap_1()
            .px_2()
            .py_2()
            .rounded_full()
            .bg(color_glass_card())
            .border_1()
            .border_color(color_border_fine())
            .shadow_lg()
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                this.toolbar_hovered = *hovered;
                this.last_toolbar_activity = Instant::now();
                cx.notify();
            }))
            .on_mouse_move(cx.listener(|this, _: &MouseMoveEvent, _, _| {
                this.last_toolbar_activity = Instant::now()
            }))
            .on_any_mouse_down(cx.listener(|_, _, _, cx| cx.stop_propagation()));
        let source = confirmed_source(&snapshot.capture_sources, snapshot.active_capture_source);
        let title = self
            .stream_device
            .as_deref()
            .map(|id| self.device_label(id))
            .unwrap_or_else(|| "Weavelo".into());
        row = row.child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .px_2()
                .min_w_0()
                .when_some(source, |row, source| {
                    row.child(capture_source_badge(
                        source,
                        &snapshot.source_icons,
                        &cx.theme(),
                    ))
                })
                .child(
                    div()
                        .size(px(6.))
                        .rounded_full()
                        .bg(if visibility.show_disconnect {
                            Hsla::from(color_accent_emerald())
                        } else {
                            cx.theme().content.disabled
                        }),
                )
                .child(
                    div()
                        .id("stream-device-title")
                        .truncate()
                        .max_w(px(if compact { 110. } else { 190. }))
                        .text_size(px(12.))
                        .tooltip(tooltip(self.native_window_title.clone()).build())
                        .child(format!("{title} ↓")),
                ),
        );
        for (panel, label, symbol) in [
            (StreamPanel::Control, "Control", IconName::Control),
            (StreamPanel::Files, "Files", IconName::Files),
            (StreamPanel::Sound, "Sound", IconName::Sound),
            (StreamPanel::Display, "Display", IconName::Display),
            (StreamPanel::More, "More", IconName::More),
        ] {
            row = row.child(
                command_button(
                    format!("stream-panel-{panel:?}"),
                    if self.drawer_open && self.stream_panel == panel {
                        ActionVariantKind::Primary
                    } else {
                        ActionVariantKind::Neutral
                    },
                    cx,
                )
                .h(px(28.))
                .px_2()
                .gap_1()
                .tooltip(
                    tooltip(
                        (if panel == StreamPanel::Control {
                            input_hint.unwrap_or(label)
                        } else {
                            label
                        })
                        .to_owned(),
                    )
                    .build(),
                )
                .child(icon(symbol).size(px(14.)))
                .when(!compact, |button| button.child(label))
                .on_click(cx.listener(move |this, _, _, cx| this.open_stream_panel(panel, cx))),
            );
            if panel == StreamPanel::Files {
                let binding = snapshot.source_binding.clone();
                row = row.child(
                    command_button("stream-new-source", ActionVariantKind::Neutral, cx)
                        .h(px(28.))
                        .px_2()
                        .gap_1()
                        .disabled(binding.is_none())
                        .tooltip(tooltip("Open another screen or app window").build())
                        .child(icon(IconName::NewView).size(px(14.)))
                        .when(!compact, |button| button.child("New view"))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.drawer_open = false;
                            this.begin_remote_source_selection(binding.clone(), cx);
                        })),
                );
                if let Some(target) = snapshot
                    .capture_sources
                    .iter()
                    .find(|item| item.source == snapshot.active_capture_source)
                    .and_then(|item| {
                        snapshot
                            .capture_sources
                            .iter()
                            .find(|candidate| item.camera_switch_target(candidate))
                    })
                    .cloned()
                {
                    let binding = snapshot.source_binding.clone();
                    row = row.child(
                        command_button("stream-switch-camera", ActionVariantKind::Neutral, cx)
                            .size(px(28.))
                            .p_0()
                            .disabled(snapshot.pending_capture_source.is_some())
                            .tooltip(tooltip("Switch camera").build())
                            .child(icon(IconName::CameraSwitch).size(px(14.)))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.start_capture_source_switch(
                                    binding.clone(),
                                    target.clone(),
                                    target.title.clone(),
                                    cx,
                                );
                            })),
                    );
                }
            }
        }
        row = row.child(
                command_button("island_toggle_fullscreen", ActionVariantKind::Neutral, cx)
                    .size(px(28.))
                    .p_0()
                    .tooltip(tooltip(if fullscreen { "Exit fullscreen · F11" } else { "Fullscreen · F11" }).build())
                    .child(icon(IconName::Maximize(fullscreen)).size(px(13.)))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.owner.release_input();
                        window.toggle_fullscreen();
                        cx.refresh_windows();
                    })),
            );
        div()
            .id("floating_auto_hide_toolbar_container")
            .absolute()
            .top(px(12.))
            .left_0()
            .right_0()
            .flex()
            .flex_col()
            .items_center()
            .gap_2()
            .child(row)
            .when(self.show_apps_menu, |tree| {
                tree.child(capture_source_menu(
                    &snapshot.capture_sources,
                    &snapshot.source_icons,
                    snapshot.source_binding.as_ref(),
                    snapshot.active_capture_source,
                    snapshot.pending_capture_source,
                    snapshot.capture_source_error.as_deref(),
                    snapshot.role.session(),
                    cx,
                ))
            })
    }

    pub(super) fn stream_controls_panel(
        &self,
        snapshot: &DashboardSnapshot,
        host_stats: Option<&UiHostStats>,
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
        div()
            .id("stream-local-panel")
            .occlude()
            .absolute()
            .top(px(62.))
            .right(px(12.))
            .bottom(px(12.))
            .w(px(370.))
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
                    .p_3()
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
                    .p_3()
                    .child(body),
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
