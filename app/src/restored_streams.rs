//! Device/source actions for one stream window. Every new view captures its
//! own target; reconnecting a transport never selects a different window.
use super::*;
use protocol::session::{CaptureSource, CaptureSourceInfo};

impl RestoredDashboard {
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
        if self.stream_device.as_deref() == Some(device.as_str())
            && self.stream_source == Some(source)
        {
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
                    view.stream_device.as_deref() == Some(device.as_str())
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
                view.stream_device.as_deref() == Some(device.as_str())
                    && view.stream_source == Some(source)
            });
            if matches {
                let _ = handle.update(cx, |_, window, _| window.activate_window());
                self.show_apps_menu = false;
                return true;
            }
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
}
