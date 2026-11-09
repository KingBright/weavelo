//! Independent source authorization, encoder demand and stale-frame rejection.
use protocol::session::{CaptureSource, CaptureSourceInfo, SubscriptionRequest, SessionCommand};
use remote_core::cameras::{CameraRegistry, NativeCamera};
use std::collections::HashMap;
use crate::publisher_viewport::{ScreenView,ScreenViewDemand};
use crate::outgoing_quality::DeliveryQuality;

pub(crate) fn screen_source(source: CaptureSource) -> bool {
    matches!(source, CaptureSource::MainDisplay | CaptureSource::Window(_))
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct CaptureDemand {
    pub subscriptions: usize,
    pub active: bool,
    pub audio: bool,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub bitrate_kbps: u32,
    pub keyframe: u64,
    pub epoch: u64,
    pub grant: u64,
    pub views: Vec<ScreenViewDemand>,
}
impl Default for CaptureDemand {
    fn default()->Self {Self{subscriptions:0,active:false,audio:false,width:1280,height:720,fps:30,bitrate_kbps:5000,keyframe:0,epoch:0,grant:0,views:Vec::new()}}
}
#[derive(Clone)]
pub(crate) struct Source {
    pub info: CaptureSourceInfo,
    pub enabled: bool,
    pub audio: bool,
    pub grant: u64,
    pub demand: CaptureDemand,
    pub capture_rotation: u16,
    // Outer None is pending; Some(None) is a completed native read without an icon.
    pub artwork: Option<Option<protocol::interaction::ApplicationIcon>>,
    pub artwork_epoch: u64,
    pub catalog_icon: Option<protocol::interaction::ApplicationIcon>,
}
pub(crate) struct ActiveRequest<'a> {
    pub stream: u64,
    pub grant: u64,
    pub request:&'a SubscriptionRequest,
    pub video:bool,
    pub audio:bool,
}
pub(crate) struct MediaSources {
    pub peer_streams:Vec<(Option<String>,usize)>,
    pub sources:HashMap<CaptureSource,Source>,
    views:HashMap<u64,ScreenView>,
    view_tickets:HashMap<u64,[u8;32]>,
    quality:Option<DeliveryQuality>,
    quality_sequence:u64,
    quality_pending:Option<PendingQuality>,
    cameras:CameraRegistry,
    sequence:u64,
}
struct PendingQuality {
    revision:u64,
    previous:DeliveryQuality,
    result:std::sync::mpsc::Sender<Result<(),String>>,
    waiting:std::collections::HashSet<u64>,
}
impl MediaSources {
    pub fn new(screen:bool,audio:bool)->Self {
        let mut this=Self{peer_streams:Vec::new(),sources:HashMap::new(),views:HashMap::new(),view_tickets:HashMap::new(),quality:None,quality_sequence:0,quality_pending:None,cameras:CameraRegistry::default(),sequence:1};
        this.sources.insert(CaptureSource::MainDisplay,Source {
            info:CaptureSourceInfo {source:CaptureSource::MainDisplay,title:"Android selected screen or app".into(),
                application:"MediaProjection".into(),process_id:None,width:1080,height:1920,supports_input:false,camera:None},
            enabled:screen,audio:screen&&audio,grant:1,demand:CaptureDemand{epoch:1,grant:1,..Default::default()},capture_rotation:0,artwork:None,artwork_epoch:1,catalog_icon:None,
        });this
    }
    fn next(&mut self)->u64 {
        self.sequence=self.sequence.checked_add(1).expect("capture generation exhausted");self.sequence
    }
    pub fn set_screen(&mut self,enabled:bool,audio:bool)->bool {
        let audio=enabled&&audio;
        let old=&self.sources[&CaptureSource::MainDisplay];
        if (old.enabled,old.audio)==(enabled,audio){return false;}
        self.views.retain(|_,view|view.request.source!=CaptureSource::MainDisplay);
        let epoch=self.next();let screen=self.sources.get_mut(&CaptureSource::MainDisplay).unwrap();
        screen.enabled=enabled;screen.audio=audio;screen.grant=epoch;
        screen.demand=CaptureDemand{epoch,grant:epoch,..Default::default()};true
    }
    pub fn set_cameras(&mut self,cameras:Vec<NativeCamera>)->Result<Vec<(CaptureSource,String)>,String> {
        let infos=self.cameras.reconcile(cameras).map_err(str::to_owned)?;
        self.sources.retain(|id,_|!matches!(id,CaptureSource::Camera(_))||infos.iter().any(|info|info.source==*id));
        let mut bindings=Vec::new();
        for info in infos {
            let key=self.cameras.resolve(info.source).unwrap().key.clone();
            bindings.push((info.source,key));
            if let Some(existing)=self.sources.get_mut(&info.source) && existing.enabled {
                existing.info=info;continue;
            }
            let epoch=self.next();
            self.sources.insert(info.source,Source{info,enabled:true,audio:false,grant:epoch,capture_rotation:0,artwork:None,artwork_epoch:1,catalog_icon:None,
                demand:CaptureDemand{epoch,grant:epoch,..Default::default()}});
        }Ok(bindings)
    }
    pub fn camera_key(&self,source:CaptureSource)->Option<&str> {self.cameras.resolve(source).map(|camera|camera.key.as_str())}
    pub fn set_applications(&mut self, infos: Vec<CaptureSourceInfo>) -> Result<(),String> {
        let mut ids=std::collections::HashSet::new();
        if infos.len()>256 || infos.iter().any(|info| !matches!(info.source,CaptureSource::Window(id) if id>0)
            || !ids.insert(info.source) || info.application.is_empty() || info.application.len()>1024
            || info.title.is_empty() || info.title.len()>1024 || info.camera.is_some()
            || !(2..=8192).contains(&info.width) || !(2..=8192).contains(&info.height)
            || u64::from(info.width)*u64::from(info.height)>33_554_432) {
            return Err("Invalid independent app catalog".into());
        }
        self.sources.retain(|id,_|!matches!(id,CaptureSource::Window(_)) || ids.contains(id));
        self.views.retain(|_,view|!matches!(view.request.source,CaptureSource::Window(_)) || ids.contains(&view.request.source));
        for mut info in infos {
            info.supports_input=false;
            if let Some(old)=self.sources.get(&info.source)
                && old.enabled && old.info.application==info.application {
                // A package catalog refresh is not a new capture geometry.
                info.width=old.info.width;info.height=old.info.height;
                let id=info.source;self.sources.get_mut(&id).unwrap().info=info;continue;
            }
            let epoch=self.next();
            self.sources.insert(info.source,Source{info,enabled:true,audio:false,grant:epoch,capture_rotation:0,artwork:None,artwork_epoch:1,catalog_icon:None,
                demand:CaptureDemand{epoch,grant:epoch,..Default::default()}});
        }
        Ok(())
    }
    pub fn catalog(&self,input_ready:bool)->Vec<CaptureSourceInfo> {
        let mut sources=self.sources.values().filter(|source|source.enabled).map(|source| {
            let mut info=source.info.clone();info.supports_input=screen_source(info.source)&&input_ready;info
        }).collect::<Vec<_>>();
        remote_core::source_picker::sort(&mut sources);sources
    }
    pub fn catalog_reply(&self, request_id:u32, input_ready:bool, known:&[[u8;32]]) -> SessionCommand {
        let sources=self.catalog(input_ready);
        let known=known.iter().copied().collect::<std::collections::HashSet<_>>();
        let mut source_icons=Vec::new();let mut icons=Vec::new();let mut included=std::collections::HashSet::new();
        for source in &sources {
            if let Some(icon)=self.sources[&source.source].catalog_icon.as_ref() {
                source_icons.push(protocol::interaction::SourceIconBinding{source:source.source,icon_id:icon.id});
                if !known.contains(&icon.id)&&included.insert(icon.id){icons.push(icon.clone());}
            }
        }
        SessionCommand::Sources{request_id,sources,source_icons,icons}
    }
    pub fn set_catalog_icons(&mut self, entries:Vec<(CaptureSource,protocol::interaction::ApplicationIcon)>)->Result<(),String> {
        if entries.len()>protocol::interaction::MAX_APPLICATION_ICONS{return Err("Too many application icons".into());}
        let mut ids=std::collections::HashSet::new();
        for (source,icon) in &entries {
            if !matches!(source,CaptureSource::Window(_)) || !ids.insert(*source) || !self.enabled(*source) {
                return Err("Application icon source is no longer available".into());
            }
            remote_core::application_icons::decode_artwork(icon,false)?;
        }
        // Validate the complete batch before replacing any live directory asset.
        for (source,icon) in entries {self.sources.get_mut(&source).unwrap().catalog_icon=Some(icon);}
        Ok(())
    }
    pub fn enabled(&self,source:CaptureSource)->bool {self.sources.get(&source).is_some_and(|source|source.enabled)}
    pub fn demand(&self,source:CaptureSource)->CaptureDemand {self.sources.get(&source).map(|source|source.demand.clone()).unwrap_or_default()}
    pub fn screen_demand(&mut self)->CaptureDemand {
        self.screen_source_demand(CaptureSource::MainDisplay)
    }
    pub fn screen_source_demand(&mut self, id:CaptureSource)->CaptureDemand {
        let mut demand=self.demand(id);
        let Some(source)=self.sources.get(&id) else {return demand;};
        if !source.enabled{return demand;}
        let info=&source.info;
        let size=[info.width,info.height];
        let quality=&self.quality;let tickets=&self.view_tickets;let pending=&self.quality_pending;
        demand.views=self.views.iter_mut().filter(|(_,view)|view.request.source==id)
            .filter_map(|(stream,view)| {
                let selected=quality.as_ref().filter(|quality|quality.source==id && quality.grant==view.grant
                    && tickets.get(stream)==Some(&quality.ticket));
                let original=view.request.clone();
                if let Some(quality)=selected {
                    let cap=quality.settings.receiver_cap(size,[original.width,original.height]);
                    view.request.width=cap[0];view.request.height=cap[1];
                    view.request.fps=view.request.fps.min(quality.settings.fps);
                    view.request.bitrate_kbps=view.request.bitrate_kbps.min(quality.settings.bitrate_kbps);
                }
                let mut result=view.demand_with_mode(*stream,size,selected.is_some());view.request=original;
                if let (Some(demand),Some(quality))=(&mut result,selected) {
                    demand.delivery_id=quality.settings.id;demand.quality_revision=quality.revision;
                    demand.quality_pending=pending.as_ref().is_some_and(|pending|pending.revision==quality.revision && pending.waiting.contains(stream));
                }
                result
            }).collect();
        demand.views.sort_by_key(|view|view.stream);demand
    }
    pub fn set_quality(&mut self,mut quality:DeliveryQuality) {
        self.quality_sequence=self.quality_sequence.checked_add(1).expect("quality revision exhausted");
        quality.revision=self.quality_sequence;self.quality=Some(quality);
    }
    #[cfg(test)]
    pub fn quality_id_for_test(&self)->Option<u64> {self.quality.as_ref().map(|quality|quality.settings.id)}
    pub fn begin_quality(&mut self,quality:DeliveryQuality)->Result<Option<(u64,std::sync::mpsc::Receiver<Result<(),String>>)>,String> {
        if self.quality_pending.is_some() {return Err("Another screen quality update is still applying".into());}
        let previous=self.quality.clone().filter(|old|old.ticket==quality.ticket).ok_or("Sending intention changed")?;
        let waiting=self.views.iter().filter(|(stream,view)|view.active && view.grant==quality.grant && view.request.source==quality.source
            && self.view_tickets.get(stream)==Some(&quality.ticket)).map(|(stream,_)|*stream).collect::<std::collections::HashSet<_>>();
        // Replacing multiple independent hardware encoders cannot be made
        // atomic; leave all streams untouched instead of a partial update.
        if waiting.len()>1 {return Err("Close extra receiving windows before changing screen quality".into());}
        self.set_quality(quality);
        if waiting.is_empty(){return Ok(None);}
        let revision=self.quality.as_ref().unwrap().revision;
        let (tx,rx)=std::sync::mpsc::channel();
        self.quality_pending=Some(PendingQuality{revision,previous,result:tx,waiting});
        Ok(Some((revision,rx)))
    }
    pub fn complete_quality(&mut self,id:u64,stream:u64,grant:u64,revision:u64,accepted:bool)->bool {
        let Some(quality)=self.quality.as_ref() else{return false;};
        if quality.settings.id!=id || quality.grant!=grant || quality.revision!=revision
            || self.view_tickets.get(&stream)!=Some(&quality.ticket)
            || !self.views.get(&stream).is_some_and(|view|view.active&&view.grant==grant&&view.request.source==quality.source)
            || self.quality_pending.as_ref().is_none_or(|pending|pending.revision!=revision || !pending.waiting.contains(&stream)) {return false;}
        if !accepted {self.finish_quality(false);}
        else {
            let pending=self.quality_pending.as_mut().unwrap();pending.waiting.remove(&stream);
            if pending.waiting.is_empty() {self.finish_quality(true);}
        }
        true
    }
    fn finish_quality(&mut self,accepted:bool) {
        if let Some(pending)=self.quality_pending.take() {
            if !accepted {
                if let Some(quality)=self.quality.clone() {self.invalidate_quality_views(&quality);}
                self.quality=Some(pending.previous);
            }
            let _=pending.result.send(if accepted {Ok(())} else {Err("The requested quality is not supported by the screen encoder".into())});
        }
    }
    fn invalidate_quality_views(&mut self,quality:&DeliveryQuality) {
        let tickets=&self.view_tickets;
        for (stream,view) in &mut self.views {
            if view.grant==quality.grant && view.request.source==quality.source && tickets.get(stream)==Some(&quality.ticket) {
                view.invalidate_geometry();
                view.keyframe=view.keyframe.saturating_add(1);
            }
        }
    }
    pub fn cancel_quality(&mut self,id:u64,revision:u64)->bool {
        if self.quality.as_ref().is_some_and(|q|q.settings.id==id&&q.revision==revision)
            && self.quality_pending.as_ref().is_some_and(|p|p.revision==revision) {self.finish_quality(false);true} else {false}
    }
    pub fn clear_quality(&mut self,ticket:[u8;32]) {
        if self.quality.as_ref().is_some_and(|q|q.ticket==ticket) {
            if self.quality_pending.is_some() {self.finish_quality(false);}
            else if let Some(quality)=self.quality.clone() {self.invalidate_quality_views(&quality);}
            self.quality=None;
        }
    }
    pub fn set_view_tickets(&mut self,tickets:HashMap<u64,[u8;32]>) {self.view_tickets=tickets;}
    pub fn viewport(&mut self,stream:u64,demand:protocol::viewport::ViewportDemand)->Result<u64,String> {
        let view=self.views.get_mut(&stream).ok_or("Screen viewport is unavailable")?;
        view.viewport(demand)
    }
    pub fn accepts_screen_frame(&self,stream:u64,grant:u64)->bool {
        self.screen_frame_source(stream,grant).is_some()
    }
    pub fn accepts_screen_geometry(&self,stream:u64,grant:u64,revision:u64)->bool {
        self.screen_frame_source(stream,grant).is_some()
            && self.views.get(&stream).is_some_and(|view|view.accepts_geometry(revision))
    }
    pub fn screen_frame_source(&self,stream:u64,grant:u64)->Option<CaptureSource> {
        let view=self.views.get(&stream).filter(|view|view.active&&view.grant==grant)?;
        self.sources.get(&view.request.source).filter(|source|source.enabled&&source.grant==grant)?;
        Some(view.request.source)
    }
    pub fn request_view_keyframe(&mut self,stream:u64) {
        if let Some(view)=self.views.get_mut(&stream){view.keyframe=view.keyframe.saturating_add(1);}
    }
    pub fn accepts_frame(&self,source:CaptureSource,epoch:u64,audio:bool)->bool {
        self.sources.get(&source).is_some_and(|source|source.enabled
            && if audio{source.grant==epoch&&source.audio&&source.demand.audio}
                else{source.demand.epoch==epoch&&source.demand.active})
    }
    pub fn fail(&mut self,source:CaptureSource,epoch:u64)->bool {
        if self.sources.get(&source).is_none_or(|entry|!entry.enabled||entry.demand.epoch!=epoch){return false;}
        let next=self.next();let entry=self.sources.get_mut(&source).unwrap();
        entry.enabled=false;entry.grant=next;entry.demand=CaptureDemand{epoch:next,grant:next,..Default::default()};true
    }
    pub fn geometry(&mut self,source:CaptureSource,epoch:u64,width:u32,height:u32,rotation:u16)->Result<u64,String> {
        if !(2..=8192).contains(&width)||!(2..=8192).contains(&height)||u64::from(width)*u64::from(height)>33_554_432
            ||!matches!(rotation,0|90|180|270){return Err("Invalid camera or screen geometry".into());}
        let entry=self.sources.get(&source).filter(|entry|entry.enabled&&entry.demand.epoch==epoch).ok_or("Capture configuration changed")?;
        if (entry.info.width,entry.info.height,entry.info.camera.map_or(entry.capture_rotation,|camera|camera.rotation))==(width,height,rotation){return Ok(epoch);}
        for view in self.views.values_mut().filter(|view|view.request.source==source) {view.invalidate_geometry();}
        let next=self.next();let entry=self.sources.get_mut(&source).unwrap();
        entry.info.width=width;entry.info.height=height;
        entry.capture_rotation=rotation;
        if let Some(camera)=&mut entry.info.camera{camera.rotation=rotation;}
        entry.demand.epoch=next;entry.demand.keyframe=entry.demand.keyframe.saturating_add(1);Ok(next)
    }
    pub fn recompute<'a>(&mut self,requests:impl IntoIterator<Item=ActiveRequest<'a>>) {
        let mut wanted=HashMap::<CaptureSource,CaptureDemand>::new();
        let mut retained=std::collections::HashSet::new();
        for active in requests {
            let request=active.request;
            let Some(source)=self.sources.get(&request.source).filter(|source|source.enabled&&source.grant==active.grant)else{continue;};
            if screen_source(request.source) {
                retained.insert(active.stream);
                let view=self.views.entry(active.stream).or_insert_with(||ScreenView::new(active.grant,request.clone()));
                if view.grant!=active.grant{*view=ScreenView::new(active.grant,request.clone());}
                view.request=request.clone();view.active=active.video;
            }
            let demand=wanted.entry(request.source).or_insert_with(||CaptureDemand{width:0,height:0,fps:0,bitrate_kbps:0,..Default::default()});
            demand.subscriptions+=1;
            demand.active|=active.video;demand.audio|=active.audio&&source.audio&&request.audio;
            if active.video {
                demand.width=demand.width.max(request.width);demand.height=demand.height.max(request.height);
                demand.fps=demand.fps.max(request.fps);demand.bitrate_kbps=demand.bitrate_kbps.max(request.bitrate_kbps);
            }
        }
        self.views.retain(|stream,_|retained.contains(stream));
        let ids=self.sources.keys().copied().collect::<Vec<_>>();
        for id in ids {
            let old=&self.sources[&id].demand;
            let mut next=wanted.remove(&id).unwrap_or_else(||CaptureDemand{subscriptions:0,active:false,audio:false,..old.clone()});
            if !next.active {next.width=old.width;next.height=old.height;next.fps=old.fps;next.bitrate_kbps=old.bitrate_kbps;}
            let changed=(old.active,old.width,old.height,old.fps,old.bitrate_kbps)
                !=(next.active,next.width,next.height,next.fps,next.bitrate_kbps);
            next.epoch=old.epoch;next.keyframe=old.keyframe;next.grant=old.grant;
            if changed{next.epoch=self.next();next.keyframe=next.keyframe.saturating_add(1);}
            let entry=self.sources.get_mut(&id).unwrap();
            if next.subscriptions==0 && entry.demand.subscriptions>0 {
                entry.artwork=None;
                // A quick reconnect can reuse the still-closing native worker.
                // Its cached icon may be republished, but old completions cannot.
                entry.artwork_epoch=entry.artwork_epoch.checked_add(1).expect("artwork generation exhausted");
            }
            entry.demand=next;
        }
    }
    pub fn request_keyframe(&mut self,source:CaptureSource){if let Some(entry)=self.sources.get_mut(&source){entry.demand.keyframe=entry.demand.keyframe.saturating_add(1);}}
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::session::CameraFacing;
    #[test]
    fn outgoing_quality_is_bound_to_ticket_source_and_grant() {
        use crate::outgoing_quality::{DeliveryQuality,QualityRequest};
        let mut media=MediaSources::new(true,false);
        let source=CaptureSource::MainDisplay;
        let grant=media.sources[&source].grant;
        let request=SubscriptionRequest{id:1,source,width:1920,height:1080,fps:60,bitrate_kbps:20_000,audio:false};
        media.recompute([ActiveRequest{stream:1,grant,request:&request,video:true,audio:false},
            ActiveRequest{stream:2,grant,request:&request,video:true,audio:false}]);
        media.set_view_tickets([(1,[1;32]),(2,[2;32])].into_iter().collect());
        let viewport=protocol::viewport::ViewportDemand{id:1,source_revision:0,revision:1,
            visible:protocol::viewport::NormalizedRect::FULL,width:360,height:800};
        media.viewport(1,viewport).unwrap();media.viewport(2,viewport).unwrap();
        let settings=QualityRequest{id:7,short_edge:540,fps:30,bitrate_kbps:10_000};
        media.set_quality(DeliveryQuality{ticket:[1;32],source,grant,settings,revision:0});
        let views=media.screen_demand().views;
        assert_eq!((views[0].geometry.width,views[0].fps,views[0].bitrate_kbps),(540,30,10_000));
        assert!(views[1].geometry.width<=360);
        assert_eq!(views[1].fps,60);
        media.clear_quality([2;32]);
        assert_eq!(media.screen_demand().views[0].fps,30);
        let directed_revision=media.screen_demand().views[0].geometry.revision;
        media.clear_quality([1;32]);
        assert!(!media.accepts_screen_geometry(1,grant,directed_revision));
        assert_eq!(media.screen_demand().views[0].fps,60);
        media.set_quality(DeliveryQuality{ticket:[1;32],source,grant:grant+1,settings,revision:0});
        assert_eq!(media.screen_demand().views[0].fps,60);
    }
    #[test]
    fn quality_update_waits_for_exact_encoder_and_rolls_back_on_rejection() {
        use crate::outgoing_quality::{DeliveryQuality,QualityRequest};
        let mut media=MediaSources::new(true,false);
        let source=CaptureSource::MainDisplay;let grant=media.sources[&source].grant;
        let request=SubscriptionRequest{id:1,source,width:1920,height:1080,fps:60,bitrate_kbps:20_000,audio:false};
        media.recompute([ActiveRequest{stream:1,grant,request:&request,video:true,audio:false},
            ActiveRequest{stream:2,grant,request:&request,video:true,audio:false}]);
        media.set_view_tickets([(1,[1;32]),(2,[2;32])].into_iter().collect());
        let quality=|fps|DeliveryQuality{ticket:[1;32],source,grant,
            settings:QualityRequest{id:7,short_edge:720,fps,bitrate_kbps:5000},revision:0};
        media.set_quality(quality(30));
        let before=media.screen_demand().views;
        let baseline=before[0].geometry.revision;
        let mut changed=quality(60);changed.settings.short_edge=540;
        let (revision,result)=media.begin_quality(changed).unwrap().unwrap();
        let pending=media.screen_demand().views;
        assert!(pending[0].quality_pending);assert!(!pending[1].quality_pending);
        assert_eq!(pending[0].quality_revision,revision);
        let tentative=pending[0].geometry.revision;
        assert_ne!(baseline,tentative);
        assert!(media.accepts_screen_geometry(1,grant,tentative));
        assert!(media.begin_quality(quality(45)).is_err());
        assert!(!media.complete_quality(7,2,grant,revision,true));
        assert!(!media.complete_quality(7,1,grant,revision+1,true));
        assert!(media.complete_quality(7,1,grant,revision,false));
        assert!(result.recv().unwrap().is_err());
        assert!(!media.accepts_screen_geometry(1,grant,tentative));
        let restored=media.screen_demand().views.remove(0);
        assert_eq!(restored.fps,30);assert!(!restored.quality_pending);
        assert!(restored.geometry.revision>tentative);
        assert!(restored.keyframe>before[0].keyframe);
        assert_eq!(media.screen_demand().views[1].keyframe,before[1].keyframe);
        let (revision,result)=media.begin_quality(quality(45)).unwrap().unwrap();
        assert!(media.complete_quality(7,1,grant,revision,true));
        assert!(!media.cancel_quality(7,revision));
        assert!(result.try_recv().unwrap().is_ok(),"ACK that beats timeout must be returned to the caller");
        assert_eq!(media.screen_demand().views[0].fps,45);
        let (revision,result)=media.begin_quality(quality(50)).unwrap().unwrap();
        assert!(media.cancel_quality(7,revision));
        assert!(!media.complete_quality(7,1,grant,revision,true));
        assert!(result.recv().unwrap().is_err());
        assert_eq!(media.screen_demand().views[0].fps,45);
        media.set_view_tickets([(1,[1;32]),(2,[1;32])].into_iter().collect());
        assert!(media.begin_quality(quality(55)).is_err());
        assert_eq!(media.screen_demand().views[0].fps,45);
    }
    #[test]
    fn app_rotation_invalidates_epoch_and_catalog_refresh_preserves_geometry() {
        let mut media=MediaSources::new(true,false);
        let source=CaptureSource::Window(91);
        let app=CaptureSourceInfo {source,title:"App".into(),application:"App".into(),process_id:None,
            width:720,height:1280,supports_input:false,camera:None};
        media.set_applications(vec![app.clone()]).unwrap();
        let grant=media.sources[&source].grant;
        let initial=media.demand(source).epoch;
        let landscape=media.geometry(source,initial,1280,720,90).unwrap();
        assert_ne!(landscape,initial);
        let reverse=media.geometry(source,landscape,1280,720,270).unwrap();
        assert_ne!(reverse,landscape);
        assert!(media.geometry(source,landscape,720,1280,0).is_err());
        media.set_applications(vec![app]).unwrap();
        assert_eq!((media.sources[&source].info.width,media.sources[&source].info.height),(1280,720));
        assert_eq!(media.sources[&source].demand.epoch,reverse);
        assert_eq!(media.sources[&source].grant,grant);
    }
    #[test]
    fn catalog_icons_are_cached_bound_and_never_replace_stream_grants() {
        let mut media=MediaSources::new(true,false);
        let source=CaptureSource::Window(7);
        media.set_applications(vec![CaptureSourceInfo {source,title:"Calculator".into(),application:"Calculator".into(),
            process_id:None,width:720,height:1280,supports_input:false,camera:None}]).unwrap();
        let grant=media.sources[&source].grant;
        let icon=remote_core::application_icons::encode_rgba(1,1,&[12,34,56,255]).unwrap();
        media.set_catalog_icons(vec![(source,icon.clone())]).unwrap();
        let SessionCommand::Sources{sources,source_icons,icons,..}=media.catalog_reply(1,false,&[]) else {panic!()};
        assert_eq!(sources.len(),2);assert_eq!(source_icons[0].source,source);assert_eq!(icons.len(),1);
        let SessionCommand::Sources{source_icons,icons,..}=media.catalog_reply(2,false,&[icon.id]) else {panic!()};
        assert_eq!(source_icons.len(),1);assert!(icons.is_empty());
        let mut corrupt=icon.clone();corrupt.png[0]^=1;
        assert!(media.set_catalog_icons(vec![(source,corrupt)]).is_err());
        assert!(media.set_catalog_icons(vec![(CaptureSource::Window(8),icon.clone())]).is_err());
        assert_eq!(media.sources[&source].catalog_icon.as_ref().unwrap().id,icon.id);
        assert_eq!(media.sources[&source].grant,grant);
        media.set_applications(vec![]).unwrap();
        let SessionCommand::Sources{source_icons,icons,..}=media.catalog_reply(3,false,&[]) else {panic!()};
        assert!(source_icons.is_empty()&&icons.is_empty());
    }
    #[test]
    fn independent_apps_keep_pixels_grants_and_paused_tasks_separate() {
        let mut media=MediaSources::new(true,false);
        let app=|id,package:&str,width,height|CaptureSourceInfo {source:CaptureSource::Window(id),
            title:package.into(),application:package.into(),process_id:None,width,height,supports_input:false,camera:None};
        let a=app(10,"app.a",720,1280);let b=app(20,"app.b",1080,1920);
        assert!(media.set_applications(vec![app(30,&"应用".repeat(80),720,1280)]).is_ok(),"Native application labels are UTF-8 names, not package identifiers");
        media.set_applications(vec![a.clone(),b.clone()]).unwrap();
        let mut first=request(a.source);first.id=4;
        let mut second=request(b.source);second.id=5;
        let ga=media.sources[&a.source].grant;let gb=media.sources[&b.source].grant;
        media.recompute([
            ActiveRequest{stream:40,grant:ga,request:&first,video:true,audio:false},
            ActiveRequest{stream:50,grant:gb,request:&second,video:true,audio:false}]);
        let va=media.screen_source_demand(a.source);let vb=media.screen_source_demand(b.source);
        assert_eq!(va.views.len(),1);assert_eq!(vb.views.len(),1);
        assert_eq!(va.views[0].geometry.source_width,720);
        assert_eq!(vb.views[0].geometry.source_width,1080);
        assert_eq!(media.screen_frame_source(40,ga),Some(a.source));
        assert_eq!(media.screen_frame_source(40,gb),None);
        assert!(media.screen_demand().views.is_empty());
        media.recompute([ActiveRequest{stream:40,grant:ga,request:&first,video:false,audio:false}]);
        let paused=media.screen_source_demand(a.source);
        assert_eq!(paused.subscriptions,1);assert!(!paused.active);assert!(paused.views.is_empty());
        media.set_applications(vec![b]).unwrap();
        assert_eq!(media.screen_frame_source(40,ga),None);
        media.set_applications(vec![a]).unwrap();
        assert_ne!(media.sources[&first.source].grant,ga,"Reopened apps cannot reuse old frame or input grants");
        assert!(media.set_applications(vec![app(20,"app.b",1080,1920),app(20,"other",720,1280)]).is_err());
    }
    fn request(source:CaptureSource)->SubscriptionRequest {SubscriptionRequest{id:7,source,width:1280,height:720,fps:30,bitrate_kbps:5000,audio:true}}
    fn active<'a>(media:&MediaSources,request:&'a SubscriptionRequest,video:bool,audio:bool)->ActiveRequest<'a> {
        ActiveRequest{stream:u64::from(request.id),grant:media.sources[&request.source].grant,request,video,audio}
    }
    fn camera()->NativeCamera {NativeCamera{key:"native-front".into(),name:"Front camera".into(),facing:CameraFacing::Front,switch_group:Some("phone".into()),width:1280,height:720,rotation:90}}
    #[test]
    fn stopping_projection_preserves_camera_demand_and_authorization() {
        let mut media=MediaSources::new(true,true);let cam=media.set_cameras(vec![camera()]).unwrap()[0].0;
        let screen=request(CaptureSource::MainDisplay);let camera=request(cam);
        media.recompute([active(&media,&screen,true,true),active(&media,&camera,true,true)]);
        let epoch=media.demand(cam).epoch;assert!(media.accepts_frame(cam,epoch,false));assert!(!media.accepts_frame(cam,epoch,true));
        media.set_screen(false,false);
        assert!(media.accepts_frame(cam,epoch,false));assert_eq!(media.catalog(false).len(),1);
        media.set_cameras(vec![]).unwrap();assert!(!media.accepts_frame(cam,epoch,false));
    }
    #[test]
    fn reconfiguration_filters_old_encoded_frames_without_revoking_the_source() {
        let mut media=MediaSources::new(true,false);let mut req=request(CaptureSource::MainDisplay);
        media.recompute([active(&media,&req,true,false)]);
        let grant=media.sources[&req.source].grant;let old=media.demand(req.source).epoch;
        req.fps=60;media.recompute([active(&media,&req,true,false)]);
        let current=media.demand(req.source).epoch;assert_ne!(old,current);
        assert!(!media.accepts_frame(req.source,old,false));assert!(media.accepts_frame(req.source,current,false));
        assert_eq!(media.sources[&req.source].grant,grant);assert!(!media.fail(req.source,old));
        assert!(media.geometry(req.source,old,1920,1080,0).is_err());
        let geometry=media.geometry(req.source,current,1920,1080,0).unwrap();
        assert!(!media.accepts_frame(req.source,current,false));assert!(media.accepts_frame(req.source,geometry,false));
    }
    #[test]
    fn multiple_viewers_share_one_demand_and_removing_one_keeps_the_other() {
        let mut media=MediaSources::new(true,false);let a=request(CaptureSource::MainDisplay);let mut b=a.clone();b.id=8;b.fps=60;b.bitrate_kbps=8000;
        media.recompute([active(&media,&a,true,false),active(&media,&b,true,false)]);
        assert_eq!(media.demand(a.source).fps,60);
        media.recompute([active(&media,&a,true,false)]);assert!(media.demand(a.source).active);assert_eq!(media.demand(a.source).fps,30);
        media.recompute([]);assert!(!media.demand(a.source).active);
    }
    #[test]
    fn audio_pause_keeps_video_generation_but_reauthorization_rejects_old_pcm() {
        let mut media=MediaSources::new(true,true);let req=request(CaptureSource::MainDisplay);
        media.recompute([active(&media,&req,true,true)]);
        let video=media.demand(req.source).epoch;let grant=media.demand(req.source).grant;
        assert!(media.accepts_frame(req.source,grant,true));
        media.recompute([active(&media,&req,true,false)]);
        assert_eq!(video,media.demand(req.source).epoch);assert!(!media.accepts_frame(req.source,grant,true));
        media.set_screen(false,false);media.set_screen(true,true);
        media.recompute([active(&media,&req,true,true)]);
        assert!(!media.accepts_frame(req.source,grant,true));
        assert!(media.accepts_frame(req.source,media.demand(req.source).grant,true));
    }
    #[test]
    fn capture_rotation_fences_old_geometry_but_roi_keeps_prior_revision() {
        use protocol::viewport::{NormalizedRect,ViewportDemand};
        let mut media=MediaSources::new(true,false);let req=request(CaptureSource::MainDisplay);
        media.recompute([active(&media,&req,true,false)]);
        let first=media.screen_demand().views.remove(0);
        assert!(media.accepts_screen_geometry(first.stream,first.grant,first.geometry.revision));
        media.viewport(first.stream,ViewportDemand{id:req.id,source_revision:0,revision:1,
            visible:NormalizedRect{x:0,y:0,width:500_000,height:500_000},width:320,height:320}).unwrap();
        let roi=media.screen_demand().views.remove(0);
        assert!(roi.geometry.revision>first.geometry.revision);
        assert!(media.accepts_screen_geometry(first.stream,first.grant,first.geometry.revision));
        let epoch=media.demand(req.source).epoch;
        let [w,h]=[roi.geometry.source_width,roi.geometry.source_height];
        media.geometry(req.source,epoch,w,h,180).unwrap();
        assert!(!media.accepts_screen_geometry(first.stream,first.grant,first.geometry.revision));
        assert!(!media.accepts_screen_geometry(roi.stream,roi.grant,roi.geometry.revision));
        let next=media.screen_demand().views.remove(0);
        assert!(next.geometry.revision>roi.geometry.revision);
        assert!(media.accepts_screen_geometry(next.stream,next.grant,next.geometry.revision));
        assert!(!media.accepts_screen_geometry(next.stream,next.grant,next.geometry.revision+1));
    }
    #[test]
    fn screen_views_keep_independent_crops_and_reject_old_grants() {
        use protocol::viewport::{NormalizedRect,ViewportDemand};
        let mut media=MediaSources::new(true,false);let a=request(CaptureSource::MainDisplay);let b=SubscriptionRequest{id:8,..a.clone()};
        media.recompute([active(&media,&a,true,false),active(&media,&b,true,false)]);
        let demand=|id,revision,x|ViewportDemand{id,source_revision:0,revision,
            visible:NormalizedRect{x,y:0,width:250_000,height:250_000},width:320,height:320};
        media.viewport(7,demand(7,2,0)).unwrap();media.viewport(8,demand(8,1,750_000)).unwrap();
        assert_eq!(media.viewport(7,demand(7,1,750_000)).unwrap(),2);
        assert!(media.viewport(7,demand(8,3,750_000)).is_err());
        let views=media.screen_demand().views;assert_eq!(views.len(),2);
        assert!(views[0].geometry.crop.x<views[1].geometry.crop.x);
        let grant=views[0].grant;
        media.recompute([active(&media,&a,false,false),active(&media,&b,true,false)]);
        assert!(!media.accepts_screen_frame(7,grant));assert!(media.accepts_screen_frame(8,grant));
        assert_eq!(media.screen_demand().views.len(),1);
        media.set_screen(false,false);media.set_screen(true,false);
        // An old subscription must never gain access to a newly authorized OS projection.
        media.recompute([ActiveRequest{stream:8,grant,request:&b,video:true,audio:false}]);
        assert!(media.screen_demand().views.is_empty());assert!(!media.accepts_screen_frame(8,grant));
    }
}
