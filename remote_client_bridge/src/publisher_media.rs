//! Independent source authorization, encoder demand and stale-frame rejection.
use protocol::session::{CaptureSource, CaptureSourceInfo, SubscriptionRequest};
use remote_core::cameras::{CameraRegistry, NativeCamera};
use std::collections::HashMap;

#[derive(Clone, Debug, serde::Serialize)]
pub struct CaptureDemand {
    pub active: bool,
    pub audio: bool,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub bitrate_kbps: u32,
    pub keyframe: u64,
    pub epoch: u64,
    pub grant: u64,
}
impl Default for CaptureDemand {
    fn default()->Self {Self{active:false,audio:false,width:1280,height:720,fps:30,bitrate_kbps:5000,keyframe:0,epoch:0,grant:0}}
}
#[derive(Clone)]
pub(crate) struct Source {
    pub info: CaptureSourceInfo,
    pub enabled: bool,
    pub audio: bool,
    pub grant: u64,
    pub demand: CaptureDemand,
}
pub(crate) struct ActiveRequest<'a> {
    pub request:&'a SubscriptionRequest,
    pub video:bool,
    pub audio:bool,
}
pub(crate) struct MediaSources {
    pub peer_streams:Vec<(Option<String>,usize)>,
    pub sources:HashMap<CaptureSource,Source>,
    cameras:CameraRegistry,
    sequence:u64,
}
impl MediaSources {
    pub fn new(screen:bool,audio:bool)->Self {
        let mut this=Self{peer_streams:Vec::new(),sources:HashMap::new(),cameras:CameraRegistry::default(),sequence:1};
        this.sources.insert(CaptureSource::MainDisplay,Source {
            info:CaptureSourceInfo {source:CaptureSource::MainDisplay,title:"Android selected screen or app".into(),
                application:"MediaProjection".into(),process_id:None,width:1080,height:1920,supports_input:false,camera:None},
            enabled:screen,audio:screen&&audio,grant:1,demand:CaptureDemand{epoch:1,grant:1,..Default::default()},
        });this
    }
    fn next(&mut self)->u64 {
        self.sequence=self.sequence.checked_add(1).expect("capture generation exhausted");self.sequence
    }
    pub fn set_screen(&mut self,enabled:bool,audio:bool)->bool {
        let audio=enabled&&audio;
        let old=&self.sources[&CaptureSource::MainDisplay];
        if (old.enabled,old.audio)==(enabled,audio){return false;}
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
            self.sources.insert(info.source,Source{info,enabled:true,audio:false,grant:epoch,
                demand:CaptureDemand{epoch,grant:epoch,..Default::default()}});
        }Ok(bindings)
    }
    pub fn camera_key(&self,source:CaptureSource)->Option<&str> {self.cameras.resolve(source).map(|camera|camera.key.as_str())}
    pub fn catalog(&self,input_ready:bool)->Vec<CaptureSourceInfo> {
        let mut sources=self.sources.values().filter(|source|source.enabled).map(|source| {
            let mut info=source.info.clone();info.supports_input=info.source==CaptureSource::MainDisplay&&input_ready;info
        }).collect::<Vec<_>>();
        sources.sort_by_key(|info|match info.source{CaptureSource::MainDisplay=>0,CaptureSource::Camera(id)=>id,_=>u64::MAX});sources
    }
    pub fn enabled(&self,source:CaptureSource)->bool {self.sources.get(&source).is_some_and(|source|source.enabled)}
    pub fn demand(&self,source:CaptureSource)->CaptureDemand {self.sources.get(&source).map(|source|source.demand.clone()).unwrap_or_default()}
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
        if (entry.info.width,entry.info.height,entry.info.camera.map_or(0,|camera|camera.rotation))==(width,height,rotation){return Ok(epoch);}
        let next=self.next();let entry=self.sources.get_mut(&source).unwrap();
        entry.info.width=width;entry.info.height=height;
        if let Some(camera)=&mut entry.info.camera{camera.rotation=rotation;}
        entry.demand.epoch=next;entry.demand.keyframe=entry.demand.keyframe.saturating_add(1);Ok(next)
    }
    pub fn recompute<'a>(&mut self,requests:impl IntoIterator<Item=ActiveRequest<'a>>) {
        let mut wanted=HashMap::<CaptureSource,CaptureDemand>::new();
        for active in requests {
            let request=active.request;
            let Some(source)=self.sources.get(&request.source).filter(|source|source.enabled)else{continue;};
            if !active.video && !(active.audio&&source.audio&&request.audio){continue;}
            let demand=wanted.entry(request.source).or_insert_with(||CaptureDemand{width:0,height:0,fps:0,bitrate_kbps:0,..Default::default()});
            demand.active|=active.video;demand.audio|=active.audio&&source.audio&&request.audio;
            if active.video {
                demand.width=demand.width.max(request.width);demand.height=demand.height.max(request.height);
                demand.fps=demand.fps.max(request.fps);demand.bitrate_kbps=demand.bitrate_kbps.max(request.bitrate_kbps);
            }
        }
        let ids=self.sources.keys().copied().collect::<Vec<_>>();
        for id in ids {
            let old=&self.sources[&id].demand;
            let mut next=wanted.remove(&id).unwrap_or_else(||CaptureDemand{active:false,audio:false,..old.clone()});
            if !next.active {next.width=old.width;next.height=old.height;next.fps=old.fps;next.bitrate_kbps=old.bitrate_kbps;}
            let changed=(old.active,old.width,old.height,old.fps,old.bitrate_kbps)
                !=(next.active,next.width,next.height,next.fps,next.bitrate_kbps);
            next.epoch=old.epoch;next.keyframe=old.keyframe;next.grant=old.grant;
            if changed{next.epoch=self.next();next.keyframe=next.keyframe.saturating_add(1);}
            self.sources.get_mut(&id).unwrap().demand=next;
        }
    }
    pub fn request_keyframe(&mut self,source:CaptureSource){if let Some(entry)=self.sources.get_mut(&source){entry.demand.keyframe=entry.demand.keyframe.saturating_add(1);}}
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::session::CameraFacing;
    fn request(source:CaptureSource)->SubscriptionRequest {SubscriptionRequest{id:7,source,width:1280,height:720,fps:30,bitrate_kbps:5000,audio:true}}
    fn camera()->NativeCamera {NativeCamera{key:"native-front".into(),name:"Front camera".into(),facing:CameraFacing::Front,switch_group:Some("phone".into()),width:1280,height:720,rotation:90}}
    #[test]
    fn stopping_projection_preserves_camera_demand_and_authorization() {
        let mut media=MediaSources::new(true,true);let cam=media.set_cameras(vec![camera()]).unwrap()[0].0;
        let screen=request(CaptureSource::MainDisplay);let camera=request(cam);
        media.recompute([ActiveRequest{request:&screen,video:true,audio:true},ActiveRequest{request:&camera,video:true,audio:true}]);
        let epoch=media.demand(cam).epoch;assert!(media.accepts_frame(cam,epoch,false));assert!(!media.accepts_frame(cam,epoch,true));
        media.set_screen(false,false);
        assert!(media.accepts_frame(cam,epoch,false));assert_eq!(media.catalog(false).len(),1);
        media.set_cameras(vec![]).unwrap();assert!(!media.accepts_frame(cam,epoch,false));
    }
    #[test]
    fn reconfiguration_filters_old_encoded_frames_without_revoking_the_source() {
        let mut media=MediaSources::new(true,false);let mut req=request(CaptureSource::MainDisplay);
        media.recompute([ActiveRequest{request:&req,video:true,audio:false}]);
        let grant=media.sources[&req.source].grant;let old=media.demand(req.source).epoch;
        req.fps=60;media.recompute([ActiveRequest{request:&req,video:true,audio:false}]);
        let current=media.demand(req.source).epoch;assert_ne!(old,current);
        assert!(!media.accepts_frame(req.source,old,false));assert!(media.accepts_frame(req.source,current,false));
        assert_eq!(media.sources[&req.source].grant,grant);assert!(!media.fail(req.source,old));
        assert!(media.geometry(req.source,old,1920,1080,0).is_err());
        let geometry=media.geometry(req.source,current,1920,1080,0).unwrap();
        assert!(!media.accepts_frame(req.source,current,false));assert!(media.accepts_frame(req.source,geometry,false));
    }
    #[test]
    fn multiple_viewers_share_one_demand_and_removing_one_keeps_the_other() {
        let mut media=MediaSources::new(true,false);let a=request(CaptureSource::MainDisplay);let mut b=a.clone();b.fps=60;b.bitrate_kbps=8000;
        media.recompute([ActiveRequest{request:&a,video:true,audio:false},ActiveRequest{request:&b,video:true,audio:false}]);
        assert_eq!(media.demand(a.source).fps,60);
        media.recompute([ActiveRequest{request:&a,video:true,audio:false}]);assert!(media.demand(a.source).active);assert_eq!(media.demand(a.source).fps,30);
        media.recompute([]);assert!(!media.demand(a.source).active);
    }
    #[test]
    fn audio_pause_keeps_video_generation_but_reauthorization_rejects_old_pcm() {
        let mut media=MediaSources::new(true,true);let req=request(CaptureSource::MainDisplay);
        media.recompute([ActiveRequest{request:&req,video:true,audio:true}]);
        let video=media.demand(req.source).epoch;let grant=media.demand(req.source).grant;
        assert!(media.accepts_frame(req.source,grant,true));
        media.recompute([ActiveRequest{request:&req,video:true,audio:false}]);
        assert_eq!(video,media.demand(req.source).epoch);assert!(!media.accepts_frame(req.source,grant,true));
        media.set_screen(false,false);media.set_screen(true,true);
        media.recompute([ActiveRequest{request:&req,video:true,audio:true}]);
        assert!(!media.accepts_frame(req.source,grant,true));
        assert!(media.accepts_frame(req.source,media.demand(req.source).grant,true));
    }
}
