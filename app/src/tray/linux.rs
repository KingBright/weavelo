use super::{
    Action, Error,
    menu::{Menu as MenuTree, Source},
};
use std::{collections::HashMap, sync::Mutex};
use tokio::sync::mpsc::UnboundedSender;
use zbus::zvariant::{ObjectPath, OwnedValue, Str, Value};

type Properties = HashMap<String, OwnedValue>;
type Layout = (i32, Properties, Vec<OwnedValue>);
struct Item {
    actions: UnboundedSender<Action>,
}

#[zbus::interface(name = "org.kde.StatusNotifierItem")]
impl Item {
    #[zbus(property)]
    fn category(&self) -> &str {
        "ApplicationStatus"
    }
    #[zbus(property)]
    fn id(&self) -> &str {
        "Weavelo"
    }
    #[zbus(property)]
    fn title(&self) -> &str {
        "Weavelo"
    }
    #[zbus(property)]
    fn status(&self) -> &str {
        "Active"
    }
    #[zbus(property)]
    fn window_id(&self) -> u32 {
        0
    }
    #[zbus(property)]
    fn icon_name(&self) -> &str {
        "fun.hackerlife.weavelo.desktop"
    }
    #[zbus(property)]
    fn icon_pixmap(&self) -> Vec<(i32, i32, Vec<u8>)> {
        vec![]
    }
    #[zbus(property)]
    fn item_is_menu(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn menu(&self) -> ObjectPath<'static> {
        ObjectPath::try_from("/MenuBar").expect("static menu path")
    }
    #[zbus(property)]
    fn tool_tip(&self) -> (String, Vec<(i32, i32, Vec<u8>)>, String, String) {
        (
            self.icon_name().into(),
            vec![],
            "Weavelo".into(),
            crate::i18n::tr("Devices, streams and settings").into(),
        )
    }
    fn activate(&self, _x: i32, _y: i32) {
        let _ = self.actions.send(Action::Open);
    }
    fn secondary_activate(&self, _x: i32, _y: i32) {
        let _ = self.actions.send(Action::Open);
    }
    fn context_menu(&self, _x: i32, _y: i32) {}
    fn scroll(&self, _delta: i32, _orientation: &str) {}
}

struct Menu {
    actions: UnboundedSender<Action>,
    source: Source,
    tree: Mutex<MenuTree>,
}
fn properties(tree: &MenuTree, id: i32, requested: &[String]) -> Properties {
    let mut values = Properties::new();
    if id == 0 {
        values.insert(
            "children-display".into(),
            OwnedValue::from(Str::from("submenu")),
        );
    } else {
        let Some(entry) = tree.find(id) else {
            return values;
        };
        // D-Bus menus use underscores for mnemonics. Device/window names are literal.
        values.insert(
            "label".into(),
            OwnedValue::from(Str::from(entry.label.replace('_', "__"))),
        );
        values.insert("enabled".into(), entry.enabled.into());
        values.insert("visible".into(), true.into());
        if !entry.children.is_empty() {
            values.insert(
                "children-display".into(),
                OwnedValue::from(Str::from("submenu")),
            );
        }
    }
    if !requested.is_empty() {
        values.retain(|key, _| requested.contains(key));
    }
    values
}
fn layout(tree: &MenuTree, id: i32, depth: i32, requested: &[String]) -> Layout {
    let entries = if id == 0 {
        tree.roots.as_slice()
    } else {
        tree.find(id).map_or(&[][..], |e| e.children.as_slice())
    };
    let children = if depth != 0 {
        entries
            .iter()
            .map(|entry| {
                OwnedValue::try_from(Value::from(layout(
                    tree,
                    entry.id,
                    if depth < 0 { -1 } else { depth - 1 },
                    requested,
                )))
                .expect("native menu layout")
            })
            .collect()
    } else {
        vec![]
    };
    (id, properties(tree, id, requested), children)
}
fn known(tree: &MenuTree, id: i32) -> bool {
    id == 0 || tree.find(id).is_some()
}
fn all_ids(tree: &MenuTree) -> Vec<i32> {
    fn walk(entries: &[super::menu::Entry], ids: &mut Vec<i32>) {
        for entry in entries {
            ids.push(entry.id);
            walk(&entry.children, ids);
        }
    }
    let mut ids = vec![0];
    walk(&tree.roots, &mut ids);
    ids
}

#[zbus::interface(name = "com.canonical.dbusmenu")]
impl Menu {
    #[zbus(property)]
    fn version(&self) -> u32 {
        3
    }
    #[zbus(property)]
    fn text_direction(&self) -> &str {
        "ltr"
    }
    #[zbus(property)]
    fn status(&self) -> &str {
        "normal"
    }
    #[zbus(property)]
    fn icon_theme_path(&self) -> Vec<String> {
        vec![]
    }
    fn get_layout(
        &self,
        parent_id: i32,
        recursion_depth: i32,
        property_names: Vec<String>,
    ) -> zbus::fdo::Result<(u32, Layout)> {
        let tree = self.tree.lock().unwrap();
        if !known(&tree, parent_id) {
            return Err(zbus::fdo::Error::InvalidArgs("Unknown menu item".into()));
        }
        Ok((
            tree.revision,
            layout(&tree, parent_id, recursion_depth, &property_names),
        ))
    }
    fn get_group_properties(
        &self,
        ids: Vec<i32>,
        property_names: Vec<String>,
    ) -> Vec<(i32, Properties)> {
        let tree = self.tree.lock().unwrap();
        let ids = if ids.is_empty() { all_ids(&tree) } else { ids };
        ids.into_iter()
            .filter(|id| known(&tree, *id))
            .map(|id| (id, properties(&tree, id, &property_names)))
            .collect()
    }
    fn get_property(&self, id: i32, name: &str) -> zbus::fdo::Result<OwnedValue> {
        let tree = self.tree.lock().unwrap();
        if !known(&tree, id) {
            return Err(zbus::fdo::Error::InvalidArgs("Unknown menu item".into()));
        }
        properties(&tree, id, &[])
            .remove(name)
            .ok_or_else(|| zbus::fdo::Error::InvalidArgs("Unknown menu property".into()))
    }
    fn event(&self, id: i32, event_id: &str, _data: OwnedValue, _timestamp: u32) {
        if event_id == "clicked" {
            if let Some(action) = self.tree.lock().unwrap().action(id) {
                let _ = self.actions.send(action);
            }
        }
    }
    fn event_group(&self, events: Vec<(i32, String, OwnedValue, u32)>) -> Vec<i32> {
        let mut unknown = vec![];
        for (id, event, data, timestamp) in events {
            if known(&self.tree.lock().unwrap(), id) {
                self.event(id, &event, data, timestamp);
            } else {
                unknown.push(id);
            }
        }
        unknown
    }
    fn about_to_show(&self, id: i32) -> bool {
        if id != 0 {
            return false;
        }
        self.source.refresh();
        self.tree.lock().unwrap().rebuild(&self.source.current());
        true
    }
    fn about_to_show_group(&self, ids: Vec<i32>) -> (Vec<i32>, Vec<i32>) {
        let updated = if ids.contains(&0) {
            self.about_to_show(0);
            vec![0]
        } else {
            vec![]
        };
        let tree = self.tree.lock().unwrap();
        (
            updated,
            ids.into_iter().filter(|id| !known(&tree, *id)).collect(),
        )
    }
}

pub(super) struct Tray {
    _connection: zbus::Connection,
}
impl Tray {
    pub async fn start(actions: UnboundedSender<Action>, source: Source) -> Result<Self, Error> {
        let mut tree = MenuTree::default();
        tree.rebuild(&source.current());
        let connection = zbus::connection::Builder::session()?
            .serve_at(
                "/StatusNotifierItem",
                Item {
                    actions: actions.clone(),
                },
            )?
            .serve_at(
                "/MenuBar",
                Menu {
                    actions,
                    source,
                    tree: Mutex::new(tree),
                },
            )?
            .build()
            .await?;
        let watcher = zbus::Proxy::new(
            &connection,
            "org.kde.StatusNotifierWatcher",
            "/StatusNotifierWatcher",
            "org.kde.StatusNotifierWatcher",
        )
        .await?;
        let name = connection
            .unique_name()
            .ok_or("Tray session-bus identity unavailable")?
            .as_str();
        let _: () = watcher.call("RegisterStatusNotifierItem", &name).await?;
        drop(watcher);
        Ok(Self {
            _connection: connection,
        })
    }
}
