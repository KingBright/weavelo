//! Management presentation and application-owned coordinator. No video surface
//! or remote input target is installed by this view.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum WindowPurpose {
    Management,
    Stream,
    Sending,
}

/// A closed management window leaves the frontend coordinator alive. Its
/// instance lock, delivery adapter and files stay owned by this process.
pub(super) struct ManagementRoot(pub(super) Entity<RestoredDashboard>);
impl Global for ManagementRoot {}

pub(super) fn show_management_window(cx: &mut App) {
    let Some(view) = cx.try_global::<ManagementRoot>().map(|root| root.0.clone()) else {
        return;
    };
    if view.read(cx).exit_group.stopping.load(Ordering::Acquire) {
        return;
    }
    let handle = view.read(cx).window_handle;
    cx.activate(true);
    if handle
        .update(cx, |_, window, _| window.activate_window())
        .is_err()
    {
        reopen_restored_window(view, cx);
    }
}

pub(super) fn reopen_restored_window(view: Entity<RestoredDashboard>, cx: &mut App) {
    if view.read(cx).exit_group.stopping.load(Ordering::Acquire) {
        return;
    }
    let options = restored_window_options(cx, view.read(cx).purpose);
    let retained = view.clone();
    let result = cx.open_window(options, move |window, cx| {
        view.update(cx, |this, cx| {
            this.window_handle = window.window_handle();
            this.native_window_title.clear();
            this.native_window_artwork=Default::default();
            if this.purpose!=WindowPurpose::Management {window_icon::apply(window,cx);}
            RestoredDashboard::observe_window(window, cx);
            this.window_active = window.is_window_active();
            this.window_visible = window.is_window_visible();
            window.focus(&this.root_focus);
            cx.notify();
        });
        let weak = view.downgrade();
        window.on_window_should_close(cx, move |_, cx| {
            weak.update(cx, |this, cx| {
                if this.exit_group.stopping.load(Ordering::Acquire) {
                    return false;
                }
                this.window_active = false;
                this.window_visible = Some(false);
                this.prepare_stream_close(cx);
                this.owner.release_input();
                this.pointer_input = PointerInputTracker::default();
                true
            })
            .unwrap_or(true)
        });
        view
    });
    if let Err(error) = result {
        retained.update(cx, |this, _| {
            this.status = format!("Could not reopen window: {error}")
        });
        eprintln!("Weavelo window: {error}");
    }
}

impl RestoredDashboard {
    pub(super) fn management_view(
        &mut self,
        snapshot: &DashboardSnapshot,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let query = self.device_query.trim().to_lowercase();
        let devices: Vec<_> = snapshot
            .devices
            .iter()
            .filter(|device| {
                query.is_empty()
                    || device.display_name.to_lowercase().contains(&query)
                    || device.device_id.to_lowercase().contains(&query)
            })
            .cloned()
            .collect();
        let side_services = SideServiceUiState {
            talkback: FeatureToggleState {
                available: self.owner.supports_talkback(),
                enabled: self.owner.talkback_enabled(),
            },
            clipboard_sync: FeatureToggleState {
                available: self.owner.supports_clipboard_sync(),
                enabled: self.owner.clipboard_sync_enabled(),
            },
            file_transfer: FeatureToggleState {
                available: self.owner.supports_file_transfer(),
                enabled: self.owner.file_transfer_enabled(),
            },
        };
        let controls = if self.active_tab == DrawerTab::Network {
            self.media_controls(cx).child(scale_mode_control(self.scale_mode, cx))
        } else { div() };
        let transfers = if self.active_tab == DrawerTab::Files { self.transfers_panel(cx) } else { div() };
        div()
            .relative()
            .size_full()
            .bg(cx.theme().surface.canvas)
            .text_color(cx.theme().content.primary)
            .font_family(crate::design_system::product_ui_font())
            .child(slide_over_management_drawer(
                true,
                self.window_handle,
                &self.device_query,
                &devices,
                &snapshot.role,
                self.mesh_pairing_snapshot.clone(),
                &self.device_name_editor,
                self.pairing_image_import.is_pending(),
                self.active_tab,
                &self.device_list_state,
                #[cfg(any(unix, windows))]
                self.stream_activity(),
                #[cfg(target_os = "linux")]
                &self.local_share_state,
                self.local_permissions.clone(),
                self.checking_permissions,
                self.input_locked,
                side_services,
                self.stream_settings.values().resolution(),
                self.stream_settings.values().fps,
                self.stream_settings.values().bitrate_kbps,
                controls,
                transfers,
                None,
                self.owner.clone(),
                cx,
            ))
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
