//! Cached native menus never wait for GUI IPC on the platform event thread.
use super::{Action, Error};
use crate::{
    UnifiedRuntimeHandle,
    desktop::{
        directed_receive::{self, Request},
        menu::{Call, DeviceAction, Section, Snapshot, StreamAction},
    },
};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::sync::{Notify, watch};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Device {
    pub id: String,
    pub name: String,
    pub online: bool,
    pub can_send: bool,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Model {
    pub gui: Option<Snapshot>,
    pub devices: Vec<Device>,
    pub sharing: usize,
    pub error: Option<String>,
}
impl Model {
    pub fn summary(&self) -> String {
        let receiving = self.gui.as_ref().map_or(0, |s| s.receiving_count);
        if receiving == 0 && self.sharing == 0 {
            "Weavelo".into()
        } else {
            format!("Weavelo · Receiving {receiving} · Sharing {}", self.sharing)
        }
    }
}

#[derive(Clone)]
pub(super) struct Source {
    current: watch::Sender<Model>,
    refresh: Arc<Notify>,
}
fn read_gui(profile: &Path, hash: &str) -> Result<Option<Snapshot>, Error> {
    let Some(directory) = crate::private_ipc::gui_directory_if_present(profile)? else {
        return Ok(None);
    };
    if crate::private_ipc::absent(&directory, "gui-receive.sock")? {
        return Ok(None);
    }
    let snapshot: Snapshot =
        serde_json::from_value(directed_receive::exchange(&directory, &Request::Menu)?)?;
    if !snapshot.ok
        || snapshot.executable_sha256 != hash
        || snapshot.renderer != crate::gui_backend::ORIGINAL_RENDERER
    {
        return Err("The Weavelo window has a different build or renderer".into());
    }
    Ok(Some(snapshot))
}
impl Source {
    pub fn start(
        profile: PathBuf,
        runtime: Arc<UnifiedRuntimeHandle>,
        hash: String,
    ) -> (Self, crate::AbortOnDropTask) {
        let (current, _) = watch::channel(Model::default());
        let source = Self {
            current,
            refresh: Arc::new(Notify::new()),
        };
        let saved = source.clone();
        let monitor = tokio::spawn(async move {
            loop {
                let selected = profile.clone();
                let expected = hash.clone();
                let result =
                    tokio::task::spawn_blocking(move || read_gui(&selected, &expected)).await;
                let (gui, error) = match result {
                    Ok(Ok(gui)) => (gui, None),
                    Ok(Err(error)) => (None, Some(error.to_string())),
                    Err(_) => (None, Some("Window status is unavailable".into())),
                };
                let mut devices: Vec<_> = runtime
                    .owner
                    .runtime()
                    .lock()
                    .unwrap()
                    .devices()
                    .into_iter()
                    .map(|d| Device {
                        id: d.device_id,
                        name: d.display_name,
                        online: d.online,
                        can_send: d.online && d.can_view,
                    })
                    .collect();
                devices.sort_by_key(|d| (!d.online, d.name.to_lowercase(), d.id.clone()));
                let sharing = gui
                    .as_ref()
                    .map_or_else(host::lifecycle::active_video_streams, |s| s.sharing);
                let next = Model {
                    gui,
                    devices,
                    sharing,
                    error,
                };
                saved.current.send_if_modified(|old| {
                    if *old == next {
                        false
                    } else {
                        *old = next;
                        true
                    }
                });
                tokio::select! {_=tokio::time::sleep(Duration::from_millis(500))=>{},_=saved.refresh.notified()=>{}}
            }
        });
        (source, crate::AbortOnDropTask(monitor))
    }
    pub fn current(&self) -> Model {
        self.current.borrow().clone()
    }
    pub fn refresh(&self) {
        self.refresh.notify_one();
    }
}

pub(super) async fn dispatch(
    profile: &Path,
    hash: &str,
    expected: Option<String>,
    call: Call,
) -> Result<(), Error> {
    let selected = profile.to_path_buf();
    let image = hash.to_owned();
    let current = tokio::task::spawn_blocking(move || read_gui(&selected, &image)).await??;
    let current = if let Some(current) = current {
        current
    } else {
        if !call.can_start_gui() {
            return Err("This window has ended".into());
        }
        super::open_gui(profile).await?;
        let selected = profile.to_path_buf();
        let image = hash.to_owned();
        tokio::task::spawn_blocking(move || {
            let until = std::time::Instant::now() + Duration::from_secs(4);
            loop {
                if let Some(current) = read_gui(&selected, &image)? {
                    return Ok::<_, Error>(current);
                }
                if std::time::Instant::now() >= until {
                    return Err("The Weavelo window is still starting".into());
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        })
        .await??
    };
    if expected.as_ref().is_some_and(|s| s != &current.generation) {
        return Err("The window owner changed; reopen the menu".into());
    }
    let directory = crate::private_ipc::gui_directory(profile)?;
    let request = Request::MenuAction {
        generation: current.generation,
        request_id: format!(
            "tray-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        ),
        deadline_ms: crate::unix_now_ms() + 5000,
        call,
    };
    let reply =
        tokio::task::spawn_blocking(move || directed_receive::exchange(&directory, &request))
            .await??;
    if reply["ok"] != true {
        return Err(reply["error"]
            .as_str()
            .unwrap_or("Menu action was not completed")
            .to_owned()
            .into());
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub(super) struct Entry {
    pub id: i32,
    pub label: String,
    pub enabled: bool,
    pub children: Vec<Entry>,
}
#[derive(Clone, Default)]
pub(super) struct Menu {
    pub revision: u32,
    pub roots: Vec<Entry>,
    actions: BTreeMap<i32, Action>,
    next: i32,
}
fn label(text: &str) -> String {
    let mut chars = text.chars().filter(|c| !c.is_control());
    let first: String = chars.by_ref().take(80).collect();
    if chars.next().is_some() {
        format!("{first}…")
    } else {
        first
    }
}
impl Menu {
    fn item(
        &mut self,
        text: &str,
        enabled: bool,
        action: Option<Action>,
        children: Vec<Entry>,
    ) -> Entry {
        self.next = self.next.checked_add(1).expect("native menu IDs exhausted");
        let id = self.next;
        if let Some(action) = action {
            self.actions.insert(id, action);
        }
        Entry {
            id,
            label: label(text),
            enabled,
            children,
        }
    }
    pub fn rebuild(&mut self, model: &Model) {
        self.actions.clear();
        self.roots.clear();
        self.revision = self.revision.wrapping_add(1);
        let ui = |call: Call| Action::Ui {
            generation: if call.can_start_gui() {
                None
            } else {
                model.gui.as_ref().map(|g| g.generation.clone())
            },
            call,
        };
        let mut roots = vec![
            self.item(&model.summary(), false, None, vec![]),
            self.item("Open Weavelo", true, Some(Action::Open), vec![]),
        ];
        if let Some(gui) = &model.gui {
            if !gui.receiving.is_empty() {
                let mut streams = vec![];
                for stream in &gui.receiving {
                    let target = stream.target.clone();
                    let children = vec![
                        self.item(
                            "Open window",
                            true,
                            Some(ui(Call::Stream {
                                target: target.clone(),
                                action: StreamAction::Open,
                            })),
                            vec![],
                        ),
                        self.item(
                            if stream.paused {
                                "Resume"
                            } else {
                                "Pause media"
                            },
                            stream.can_pause,
                            Some(ui(Call::Stream {
                                target: target.clone(),
                                action: StreamAction::Pause(!stream.paused),
                            })),
                            vec![],
                        ),
                        self.item(
                            "End this stream",
                            true,
                            Some(ui(Call::Stream {
                                target,
                                action: StreamAction::End,
                            })),
                            vec![],
                        ),
                    ];
                    streams.push(self.item(&stream.title, true, None, children));
                }
                roots.push(self.item("Stream windows", true, None, streams));
            }
            if !gui.sending.is_empty() {
                let mut sends = vec![];
                for send in &gui.sending {
                    let children = vec![
                        self.item(
                            "Open sending window",
                            true,
                            Some(ui(Call::OpenSend(send.request.clone()))),
                            vec![],
                        ),
                        self.item(
                            "Stop this send",
                            true,
                            Some(ui(Call::StopSend(send.request.clone()))),
                            vec![],
                        ),
                    ];
                    sends.push(self.item(&send.title, true, None, children));
                }
                roots.push(self.item("Sending", true, None, sends));
            }
        }
        let mut devices = vec![];
        let mut files = vec![];
        for device in &model.devices {
            let call = |action| {
                Some(ui(Call::Device {
                    id: device.id.clone(),
                    action,
                }))
            };
            let children = vec![
                self.item(
                    "View screen or window",
                    device.online,
                    call(DeviceAction::View),
                    vec![],
                ),
                self.item(
                    "Send screen or window",
                    device.can_send,
                    call(DeviceAction::Send),
                    vec![],
                ),
                self.item("Files", device.online, call(DeviceAction::Files), vec![]),
                self.item("Device details", true, call(DeviceAction::Details), vec![]),
            ];
            let name = if device.online {
                device.name.clone()
            } else {
                format!("{} · Offline", device.name)
            };
            devices.push(self.item(&name, true, None, children));
            if device.online {
                files.push(self.item(&device.name, true, call(DeviceAction::Files), vec![]));
            }
        }
        if !devices.is_empty() {
            roots.push(self.item("Devices", true, None, devices));
        }
        if !files.is_empty() {
            roots.push(self.item("Send files to…", true, None, files));
        }
        roots.push(self.item(
            "Transfer center",
            true,
            Some(ui(Call::Manager(Section::Transfers))),
            vec![],
        ));
        let settings = vec![
            self.item(
                "Sharing & permissions",
                true,
                Some(ui(Call::Manager(Section::Sharing))),
                vec![],
            ),
            self.item(
                "Preferences",
                true,
                Some(ui(Call::Manager(Section::Preferences))),
                vec![],
            ),
            self.item(
                "Pairing & device name",
                true,
                Some(ui(Call::Manager(Section::Pairing))),
                vec![],
            ),
        ];
        roots.push(self.item("Sharing & settings", true, None, settings));
        if model.gui.as_ref().is_some_and(|g| !g.sending.is_empty()) {
            roots.push(self.item(
                "Stop all sent streams",
                true,
                Some(ui(Call::StopAllSends)),
                vec![],
            ));
        }
        if model.error.is_some() {
            roots.push(self.item("Window status unavailable", false, None, vec![]));
        }
        roots.push(self.item("Quit Weavelo", true, Some(Action::Quit), vec![]));
        self.roots = roots;
    }
    pub fn action(&self, id: i32) -> Option<Action> {
        self.find(id).filter(|e| e.enabled)?;
        self.actions.get(&id).cloned()
    }
    pub fn find(&self, id: i32) -> Option<&Entry> {
        fn walk(entries: &[Entry], id: i32) -> Option<&Entry> {
            for entry in entries {
                if entry.id == id {
                    return Some(entry);
                }
                if let Some(e) = walk(&entry.children, id) {
                    return Some(e);
                }
            }
            None
        }
        walk(&self.roots, id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    #[test]
    fn closed_gui_is_absent_and_two_endpoint_owners_still_fail() {
        use std::os::unix::{fs::PermissionsExt, net::UnixListener};
        let profile = tempfile::tempdir().unwrap();
        std::fs::set_permissions(profile.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(read_gui(profile.path(), &"1".repeat(64)).unwrap().is_none());
        assert!(
            crate::private_ipc::gui_directory(profile.path()).is_err(),
            "commands still require an owner"
        );
        let attached = profile.path().join("gui-state");
        std::fs::create_dir(&attached).unwrap();
        std::fs::set_permissions(&attached, std::fs::Permissions::from_mode(0o700)).unwrap();
        let standalone = UnixListener::bind(profile.path().join("gui-receive.sock")).unwrap();
        std::fs::set_permissions(
            profile.path().join("gui-receive.sock"),
            std::fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        assert_eq!(
            crate::private_ipc::gui_directory_if_present(profile.path()).unwrap(),
            Some(profile.path().to_owned())
        );
        let frontend = UnixListener::bind(attached.join("gui-receive.sock")).unwrap();
        std::fs::set_permissions(
            attached.join("gui-receive.sock"),
            std::fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        assert!(
            read_gui(profile.path(), &"1".repeat(64)).is_err(),
            "ambiguous owners never launch a replacement"
        );
        drop((standalone, frontend));
    }
    #[test]
    fn reordered_devices_never_reuse_native_action_ids() {
        let mut model = Model {
            devices: vec![Device {
                id: "a".into(),
                name: "A".into(),
                online: true,
                can_send: true,
            }],
            ..Default::default()
        };
        let mut menu = Menu::default();
        menu.rebuild(&model);
        let old=menu.actions.iter().find(|(_,a)|matches!(a,Action::Ui{call:Call::Device{id,action:DeviceAction::Files},..} if id=="a")).map(|(id,_)|*id).unwrap();
        model.devices[0].id = "b".into();
        menu.rebuild(&model);
        assert!(menu.action(old).is_none());
        assert!(menu.actions.values().any(|a|matches!(a,Action::Ui{call:Call::Device{id,action:DeviceAction::Files},..} if id=="b")));
    }
    #[test]
    fn menu_keeps_window_generation_and_source_target() {
        let target = crate::desktop::menu::StreamTarget {
            entity: 9,
            device: "a".into(),
            source: Some(protocol::session::CaptureSource::Window(7)),
        };
        let model = Model {
            gui: Some(Snapshot {
                ok: true,
                generation: "one".into(),
                receiving: vec![crate::desktop::menu::Receiving {
                    target: target.clone(),
                    title: "A".into(),
                    paused: false,
                    can_pause: true,
                }],
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut menu = Menu::default();
        menu.rebuild(&model);
        assert!(menu.actions.values().any(|a| *a
            == Action::Ui {
                generation: Some("one".into()),
                call: Call::Stream {
                    target: target.clone(),
                    action: StreamAction::End
                }
            }));
    }
}
