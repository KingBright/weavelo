//! A sending window presents one retained delivery. Closing it never drops the
//! sender; explicit stop removes only its captured request from the coordinator.
use super::*;
use crate::desktop::directed_send::SendProgress;
use protocol::session::{CaptureSource, CaptureSourceInfo};

#[derive(Clone, serde::Serialize)]
pub(super) struct SendActivity {
    pub request_id: String,
    pub target: String,
    pub target_name: String,
    pub source: String,
    pub status: String,
    pub finished: bool,
}

#[derive(Clone)]
pub(super) struct SendingContext {
    request: String,
    target: String,
    sources: Vec<CaptureSource>,
    catalog: Vec<CaptureSourceInfo>,
    artwork: SourceArtwork,
    progress: SendProgress,
}

fn source_name(source: CaptureSource) -> String {
    match source {
        CaptureSource::MainDisplay => "Main display".into(),
        CaptureSource::Display(id) => format!("Display {id}"),
        CaptureSource::Window(id) => format!("Window {id}"),
        CaptureSource::Camera(id) => format!("Camera {id}"),
    }
}

impl SendingContext {
    fn label(&self) -> String {
        if let [source] = self.sources.as_slice() {
            self.catalog
                .iter()
                .find(|info| info.source == *source)
                .map(capture_source_label)
                .unwrap_or_else(|| source_name(*source))
        } else {
            format!("{} sources", self.sources.len())
        }
    }

    fn observe(&mut self, progress: Option<SendProgress>) {
        if let Some(mut progress) = progress {
            if !progress.started && matches!(progress.result, Some(Ok(()))) {
                let error = "Receiver ended delivery before video started".to_string();
                progress.status = error.clone();
                progress.result = Some(Err(error));
            }
            self.progress = progress;
        } else if self.progress.result.is_none() {
            self.progress.status = "Sending stopped".into();
            self.progress.result = Some(Ok(()));
        }
    }
}

impl RestoredDashboard {
    pub(super) fn send_activity(&self) -> Vec<SendActivity> {
        let devices = self.owner.snapshot().devices;
        self.directed_send
            .lock()
            .unwrap()
            .values()
            .map(|sender| SendActivity {
                request_id: sender.request_id.clone(),
                target: sender.target.clone(),
                target_name: devices
                    .iter()
                    .find(|device| device.device_id == sender.target)
                    .map(|device| device.display_name.clone())
                    .unwrap_or_else(|| sender.target.clone()),
                source: sender
                    .sources
                    .iter()
                    .map(|source| source_name(*source))
                    .collect::<Vec<_>>()
                    .join(", "),
                status: sender.poll(),
                finished: sender.is_finished(),
            })
            .collect()
    }

    pub(super) fn poll_sending_context(&mut self) {
        let Some(context) = self.sending_context.as_mut() else {
            return;
        };
        let progress = self
            .directed_send
            .lock()
            .unwrap()
            .get(&context.request)
            .filter(|sender| sender.target == context.target && sender.sources == context.sources)
            .map(|sender| sender.progress());
        context.observe(progress);
    }

    fn refresh_sending_catalog(&mut self, cx: &mut Context<Self>) {
        let Some(context) = self
            .sending_context
            .as_ref()
            .filter(|context| context.catalog.is_empty())
        else {
            return;
        };
        let Some(client) = self.background.clone() else {
            return;
        };
        let request = context.request.clone();
        let scope = remote_core::shared_files::current_share_scope();
        let work =
            tokio::task::spawn_blocking(move || source_selection::local_source_catalog(&client));
        cx.spawn(async move |view, cx| {
            let Ok(Ok((sources, assets, _))) = work.await else {
                return;
            };
            let _ = view.update(cx, |this, cx| {
                if scope != remote_core::shared_files::current_share_scope() {
                    return;
                }
                if let Some(context) = this
                    .sending_context
                    .as_mut()
                    .filter(|context| context.request == request)
                {
                    context.catalog = sources
                        .into_iter()
                        .filter(|source| context.sources.contains(&source.source))
                        .collect();
                    context.artwork = ApplicationArtwork::default().update(assets);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub(super) fn open_sending_window(
        &mut self,
        request: &str,
        catalog: Vec<CaptureSourceInfo>,
        artwork: SourceArtwork,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.exit_group.stopping.load(Ordering::Acquire) {
            return false;
        }
        if self
            .sending_context
            .as_ref()
            .is_some_and(|context| context.request == request)
        {
            let handle = self.window_handle;
            cx.defer(move |cx| {
                let _ = handle.update(cx, |_, window, _| window.activate_window());
            });
            return true;
        }
        for window in cx.windows() {
            if window == self.window_handle {
                continue;
            }
            let Some(window) = window.downcast::<RestoredDashboard>() else {
                continue;
            };
            if window.read(cx).is_ok_and(|view| {
                view.sending_context
                    .as_ref()
                    .is_some_and(|context| context.request == request)
            }) {
                let _ = window.update(cx, |_, window, _| window.activate_window());
                return true;
            }
        }
        let Some(context) = self
            .directed_send
            .lock()
            .unwrap()
            .get(request)
            .map(|sender| SendingContext {
                request: request.into(),
                target: sender.target.clone(),
                sources: sender.sources.clone(),
                catalog: catalog
                    .into_iter()
                    .filter(|info| sender.sources.contains(&info.source))
                    .collect(),
                artwork,
                progress: sender.progress(),
            })
        else {
            self.status = "This sending request has ended".into();
            return false;
        };
        let Some(window) = open_restored_window(
            self.runtime.clone(),
            None,
            Default::default(),
            None,
            self.background.clone(),
            self.exit_group.clone(),
            cx,
        ) else {
            self.status = "Sending continues; open it from Activity".into();
            return false;
        };
        window
            .update(cx, |this, window, cx| {
                this.purpose = WindowPurpose::Sending;
                this.stream_device = Some(context.target.clone());
                this.sending_context = Some(context);
                this.drawer_open = false;
                this.owner.release_input();
                this.refresh_sending_catalog(cx);
                window.activate_window();
                cx.notify();
            })
            .is_ok()
    }

    pub(super) fn sending_view(
        &mut self,
        snapshot: &DashboardSnapshot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(context) = self.sending_context.clone() else {
            return div().into_any_element();
        };
        let device = snapshot
            .devices
            .iter()
            .find(|device| device.device_id == context.target)
            .map(|device| device.display_name.clone())
            .unwrap_or_else(|| context.target.clone());
        let source = context.label();
        let title = format!("{source} → {device} · Sending — Weavelo");
        self.native_window_title = title.clone();
        self.status = context.progress.status.clone();
        window.set_window_title(&title);
        let finished = context.progress.result.is_some();
        let failed = context.progress.result.as_ref().is_some_and(Result::is_err);
        let theme = cx.theme().clone();
        let request = context.request.clone();
        let target = context.target.clone();
        let send_target = target.clone();
        let files_target = target.clone();
        let receive_target = target.clone();
        let fullscreen = window.is_fullscreen();
        let mut sources = div().flex().flex_col().gap_2();
        for selected in &context.sources {
            let info = context.catalog.iter().find(|info| info.source == *selected);
            sources = sources.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(match info {
                        Some(info) => capture_source_badge(info, &context.artwork, &theme),
                        None => icon(IconName::Sharing).size(px(22.)).into_any_element(),
                    })
                    .child(
                        div().text_size(px(15.)).child(
                            info.map(capture_source_label)
                                .unwrap_or_else(|| source_name(*selected)),
                        ),
                    ),
            );
        }
        div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.surface.canvas)
            .text_color(theme.content.primary)
            .font_family(crate::design_system::product_ui_font())
            .overflow_hidden()
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .p_3()
                    .border_b_1()
                    .border_color(color_border_fine())
                    .child(icon(IconName::Sharing).size(px(16.)))
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .truncate()
                            .text_size(px(12.))
                            .child(title),
                    )
                    .child(
                        command_button("sending-files", ActionVariantKind::Neutral, cx)
                            .child("Files")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.open_device_files(files_target.clone(), cx)
                            })),
                    )
                    .child(
                        command_button("sending-new-source", ActionVariantKind::Neutral, cx)
                            .child("New source")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.begin_delivery_selection(send_target.clone(), cx)
                            })),
                    )
                    .child(
                        command_button("sending-minimize", ActionVariantKind::Neutral, cx)
                            .tooltip(tooltip("Minimize").build())
                            .child(icon(IconName::Minimize).size(px(14.)))
                            .on_click(|_, window, _| window.minimize_window()),
                    )
                    .child(
                        command_button("sending-fullscreen", ActionVariantKind::Neutral, cx)
                            .tooltip(
                                tooltip(if fullscreen { "Restore" } else { "Fullscreen" }).build(),
                            )
                            .child(icon(IconName::Maximize(fullscreen)).size(px(14.)))
                            .on_click(|_, window, _| window.toggle_fullscreen()),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_4()
                    .p_4()
                    .child(
                        div()
                            .w(px(520.))
                            .max_w(relative(0.95))
                            .flex()
                            .flex_col()
                            .gap_3()
                            .p_4()
                            .rounded_xl()
                            .bg(color_glass_card())
                            .border_1()
                            .border_color(color_border_fine())
                            .child(sources)
                            .child(div().text_size(px(12.)).child(format!("To {device}")))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(div().size(px(7.)).rounded_full().bg(if failed {
                                        theme.status.error.bg
                                    } else if finished {
                                        theme.content.disabled
                                    } else if context.progress.started {
                                        color_accent_emerald().into()
                                    } else {
                                        color_accent_cyan().into()
                                    }))
                                    .child(div().text_size(px(12.)).child(context.progress.status)),
                            )
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(theme.content.secondary)
                                    .child("Closing this window keeps sending in the background."),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        command_button(
                                            "sending-view-peer",
                                            ActionVariantKind::Neutral,
                                            cx,
                                        )
                                        .child("View device")
                                        .on_click(
                                            cx.listener(move |this, _, _, cx| {
                                                this.open_device_window(&receive_target, cx)
                                            }),
                                        ),
                                    )
                                    .child(
                                        command_button(
                                            "sending-management",
                                            ActionVariantKind::Neutral,
                                            cx,
                                        )
                                        .child("Activity")
                                        .on_click(
                                            |_, _, cx| {
                                                show_management_window(cx);
                                                if let Some(root) = cx
                                                    .try_global::<ManagementRoot>()
                                                    .map(|root| root.0.clone())
                                                {
                                                    root.update(cx, |this, cx| {
                                                        this.active_tab = DrawerTab::Activity;
                                                        cx.notify();
                                                    });
                                                }
                                            },
                                        ),
                                    )
                                    .child(
                                        command_button(
                                            "sending-stop",
                                            ActionVariantKind::Danger,
                                            cx,
                                        )
                                        .disabled(finished)
                                        .debug_selector(|| "sending-stop".into())
                                        .child(if context.sources.len() == 1 {
                                            "Stop this send"
                                        } else {
                                            "Stop these sources"
                                        })
                                        .on_click(
                                            cx.listener(move |this, _, _, cx| {
                                                this.directed_send.lock().unwrap().remove(&request);
                                                this.poll_sending_context();
                                                cx.notify();
                                            }),
                                        ),
                                    ),
                            ),
                    ),
            )
            .when(
                self.drawer_open && self.stream_panel == StreamPanel::Files,
                |tree| tree.child(self.stream_controls_panel(snapshot, None, cx)),
            )
            .when_some(self.source_selection.as_ref(), |tree, selection| {
                tree.child(source_selection::render(
                    selection,
                    #[cfg(target_os = "linux")]
                    self.local_share_state.pending(),
                    cx,
                ))
            })
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Arc, CaptureSource, CaptureSourceInfo, FrontendExitGroup, RestoredDashboard, SendProgress,
        SendingContext, UnifiedRuntimeHandle, WindowPurpose, component, open_restored_window,
        product_window_appearance, weavelo_themes,
    };
    use gpui::TestAppContext;

    /// This fixture owns only a cancellable worker, not capture or network I/O.
    #[gpui::test]
    fn closing_and_reopening_sender_keeps_other_sends_and_files(cx: &mut TestAppContext) {
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
        let manager = cx.update(|cx| {
            component::init(cx).unwrap();
            crate::product_components::theme::install(
                product_window_appearance(),
                weavelo_themes(),
                cx,
            );
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
        let view = manager.root(cx).unwrap();
        view.update(cx, |this, _| this.purpose = WindowPurpose::Management);
        let (first, first_stop) = crate::desktop::directed_send::SendHandle::waiting_fixture(
            "send-a",
            "peer-a",
            vec![CaptureSource::Window(7)],
        );
        let (second, second_stop) = crate::desktop::directed_send::SendHandle::waiting_fixture(
            "send-b",
            "peer-b",
            vec![CaptureSource::Display(4)],
        );
        group
            .directed_send
            .lock()
            .unwrap()
            .extend([("send-a".into(), first), ("send-b".into(), second)]);
        let files = view.read_with(cx, |this, _| this.device_files("peer-b"));
        view.update(cx, |this, cx| {
            assert!(this.open_sending_window(
                "send-a",
                Vec::new(),
                Arc::new(Default::default()),
                cx
            ))
        });
        let first_window = cx.update(|cx| {
            cx.windows()
                .into_iter()
                .find_map(|window| {
                    let window = window.downcast::<RestoredDashboard>()?;
                    if window == manager {
                        return None;
                    }
                    window
                        .read(cx)
                        .is_ok_and(|view| {
                            view.sending_context
                                .as_ref()
                                .is_some_and(|context| context.request == "send-a")
                        })
                        .then_some(window)
                })
                .unwrap()
        });
        let first_view = first_window.root(cx).unwrap();
        cx.run_until_parked();
        assert_eq!(
            first_view.read_with(cx, |this, _| this.purpose),
            WindowPurpose::Sending
        );
        assert_eq!(
            first_view.read_with(cx, |this, _| this.stream_device.clone()),
            Some("peer-a".into())
        );
        view.update(cx, |this, cx| {
            assert!(this.open_sending_window(
                "send-a",
                Vec::new(),
                Arc::new(Default::default()),
                cx
            ))
        });
        assert_eq!(
            cx.update(|cx| cx.windows().len()),
            2,
            "same request focuses its sender window"
        );
        first_window
            .update(cx, |_, window, _| window.remove_window())
            .unwrap();
        drop(first_view);
        cx.run_until_parked();
        assert_eq!(group.directed_send.lock().unwrap().len(), 2);
        assert!(!*first_stop.borrow());
        assert!(!*second_stop.borrow());
        view.update(cx, |this, cx| {
            assert!(this.open_sending_window(
                "send-a",
                Vec::new(),
                Arc::new(Default::default()),
                cx
            ))
        });
        cx.run_until_parked();
        let reopened = cx.update(|cx| {
            cx.windows()
                .into_iter()
                .find(|window| *window != manager.into())
                .unwrap()
        });
        {
            let mut visual = gpui::VisualTestContext::from_window(reopened, cx);
            visual.run_until_parked();
            let bounds = visual
                .debug_bounds("sending-stop")
                .expect("reopened sender stop button");
            visual.simulate_click(bounds.center(), gpui::Modifiers::none());
        }
        cx.run_until_parked();
        assert!(*first_stop.borrow(), "explicit stop cancels this worker");
        assert!(!*second_stop.borrow(), "another target keeps sending");
        assert!(group.directed_send.lock().unwrap().contains_key("send-b"));
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
        group.directed_send.lock().unwrap().clear();
        cx.update(|cx| {
            for window in cx.windows() {
                let _ = window.update(cx, |_, window, _| window.remove_window());
            }
        });
    }

    #[test]
    fn sending_context_retains_source_and_reports_stop_or_failure_without_another_flow() {
        let mut context = SendingContext {
            request: "send-a".into(),
            target: "peer-a".into(),
            sources: vec![CaptureSource::Window(7)],
            catalog: vec![CaptureSourceInfo {
                source: CaptureSource::Window(7),
                title: "Notes".into(),
                application: "Editor".into(),
                process_id: Some(42),
                width: 800,
                height: 600,
                supports_input: true,
                camera: None,
            }],
            artwork: Arc::new(Default::default()),
            progress: SendProgress {
                status: "Sending".into(),
                started: true,
                result: None,
                confirmation_timeout: None,
            },
        };
        assert_eq!(context.label(), "Editor · Notes");
        let mut ended_before_frame = context.clone();
        ended_before_frame.observe(Some(SendProgress {
            status: "Sending stopped".into(),
            started: false,
            result: Some(Ok(())),
            confirmation_timeout: None,
        }));
        assert!(
            ended_before_frame
                .progress
                .status
                .contains("before video started")
        );
        assert!(matches!(ended_before_frame.progress.result, Some(Err(_))));
        let mut cancelled_before_frame = context.clone();
        cancelled_before_frame.progress.started = false;
        cancelled_before_frame.observe(None);
        assert!(matches!(
            cancelled_before_frame.progress.result,
            Some(Ok(()))
        ));
        context.observe(None);
        assert_eq!(context.progress.status, "Sending stopped");
        assert!(matches!(context.progress.result, Some(Ok(()))));
        context.observe(Some(SendProgress {
            status: "Delivery stopped: source ended".into(),
            started: true,
            result: Some(Err("source ended".into())),
            confirmation_timeout: None,
        }));
        context.observe(None);
        assert!(context.progress.status.contains("source ended"));
        assert_eq!(context.request, "send-a");
        assert_eq!(context.target, "peer-a");
        assert_eq!(context.sources, vec![CaptureSource::Window(7)]);
    }
}
