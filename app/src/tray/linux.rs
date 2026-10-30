use super::{Action, Error};
use std::collections::HashMap;
use tokio::sync::mpsc::UnboundedSender;
use zbus::zvariant::{ObjectPath, OwnedValue, Str, Value};

type Properties = HashMap<String, OwnedValue>;
type Layout = (i32, Properties, Vec<OwnedValue>);
struct Item { actions: UnboundedSender<Action> }

#[zbus::interface(name = "org.kde.StatusNotifierItem")]
impl Item {
    #[zbus(property)]
    fn category(&self) -> &str { "ApplicationStatus" }
    #[zbus(property)]
    fn id(&self) -> &str { "Weavelo" }
    #[zbus(property)]
    fn title(&self) -> &str { "Weavelo" }
    #[zbus(property)]
    fn status(&self) -> &str { "Active" }
    #[zbus(property)]
    fn window_id(&self) -> u32 { 0 }
    #[zbus(property)]
    fn icon_name(&self) -> &str { "fun.hackerlife.weavelo.desktop" }
    #[zbus(property)]
    fn icon_pixmap(&self) -> Vec<(i32,i32,Vec<u8>)> { vec![] }
    #[zbus(property)]
    fn item_is_menu(&self) -> bool { false }
    #[zbus(property)]
    fn menu(&self) -> ObjectPath<'static> { ObjectPath::try_from("/MenuBar").expect("static menu path") }
    #[zbus(property)]
    fn tool_tip(&self) -> (String,Vec<(i32,i32,Vec<u8>)>,String,String) {
        (self.icon_name().into(),vec![],"Weavelo".into(),"Open Weavelo or quit the app".into())
    }
    fn activate(&self, _x: i32, _y: i32) { let _=self.actions.send(Action::Open); }
    fn secondary_activate(&self, _x: i32, _y: i32) { let _=self.actions.send(Action::Open); }
    fn context_menu(&self, _x: i32, _y: i32) {}
    fn scroll(&self, _delta: i32, _orientation: &str) {}
}

struct Menu { actions: UnboundedSender<Action> }
fn properties(id: i32, requested: &[String]) -> Properties {
    let mut values = Properties::new();
    if id==0 { values.insert("children-display".into(),OwnedValue::from(Str::from("submenu"))); }
    else {
        let label=if id==1 {"Open Weavelo"} else {"Quit Weavelo"};
        values.insert("label".into(),OwnedValue::from(Str::from(label)));
        values.insert("enabled".into(),true.into());values.insert("visible".into(),true.into());
    }
    if !requested.is_empty() { values.retain(|key,_|requested.contains(key)); }
    values
}
fn layout(id: i32, depth: i32, requested: &[String]) -> Layout {
    let children = if id==0 && depth!=0 {
        [1,2].into_iter().map(|id| {
            OwnedValue::try_from(Value::from((id,properties(id,requested),Vec::<OwnedValue>::new())))
                .expect("static native menu layout")
        }).collect()
    } else {vec![]};
    (id,properties(id,requested),children)
}
fn known(id: i32) -> bool { (0..=2).contains(&id) }

#[zbus::interface(name = "com.canonical.dbusmenu")]
impl Menu {
    #[zbus(property)]
    fn version(&self) -> u32 { 3 }
    #[zbus(property)]
    fn text_direction(&self) -> &str { "ltr" }
    #[zbus(property)]
    fn status(&self) -> &str { "normal" }
    #[zbus(property)]
    fn icon_theme_path(&self) -> Vec<String> { vec![] }
    fn get_layout(&self, parent_id: i32, recursion_depth: i32, property_names: Vec<String>) -> zbus::fdo::Result<(u32,Layout)> {
        if !known(parent_id) {return Err(zbus::fdo::Error::InvalidArgs("Unknown menu item".into()));}
        Ok((1,layout(parent_id,recursion_depth,&property_names)))
    }
    fn get_group_properties(&self, ids: Vec<i32>, property_names: Vec<String>) -> Vec<(i32,Properties)> {
        ids.into_iter().filter(|id|known(*id)).map(|id|(id,properties(id,&property_names))).collect()
    }
    fn get_property(&self, id: i32, name: &str) -> zbus::fdo::Result<OwnedValue> {
        if !known(id) {return Err(zbus::fdo::Error::InvalidArgs("Unknown menu item".into()));}
        properties(id,&[]).remove(name).ok_or_else(||zbus::fdo::Error::InvalidArgs("Unknown menu property".into()))
    }
    fn event(&self, id: i32, event_id: &str, _data: OwnedValue, _timestamp: u32) {
        if event_id=="clicked" {
            match id {1=>{let _=self.actions.send(Action::Open);},2=>{let _=self.actions.send(Action::Quit);},_=>{}}
        }
    }
    fn event_group(&self, events: Vec<(i32,String,OwnedValue,u32)>) -> Vec<i32> {
        let mut unknown=vec![];
        for (id,event,data,timestamp) in events {
            if known(id) {self.event(id,&event,data,timestamp);} else {unknown.push(id);}
        }
        unknown
    }
    fn about_to_show(&self, _id: i32) -> bool { false }
    fn about_to_show_group(&self, ids: Vec<i32>) -> (Vec<i32>,Vec<i32>) {
        (vec![],ids.into_iter().filter(|id|!known(*id)).collect())
    }
}

pub(super) struct Tray { _connection: zbus::Connection }
impl Tray {
    pub async fn start(actions: UnboundedSender<Action>) -> Result<Self, Error> {
        let connection=zbus::connection::Builder::session()?
            .serve_at("/StatusNotifierItem",Item{actions:actions.clone()})?
            .serve_at("/MenuBar",Menu{actions})?.build().await?;
        let watcher=zbus::Proxy::new(&connection,"org.kde.StatusNotifierWatcher",
            "/StatusNotifierWatcher","org.kde.StatusNotifierWatcher").await?;
        let name=connection.unique_name().ok_or("Tray session-bus identity unavailable")?.as_str();
        let _: ()=watcher.call("RegisterStatusNotifierItem",&name).await?;
        drop(watcher);
        Ok(Self{_connection:connection})
    }
}
