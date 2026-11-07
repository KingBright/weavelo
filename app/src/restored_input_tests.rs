//! Opt-in full restored GPUI event-routing regression. Uses a local packet recorder,
//! never a real host, native screen capture, device identity, or OS input injection.
use super::{
    StreamPanel, FrontendExitGroup, RestoredDashboard, UnifiedRuntimeConfig, component, weavelo_themes,
    start_unified_runtime,
};
use crate::desktop::OriginalGuiSession;
use gpui::{
    KeyDownEvent, KeyUpEvent, Keystroke, Modifiers, MouseButton, TestAppContext, WindowAppearance,
    point, px, size,
};
use protocol::{
    ControlMessage, InputEvent, MomentumPhase as ProtocolMomentumPhase, ScrollGesture,
    ScrollPhase as ProtocolScrollPhase, session::SessionCommand,
};
use remote_core::VideoFrame;
use remote_core::{
    net::{MultiplexedPacket, UdpMultiplexer},
    workspace_session::WorkspaceConnection,
};
use std::{
    sync::{Arc, Mutex, atomic::AtomicBool},
    time::Duration,
};

#[gpui::test]
#[ignore = "local protocol recorder; run explicitly without any real user session key"]
fn restored_gpui_preserves_press_release_keyboard_and_overlay_isolation(cx: &mut TestAppContext) {
    run_input_case(cx,false);
}

#[gpui::test]
#[ignore = "local native decoder and protocol recorder; no real user session key"]
fn viewport_frames_preserve_remote_input_and_keep_local_navigation_local(cx:&mut TestAppContext) {
    run_input_case(cx,true);
}

fn run_input_case(cx:&mut TestAppContext,viewport_frame:bool) {
    assert!(
        remote_core::session_crypto::load_session_psk().is_none(),
        "Do not inherit a user's network key"
    );
    remote_core::init_crypto_provider();
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let _entered = rt.enter();
    let output = Arc::new(Mutex::new(Vec::new()));
    let events_out = output.clone();
    let temp = tempfile::tempdir().unwrap();
    let (runtime, session, recorder, recovery) = rt.block_on(async {
        let mux = UdpMultiplexer::bind("127.0.0.1:0").await.unwrap();
        let addr = mux.local_addr().unwrap();
        let (sender, rx) = mux.split();
        let recorder = tokio::spawn(async move {
            while let Ok(packet) = rx.recv().await {
                match packet {
                    MultiplexedPacket::Control(ControlMessage::Session(cmd), from) => {
                        if let SessionCommand::Open { connection_id, .. } = *cmd {
                            sender
                                .send_control(
                                    &ControlMessage::Session(Box::new(SessionCommand::Opened {
                                        connection_id,
                                        version: protocol::session::SESSION_VERSION,
                                        max_subscriptions: 8,
                                        files: false,
                                        clipboard: false,
                                        window_capture: false,
                                    })),
                                    from,
                                )
                                .await
                                .unwrap();
                        } else if let SessionCommand::Subscribe(request) = *cmd {
                            sender
                                .send_control(
                                    &ControlMessage::Session(Box::new(SessionCommand::Subscribed {
                                        id: request.id,
                                        audio_owner: None,
                                        supports_input: true,
                                        max_touch_points: 0,
                                    })),
                                    from,
                                )
                                .await
                                .unwrap();
                        } else if let SessionCommand::RequestInputBinding{connection_id,id,source_revision,request,..}=*cmd {
                            sender.send_control(&ControlMessage::Session(Box::new(SessionCommand::InputBinding {
                                binding:protocol::realtime_input::InputBinding{connection_id,subscription_id:id,source_revision,epoch:[9;16]},request,
                                host_time_ms:remote_core::session_crypto::now_unix_ms(),
                            })),from).await.unwrap();
                        } else if let SessionCommand::RealtimeInput(packet)=*cmd {
                            // Compare GPUI routing intent after protected-wire decoding.
                            // The dedicated desktop wire test checks the actual envelope.
                            let command=match packet.payload {
                                protocol::realtime_input::InputPayload::Event(event)=>Some(SessionCommand::SourceInput{id:packet.binding.subscription_id,source_revision:packet.binding.source_revision,event}),
                                protocol::realtime_input::InputPayload::Release=>Some(SessionCommand::ReleaseInput{id:packet.binding.subscription_id}),
                                _=>None,
                            };
                            if let Some(command)=command {events_out.lock().unwrap().push(command);}
                        } else {
                            events_out.lock().unwrap().push(*cmd);
                        }
                    }
                    MultiplexedPacket::Control(ControlMessage::Ping { client_send_ts }, from) => {
                        sender
                            .send_control(
                                &ControlMessage::Pong {
                                    client_send_ts,
                                    host_recv_ts: client_send_ts,
                                    host_send_ts: client_send_ts,
                                },
                                from,
                            )
                            .await
                            .unwrap();
                    }
                    _ => {}
                }
            }
        });
        let (conn, events) = WorkspaceConnection::connect_media(addr)
            .await
            .unwrap();
        let mut cfg = UnifiedRuntimeConfig::app_defaults();
        cfg.mesh_dir = temp.path().join("network");
        cfg.enable_discovery = false;
        cfg.enable_passive_host = false;
        cfg.enable_client_receiver = false;
        cfg.enable_viewer_media = false;
        cfg.enable_workspace_viewer = false;
        cfg.enable_session_timeout_monitor = false;
        cfg.enable_p2p = false;
        cfg.relay_endpoint = None;
        cfg.enable_clipboard_sync = false;
        cfg.enable_file_transfer = false;
        cfg.enable_talkback = false;
        let runtime = Arc::new(start_unified_runtime(cfg).await.unwrap());
        let mut session = OriginalGuiSession::new(
            "local-test-only".into(),
            "Input packet recorder".into(),
            "Loopback".into(),
            conn,
            events,
        );
        session.media =
            Some(client::ClientMediaRuntime::start_video_only(session.stats.clone()).unwrap());
        let recovery = session.stats.clone();
        let initial_source = recovery.video_recovery.snapshot().epoch.source;
        let (video,_)=remote_core::video_queue::channel(1);
        session.conn.subscribe(session.request.clone(),video,session.stats.clone(),session.host_stats.clone()).await.unwrap();
        tokio::time::timeout(Duration::from_secs(2),async {
            while !session.conn.input_ready(session.video_id,0) {tokio::time::sleep(Duration::from_millis(5)).await;}
        }).await.unwrap();
        // The input grant is independent of the video-subscription receipt.
        // Wait for the fake host's Subscribed response to establish the decoder
        // source epoch before enqueuing its synthetic frame.
        tokio::time::timeout(Duration::from_secs(2), async {
            while recovery.video_recovery.snapshot().epoch.source == initial_source {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }).await.expect("synthetic host did not accept the video subscription");
        // Decode the first access unit of the existing synthetic HEVC fixture so pointer mapping uses a
        // real native frame and its dimensions. This is neither screen capture
        // nor physical presentation; attach_recorder_session supplies the test
        // paint precondition for the protocol-routing fixture.
        let media = session.media.as_ref().unwrap();
        #[cfg(target_os = "macos")]
        media.set_video_payload_active(true);
        let mut payload=Vec::new();
        if viewport_frame {
            protocol::viewport::FrameGeometry{source_revision:0,demand_revision:0,revision:1,source_width:2304,source_height:1296,
                crop:protocol::viewport::PixelRect{x:0,y:0,width:2304,height:1296},width:128,height:72}.write_header(&mut payload);
        }
        payload.extend_from_slice(include_bytes!("../tests/fixtures/input-first-frame.h265"));
        let decode_queue = media.decode_tx.try_send((protocol::RtpPacket {
            header: protocol::RtpHeader { version: 2, payload_type: 96,
                sequence_number: 1, timestamp: 100, ssrc: 7 },
            payload,
        }, Default::default(), recovery.video_recovery.snapshot().epoch.source)).unwrap();
        assert!(decode_queue.accepted, "synthetic native HEVC fixture was not accepted by the decoder queue: {decode_queue:?}");
        let decoded = tokio::time::timeout(Duration::from_secs(60), async {
            while media.shared_frame().lock().unwrap().is_none() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }).await;
        if decoded.is_err() {
            let recovery_snapshot = recovery.video_recovery.snapshot();
            let decode_status = media.decode_status.lock().unwrap().clone();
            panic!(
                "synthetic native frame did not decode: timeout; decode_status={decode_status:?}; \
                 decoder_queue_push={decode_queue:?}; decoded={}; decode_errors={}; queue_dropped={}; \
                 reference_gaps={}; reference_skipped={}; keyframe_requests={}; recovery={recovery_snapshot:?}",
                recovery.video_frames_decoded.load(std::sync::atomic::Ordering::Relaxed),
                recovery.video_decode_errors.load(std::sync::atomic::Ordering::Relaxed),
                recovery.video_decode_queue_dropped.load(std::sync::atomic::Ordering::Relaxed),
                recovery.video_decoder_reference_gaps.load(std::sync::atomic::Ordering::Relaxed),
                recovery.video_decoder_reference_skipped.load(std::sync::atomic::Ordering::Relaxed),
                recovery.video_keyframe_requests.load(std::sync::atomic::Ordering::Relaxed),
            );
        }
        (runtime, session, recorder, recovery)
    });
    cx.update(|cx| {
        component::init(cx).expect("Ely test initialization");
        crate::product_components::theme::install(WindowAppearance::Dark, weavelo_themes(), cx);
    });
    let (view, cx) = cx.add_window_view(move |window, cx| {
        let mut view =
            RestoredDashboard::new(runtime, None, Arc::new(AtomicBool::new(false)), None, Arc::new(FrontendExitGroup::default()), window, cx);
        view.owner.attach_recorder_session(session);
        view.pause_when_inactive = false;
        view.drawer_open = false;
        view.toolbar_revealed = false;
        view.input_locked = false;
        view.text_mode = false;
        view
    });
    cx.simulate_resize(size(px(1000.), px(700.)));
    view.update(cx, |v, cx| { v.window_handle.update(cx, |_, window, _| window.activate_window()).unwrap(); });
    cx.run_until_parked();
    if viewport_frame {
        view.update(cx,|view,cx|{view.poll_viewport();cx.notify();});cx.run_until_parked();
    }
    // Initial setup can issue source discovery; clear that before the event trace.
    std::thread::sleep(Duration::from_millis(60));
    output.lock().unwrap().clear();
    let pos = point(px(500.), px(350.));
    let readiness = view.read_with(cx, |v, _| {
        let state = v.owner.snapshot();
        let surface = *v.video_surface_bounds.lock().unwrap();
        let pointer = v.pointer_event_at(pos);
        format!("locked={} supports={} panel={} active={} visible={:?} frame={:?} surface={:?} input={} eligible={} pointer={:?}",
            v.input_locked, v.capture_supports_input, v.drawer_open, v.window_active, v.window_visible,
            v.current_frame.as_ref().map(|frame| (frame.width(), frame.height())),
            surface, state.input_status, state.input_eligible, pointer)
    });
    println!("RESTORED_GPUI_READINESS {readiness}");

    cx.simulate_mouse_down(pos, MouseButton::Left, Modifiers::none());
    view.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();
    view.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();
    cx.simulate_mouse_move(
        point(px(550.), px(360.)),
        Some(MouseButton::Left),
        Modifiers::none(),
    );
    let release_pos = point(px(565.), px(375.));
    let final_move = view.read_with(cx, |v, _| v.pointer_event_at(release_pos)).unwrap();
    cx.simulate_mouse_up(
        release_pos,
        MouseButton::Left,
        Modifiers::none(),
    );
    cx.simulate_event(KeyDownEvent {
        keystroke: Keystroke::parse("a").unwrap(),
        is_held: false,
        prefer_character_input: false,
    });
    cx.simulate_event(KeyUpEvent {
        keystroke: Keystroke::parse("a").unwrap(),
    });
    std::thread::sleep(Duration::from_millis(150));
    let trace = output.lock().unwrap().clone();
    println!("RESTORED_GPUI_PROTOCOL_TRACE {trace:?}");
    let filtered: Vec<_> = trace
        .iter()
        .filter_map(|command| match command {
            SessionCommand::Input { event, .. } | SessionCommand::SourceInput { event, .. } => {
                Some(event.clone())
            }
            _ => None,
        })
        .collect();
    assert!(
        filtered.contains(&InputEvent::MouseDown(0)),
        "GPUI swallowed the press"
    );
    assert!(
        filtered.contains(&InputEvent::MouseUp(0)),
        "GPUI swallowed the release"
    );
    let release = filtered.iter().position(|event| *event == InputEvent::MouseUp(0)).unwrap();
    assert!(release > 0 && filtered[release - 1] == final_move,
        "Release must land at its own coordinates, even without a final mouse-move event");
    assert!(
        filtered.contains(&InputEvent::Key {
            key_code: 0,
            pressed: true,
            modifiers: 0
        }),
        "Keyboard down did not traverse the production root handler"
    );
    assert!(
        filtered.contains(&InputEvent::Key {
            key_code: 0,
            pressed: false,
            modifiers: 0
        }),
        "Keyboard up was swallowed"
    );
    assert!(
        !trace
            .iter()
            .any(|c| matches!(c, SessionCommand::ReleaseInput { .. })),
        "Render cycle released held input before the up event"
    );
    // Trackpad scrolling has its own position and preserves phase-only gesture
    // boundaries through the production root handler and realtime input path.
    output.lock().unwrap().clear();
    let scroll_pos=point(px(620.),px(430.));
    let scroll_pointer=view.read_with(cx,|v,_|v.pointer_event_at(scroll_pos)).unwrap();
    let scroll_samples = [
        (0.0, 0.0, gpui::ScrollPhase::Began, gpui::MomentumPhase::None),
        (0.0, 42.0, gpui::ScrollPhase::Changed, gpui::MomentumPhase::None),
        (0.0, 0.0, gpui::ScrollPhase::Ended, gpui::MomentumPhase::None),
        (0.0, 0.0, gpui::ScrollPhase::None, gpui::MomentumPhase::Began),
        (0.0, -7.0, gpui::ScrollPhase::None, gpui::MomentumPhase::Changed),
        (0.0, 0.0, gpui::ScrollPhase::None, gpui::MomentumPhase::Ended),
    ];
    let expected_scroll = |delta_x, delta_y, phase, momentum| InputEvent::MouseScroll {
        delta_x,
        delta_y,
        gesture: ScrollGesture { phase, momentum },
    };
    let expected_scrolls = [
        expected_scroll(0, 0, ProtocolScrollPhase::Began, ProtocolMomentumPhase::None),
        expected_scroll(0, 42, ProtocolScrollPhase::Changed, ProtocolMomentumPhase::None),
        expected_scroll(0, 0, ProtocolScrollPhase::Ended, ProtocolMomentumPhase::None),
        expected_scroll(0, 0, ProtocolScrollPhase::None, ProtocolMomentumPhase::Began),
        expected_scroll(0, -7, ProtocolScrollPhase::None, ProtocolMomentumPhase::Changed),
        expected_scroll(0, 0, ProtocolScrollPhase::None, ProtocolMomentumPhase::Ended),
    ];
    for (delta_x, delta_y, scroll_phase, momentum_phase) in scroll_samples {
        cx.simulate_event(gpui::ScrollWheelEvent {
            position: scroll_pos,
            delta: gpui::ScrollDelta::Pixels(point(px(delta_x), px(delta_y))),
            modifiers: Modifiers::none(),
            touch_phase: gpui::TouchPhase::Moved,
            scroll_phase,
            momentum_phase,
        });
    }
    // A default zero-delta event is not a gesture boundary and should be omitted.
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: scroll_pos,
        delta: gpui::ScrollDelta::Pixels(point(px(0.), px(0.))),
        modifiers: Modifiers::none(),
        touch_phase: gpui::TouchPhase::Moved,
        ..Default::default()
    });
    std::thread::sleep(Duration::from_millis(100));
    let scroll_trace=output.lock().unwrap().iter().filter_map(|command|match command {
        SessionCommand::Input{event,..}|SessionCommand::SourceInput{event,..}=>Some(event.clone()),_=>None,
    }).collect::<Vec<_>>();
    let scrolls = scroll_trace
        .iter()
        .filter(|event| matches!(event, InputEvent::MouseScroll { .. }))
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(scrolls, expected_scrolls.to_vec(), "scroll deltas and phase-only boundaries must retain order");
    let scroll=scroll_trace.iter().position(|event|matches!(event,InputEvent::MouseScroll{delta_x:0,delta_y:42,..})).expect("trackpad scroll must reach the host");
    assert!(scroll>0 && scroll_trace[0]==scroll_pointer,"gesture starts at its inverse-transformed position");
    assert_eq!(scroll_trace.iter().filter(|event| matches!(event, InputEvent::MouseMoveAbsolute { .. })).count(), 1,
        "continuation and cleanup must not enqueue takeover-capable pointer moves");
    output.lock().unwrap().clear();
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: scroll_pos, delta: gpui::ScrollDelta::Pixels(point(px(0.), px(0.))),
        modifiers: Modifiers::none(), touch_phase: gpui::TouchPhase::Moved,
        scroll_phase: gpui::ScrollPhase::Began, momentum_phase: gpui::MomentumPhase::None,
    });
    view.update(cx, |v, _| v.reset_pointer_scroll());
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: scroll_pos, delta: gpui::ScrollDelta::Pixels(point(px(0.), px(19.))),
        modifiers: Modifiers::none(), touch_phase: gpui::TouchPhase::Moved,
        scroll_phase: gpui::ScrollPhase::Changed, momentum_phase: gpui::MomentumPhase::None,
    });
    std::thread::sleep(Duration::from_millis(100));
    let interrupted=output.lock().unwrap().iter().filter_map(|command|match command {
        SessionCommand::Input{event: event @ InputEvent::MouseScroll{..},..}
        | SessionCommand::SourceInput{event: event @ InputEvent::MouseScroll{..},..}=>Some(event.clone()),_=>None,
    }).collect::<Vec<_>>();
    assert_eq!(interrupted, vec![
        expected_scroll(0,0,ProtocolScrollPhase::Began,ProtocolMomentumPhase::None),
        expected_scroll(0,0,ProtocolScrollPhase::Cancelled,ProtocolMomentumPhase::None),
    ], "focus cancellation must reach the bound source and reject its late continuation");
    output.lock().unwrap().clear();
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: scroll_pos, delta: gpui::ScrollDelta::Pixels(point(px(0.), px(0.))),
        modifiers: Modifiers::none(), touch_phase: gpui::TouchPhase::Moved,
        scroll_phase: gpui::ScrollPhase::Began, momentum_phase: gpui::MomentumPhase::None,
    });
    // Simulate the gesture's source becoming stale while the current view uses
    // a different source revision. The old cancellation must not reach it.
    view.update(cx, |v, _| {
        v.pointer_input.scroll_binding.as_mut().expect("begin must arm the gesture").source_revision += 1;
    });
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: scroll_pos, delta: gpui::ScrollDelta::Pixels(point(px(0.), px(19.))),
        modifiers: Modifiers::none(), touch_phase: gpui::TouchPhase::Moved,
        scroll_phase: gpui::ScrollPhase::Changed, momentum_phase: gpui::MomentumPhase::None,
    });
    std::thread::sleep(Duration::from_millis(100));
    let stale=output.lock().unwrap().iter().filter_map(|command|match command {
        SessionCommand::Input{event: event @ InputEvent::MouseScroll{..},..}
        | SessionCommand::SourceInput{event: event @ InputEvent::MouseScroll{..},..}=>Some(event.clone()),_=>None,
    }).collect::<Vec<_>>();
    assert_eq!(stale,vec![expected_scroll(0,0,ProtocolScrollPhase::Began,ProtocolMomentumPhase::None)],
        "a stale source gesture must not send continuation or cancellation to the current source");
    // The current stream's local panel owns clicks and keyboard input.
    view.update(cx, |v, cx| {
        v.open_stream_panel(StreamPanel::Control, cx);
    });
    cx.run_until_parked();
    std::thread::sleep(Duration::from_millis(60));
    output.lock().unwrap().clear();
    cx.simulate_click(point(px(120.), px(300.)), Modifiers::none());
    cx.simulate_event(KeyDownEvent {
        keystroke: Keystroke::parse("b").unwrap(),
        is_held: false,
        prefer_character_input: false,
    });
    cx.simulate_event(KeyUpEvent {
        keystroke: Keystroke::parse("b").unwrap(),
    });
    std::thread::sleep(Duration::from_millis(100));
    assert!(
        !output.lock().unwrap().iter().any(|c| matches!(
            c,
            SessionCommand::Input { .. } | SessionCommand::SourceInput { .. }
        )),
        "Local overlay leaked input into the remote device"
    );
    view.update(cx, |v, cx| {
        v.drawer_open = false;
        v.show_apps_menu = true;
        v.owner.release_input();
        cx.notify();
    });
    cx.run_until_parked();
    std::thread::sleep(Duration::from_millis(60));
    output.lock().unwrap().clear();
    cx.simulate_event(KeyDownEvent {
        keystroke: Keystroke::parse("down").unwrap(),
        is_held: false,
        prefer_character_input: false,
    });
    cx.simulate_event(KeyUpEvent {
        keystroke: Keystroke::parse("down").unwrap(),
    });
    cx.simulate_click(pos, Modifiers::none());
    std::thread::sleep(Duration::from_millis(100));
    assert!(
        !output.lock().unwrap().iter().any(|c| matches!(
            c,
            SessionCommand::Input { .. } | SessionCommand::SourceInput { .. }
        )),
        "Dismissing the apps menu leaked a remote click or key"
    );
    // Explicit locking leaves picture navigation available but disables remote input.
    view.update(cx, |v, cx| {
        v.drawer_open = false;
        v.set_input_locked(true);
        cx.notify();
    });
    cx.run_until_parked();
    std::thread::sleep(Duration::from_millis(50));
    output.lock().unwrap().clear();
    cx.simulate_click(pos, Modifiers::none());
    std::thread::sleep(Duration::from_millis(100));
    assert!(
        !output.lock().unwrap().iter().any(|c| matches!(
            c,
            SessionCommand::Input { .. } | SessionCommand::SourceInput { .. }
        )),
        "Locked view emitted input"
    );
    // Recovery must still cancel an active drag even with Enable control on.
    // Do not make the test green by weakening production readiness checks.
    view.update(cx, |v, cx| {
        v.drawer_open = false;
        v.show_apps_menu = false;
        v.set_input_locked(false);
        cx.notify();
    });
    cx.run_until_parked();
    std::thread::sleep(Duration::from_millis(60));
    if viewport_frame {
        let off_center=point(px(750.),px(350.));
        let before=view.read_with(cx,|view,_|view.pointer_event_at(off_center)).unwrap();
        view.update(cx,|view,cx|view.change_view_zoom(2.,None,cx));cx.run_until_parked();
        let after=view.read_with(cx,|view,_|view.pointer_event_at(off_center)).unwrap();
        assert_ne!(before,after,"Local zoom did not change the inverse source coordinates");
        view.update(cx,|view,cx|{view.viewport.lock().unwrap().toggle_navigation();view.poll_viewport();cx.notify();});cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(60));output.lock().unwrap().clear();
        cx.simulate_event(gpui::ScrollWheelEvent{position:pos,delta:gpui::ScrollDelta::Pixels(point(px(0.),px(42.))),
            modifiers:Modifiers::none(),touch_phase:gpui::TouchPhase::Moved,..Default::default()});
        cx.simulate_mouse_down(pos,MouseButton::Left,Modifiers::none());
        cx.simulate_mouse_move(point(px(575.),px(350.)),Some(MouseButton::Left),Modifiers::none());
        cx.simulate_mouse_move(point(px(650.),px(350.)),Some(MouseButton::Left),Modifiers::none());
        cx.simulate_mouse_up(point(px(650.),px(350.)),MouseButton::Left,Modifiers::none());cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(60));
        assert!(!output.lock().unwrap().iter().any(|command|matches!(command,SessionCommand::Input{..}|SessionCommand::SourceInput{..})),
            "Local view navigation injected input into the remote app");
        view.update(cx,|view,cx|{view.viewport.lock().unwrap().toggle_navigation();view.poll_viewport();cx.notify();});cx.run_until_parked();
        assert!(matches!(view.read_with(cx,|view,_|view.pointer_event_at(pos)),Some(InputEvent::MouseMoveAbsolute{x,..}) if x<32000),
            "Local drag did not pan the painted source");
        view.update(cx,|view,cx|{view.viewport.lock().unwrap().reset();view.poll_viewport();cx.notify();});cx.run_until_parked();
    }
    output.lock().unwrap().clear();
    cx.simulate_mouse_down(pos, MouseButton::Left, Modifiers::none());
    std::thread::sleep(Duration::from_millis(60));
    assert!(
        output.lock().unwrap().iter().any(|c| matches!(
            c,
            SessionCommand::Input {
                event: InputEvent::MouseDown(0),
                ..
            } | SessionCommand::SourceInput {
                event: InputEvent::MouseDown(0),
                ..
            }
        )),
        "recovery fixture drag never began"
    );
    recovery
        .video_recovery
        .require_keyframe(recovery.video_recovery.snapshot().epoch.source);
    view.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();
    std::thread::sleep(Duration::from_millis(60));
    assert!(
        output
            .lock()
            .unwrap()
            .iter()
            .any(|c| matches!(c, SessionCommand::ReleaseInput { .. })),
        "reference recovery did not release held input"
    );
    output.lock().unwrap().clear();
    cx.simulate_mouse_up(pos, MouseButton::Left, Modifiers::none());
    cx.simulate_event(KeyDownEvent {
        keystroke: Keystroke::parse("c").unwrap(),
        is_held: false,
        prefer_character_input: false,
    });
    cx.simulate_event(KeyUpEvent {
        keystroke: Keystroke::parse("c").unwrap(),
    });
    std::thread::sleep(Duration::from_millis(60));
    assert!(
        !output.lock().unwrap().iter().any(|c| matches!(
            c,
            SessionCommand::Input { .. } | SessionCommand::SourceInput { .. }
        )),
        "recovering video accepted remote input"
    );
    recovery
        .video_recovery
        .publish(recovery.video_recovery.snapshot().epoch, || ());
    view.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();
    cx.simulate_click(pos, Modifiers::none());
    std::thread::sleep(Duration::from_millis(60));
    assert!(
        !output.lock().unwrap().iter().any(|c| matches!(
            c,
            SessionCommand::Input { .. } | SessionCommand::SourceInput { .. }
        )),
        "decoder readiness without a newly displayed frame resumed input"
    );
    view.update(cx, |v, _| v.owner.close_all());
    recorder.abort();
}
