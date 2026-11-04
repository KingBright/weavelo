//! Device/source actions for one stream window. Every new view captures its
//! own target; reconnecting a transport never selects a different window.
use super::*;
use protocol::session::{CaptureSource, CaptureSourceInfo};

impl RestoredDashboard {
    pub(super) fn prepare_stream_close(&mut self, cx: &mut Context<Self>) {
        if self.purpose != WindowPurpose::Stream { return; }
        self.native_window_artwork.close();
        self.session_commands.begin();
        self.invalidate_source_commands();
        self.source_selection=None;
        self.owner.release_input();
        self.pointer_input=PointerInputTracker::default();
        self.owner.close_received_window();
        self.owner.close_all();
        self.current_frame=None;
        let pips=std::mem::take(&mut self.pip_windows);
        cx.defer(move|cx|for pip in pips {let _=pip.update(cx,|_,window,_|window.remove_window());});
    }

    pub(super) fn end_stream_window(&mut self, cx: &mut Context<Self>) {
        if self.purpose != WindowPurpose::Stream { return; }
        self.prepare_stream_close(cx);
        let handle=self.window_handle;
        cx.defer(move|cx| {let _=handle.update(cx,|_,window,_|window.remove_window());});
    }

    pub(super) fn idle_stream_stage(&self, cx: &mut Context<Self>) -> AnyElement {
        let source_selected=self.stream_source.is_some();
        let title=if source_selected {"Disconnected"} else {"Choose a source"};
        let detail=if self.status=="Ready" {"This stream is not connected".into()} else {self.status.clone()};
        let target=self.stream_device.clone();
        let can_reconnect=target.is_some();
        div().size_full().flex().items_center().justify_center().p_4()
            .child(div().w(px(420.)).max_w(relative(0.95)).flex().flex_col().gap_3().p_4()
                .rounded_xl().bg(color_glass_card()).border_1().border_color(color_border_fine())
                .child(icon(IconName::Receiving).size(px(24.)))
                .child(div().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).child(title))
                .child(div().text_size(px(12.)).child(self.native_window_title.clone()))
                .child(div().text_size(px(11.)).text_color(cx.theme().content.secondary).child(detail))
                .child(command_button("reconnect-fixed-stream",ActionVariantKind::Primary,cx)
                    .disabled(!can_reconnect).child(if source_selected {"Reconnect this stream"} else {"Choose a source"})
                    .on_click(cx.listener(move|this,_,_,cx| {
                        if let Some(device)=target.clone() {
                            if let Some(source)=this.stream_source {
                                this.start_stream_connection_capture(device,None,Some(source),cx);
                            } else {this.start_source_browser(device,cx);}
                        }
                    }))))
            .into_any_element()
    }

    pub(super) fn start_source_browser(&mut self, device: String, cx: &mut Context<Self>) {
        if self.exit_group.stopping.load(Ordering::Acquire) || self.stream_source.is_some() {
            return;
        }
        if self
            .stream_device
            .as_ref()
            .is_some_and(|current| current != &device)
        {
            return;
        }
        self.stream_device = Some(device.clone());
        self.status = "Connecting to source directory".into();
        let ticket = self.session_commands.begin();
        let owner = self.owner.clone();
        cx.spawn(async move |view, cx| {
            if !view
                .update(cx, |this, _| this.session_commands.is_current(ticket))
                .unwrap_or(false)
            {
                return;
            }
            let result = owner.connect_source_browser(&device).await;
            let _ = view.update(cx, |this, cx| {
                if !this.session_commands.is_current(ticket) {
                    return;
                }
                this.status = match result {
                    Ok(()) => "Choose a screen, window or camera".into(),
                    Err(error) => format!("Source connection failed: {error}"),
                };
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn poll_source_browser(&mut self, cx: &mut Context<Self>) {
        if self.purpose == WindowPurpose::Stream
            && self.stream_device.is_some()
            && self.stream_source.is_none()
            && self.source_selection.is_none()
            && let Some(binding) = self.owner.source_view_binding()
            && self.owner.bound_catalog_revision(&binding).is_some()
        {
            self.begin_remote_source_selection(Some(binding), cx);
        }
        self.poll_remote_source_selection(cx);
    }

    pub(super) fn open_enumerated_source(
        &mut self,
        binding: Option<SourceViewBinding>,
        source: CaptureSourceInfo,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(binding) = binding else {
            return false;
        };
        let device = match self
            .owner
            .validate_source_selection(&binding, std::slice::from_ref(&source))
        {
            Ok(device) => device,
            Err(error) => {
                self.status = error;
                cx.notify();
                return false;
            }
        };
        if self
            .stream_device
            .as_ref()
            .is_some_and(|expected| expected != &device)
        {
            self.status = "Device context changed; reopen sources".into();
            cx.notify();
            return false;
        }
        self.open_stream_source(device, source.source, cx)
    }

    pub(super) fn open_stream_source(
        &mut self,
        device: String,
        source: CaptureSource,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.exit_group.stopping.load(Ordering::Acquire) {
            return false;
        }
        if self.focus_stream_source(&device, source, cx) {
            return true;
        }
        let Some(handle) = open_restored_window(
            self.runtime.clone(),
            None,
            Default::default(),
            None,
            self.background.clone(),
            self.exit_group.clone(),
            cx,
        ) else {
            self.status = "Could not open the stream window".into();
            return false;
        };
        let opened = handle
            .update(cx, |this, _, cx| {
                this.start_stream_connection_capture(device, None, Some(source), cx)
            })
            .is_ok();
        if opened {
            self.show_apps_menu = false;
        }
        opened
    }
    pub(super) fn focus_stream_source(
        &mut self,
        device: &str,
        source: CaptureSource,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.stream_device.as_deref() == Some(device) && self.stream_source == Some(source) {
            let handle = self.window_handle;
            let retained = cx.entity();
            cx.defer(move |cx| {
                if handle
                    .update(cx, |_, window, _| window.activate_window())
                    .is_err()
                {
                    reopen_restored_window(retained, cx);
                }
            });
            self.show_apps_menu = false;
            return true;
        }
        for handle in cx.windows() {
            if handle == self.window_handle {
                continue;
            }
            if let Some(pip) = handle.downcast::<PopoutStreamView>() {
                let Some(primary) = pip.read(cx).ok().map(|view| view._primary.clone()) else {
                    continue;
                };
                if primary.entity_id() == cx.entity().entity_id() {
                    continue;
                }
                let matches = {
                    let view = primary.read(cx);
                    view.stream_device.as_deref() == Some(device)
                        && view.stream_source == Some(source)
                };
                if matches {
                    let handle = primary.read(cx).window_handle;
                    if handle
                        .update(cx, |_, window, _| window.activate_window())
                        .is_err()
                    {
                        reopen_restored_window(primary, cx);
                    }
                    self.show_apps_menu = false;
                    return true;
                }
                continue;
            }
            let Some(handle) = handle.downcast::<RestoredDashboard>() else {
                continue;
            };
            let matches = handle.read(cx).is_ok_and(|view| {
                view.stream_device.as_deref() == Some(device) && view.stream_source == Some(source)
            });
            if matches {
                let _ = handle.update(cx, |_, window, _| window.activate_window());
                self.show_apps_menu = false;
                return true;
            }
        }
        false
    }
}
