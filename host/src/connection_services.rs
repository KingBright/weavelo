use super::*;

pub(super) struct ConnectionServicesConfig {
    pub(super) clipboard_owner: remote_core::clipboard_file_runtime::ClipboardOwnerSlot,
    pub(super) clipboard_owner_rx: watch::Receiver<Option<remote_core::clipboard_file_runtime::ClipboardOwnership>>,
    pub(super) session_id: u32,
    pub(super) scheduled_sender: ScheduledDataSender,
    pub(super) cancel_rx: broadcast::Receiver<()>,
    pub(super) file_inbound_rx: Option<mpsc::Receiver<DataEnvelope>>,
    pub(super) talkback_inbound_rx: Option<mpsc::Receiver<DataEnvelope>>,
    #[cfg(target_os = "macos")]
    pub(super) talkback_settings_rx:
        Option<watch::Receiver<crate::talkback_player::TalkbackPlaybackSettings>>,
    #[cfg(not(target_os = "macos"))]
    pub(super) talkback_settings_rx: Option<watch::Receiver<()>>,
    pub(super) host_send_file: Option<PathBuf>,
}

pub(super) async fn run(config: ConnectionServicesConfig) -> Result<(), String> {
    let mut tasks = tokio::task::JoinSet::new();
    let ConnectionServicesConfig {
        clipboard_owner,
        mut clipboard_owner_rx,
        session_id,
        scheduled_sender,
        mut cancel_rx,
        file_inbound_rx,
        talkback_inbound_rx,
        talkback_settings_rx,
        host_send_file,
    } = config;
    #[cfg(not(target_os = "macos"))]
    let _ = (session_id, talkback_inbound_rx, talkback_settings_rx);
    let scheduled_media_sender = Some(scheduled_sender);
    let (_file_command_tx_guard, _file_transfer_task, _file_event_logger) =
        if let (Some(inbound_rx), Some(sender)) = (file_inbound_rx, scheduled_media_sender.clone())
        {
            let (command_tx, command_rx) = mpsc::channel(16);
            let (event_tx, mut event_rx) = mpsc::unbounded_channel();
            let (forward_tx, forward_rx) = mpsc::unbounded_channel();
            let clipboard_commands = command_tx.clone();
            let mut clipboard_cancel = cancel_rx.resubscribe();
            let bridge_owner = clipboard_owner.clone();
            tasks.spawn(async move {
                let state = Arc::new(std::sync::Mutex::new(remote_core::clipboard_file_runtime::ClipboardFileSyncState::default()));
                let mut bridge: Option<(remote_core::clipboard_file_runtime::ClipboardOwnership, mpsc::UnboundedSender<FileTransferEvent>, broadcast::Sender<()>)> = None;
                let mut pending = Vec::new();
                loop {
                    tokio::select! {
                        biased;
                        _ = clipboard_cancel.recv() => break,
                        changed = clipboard_owner_rx.changed() => {
                            if changed.is_err() { break; }
                            if let Some((_, _, cancel)) = bridge.take() { let _ = cancel.send(()); }
                            pending.retain(|event| matches!(event, FileTransferEvent::IncomingClipboardReady { ownership: Some(token), .. } if token.current()));
                            let owner = clipboard_owner_rx.borrow_and_update().clone();
                            if let Some(owner) = owner.filter(|owner| owner.current()) {
                                #[cfg(target_os = "macos")]
                                let provider = MacClipboardProvider::new();
                                #[cfg(target_os = "windows")]
                                let provider = WindowsClipboardProvider::new();
                                #[cfg(target_os = "linux")]
                                let provider = LinuxClipboardProvider::new();
                                let (tx, rx) = mpsc::unbounded_channel();
                                let (cancel, cancel_rx) = broadcast::channel(1);
                                for event in pending.drain(..) {
                                    if matches!(&event, FileTransferEvent::IncomingClipboardReady { ownership: Some(token), .. } if token == &owner && token.current()) { let _ = tx.send(event); }
                                }
                                bridge = Some((owner.clone(), tx, cancel));
                                let commands = clipboard_commands.clone();
                                let state = state.clone();
                                tokio::spawn(async move {
                                    let gate = owner.0;
                                    let observation = remote_core::clipboard_file_runtime::process_clipboard_observation();
                                    let _ = remote_core::clipboard_file_runtime::run_clipboard_file_sync(
                                        remote_core::clipboard_file_runtime::OwnedClipboardProvider::new(provider, gate.clone()).with_observation(observation.clone()),
                                        commands, rx, None, cancel_rx,
                                        remote_core::clipboard_file_runtime::ClipboardFileSyncConfig { enabled: Some(gate), state: Some(state), observation: Some(observation), ..Default::default() },
                                    ).await;
                                });
                            }
                        }
                        event = event_rx.recv() => {
                            let Some(event) = event else { break };
                            if let FileTransferEvent::IncomingClipboardReady { ownership: Some(owner), .. } = &event {
                                if owner.current() {
                                    if let Some((current, tx, _)) = &bridge {
                                        if current == owner { let _ = tx.send(event.clone()); }
                                        else { pending.push(event.clone()); }
                                    } else { pending.push(event.clone()); }
                                }
                            }
                            let _ = forward_tx.send(event);
                        }
                    }
                }
                if let Some((_, _, cancel)) = bridge { let _ = cancel.send(()); }
            });
            event_rx = forward_rx;
            let _file_clipboard_enabled = env_flag_enabled("WEAVELO_FILE_CLIPBOARD");
            let receive_dir = remote_core::desktop_storage::receive_directory()
                .map_err(|error| format!("Independent file receive directory unavailable: {error}"))?;
            let runtime_cancel_rx = cancel_rx.resubscribe();
            let runtime_task = tasks.spawn(async move {
                if let Err(err) = run_file_transfer_runtime(
                    sender,
                    command_rx,
                    inbound_rx,
                    event_tx,
                    runtime_cancel_rx,
                    FileTransferRuntimeConfig {
                        receive_dir,
                        clipboard_owner: Some(bridge_owner),
                        ..FileTransferRuntimeConfig::default()
                    },
                )
                .await
                {
                    eprintln!("File transfer task error: {}", err);
                }
            });
            let logger_cancel_rx = cancel_rx.resubscribe();
            let event_logger = tasks.spawn(async move {
                let mut cancel_rx = logger_cancel_rx;
                loop {
                    tokio::select! {
                        _ = cancel_rx.recv() => break,
                        event = event_rx.recv() => {
                            let Some(event) = event else { break };
                            log_file_transfer_event("Host", event);
                        }
                    }
                }
            });
            if let Some(send_file) = host_send_file {
                let auto_send_tx = command_tx.clone();
                tasks.spawn(async move {
                    tokio::time::sleep(Duration::from_millis(300)).await;
                    println!(
                        "Host auto-enqueueing send file on session startup: {}",
                        send_file.display()
                    );
                    if let Err(err) = auto_send_tx
                        .send(FileTransferCommand::SendFile {
                            path: send_file,
                            mime_type: None,
                        })
                        .await
                    {
                        eprintln!("Failed to enqueue initial host send file: {}", err);
                    }
                });
            }
            let command_tx_guard = Some(command_tx);
            (command_tx_guard, Some(runtime_task), Some(event_logger))
        } else {
            (None, None, None)
        };

    #[cfg(target_os = "macos")]
    let mut talkback_player = if let (Some(inbound_rx), Some(settings_rx)) =
        (talkback_inbound_rx, talkback_settings_rx)
    {
        match crate::talkback_player::TalkbackPlayer::new(session_id, inbound_rx, settings_rx) {
            Ok(player) => Some(player),
            Err(err) => {
                eprintln!("Failed to initialize viewer talkback playback: {}", err);
                None
            }
        }
    } else {
        None
    };

    let _ = cancel_rx.recv().await;
    tasks.abort_all();
    let mut failed = false;
    while let Some(result) = tasks.join_next().await {
        failed |= matches!(result,Err(error) if !error.is_cancelled());
    }
    #[cfg(target_os = "macos")]
    if let Some(player) = talkback_player.as_mut() {
        failed |= player.shutdown().await.is_err();
    }
    if failed {
        Err("connection service child failed during teardown".into())
    } else {
        Ok(())
    }
}

pub(super) fn start_clipboard(
    sender: ScheduledDataSender,
    gate: Arc<std::sync::atomic::AtomicBool>,
    history: Arc<tokio::sync::Mutex<remote_core::clipboard_sync::ClipboardSyncHistory>>,
    serial: Arc<tokio::sync::Mutex<()>>,
) -> (
    mpsc::Sender<DataEnvelope>,
    broadcast::Sender<()>,
    tokio::task::JoinHandle<()>,
) {
    let (inbound_tx, inbound_rx) = mpsc::channel(256);
    let (cancel, cancel_rx) = broadcast::channel(2);
    let task = tokio::spawn(async move {
        let _serial = serial.lock().await;
        if !gate.load(std::sync::atomic::Ordering::Acquire) { return; }
        #[cfg(target_os = "macos")]
        let provider = MacClipboardProvider::new();
        #[cfg(target_os = "linux")]
        let provider = LinuxClipboardProvider::new();
        #[cfg(target_os = "windows")]
        let provider = WindowsClipboardProvider::new();
        let observation = remote_core::clipboard_file_runtime::process_clipboard_observation();
        if let Err(error) = remote_core::clipboard_runtime::run_owned_clipboard_sync(
            remote_core::clipboard_file_runtime::OwnedClipboardProvider::new(provider, gate.clone()).with_observation(observation.clone()),
            sender,
            inbound_rx,
            cancel_rx,
            ClipboardSyncRunnerConfig { observation: Some(observation), ..Default::default() },
            gate,
            history,
        )
        .await
        {
            eprintln!("Clipboard: {error}");
        }
    });
    (inbound_tx, cancel, task)
}
