//! Native menus use the same window, device and sending actions as the GUI.
use super::*;
use crate::desktop::menu::{
    Call, DeviceAction, Receiving, Section, Sending, Snapshot, StreamAction, StreamTarget,
};

impl RestoredDashboard {
    fn menu_streams(&self, cx: &Context<Self>) -> Vec<Entity<Self>> {
        let mut seen = BTreeSet::new();
        cx.windows()
            .into_iter()
            .filter_map(|window| {
                if window == self.window_handle {
                    return None;
                }
                let entity = if let Some(handle) = window.downcast::<Self>() {
                    handle.entity(cx).ok()?
                } else {
                    let handle = window.downcast::<PopoutStreamView>()?;
                    handle.read(cx).ok()?._primary.clone()
                };
                let view = entity.read(cx);
                (view.purpose == WindowPurpose::Stream
                    && Arc::ptr_eq(&view.exit_group, &self.exit_group)
                    && seen.insert(entity.entity_id()))
                .then_some(entity.clone())
            })
            .collect()
    }

    pub(super) fn native_menu_snapshot(&self, generation: &str, cx: &Context<Self>) -> Snapshot {
        let receiving = self
            .menu_streams(cx)
            .into_iter()
            .filter_map(|entity| {
                let view = entity.read(cx);
                Some(Receiving {
                    target: StreamTarget {
                        entity: entity.entity_id().as_u64(),
                        device: view.stream_device.clone()?,
                        source: view.stream_source,
                    },
                    title: if view.native_window_title.is_empty() {
                        view.stream_device.clone()?
                    } else {
                        view.native_window_title.clone()
                    },
                    paused: view.manual_media_paused,
                    can_pause: view.owner.receiving_streams() > 0,
                })
            })
            .collect();
        Snapshot {
            ok: true,
            generation: generation.into(),
            executable_sha256: self
                .directed_receive
                .as_ref()
                .map(|a| a.executable_sha256.clone())
                .unwrap_or_default(),
            renderer: crate::gui_backend::ORIGINAL_RENDERER.into(),
            sharing: self.background.as_ref().map_or(0, |b| b.outgoing_streams()),
            receiving_count: self
                .exit_group
                .owners()
                .iter()
                .map(|o| o.receiving_streams())
                .sum(),
            receiving,
            sending: self
                .send_activity()
                .into_iter()
                .filter(|s| !s.finished)
                .map(|s| Sending {
                    request: s.request_id,
                    title: format!("{} → {}", s.source, s.target_name),
                    status: s.status,
                })
                .collect(),
        }
    }

    pub(super) fn set_stream_paused(&mut self, paused: bool, cx: &mut Context<Self>) {
        self.manual_media_paused = paused;
        if paused {
            self.owner.release_input();
            self.pointer_input = PointerInputTracker::default();
        }
        self.sync_media_pause();
        cx.notify();
    }

    pub(super) fn dispatch_menu_action(
        &mut self,
        call: Call,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        if !call.valid() {
            return Err("Invalid menu target".into());
        }
        match call {
            Call::Manager(section) => {
                self.active_tab = match section {
                    Section::Devices => DrawerTab::Devices,
                    Section::Activity => DrawerTab::Activity,
                    Section::Transfers => DrawerTab::Files,
                    Section::Preferences => DrawerTab::Network,
                    Section::Sharing => DrawerTab::Security,
                    Section::Pairing => DrawerTab::DeviceGroup,
                };
                if section == Section::Sharing {
                    self.refresh_local_permissions(cx);
                }
                cx.defer(show_management_window);
            }
            Call::Device { id, action } => {
                let device = self
                    .snapshot()
                    .devices
                    .into_iter()
                    .find(|d| d.device_id == id)
                    .ok_or("This device is no longer in the current network")?;
                if !device.online && action != DeviceAction::Details {
                    return Err("This device is offline".into());
                }
                let action = match action {
                    DeviceAction::View => DeviceListAction::OpenWorkspace(id),
                    DeviceAction::Send => DeviceListAction::SendScreen(id),
                    DeviceAction::Files => DeviceListAction::OpenFiles(id),
                    DeviceAction::Details => {
                        self.active_tab = DrawerTab::Devices;
                        self.device_list_state.details = Some(id);
                        self.device_list_state.more = None;
                        cx.defer(show_management_window);
                        cx.notify();
                        return Ok(());
                    }
                };
                let show_manager = action_needs_manager(&action);
                self.dispatch_device_action(action, cx);
                if show_manager {
                    cx.defer(show_management_window);
                }
            }
            Call::Stream { target, action } => {
                let entity = self
                    .menu_streams(cx)
                    .into_iter()
                    .find(|e| e.entity_id().as_u64() == target.entity)
                    .ok_or("This stream window has ended")?;
                let view = entity.read(cx);
                if view.stream_device.as_deref() != Some(target.device.as_str())
                    || view.stream_source != target.source
                {
                    return Err("The stream source changed; reopen the menu".into());
                }
                if matches!(action, StreamAction::Pause(_)) && view.owner.receiving_streams() == 0 {
                    return Err("This stream is not receiving media".into());
                }
                if action == StreamAction::Open {
                    let handle = view.window_handle;
                    cx.defer(move |cx| {
                        cx.activate(true);
                        if handle
                            .update(cx, |_, window, _| window.activate_window())
                            .is_err()
                        {
                            reopen_restored_window(entity, cx);
                        }
                    });
                } else {
                    entity.update(cx, |view, cx| match action {
                        StreamAction::Pause(paused) => view.set_stream_paused(paused, cx),
                        StreamAction::End => view.end_stream_window(cx),
                        StreamAction::Open => unreachable!(),
                    });
                }
            }
            Call::OpenSend(request) => {
                if !self
                    .directed_send
                    .lock()
                    .unwrap()
                    .get(&request)
                    .is_some_and(|s| !s.is_finished())
                {
                    return Err("This sending request has ended".into());
                }
                if !self.open_sending_window(&request, Vec::new(), Arc::new(Default::default()), true, cx)
                {
                    return Err("Could not open the sending window".into());
                }
            }
            Call::StopSend(request) => {
                if !self.directed_send.lock().unwrap().contains_key(&request) {
                    return Err("This sending request has ended".into());
                }
                self.stop_sending_requests(&[request], cx);
            }
            Call::StopAllSends => self.stop_all_sending(cx),
        }
        cx.notify();
        Ok(())
    }
}

fn action_needs_manager(action: &DeviceListAction) -> bool {
    matches!(
        action,
        DeviceListAction::OpenFiles(_) | DeviceListAction::SendScreen(_)
    )
}

#[cfg(test)]
mod tests {
    use super::{
        Call, FrontendExitGroup, StreamAction, UnifiedRuntimeHandle,
        WindowPurpose, component, open_restored_window, product_window_appearance, weavelo_themes,
    };
    use gpui::TestAppContext;
    use std::sync::Arc;

    #[gpui::test]
    fn native_menu_keeps_stream_identity_and_only_ends_its_target(cx: &mut TestAppContext) {
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
                runtime.clone(),
                None,
                Default::default(),
                None,
                None,
                group.clone(),
                cx,
            )
            .unwrap()
        });
        let manager_view = manager.root(cx).unwrap();
        manager_view.update(cx, |view, _| view.purpose = WindowPurpose::Management);
        let first = cx.update(|cx| {
            open_restored_window(
                runtime.clone(),
                None,
                Default::default(),
                None,
                None,
                group.clone(),
                cx,
            )
            .unwrap()
        });
        let second = cx.update(|cx| {
            open_restored_window(runtime, None, Default::default(), None, None, group, cx).unwrap()
        });
        let first_view = first.root(cx).unwrap();
        let second_view = second.root(cx).unwrap();
        for (view, source) in [(&first_view, 7), (&second_view, 8)] {
            view.update(cx, |view, _| {
                view.stream_device = Some("peer".into());
                view.stream_source = Some(protocol::session::CaptureSource::Window(source));
            });
        }
        let target = manager_view.update(cx, |view, cx| {
            let menu = view.native_menu_snapshot("fixture", cx);
            assert_eq!(menu.receiving.len(), 2);
            menu.receiving
                .into_iter()
                .find(|stream| stream.target.entity == first_view.entity_id().as_u64())
                .unwrap()
                .target
        });
        assert!(
            manager_view
                .update(cx, |view, cx| {
                    view.dispatch_menu_action(
                        Call::Stream {
                            target: target.clone(),
                            action: StreamAction::Pause(true),
                        },
                        cx,
                    )
                })
                .is_err(),
            "a disconnected window cannot be paused"
        );
        assert!(!first_view.read_with(cx, |view, _| view.manual_media_paused));
        assert!(!second_view.read_with(cx, |view, _| view.manual_media_paused));
        let mut changed = target.clone();
        changed.source = Some(protocol::session::CaptureSource::Window(8));
        assert!(
            manager_view
                .update(cx, |view, cx| view.dispatch_menu_action(
                    Call::Stream {
                        target: changed,
                        action: StreamAction::End
                    },
                    cx
                ))
                .is_err()
        );
        manager_view.update(cx, |view, cx| {
            view.dispatch_menu_action(
                Call::Stream {
                    target: target.clone(),
                    action: StreamAction::Open,
                },
                cx,
            )
            .unwrap()
        });
        cx.run_until_parked();
        assert_eq!(
            cx.update(|cx| cx.windows().len()),
            3,
            "opening an existing stream never creates another connection/window"
        );
        manager_view.update(cx, |view, cx| {
            view.dispatch_menu_action(
                Call::Stream {
                    target: target.clone(),
                    action: StreamAction::End,
                },
                cx,
            )
            .unwrap()
        });
        cx.run_until_parked();
        assert!(first.update(cx, |_, _, _| ()).is_err());
        assert!(second.update(cx, |_, _, _| ()).is_ok());
        assert!(manager.update(cx, |_, _, _| ()).is_ok());
        assert!(
            manager_view
                .update(cx, |view, cx| view.dispatch_menu_action(
                    Call::Stream {
                        target,
                        action: StreamAction::End
                    },
                    cx
                ))
                .is_err()
        );
        cx.update(|cx| {
            for window in cx.windows() {
                let _ = window.update(cx, |_, window, _| window.remove_window());
            }
        });
    }
}
