use super::*;
use protocol::session::{CaptureSource, CaptureSourceInfo, SessionCommand, SourceCategory, MAX_SUBSCRIPTIONS};
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
    pending_request: Option<String>,
    error: Option<String>,
}
impl Selection {
    pub(super) fn is_delivery(&self) -> bool { matches!(self.target, Target::Send(_)) }
    pub(super) fn local_delivery_is_current(&self) -> bool {self.is_delivery()&&self.scope==current_share_scope()}
    pub(super) fn pending_delivery(&self) -> bool { self.pending_request.is_some() }
    pub(super) fn observe_delivery(&mut self, progress:Option<(&str,&crate::desktop::directed_send::SendProgress)>) -> bool {
        let Some(pending)=self.pending_request.as_deref() else {return false;};
        let Some((_,progress))=progress.filter(|(request,_)|*request==pending) else {
            self.pending_request=None;self.error=Some("Sending cancelled".into());return false;
        };
        if progress.started {return true;}
        if let Some(result)=&progress.result {
            self.pending_request=None;
            self.error=Some(match result {
                Err(error)=>format!("Could not send: {error}"),
                Ok(())=>"Receiver ended delivery before video started".into(),
            });
        }
        false
    }
    fn toggle(&mut self,source:CaptureSource) {
        if self.pending_delivery() {return;}
        if !self.sources.iter().any(|item|item.source==source) {self.error=Some("The source list changed".into());return;}
        if self.selected.contains(&source) {self.selected.retain(|selected|*selected!=source);self.error=None;}
        else if self.selected.len()<MAX_SUBSCRIPTIONS {self.selected.push(source);self.error=None;}
        else {self.error=Some("Choose up to eight sources".into());}
    }
    fn replace(&mut self, mut sources: Vec<CaptureSourceInfo>, artwork: SourceArtwork) {
        sources.retain(|source|source.validate_camera().is_ok());
        // MainDisplay is an alias; don't offer it beside the concrete displays.
        if sources.iter().any(|source|matches!(source.source,CaptureSource::Display(_))) {
            sources.retain(|source|source.source!=CaptureSource::MainDisplay);
        }
        let mut seen=std::collections::HashSet::new();
        sources.retain(|source|seen.insert(source.source));
        self.selected.retain(|id| sources.iter().any(|source| source.source == *id));
        if self.selected.is_empty() {self.selected=sources.first().map(|source|vec![source.source]).unwrap_or_default();}
        self.sources=sources;self.artwork=artwork;self.loading=false;
        self.error=self.sources.is_empty().then(||"Choose a screen in Sharing & permissions first".into());
    }
}

impl RestoredDashboard {
    pub(super) fn begin_delivery_selection(&mut self, device:String,cx:&mut Context<Self>) {
        if self.background.is_none() {self.status="Sharing service is unavailable".into();cx.notify();return;}
        if self.directed_send.lock().unwrap().as_ref().is_some_and(|sender|!sender.is_finished()) {
            self.status="Stop the current delivery before choosing another target".into();cx.notify();return;
        }
        let name=self.owner.snapshot().devices.iter().find(|item|item.device_id==device)
            .map(|item|item.display_name.clone()).unwrap_or_else(||device.clone());
        self.owner.release_input();self.pointer_input=PointerInputTracker::default();self.show_apps_menu=false;
        let scope=current_share_scope();
        self.source_selection=Some(Selection {target:Target::Send(device),scope,title:format!("Send to {name}"),
            sources:Vec::new(),selected:Vec::new(),artwork:Arc::new(Default::default()),loading:true,pending_request:None,error:None});
        self.refresh_delivery_sources(cx);
        cx.notify();
    }
    pub(super) fn refresh_delivery_sources(&mut self, cx:&mut Context<Self>) {
        let Some(selection)=self.source_selection.as_mut() else {return;};
        if selection.pending_delivery() {return;}
        let Target::Send(device)=&selection.target else {return;};
        let device=device.clone();let scope=selection.scope;
        let Some(client)=self.background.clone() else {
            selection.loading=false;selection.error=Some("Sharing service is unavailable".into());cx.notify();return;
        };
        selection.loading=true;
        let work=tokio::task::spawn_blocking(move || {
            let sources=client.capture_sources().map_err(|error|error.to_string())?;
            #[cfg(target_os="linux")]
            let message=if sources.is_empty() {
                client.sharing_call(crate::sharing_permissions::SharingCall::Status)
                    .map_err(|error|error.to_string())?.message
            } else {None};
            #[cfg(not(target_os="linux"))]
            let message:Option<String>=None;
            let command=host::capture_sources::with_artwork(sources);
            let SessionCommand::Sources {sources,source_icons,icons,..}=command else {return Err("Invalid local source directory".into());};
            let mut cache=remote_core::application_icons::SourceIconCache::default();
            cache.update(sources.iter().map(|source|source.source),&source_icons,&icons)?;
            Ok::<_,String>((sources,cache.assets(),message))
        });
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
        let Some(binding)=binding.filter(|binding|self.owner.source_binding_is_current(binding)) else {return;};
        let snapshot=self.owner.snapshot();
        self.owner.release_input();self.pointer_input=PointerInputTracker::default();self.show_apps_menu=false;
        let mut selection=Selection {target:Target::Open(binding),scope:current_share_scope(),title:"Choose screens & app windows".into(),
            sources:Vec::new(),selected:Vec::new(),artwork:Arc::new(Default::default()),loading:false,pending_request:None,error:None};
        let artwork=self.artwork_cache.borrow_mut().update(snapshot.source_icons);
        selection.replace((*snapshot.sources).clone(),artwork);
        if selection.sources.iter().any(|source|source.source==snapshot.active_source) {selection.selected=vec![snapshot.active_source];}
        self.source_selection=Some(selection);cx.notify();
    }
    pub(super) fn confirm_source_selection(&mut self,cx:&mut Context<Self>) {
        let Some(mut selection)=self.source_selection.take() else {return;};
        if selection.pending_delivery() {self.source_selection=Some(selection);return;}
        let delivery=selection.is_delivery();
        let result=(|| -> Result<(),String> {
            if selection.loading || selection.scope!=current_share_scope() {return Err("The source selection changed; try again".into());}
            protocol::session::validate_delivery_sources(&selection.selected)?;
            let sources=selection.selected.iter().map(|id|selection.sources.iter().find(|source|source.source==*id)
                .cloned().ok_or("The source list changed".to_owned())).collect::<Result<Vec<_>,_>>()?;
            match &selection.target {
                Target::Send(device)=>self.send_screen(device.clone(),selection.selected.clone()),
                Target::Open(binding)=>{
                    let device=self.owner.validate_source_selection(binding,&sources)?;
                    if self.exit_group.owners().len().saturating_sub(1)+sources.len()>MAX_SUBSCRIPTIONS {
                        return Err("Close a viewing window to open these sources".into());
                    }
                    let mut sources=sources.into_iter();
                    let first=sources.next().ok_or("Choose a source")?;
                    if !self.start_capture_source_switch(Some(binding.clone()),first.clone(),first.title.clone(),cx) {
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
        if let Err(error)=result {
            if delivery {selection.loading=false;selection.error=Some(error.clone());self.source_selection=Some(selection);}
            self.status=error;
        } else if delivery {
            selection.error=None;
            selection.pending_request=self.directed_send.lock().unwrap().as_ref().map(|sender|sender.request_id.clone());
            self.source_selection=Some(selection);
        }
        cx.notify();
    }
    pub(super) fn cancel_source_selection(&mut self) {
        let pending=self.source_selection.take().and_then(|selection|selection.pending_request);
        if let Some(pending)=pending {
            let mut sender=self.directed_send.lock().unwrap();
            if sender.as_ref().is_some_and(|sender|sender.request_id==pending) {sender.take();}
        }
    }
}

pub(super) fn render(selection:&Selection,#[cfg(target_os="linux")] sharing_pending:bool,cx:&mut Context<RestoredDashboard>)->Div {
    let theme=cx.theme().clone();let view=cx.weak_entity();
    let mut rows=div().id("source_selection_rows").max_h(px(340.)).overflow_y_scroll().flex().flex_col().gap_1();
    for category in [SourceCategory::Screen,SourceCategory::Window,SourceCategory::Camera] {
      let windows=category==SourceCategory::Window;
      let mut heading=false;
      for source in selection.sources.iter().filter(|source|source.source.category()==category) {
        if !heading {
            rows=rows.child(div().mt_2().text_size(px(10.)).text_color(theme.content.secondary)
                .child(match category {SourceCategory::Screen=>"Entire screens",SourceCategory::Window=>"App windows",SourceCategory::Camera=>"Cameras"}));heading=true;
        }
        let selected=selection.selected.contains(&source.source);let id=source.source;let view=view.clone();
        rows=rows.child(command_button(format!("source_selection_{id:?}"),if selected {ActionVariantKind::Primary}else{ActionVariantKind::Neutral},cx)
            .w_full().min_h(px(34.)).px_2().gap_2().disabled(selection.pending_delivery())
            .on_click(move|_,_,cx| {let _=view.update(cx,|this,cx| {
                if let Some(selection)=this.source_selection.as_mut() {
                    selection.toggle(id);
                }cx.notify();
            });})
            .child(if selected {"✓"}else{"○"})
            .child(capture_source_badge(source,&selection.artwork,&theme))
            .child(div().flex_1().flex().flex_col().text_size(px(11.)).child(source.title.clone())
                .when(windows&&!source.application.is_empty(),|label|label.child(div().text_size(px(10.))
                    .text_color(theme.content.secondary).child(source.application.clone())))));
      }
    }
    let view=cx.weak_entity();let confirm=cx.weak_entity();
    #[cfg(target_os="linux")]
    let share_choices={
        let mut choices=div().flex().items_center().gap_2();
        if matches!(selection.target,Target::Send(_)) {
            for (id,label,kind) in [
                ("delivery-window","App window",crate::sharing_permissions::ShareKind::Window),
                ("delivery-screen","Screen",crate::sharing_permissions::ShareKind::Display),
                ("delivery-control","Screen + control",crate::sharing_permissions::ShareKind::ControlledDisplay),
            ] {
                choices=choices.child(command_button(id,ActionVariantKind::Neutral,cx)
                    .h(px(26.)).text_size(px(10.)).disabled(sharing_pending||selection.pending_delivery())
                    .on_click(cx.listener(move|this,_,_,cx|this.dispatch_local_share(crate::desktop::local_share::Action::Select(kind),cx)))
                    .child(label));
            }
        }
        choices
    };
    let panel=div().id("source_selection_panel").occlude().w(px(460.)).max_w(relative(0.95))
        .p_4().flex().flex_col().gap_3().bg(color_glass_card()).border_1().border_color(color_border_fine()).rounded_xl().shadow_2xl()
        .on_any_mouse_down(cx.listener(|_,_,_,cx|cx.stop_propagation()))
        .child(div().text_size(px(14.)).font_weight(FontWeight::BOLD).child(selection.title.clone()))
        .child(div().text_size(px(11.)).text_color(theme.content.secondary).child("Select one or more screens or app windows."))
        .children({#[cfg(target_os="linux")] {Some(share_choices)} #[cfg(not(target_os="linux"))] {None::<Div>}})
        .when(selection.loading,|panel|panel.child(div().text_size(px(11.)).child("Loading sources…")))
        .when(selection.pending_delivery(),|panel|panel.child(div().text_size(px(11.)).child("Connecting to receiver…")))
        .child(rows)
        .when_some(selection.error.as_ref(),|panel,error|panel.child(div().text_size(px(11.)).text_color(theme.status.error.bg).child(error.clone())))
        .child(div().flex().items_center().justify_end().gap_2()
            .child(div().flex_1().text_size(px(10.)).child(format!("{} selected",selection.selected.len())))
            .child(command_button("source_selection_cancel",ActionVariantKind::Neutral,cx).on_click(move|_,_,cx|{
                let _=view.update(cx,|this,cx|{this.cancel_source_selection();cx.notify();});
            }).child("Cancel"))
            .child(command_button("source_selection_confirm",ActionVariantKind::Primary,cx)
                .disabled(selection.loading||selection.pending_delivery()||selection.selected.is_empty())
                .on_click(move|_,_,cx|{let _=confirm.update(cx,|this,cx|this.confirm_source_selection(cx));})
                .child(if selection.pending_delivery() {"Connecting…"}else if matches!(selection.target,Target::Send(_)) {"Send"}else{"Open"})));
    div().occlude().absolute().inset_0().flex().items_center().justify_center().bg(rgba(0x00000088)).child(panel)
}

#[cfg(test)]
mod tests {
    use super::{Selection,Target,CaptureSource,CaptureSourceInfo};
    use std::sync::Arc;
    fn selection()->Selection {
        Selection {target:Target::Send("receiver".into()),scope:Some([3;32]),title:"Send".into(),
            sources:Vec::new(),selected:Vec::new(),artwork:Arc::new(Default::default()),loading:true,pending_request:None,error:None}
    }
    fn source(source:CaptureSource)->CaptureSourceInfo {
        CaptureSourceInfo {source,title:format!("{source:?}"),application:String::new(),process_id:None,
            width:1280,height:720,supports_input:false, camera: None, }
    }
    fn pending_selection()->Selection {
        let mut selection=selection();
        selection.replace(vec![source(CaptureSource::Display(1))],Arc::new(Default::default()));
        selection.pending_request=Some("attempt-1".into());selection
    }
    fn progress(started:bool,result:Option<Result<(),String>>)->crate::desktop::directed_send::SendProgress {
        crate::desktop::directed_send::SendProgress {status:"Connecting…".into(),started,result,confirmation_timeout:None}
    }
    #[test]
    fn asynchronous_failure_keeps_the_selected_source_and_can_be_retried() {
        let mut selection=pending_selection();
        assert!(!selection.observe_delivery(Some(("attempt-1",&progress(false,None)))));
        selection.toggle(CaptureSource::Display(1));
        assert_eq!(selection.selected,vec![CaptureSource::Display(1)]);
        assert!(!selection.observe_delivery(Some(("attempt-1",&progress(false,Some(Err("Receiver is busy".into())))))));
        assert!(!selection.pending_delivery());
        assert_eq!(selection.selected,vec![CaptureSource::Display(1)]);
        assert!(selection.error.as_ref().unwrap().contains("Receiver is busy"));
        selection.pending_request=Some("attempt-2".into());selection.error=None;
        assert!(!selection.observe_delivery(Some(("attempt-2",&progress(false,None)))));
        assert!(selection.observe_delivery(Some(("attempt-2",&progress(true,None)))));
    }
    #[test]
    fn stale_attempt_cannot_dismiss_a_new_source_selection() {
        let mut selection=pending_selection();
        assert!(!selection.observe_delivery(Some(("older-attempt",&progress(true,Some(Ok(())))))));
        assert!(!selection.pending_delivery());
        assert_eq!(selection.error.as_deref(),Some("Sending cancelled"));
        assert_eq!(selection.selected,vec![CaptureSource::Display(1)]);
    }
    #[test]
    fn receiver_ending_before_first_frame_keeps_a_visible_failure() {
        let mut selection=pending_selection();
        assert!(!selection.observe_delivery(Some(("attempt-1",&progress(false,Some(Ok(())))))));
        assert!(selection.error.as_ref().unwrap().contains("before video started"));
        assert!(!selection.pending_delivery());
    }
    #[test]
    fn successful_first_frame_dismisses_the_waiting_dialog_only_for_its_attempt() {
        let mut selection=pending_selection();
        assert!(selection.observe_delivery(Some(("attempt-1",&progress(true,Some(Ok(())))))));
        let mut cancelled=pending_selection();
        assert!(!cancelled.observe_delivery(None));
        assert!(!cancelled.pending_delivery());
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
    fn refreshing_shared_sources_retains_selected_windows_and_drops_ended_ones() {
        let mut selection=selection();
        let sources=|| (1..=3).map(|id|source(CaptureSource::Window(id))).collect();
        selection.replace(sources(),Arc::new(Default::default()));
        selection.toggle(CaptureSource::Window(2));
        selection.replace(sources(),Arc::new(Default::default()));
        assert_eq!(selection.selected,vec![CaptureSource::Window(1),CaptureSource::Window(2)]);
        selection.replace(vec![source(CaptureSource::Window(2)),source(CaptureSource::Window(4))],Arc::new(Default::default()));
        assert_eq!(selection.selected,vec![CaptureSource::Window(2)]);
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
