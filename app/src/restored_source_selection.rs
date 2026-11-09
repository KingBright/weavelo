use super::*;
use protocol::session::{CaptureSource, CaptureSourceInfo, SessionCommand, SourceCategory, MAX_SUBSCRIPTIONS};
use remote_core::shared_files::{ShareScope, current_share_scope};

pub(super) enum Target {
    Send(String),
    Open(SourceViewBinding),
    Replace(SourceViewBinding),
}
struct CatalogState { revision:u64, ready:bool }
pub(super) fn local_source_catalog(client: &crate::background_service::Client)
    -> Result<(Vec<CaptureSourceInfo>, remote_core::application_icons::SourceIconAssets, Option<String>), String>
{
    let sources=client.capture_sources().map_err(|error|error.to_string())?;
    #[cfg(target_os="linux")]
    let message=if sources.is_empty() {
        client.sharing_call(crate::sharing_permissions::SharingCall::Status).map_err(|error|error.to_string())?.message
    } else {None};
    #[cfg(not(target_os="linux"))]
    let message:Option<String>=None;
    let command=host::capture_sources::with_artwork(sources);
    let SessionCommand::Sources {sources,source_icons,icons,..}=command else {return Err("Invalid local source directory".into());};
    let mut cache=remote_core::application_icons::SourceIconCache::default();
    cache.update(sources.iter().map(|source|source.source),&source_icons,&icons)?;
    Ok((sources,cache.assets(),message))
}
pub(super) struct Selection {
    target: Target,
    scope: ShareScope,
    title: String,
    sources: Vec<CaptureSourceInfo>,
    query: String,
    pub(super) selected: Vec<CaptureSource>,
    artwork: SourceArtwork,
    pub(super) loading: bool,
    error: Option<String>,
    catalog: Option<CatalogState>,
}
impl Selection {
    pub(super) fn is_delivery(&self) -> bool { matches!(self.target, Target::Send(_)) }
    pub(super) fn local_delivery_is_current(&self) -> bool {self.is_delivery()&&self.scope==current_share_scope()}
    pub(super) fn toggle(&mut self,source:CaptureSource) {
        if !self.catalog_ready() {return;}
        if !self.sources.iter().any(|item|item.source==source) {self.error=Some("The source list changed".into());return;}
        if self.selected.contains(&source) {self.selected.retain(|selected|*selected!=source);self.error=None;}
        else if matches!(self.target,Target::Replace(_)) {self.selected=vec![source];self.error=None;}
        else if self.selected.len()<MAX_SUBSCRIPTIONS {self.selected.push(source);self.error=None;}
        else {self.error=Some("Choose up to eight sources".into());}
    }
    fn replace(&mut self, mut sources: Vec<CaptureSourceInfo>, artwork: SourceArtwork) {
        let had_selection=!self.selected.is_empty();
        sources.retain(|source|source.validate_camera().is_ok());
        // MainDisplay is an alias; don't offer it beside the concrete displays.
        if sources.iter().any(|source|matches!(source.source,CaptureSource::Display(_))) {
            sources.retain(|source|source.source!=CaptureSource::MainDisplay);
        }
        let mut seen=std::collections::HashSet::new();
        sources.retain(|source|seen.insert(source.source));
        remote_core::source_picker::sort(&mut sources);
        self.selected.retain(|id| sources.iter().any(|source| source.source == *id));
        self.sources=sources;self.artwork=artwork;self.loading=false;
        self.error=(self.sources.is_empty() && self.is_delivery()).then(||"Choose a local source in Sharing & permissions.".into());
        if self.error.is_none() && had_selection && self.selected.is_empty() {
            self.error=Some("Selected sources ended. Choose another source.".into());
        }
    }
    fn catalog_ready(&self)->bool {self.catalog.as_ref().is_none_or(|catalog|catalog.ready)}
    fn visible_sources(&self)->impl Iterator<Item=&CaptureSourceInfo> {
        let query=self.query.trim().to_lowercase();
        self.sources.iter().filter(move|source|remote_core::source_picker::matches_name(source,&query))
    }
    pub(super) fn fail_remote_query(&mut self,error:String) {
        if let Some(catalog)=self.catalog.as_mut() {catalog.ready=false;self.loading=false;self.error=Some(error);}
    }
}

impl RestoredDashboard {
    pub(super) fn begin_delivery_selection(&mut self, device:String,cx:&mut Context<Self>) {
        if self.background.is_none() {self.status="Sharing service is unavailable".into();cx.notify();return;}
        let name=self.owner.snapshot().devices.iter().find(|item|item.device_id==device)
            .map(|item|item.display_name.clone()).unwrap_or_else(||device.clone());
        self.owner.release_input();self.pointer_input=PointerInputTracker::default();self.show_apps_menu=false;
        let scope=current_share_scope();
        self.source_selection=Some(Selection {target:Target::Send(device),scope,title:format!("Send to {name}"),
            sources:Vec::new(),query:String::new(),selected:Vec::new(),artwork:Arc::new(Default::default()),loading:true,error:None,catalog:None});
        self.refresh_delivery_sources(cx);
        cx.notify();
    }
    pub(super) fn refresh_delivery_sources(&mut self, cx:&mut Context<Self>) {
        let Some(selection)=self.source_selection.as_mut() else {return;};
        let Target::Send(device)=&selection.target else {return;};
        let device=device.clone();let scope=selection.scope;
        let Some(client)=self.background.clone() else {
            selection.loading=false;selection.error=Some("Sharing service is unavailable".into());cx.notify();return;
        };
        selection.loading=true;
        let work=tokio::task::spawn_blocking(move || local_source_catalog(&client));
        cx.spawn(async move |view,cx| {
            let result=work.await.unwrap_or_else(|_|Err("Source lookup stopped; try again".into()));
            let _=view.update(cx,|this,cx| {
                if let Some(selection)=this.source_selection.as_mut()
                    && matches!(&selection.target,Target::Send(target) if *target==device) && selection.scope==scope
                {
                    selection.loading=false;
                    if scope!=current_share_scope() {selection.error=Some("The device network changed; choose the sources again".into());}
                    else {match result {
                        Ok((sources,assets,message))=>{
                            selection.replace(sources,ApplicationArtwork::default().update(assets));
                            if selection.sources.is_empty() && message.is_some() {selection.error=message;}
                        },
                        Err(error)=>selection.error=Some(error),
                    }}
                }
                cx.notify();
            });
        }).detach();
        cx.notify();
    }
    pub(super) fn begin_remote_source_selection(&mut self,binding:Option<SourceViewBinding>,cx:&mut Context<Self>) {
        self.begin_remote_sources(binding,false,cx);
    }
    pub(super) fn begin_remote_source_replacement(&mut self,binding:Option<SourceViewBinding>,cx:&mut Context<Self>) {
        self.begin_remote_sources(binding,true,cx);
    }
    fn begin_remote_sources(&mut self,binding:Option<SourceViewBinding>,replace:bool,cx:&mut Context<Self>) {
        let Some(binding)=binding.filter(|binding|self.owner.source_binding_is_current(binding)) else {return;};
        let snapshot=self.owner.snapshot();
        let query=binding.clone();
        self.owner.release_input();self.pointer_input=PointerInputTracker::default();self.show_apps_menu=false;
        let mut selection=Selection {target:if replace {Target::Replace(binding)}else{Target::Open(binding)},scope:current_share_scope(),title:if replace {"Replace current source"}else{"Open screens & app windows"}.into(),
            sources:Vec::new(),query:String::new(),selected:Vec::new(),artwork:Arc::new(Default::default()),loading:false,error:None,catalog:None};
        let artwork=self.artwork_cache.borrow_mut().update(snapshot.source_icons);
        selection.replace((*snapshot.sources).clone(),artwork);
        // Opening an additional source must not silently include the desktop
        // or the stream already being viewed. Only replacement has a default.
        if replace && selection.sources.iter().any(|source|source.source==snapshot.active_source) {selection.selected=vec![snapshot.active_source];}
        selection.catalog=Some(CatalogState {revision:snapshot.source_catalog_revision,ready:false});
        selection.loading=true;selection.error=None;
        self.source_selection=Some(selection);
        self.request_bound_capture_sources(query,cx);cx.notify();
    }
    pub(super) fn poll_remote_source_selection(&mut self,cx:&mut Context<Self>) {
        let Some((binding,revision))=self.source_selection.as_ref().and_then(|selection| {
            let binding=match &selection.target {Target::Open(binding)|Target::Replace(binding)=>binding.clone(),_=>return None};
            Some((binding,selection.catalog.as_ref()?.revision))
        }) else {return;};
        let Some(current)=self.owner.bound_catalog_revision(&binding) else {
            if let Some(selection)=self.source_selection.as_mut() {
                selection.fail_remote_query("Connection changed. Close and reopen sources.".into());
            }
            return;
        };
        if current==revision {return;}
        let snapshot=self.owner.snapshot();
        if !self.owner.source_binding_is_current(&binding) {return;}
        let artwork=self.artwork_cache.borrow_mut().update(snapshot.source_icons);
        if let Some(selection)=self.source_selection.as_mut() {
            selection.replace((*snapshot.sources).clone(),artwork);
            let ready=snapshot.source_catalog_error.is_none();
            selection.catalog=Some(CatalogState{revision:snapshot.source_catalog_revision,ready});
            if let Some(error)=snapshot.source_catalog_error {selection.error=Some(error);}
        }
        cx.notify();
    }
    pub(super) fn refresh_remote_source_selection(&mut self,cx:&mut Context<Self>) {
        let Some((mut binding,replace,selected,scope))=self.source_selection.as_ref().and_then(|selection| {
            let (binding,replace)=match &selection.target {Target::Open(binding)=>(binding.clone(),false),Target::Replace(binding)=>(binding.clone(),true),_=>return None};
            Some((binding,replace,selection.selected.clone(),selection.scope))
        }) else {return;};
        if scope!=current_share_scope() {
            if let Some(selection)=self.source_selection.as_mut() {selection.fail_remote_query("Device network changed. Reopen this device.".into());}
            cx.notify();return;
        }
        if self.owner.bound_catalog_revision(&binding).is_none() {
            if let Some(current)=self.stream_device.as_deref().and_then(|device|self.owner.source_view_binding_for_device(device)) {
                binding=current;
            } else if self.stream_source.is_none() && let Some(device)=self.stream_device.clone() {
                self.source_selection=None;self.start_source_browser(device,cx);return;
            } else {
                if let Some(selection)=self.source_selection.as_mut() {selection.fail_remote_query("Reconnect this stream, then refresh sources.".into());}
                cx.notify();return;
            }
        }
        self.begin_remote_sources(Some(binding),replace,cx);
        if let Some(selection)=self.source_selection.as_mut() {selection.selected=selected;}
    }
    pub(super) fn confirm_source_selection(&mut self,cx:&mut Context<Self>) {
        let Some(mut selection)=self.source_selection.take() else {return;};
        let result=(|| -> Result<(),String> {
            if selection.loading || !selection.catalog_ready() || selection.scope!=current_share_scope() {return Err("The source selection changed; try again".into());}
            protocol::session::validate_delivery_sources(&selection.selected)?;
            let sources=selection.selected.iter().map(|id|selection.sources.iter().find(|source|source.source==*id)
                .cloned().ok_or("The source list changed".to_owned())).collect::<Result<Vec<_>,_>>()?;
            match &selection.target {
                Target::Send(device)=>{
                    for source in sources {
                        let request=self.send_screen(device.clone(),vec![source.source])?;
                        self.exit_group.send_labels.lock().unwrap().insert(request,capture_source_label(&source));
                    }
                    Ok(())
                },
                Target::Open(binding)=>{
                    let device=self.owner.validate_source_selection(binding,&sources)?;
                    if self.stream_device.as_ref().is_some_and(|expected|expected!=&device) {return Err("Device context changed; reopen sources".into());}
                    let mut sources=sources.into_iter();
                    let mut discard_browser=false;
                    if self.stream_source.is_none() && self.stream_device.as_deref()==Some(device.as_str()) {
                        let first=sources.next().ok_or("Choose a source")?;
                        if self.focus_stream_source(&device,first.source,cx) {discard_browser=true;}
                        else {self.start_stream_connection_capture(device.clone(),None,Some(first.source),cx);}
                    }
                    for source in sources {
                        if !self.open_stream_source(device.clone(),source.source,cx) {return Err("Could not open the viewing window".into());}
                    }
                    if discard_browser {
                        let handle=self.window_handle;
                        cx.defer(move|cx|{let _=handle.update(cx,|_,window,_|window.remove_window());});
                    }
                    Ok(())
                }
                Target::Replace(binding)=>{
                    if sources.len()!=1 {return Err("Choose one source to replace this stream".into());}
                    let device=self.owner.validate_source_selection(binding,&sources)?;
                    if self.stream_device.as_ref().is_some_and(|expected|expected!=&device) {return Err("Device context changed; reopen sources".into());}
                    let source=sources.into_iter().next().ok_or("Choose a source")?;
                    if self.start_capture_source_switch(Some(binding.clone()),source.clone(),source.title.clone(),cx) {Ok(())}
                    else {Err("The source list changed; choose again".into())}
                }
            }
        })();
        if let Err(error)=result {
            selection.loading=false;selection.error=Some(error.clone());self.source_selection=Some(selection);
            self.status=error;
        }
        cx.notify();
    }
    pub(super) fn cancel_source_selection(&mut self,cx:&mut Context<Self>) {
        self.source_selection=None;
        if self.purpose==WindowPurpose::Stream && self.stream_device.is_some() && self.stream_source.is_none() {
            let handle=self.window_handle;
            cx.defer(move|cx|{let _=handle.update(cx,|_,window,_|window.remove_window());});
        }
    }
}

pub(super) fn render(selection:&Selection,#[cfg(target_os="linux")] sharing_pending:bool,cx:&mut Context<RestoredDashboard>)->Div {
    let theme=cx.theme().clone();let view=cx.weak_entity();
    let visible=selection.visible_sources().collect::<Vec<_>>();
    let mut rows=div().id("source_selection_rows").min_h(px(64.)).max_h(px(280.)).flex_1().overflow_y_scroll().flex().flex_col().gap_1();
    for category in [SourceCategory::Screen,SourceCategory::Window,SourceCategory::Camera] {
      let windows=category==SourceCategory::Window;
      let mut heading=false;
      let mut application=None;
      for source in visible.iter().copied().filter(|source|source.source.category()==category) {
        if !heading {
            rows=rows.child(div().mt_2().text_size(px(10.)).text_color(theme.content.secondary)
                .child(crate::i18n::tr(match category {SourceCategory::Screen=>"Entire screens",SourceCategory::Window=>"App windows",SourceCategory::Camera=>"Cameras"})));heading=true;
        }
        if windows {
            let name=remote_core::source_picker::application_group(source);
            let key=name.to_lowercase();
            if application.as_ref()!=Some(&key) {
                rows=rows.child(div().pl_2().pt_2().text_size(px(11.)).font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.content.secondary).child(name.to_owned()));application=Some(key);
            }
        }
        let selected=selection.selected.contains(&source.source);let id=source.source;let view=view.clone();
        let full_title=if windows&&!source.application.trim().is_empty() {format!("{} · {}",source.application.trim(),source.title)}else{source.title.clone()};
        rows=rows.child(command_button(format!("source_selection_{id:?}"),if selected {ActionVariantKind::Primary}else{ActionVariantKind::Neutral},cx)
            .w_full().min_w(px(0.)).min_h(px(34.)).px_2().justify_start().stretch_content()
            .when(windows,|row|row.pl_4()).disabled(!selection.catalog_ready())
            .tooltip(tooltip(full_title).build())
            .on_click(move|_,_,cx| {let _=view.update(cx,|this,cx| {
                if let Some(selection)=this.source_selection.as_mut() {
                    selection.toggle(id);
                }cx.notify();
            });})
            .child(div().debug_selector(||"source-picker-row".into()).w_full().min_w(px(0.)).flex().items_center().gap_2()
                .child(div().w(px(14.)).flex_none().child(if selected {"✓"}else{"○"}))
                .child(capture_source_badge(source,&selection.artwork,&theme))
                .child(div().debug_selector(||"source-picker-title".into()).flex_1().min_w(px(0.)).truncate().text_size(px(11.)).child(source.title.clone()))));
      }
    }
    if visible.is_empty()&&!selection.query.trim().is_empty()&&!selection.loading {
        rows=rows.child(div().flex().items_center().justify_between().text_size(px(11.)).text_color(theme.content.secondary)
            .child(crate::i18n::tr("No matching sources"))
            .child(command_button("clear_source_filter",ActionVariantKind::Neutral,cx).child(crate::i18n::tr("Clear search"))
                .on_click(cx.listener(|this,_,_,cx|{if let Some(selection)=this.source_selection.as_mut(){selection.query.clear();}cx.notify();}))));
    }
    let view=cx.weak_entity();let confirm=cx.weak_entity();
    #[cfg(target_os="linux")]
    let share_choices={
        let mut choices=div().flex().items_center().gap_2();
        if matches!(selection.target,Target::Send(_)) {
            for (id,label,kind) in [
                ("delivery-window",crate::i18n::tr("App window"),crate::sharing_permissions::ShareKind::Window),
                ("delivery-screen",crate::i18n::tr("Screen"),crate::sharing_permissions::ShareKind::Display),
                ("delivery-control",crate::i18n::tr("Screen + control"),crate::sharing_permissions::ShareKind::ControlledDisplay),
            ] {
                choices=choices.child(command_button(id,ActionVariantKind::Neutral,cx)
                    .h(px(26.)).text_size(px(10.)).disabled(sharing_pending)
                    .on_click(cx.listener(move|this,_,_,cx|this.dispatch_local_share(crate::desktop::local_share::Action::Select(kind),cx)))
                    .child(label));
            }
        }
        choices
    };
    let quick_screen = (!selection.loading && selection.catalog_ready() && !selection.is_delivery())
        .then(|| selection.sources.iter().find(|source| source.source.category() == SourceCategory::Screen).cloned())
        .flatten();
    let panel=div().id("source_selection_panel").occlude().w(px(460.)).max_w(relative(0.95))
        .max_h(relative(0.94))
        .p_4().flex().flex_col().gap_3().bg(color_glass_card()).border_1().border_color(color_border_fine()).rounded_xl().shadow_2xl()
        .on_any_mouse_down(cx.listener(|_,_,_,cx|cx.stop_propagation()))
        .child(div().flex().items_center().justify_between().gap_2()
            .child(div().text_size(px(14.)).font_weight(FontWeight::BOLD).child(crate::i18n::message(&selection.title)))
            .when_some(quick_screen, |header, screen| {
                let quick_view = cx.weak_entity();
                let screen_id = screen.source;
                header.child(
                    command_button("source_selection_quick_screen", ActionVariantKind::Primary, cx)
                        .h(px(28.))
                        .px_2p5()
                        .text_size(px(11.))
                        .tooltip(tooltip(screen.title.clone()).build())
                        .on_click(move |_, _, cx| {
                            let _ = quick_view.update(cx, |this, cx| {
                                if let Some(selection) = this.source_selection.as_mut() {
                                    selection.selected = vec![screen_id];
                                }
                                this.confirm_source_selection(cx);
                            });
                        })
                        .child(icon(IconName::Display).size(px(13.)))
                        .child(crate::i18n::tr("Open main screen")),
                )
            }))
        .child(component::text_input("source-search").placeholder(crate::i18n::tr("Search apps and windows")).height(px(32.).into()).content(selection.query.clone())
            .on_change({let view=cx.weak_entity();move|text,_,cx|{let _=view.update(cx,|this,cx|{
                if let Some(selection)=this.source_selection.as_mut(){selection.query=text.to_string();}cx.notify();
            });}}))
        .children({#[cfg(target_os="linux")] {Some(share_choices)} #[cfg(not(target_os="linux"))] {None::<Div>}})
        .when(selection.loading,|panel|panel.child(div().text_size(px(11.)).child(crate::i18n::tr("Loading sources…"))))
        .when(!selection.loading && selection.catalog_ready() && (!selection.is_delivery() || !selection.sources.is_empty())
            && selection.sources.iter().all(|source|source.source.category()==SourceCategory::Camera),|panel|
            panel.child(div().text_size(px(11.)).text_color(theme.content.secondary)
                .child(crate::i18n::tr(if selection.is_delivery() {"Screen not shared. Open Sharing & permissions."}
                    else {"Screen not shared. Start sharing on the remote device."}))))
        .child(rows)
        .when_some(selection.error.as_ref(),|panel,error|panel.child(div().text_size(px(11.)).text_color(theme.status.error.bg).child(crate::i18n::message(error))))
        .child(div().flex().items_center().justify_end().gap_2()
            .child(div().flex_1().text_size(px(10.)).child(crate::i18n::tr_format!("{} selected",selection.selected.len())))
            .when(!selection.is_delivery(),|row|row.child(command_button("source_selection_refresh",ActionVariantKind::Neutral,cx)
                .disabled(selection.loading).child(crate::i18n::tr("Refresh sources"))
                .on_click(cx.listener(|this,_,_,cx|this.refresh_remote_source_selection(cx)))))
            .child(command_button("source_selection_cancel",ActionVariantKind::Neutral,cx).on_click(move|_,_,cx|{
                let _=view.update(cx,|this,cx|{this.cancel_source_selection(cx);cx.notify();});
            }).child(crate::i18n::tr("Cancel")))
            .child(command_button("source_selection_confirm",ActionVariantKind::Primary,cx)
                .disabled(selection.loading||!selection.catalog_ready()||selection.selected.is_empty())
                .on_click(move|_,_,cx|{let _=confirm.update(cx,|this,cx|this.confirm_source_selection(cx));})
                .child(crate::i18n::tr(if matches!(selection.target,Target::Send(_)) {"Send"}else if matches!(selection.target,Target::Replace(_)) {"Replace"}else{"Open"}))));
    div().occlude().absolute().inset_0().flex().items_center().justify_center().bg(rgba(0x00000088)).child(panel)
}

#[cfg(test)]
mod tests {
    use super::{Selection,Target,CaptureSource,CaptureSourceInfo};
    use std::sync::Arc;
    fn selection()->Selection {
        Selection {target:Target::Send("receiver".into()),scope:Some([3;32]),title:"Send".into(),
            sources:Vec::new(),query:String::new(),selected:Vec::new(),artwork:Arc::new(Default::default()),loading:true,error:None,catalog:None}
    }
    fn source(source:CaptureSource)->CaptureSourceInfo {
        CaptureSourceInfo {source,title:format!("{source:?}"),application:String::new(),process_id:None,
            width:1280,height:720,supports_input:false, camera: None, }
    }
    #[gpui::test]
    fn short_and_long_titles_share_a_left_edge_and_fit_compact_windows(cx:&mut gpui::TestAppContext) {
        use super::super::{component,open_restored_window,product_window_appearance,weavelo_themes,UnifiedRuntimeHandle};
        let executor=tokio::runtime::Runtime::new().unwrap();let _entered=executor.enter();
        let backend=executor.block_on(crate::UnifiedServiceOwner::start(crate::UnifiedServiceOwnerConfig::default())).unwrap();
        let runtime=Arc::new(UnifiedRuntimeHandle::from_workspace_owner(Arc::new(backend)));
        let window=cx.update(|cx|{
            component::init(cx).unwrap();
            crate::product_components::theme::install(product_window_appearance(),weavelo_themes(),cx);
            open_restored_window(runtime,None,Default::default(),None,None,Default::default(),cx).unwrap()
        });
        let view=window.root(cx).unwrap();
        let mut picker=selection();picker.scope=remote_core::shared_files::current_share_scope();
        let mut short=source(CaptureSource::Window(7));short.application="Fixture".into();short.title="Short".into();
        let mut long=source(CaptureSource::Window(8));long.application="Fixture".into();long.title=format!("Long {}","title and details ".repeat(24));
        picker.replace(vec![long,short],Arc::new(Default::default()));
        view.update(cx,|this,cx|{this.source_selection=Some(picker);cx.notify();});
        let mut visual=gpui::VisualTestContext::from_window(window.into(),cx);
        for width in [520.,580.] {
            visual.simulate_resize(gpui::size(gpui::px(width),gpui::px(440.)));
            let mut left=None;
            for query in ["Short","Long"] {
                visual.update(|_,cx|view.update(cx,|this,cx|{this.source_selection.as_mut().unwrap().query=query.into();cx.notify();}));
                visual.run_until_parked();
                let title=visual.debug_bounds("source-picker-title").expect("Visible source title");
                let row=visual.debug_bounds("source-picker-row").expect("Visible source row");
                assert_eq!(*left.get_or_insert(title.origin.x),title.origin.x,"Title length moved its column");
                assert!(title.right()<=row.right(),"Long title escaped its row: {title:?}, {row:?}");
                assert!(row.right()<=gpui::px(width),"The row exceeded the compact window: {row:?}");
            }
        }
        window.update(&mut visual,|_,window,_|window.remove_window()).unwrap();
    }
    #[test]
    fn concrete_display_replaces_its_alias_and_duplicate_catalog_entries() {
        let mut selection=selection();
        selection.replace(vec![source(CaptureSource::MainDisplay),source(CaptureSource::Display(1)),
            source(CaptureSource::Window(2)),source(CaptureSource::Window(2))],Arc::new(Default::default()));
        assert_eq!(selection.sources.len(),2);assert!(selection.selected.is_empty());
        assert!(!selection.loading);assert!(selection.error.is_none());
    }
    #[test]
    fn refreshing_shared_sources_retains_selected_windows_and_drops_ended_ones() {
        let mut selection=selection();
        let sources=|| (1..=3).map(|id|source(CaptureSource::Window(id))).collect();
        selection.replace(sources(),Arc::new(Default::default()));
        selection.toggle(CaptureSource::Window(1));
        selection.toggle(CaptureSource::Window(2));
        selection.replace(sources(),Arc::new(Default::default()));
        assert_eq!(selection.selected,vec![CaptureSource::Window(1),CaptureSource::Window(2)]);
        selection.replace(vec![source(CaptureSource::Window(2)),source(CaptureSource::Window(4))],Arc::new(Default::default()));
        assert_eq!(selection.selected,vec![CaptureSource::Window(2)]);
    }
    #[test]
    fn a_disappearing_window_never_silently_selects_the_desktop() {
        let mut selection=selection();
        selection.replace(vec![source(CaptureSource::Window(7))],Arc::new(Default::default()));
        selection.toggle(CaptureSource::Window(7));
        selection.replace(vec![source(CaptureSource::Display(1))],Arc::new(Default::default()));
        assert!(selection.selected.is_empty());
        assert!(selection.error.as_ref().unwrap().contains("ended"));
        selection.toggle(CaptureSource::Display(1));
        selection.toggle(CaptureSource::Display(1));
        selection.replace(vec![source(CaptureSource::Display(1))],Arc::new(Default::default()));
        assert!(selection.selected.is_empty(),"refresh preserves an explicit empty selection");
    }
    #[test]
    fn filtering_by_app_or_title_keeps_hidden_selections_and_refreshes_names() {
        let mut selection=selection();
        let mut editor=source(CaptureSource::Window(7));editor.application="Editor".into();editor.title="中文 Notes".into();
        let mut terminal=source(CaptureSource::Window(8));terminal.application="Terminal".into();terminal.title="Shell".into();
        selection.replace(vec![terminal.clone(),editor.clone()],Arc::new(Default::default()));
        selection.toggle(editor.source);selection.query="  SHELL ".into();
        assert_eq!(selection.visible_sources().map(|s|s.source).collect::<Vec<_>>(),vec![terminal.source]);
        assert_eq!(selection.selected,vec![editor.source]);
        selection.query="EDITOR".into();assert_eq!(selection.visible_sources().count(),1);
        editor.title="Updated 中文".into();selection.replace(vec![editor.clone()],Arc::new(Default::default()));
        selection.query="中文".into();assert_eq!(selection.visible_sources().next().unwrap().title,"Updated 中文");
        selection.query="missing".into();assert_eq!(selection.visible_sources().count(),0);
        assert_eq!(selection.selected,vec![editor.source]);
        selection.query.clear();assert_eq!(selection.visible_sources().count(),1);
    }
    #[test]
    fn stale_buttons_cannot_select_absent_sources_and_selection_stays_bounded() {
        let mut selection=selection();
        selection.replace((1..=9).map(|id|source(CaptureSource::Window(id))).collect(),Arc::new(Default::default()));
        selection.toggle(CaptureSource::Window(99));assert!(selection.selected.is_empty());
        selection.toggle(CaptureSource::Window(1));
        for id in 2..=9 {selection.toggle(CaptureSource::Window(id));}
        assert_eq!(selection.selected.len(),8);assert!(!selection.selected.contains(&CaptureSource::Window(9)));
        selection.toggle(CaptureSource::Window(1));selection.toggle(CaptureSource::Window(9));
        assert_eq!(selection.selected.len(),8);assert!(selection.error.is_none());
        selection.replace(vec![source(CaptureSource::Display(3))],Arc::new(Default::default()));
        selection.toggle(CaptureSource::Window(9));assert!(selection.selected.is_empty(),"stale window actions must not select a different desktop");
    }
}
