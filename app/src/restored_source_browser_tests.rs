//! Real loopback control packets and the production source-picker controller.
//! The recorder captures no screen, injects no OS input and proves no remote
//! authentication or physical video presentation.
use super::{
    RestoredDashboard, StreamStartOptions, UnifiedRuntimeHandle, component, open_restored_window,
    product_window_appearance, weavelo_themes,
};
use gpui::{Entity, TestAppContext};
use protocol::{
    ControlMessage,
    session::{CaptureSource, CaptureSourceInfo, SessionCommand},
};
use remote_core::{
    discovery::{DiscoveryAnnouncement, DiscoveryCapabilities, DiscoveryPeerCache, DiscoveryScope},
    net::{MultiplexedPacket, UdpMultiplexer},
};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

fn pump_until(
    cx: &mut TestAppContext,
    view: &Entity<RestoredDashboard>,
    ready: impl Fn(&RestoredDashboard) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        cx.run_until_parked();
        view.update(cx, |this, cx| {
            this.owner.poll();
            this.poll_source_browser(cx);
        });
        if view.read_with(cx, |this, _| ready(this)) {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("source browser did not reach its expected control state");
}

#[gpui::test]
fn source_browser_waits_for_an_explicit_choice_and_reuses_its_control_connection(
    cx: &mut TestAppContext,
) {
    let executor = tokio::runtime::Runtime::new().unwrap();
    let _entered = executor.enter();
    remote_core::init_crypto_provider();
    assert!(remote_core::session_crypto::paired_file_session_key().is_none());
    let mux = executor
        .block_on(UdpMultiplexer::bind("127.0.0.1:0"))
        .unwrap();
    let address = mux.local_addr().unwrap();
    let (sender, receiver) = mux.split();
    let (subscriptions, mut observed) = tokio::sync::mpsc::unbounded_channel();
    let server = executor.spawn(async move {
        let mut queries = 0;
        while let Ok(packet) = receiver.recv().await {
            let MultiplexedPacket::Control(ControlMessage::Session(command), from) = packet else {
                continue;
            };
            let response = match *command {
                SessionCommand::Open { connection_id, .. } => SessionCommand::Opened {
                    connection_id,
                    version: protocol::session::SESSION_VERSION,
                    max_subscriptions: 8,
                    files: false,
                    clipboard: false,
                    window_capture: true,
                },
                SessionCommand::ListSources { request_id, .. } => {
                    queries += 1;
                    if queries == 2 {
                        SessionCommand::Error {
                            request_id,
                            reason: "Fixture directory unavailable".into(),
                        }
                    } else {
                        SessionCommand::Sources {
                            request_id,
                            sources: if queries == 1 {
                                vec![]
                            } else {
                                vec![CaptureSourceInfo {
                                    source: CaptureSource::Window(77),
                                    title: "Chosen fixture window".into(),
                                    application: "Fixture".into(),
                                    process_id: Some(77),
                                    width: 640,
                                    height: 360,
                                    supports_input: false,
                                    camera: None,
                                }]
                            },
                            source_icons: vec![],
                            icons: vec![],
                        }
                    }
                }
                SessionCommand::Subscribe(request) => {
                    subscriptions.send(request.clone()).unwrap();
                    SessionCommand::Subscribed {
                        id: request.id,
                        audio_owner: None,
                        supports_input: false,
                    }
                }
                SessionCommand::Unsubscribe { id } => SessionCommand::Unsubscribed { id },
                _ => continue,
            };
            sender
                .send_control(&ControlMessage::Session(Box::new(response)), from)
                .await
                .unwrap();
        }
    });
    let device = "abababababababababababababababab";
    let mut cache = DiscoveryPeerCache::new("source-browser-fixture", "local-fixture");
    cache
        .apply_announcement(
            DiscoveryAnnouncement {
                network_name: "source-browser-fixture".into(),
                device_id: device.into(),
                display_name: "Source fixture".into(),
                control_port: address.port(),
                virtual_ip: None,
                capabilities: DiscoveryCapabilities::all_interactive(),
                scope: DiscoveryScope::Lan,
                ttl: Duration::from_secs(60),
            },
            address,
            crate::unix_now_ms(),
        )
        .unwrap();
    let snapshot = cache.snapshot();
    let (_peers_tx, peers) = tokio::sync::watch::channel(snapshot.clone());
    let backend = executor
        .block_on(crate::UnifiedServiceOwner::from_background_snapshot(peers))
        .unwrap();
    backend
        .runtime()
        .lock()
        .unwrap()
        .apply_discovery_snapshot(&snapshot);
    let runtime = Arc::new(UnifiedRuntimeHandle::from_workspace_owner(Arc::new(
        backend,
    )));
    let window = cx.update(|cx| {
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
            Some(device.into()),
            None,
            Default::default(),
            cx,
        )
        .unwrap()
    });
    let view = window.root(cx).unwrap();
    pump_until(cx, &view, |this| {
        this.owner.snapshot().source_catalog_revision == 1
    });
    let owner = view.read_with(cx, |this, _| this.owner.clone());
    let connection = owner.active_connection().unwrap().id;
    assert!(owner.snapshot().sources.is_empty());
    assert!(
        observed.try_recv().is_err(),
        "enumerating an empty catalog must not subscribe to the default desktop"
    );
    view.update(cx, |this, cx| this.confirm_source_selection(cx));
    assert!(
        observed.try_recv().is_err(),
        "the handler also rejects an empty choice"
    );
    assert!(
        executor.block_on(owner.connect_files(device)).is_err(),
        "fixture file access is unavailable; source access remains independent"
    );
    assert_eq!(owner.active_connection().unwrap().id, connection);

    view.update(cx, |this, cx| this.refresh_remote_source_selection(cx));
    pump_until(cx, &view, |this| {
        this.owner.snapshot().source_catalog_revision == 2
    });
    assert_eq!(
        owner.snapshot().source_catalog_error.as_deref(),
        Some("Fixture directory unavailable")
    );
    assert!(observed.try_recv().is_err());
    view.update(cx, |this, cx| this.refresh_remote_source_selection(cx));
    pump_until(cx, &view, |this| {
        this.owner.snapshot().source_catalog_revision == 3
    });
    assert!(owner.snapshot().source_catalog_error.is_none());
    assert!(
        observed.try_recv().is_err(),
        "a valid catalog alone must still not start video"
    );
    executor
        .block_on(owner.reconnect_active(StreamStartOptions::default()))
        .unwrap();
    let replacement = owner.active_connection().unwrap().id;
    assert_ne!(replacement, connection);
    assert!(
        observed.try_recv().is_err(),
        "reconnecting a source browser must not start a desktop stream"
    );
    assert_eq!(
        view.read_with(cx, |this, _| this.stream_device.clone())
            .as_deref(),
        Some(device)
    );
    assert_eq!(view.read_with(cx, |this, _| this.stream_source), None);
    view.update(cx, |this, cx| this.refresh_remote_source_selection(cx));
    pump_until(cx, &view, |this| {
        this.owner.snapshot().source_catalog_revision == 1
    });
    let connection = replacement;
    view.update(cx, |this, cx| this.confirm_source_selection(cx));
    pump_until(cx, &view, |this| this.owner.receiving_streams() > 0);
    let request = executor
        .block_on(tokio::time::timeout(
            Duration::from_secs(2),
            observed.recv(),
        ))
        .unwrap()
        .unwrap();
    assert_eq!(
        request.source,
        CaptureSource::Window(77),
        "file-to-video reuse must honor the chosen window"
    );
    assert_eq!(
        owner.active_connection().unwrap().id,
        connection,
        "source choice upgrades the same control connection"
    );
    assert_eq!(
        view.read_with(cx, |this, _| this.stream_source),
        Some(CaptureSource::Window(77))
    );
    assert_eq!(
        cx.update(|cx| cx.windows().len()),
        1,
        "the first choice uses its source-browser window"
    );
    owner.close_all();
    window
        .update(cx, |_, window, _| window.remove_window())
        .unwrap();
    drop(view);
    cx.run_until_parked();
    server.abort();
    let _ = executor.block_on(server);
}
