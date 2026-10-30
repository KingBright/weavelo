use super::*;
use protocol::session::{CaptureSource, CaptureSourceInfo, SessionCommand, MAX_SUBSCRIPTIONS};
use remote_core::shared_files::{ShareScope, current_share_scope};

pub(super) enum Target {
    Send(String),
    Open(SourceViewBinding),
}
pub(super) struct Selection {
    target: Target,
    scope: ShareScope,
    title: String,
    sources: Vec<CaptureSourceInfo>,
    selected: Vec<CaptureSource>,
    artwork: SourceArtwork,
    loading: bool,
    error: Option<String>,
}
impl Selection {
    fn toggle(&mut self,source:CaptureSource) {
        if !self.sources.iter().any(|item|item.source==source) {self.error=Some("The source list changed".into());return;}
        if self.selected.contains(&source) {self.selected.retain(|selected|*selected!=source);self.error=None;}
        else if self.selected.len()<MAX_SUBSCRIPTIONS {self.selected.push(source);self.error=None;}
        else {self.error=Some("Choose up to eight sources".into());}
    }
    fn replace(&mut self, mut sources: Vec<CaptureSourceInfo>, artwork: SourceArtwork) {
        // MainDisplay is an alias; don't offer it beside the concrete displays.
        if sources.iter().any(|source|matches!(source.source,CaptureSource::Display(_))) {
            sources.retain(|source|source.source!=CaptureSource::MainDisplay);
        }
        let mut seen=std::collections::HashSet::new();
        sources.retain(|source|seen.insert(source.source));
        self.selected=sources.first().map(|source|vec![source.source]).unwrap_or_default();
        self.sources=sources;self.artwork=artwork;self.loading=false;
        self.error=self.sources.is_empty().then(||"Choose a screen in Sharing & permissions first".into());
    }
}

impl RestoredDashboard {
    pub(super) fn begin_delivery_selection(&mut self, device:String,cx:&mut Context<Self>) {
        if self._instance.is_none() {self.status="Send from the main device window".into();cx.notify();return;}
        if self.directed_send.as_ref().is_some_and(|sender|!sender.is_finished()) {
            self.status="Stop the current delivery before choosing another target".into();cx.notify();return;
        }
        let Some(client)=self.background.clone() else {
            self.status="Sharing service is unavailable".into();cx.notify();return;
        };
        let name=self.owner.snapshot().devices.iter().find(|item|item.device_id==device)
            .map(|item|item.display_name.clone()).unwrap_or_else(||device.clone());
        self.owner.release_input();self.pointer_input=PointerInputTracker::default();self.show_apps_menu=false;
        let scope=current_share_scope();
        self.source_selection=Some(Selection {target:Target::Send(device),scope,title:format!("Send to {name}"),
            sources:Vec::new(),selected:Vec::new(),artwork:Arc::new(Default::default()),loading:true,error:None});
        if self.source_query_inflight {cx.notify();return;}
        self.source_query_inflight=true;
        let work=tokio::task::spawn_blocking(move || {
            let command=host::capture_sources::with_artwork(client.capture_sources().map_err(|error|error.to_string())?);
            let SessionCommand::Sources {sources,source_icons,icons,..}=command else {return Err("Invalid local source directory".into());};
            let mut cache=remote_core::application_icons::SourceIconCache::default();
            cache.update(sources.iter().map(|source|source.source),&source_icons,&icons)?;
            Ok::<_,String>((sources,cache.assets()))
        });
        cx.spawn(async move |view,cx| {
            let result=work.await.unwrap_or_else(|_|Err("Source lookup stopped; try again".into()));
            let _=view.update(cx,|this,cx| {
                this.source_query_inflight=false;
                if let Some(selection)=this.source_selection.as_mut()
                    && matches!(selection.target,Target::Send(_)) && selection.scope==scope
                {
                    selection.loading=false;
                    if scope!=current_share_scope() {selection.error=Some("The device network changed; choose the sources again".into());}
                    else {match result {
                        Ok((sources,assets))=>selection.replace(sources,ApplicationArtwork::default().update(assets)),
                        Err(error)=>selection.error=Some(error),
                    }}
                }
                cx.notify();
            });
        }).detach();
        cx.notify();
    }
    pub(super) fn begin_remote_source_selection(&mut self,binding:Option<SourceViewBinding>,cx:&mut Context<Self>) {
        let Some(binding)=binding.filter(|binding|self.owner.source_binding_is_current(binding)) else {return;};
        let snapshot=self.owner.snapshot();
        self.owner.release_input();self.pointer_input=PointerInputTracker::default();self.show_apps_menu=false;
        let mut selection=Selection {target:Target::Open(binding),scope:current_share_scope(),title:"Open screens & apps".into(),
            sources:Vec::new(),selected:Vec::new(),artwork:Arc::new(Default::default()),loading:false,error:None};
        let artwork=self.artwork_cache.borrow_mut().update(snapshot.source_icons);
        selection.replace((*snapshot.sources).clone(),artwork);
        if selection.sources.iter().any(|source|source.source==snapshot.active_source) {selection.selected=vec![snapshot.active_source];}
        self.source_selection=Some(selection);cx.notify();
    }
    pub(super) fn confirm_source_selection(&mut self,cx:&mut Context<Self>) {
        let Some(selection)=self.source_selection.take() else {return;};
        let result=(|| -> Result<(),String> {
            if selection.loading || selection.scope!=current_share_scope() {return Err("The source selection changed; try again".into());}
            protocol::session::validate_delivery_sources(&selection.selected)?;
            let sources=selection.selected.iter().map(|id|selection.sources.iter().find(|source|source.source==*id)
                .cloned().ok_or("The source list changed".to_owned())).collect::<Result<Vec<_>,_>>()?;
            match selection.target {
                Target::Send(device)=>self.send_screen(device,selection.selected),
                Target::Open(binding)=>{
                    let device=self.owner.validate_source_selection(&binding,&sources)?;
                    if self.exit_group.owners().len().saturating_sub(1)+sources.len()>MAX_SUBSCRIPTIONS {
                        return Err("Close a viewing window to open these sources".into());
                    }
                    let mut sources=sources.into_iter();
                    let first=sources.next().ok_or("Choose a source")?;
                    if !self.start_capture_source_switch(Some(binding),first.clone(),first.title.clone(),cx) {
                        return Err("The source list changed; choose the sources again".into());
                    }
                    for source in sources {
                        let handle=open_restored_window(self.runtime.clone(),None,Arc::new(AtomicBool::new(false)),None,
                            self.background.clone(),self.exit_group.clone(),cx).ok_or("Could not open the viewing window")?;
                        handle.update(cx,|this,_,cx|this.start_stream_connection_capture(device.clone(),None,Some(source.source),cx))
                            .map_err(|error|error.to_string())?;
                    }
                    Ok(())
                }
            }
        })();
        if let Err(error)=result {self.status=error;}
        cx.notify();
    }
}

pub(super) fn render(selection:&Selection,cx:&mut Context<RestoredDashboard>)->Div {
    let theme=cx.theme().clone();let view=cx.weak_entity();
    let mut rows=div().id("source_selection_rows").max_h(px(340.)).overflow_y_scroll().flex().flex_col().gap_1();
    for source in &selection.sources {
        let selected=selection.selected.contains(&source.source);let id=source.source;let view=view.clone();
        rows=rows.child(command_button(format!("source_selection_{id:?}"),if selected {ActionVariantKind::Primary}else{ActionVariantKind::Neutral},cx)
            .w_full().min_h(px(34.)).px_2().gap_2()
            .on_click(move|_,_,cx| {let _=view.update(cx,|this,cx| {
                if let Some(selection)=this.source_selection.as_mut() {
                    selection.toggle(id);
                }cx.notify();
            });})
            .child(if selected {"✓"}else{"○"})
            .child(capture_source_badge(source,&selection.artwork,&theme))
            .child(div().flex_1().text_size(px(11.)).child(source.title.clone())));
    }
    let view=cx.weak_entity();let confirm=cx.weak_entity();
    let panel=div().id("source_selection_panel").occlude().w(px(460.)).max_w(relative(0.95))
        .p_4().flex().flex_col().gap_3().bg(color_glass_card()).border_1().border_color(color_border_fine()).rounded_xl().shadow_2xl()
        .on_any_mouse_down(cx.listener(|_,_,_,cx|cx.stop_propagation()))
        .child(div().text_size(px(14.)).font_weight(FontWeight::BOLD).child(selection.title.clone()))
        .child(div().text_size(px(11.)).text_color(theme.content.secondary).child("Choose screens or app windows."))
        .when(selection.loading,|panel|panel.child(div().text_size(px(11.)).child("Loading sources…")))
        .child(rows)
        .when_some(selection.error.as_ref(),|panel,error|panel.child(div().text_size(px(11.)).text_color(theme.status.error.bg).child(error.clone())))
        .child(div().flex().items_center().justify_end().gap_2()
            .child(div().flex_1().text_size(px(10.)).child(format!("{} selected",selection.selected.len())))
            .child(command_button("source_selection_cancel",ActionVariantKind::Neutral,cx).on_click(move|_,_,cx|{
                let _=view.update(cx,|this,cx|{this.source_selection=None;cx.notify();});
            }).child("Cancel"))
            .child(command_button("source_selection_confirm",ActionVariantKind::Primary,cx)
                .disabled(selection.loading||selection.selected.is_empty())
                .on_click(move|_,_,cx|{let _=confirm.update(cx,|this,cx|this.confirm_source_selection(cx));})
                .child(if matches!(selection.target,Target::Send(_)) {"Send"}else{"Open"})));
    div().occlude().absolute().inset_0().flex().items_center().justify_center().bg(rgba(0x00000088)).child(panel)
}

#[cfg(test)]
mod tests {
    use super::{Selection,Target,CaptureSource,CaptureSourceInfo};
    use std::sync::Arc;
    fn selection()->Selection {
        Selection {target:Target::Send("receiver".into()),scope:Some([3;32]),title:"Send".into(),
            sources:Vec::new(),selected:Vec::new(),artwork:Arc::new(Default::default()),loading:true,error:None}
    }
    fn source(source:CaptureSource)->CaptureSourceInfo {
        CaptureSourceInfo {source,title:format!("{source:?}"),application:String::new(),process_id:None,
            width:1280,height:720,supports_input:false}
    }
    #[test]
    fn concrete_display_replaces_its_alias_and_duplicate_catalog_entries() {
        let mut selection=selection();
        selection.replace(vec![source(CaptureSource::MainDisplay),source(CaptureSource::Display(1)),
            source(CaptureSource::Window(2)),source(CaptureSource::Window(2))],Arc::new(Default::default()));
        assert_eq!(selection.sources.len(),2);assert_eq!(selection.selected,vec![CaptureSource::Display(1)]);
        assert!(!selection.loading);assert!(selection.error.is_none());
    }
    #[test]
    fn stale_buttons_cannot_select_absent_sources_and_selection_stays_bounded() {
        let mut selection=selection();
        selection.replace((1..=9).map(|id|source(CaptureSource::Window(id))).collect(),Arc::new(Default::default()));
        selection.toggle(CaptureSource::Window(99));assert_eq!(selection.selected.len(),1);
        for id in 2..=9 {selection.toggle(CaptureSource::Window(id));}
        assert_eq!(selection.selected.len(),8);assert!(!selection.selected.contains(&CaptureSource::Window(9)));
        selection.toggle(CaptureSource::Window(1));selection.toggle(CaptureSource::Window(9));
        assert_eq!(selection.selected.len(),8);assert!(selection.error.is_none());
        selection.replace(vec![source(CaptureSource::Display(3))],Arc::new(Default::default()));
        selection.toggle(CaptureSource::Window(9));assert_eq!(selection.selected,vec![CaptureSource::Display(3)]);
    }
}
