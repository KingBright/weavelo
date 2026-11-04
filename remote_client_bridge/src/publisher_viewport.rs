//! Per-subscription screen geometry; camera encoders keep their source fanout.
use protocol::{session::SubscriptionRequest, viewport::{FrameGeometry, ViewportDemand}};
use remote_core::viewport::{ViewportPlanner, bitrate_for_pixels};
use std::time::Instant;

pub(crate) struct ScreenView {
    pub grant: u64,
    pub request: SubscriptionRequest,
    pub active: bool,
    pub keyframe: u64,
    planner: ViewportPlanner,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct ScreenViewDemand {
    pub stream: u64,
    pub grant: u64,
    pub keyframe: u64,
    pub fps: u32,
    pub bitrate_kbps: u32,
    pub geometry: FrameGeometry,
}

impl ScreenView {
    pub fn new(grant:u64,request:SubscriptionRequest)->Self {
        Self{grant,planner:ViewportPlanner::for_source(request.source),request,active:true,keyframe:0}
    }
    pub fn viewport(&mut self,demand:ViewportDemand)->Result<u64,String> {
        if demand.id!=self.request.id || demand.source_revision!=0 {
            return Err("Viewport belongs to another screen subscription".into());
        }
        self.planner.update(demand,Instant::now()).map_err(str::to_owned)?;
        Ok(self.planner.demand().map_or(0,|demand|demand.revision))
    }
    pub fn demand(&mut self,stream:u64,source:[u32;2])->Option<ScreenViewDemand> {
        if !self.active{return None;}
        let geometry=self.planner.plan(source,[self.request.width,self.request.height],0,Instant::now()).ok()?;
        Some(ScreenViewDemand{stream,grant:self.grant,keyframe:self.keyframe,fps:self.request.fps,
            bitrate_kbps:bitrate_for_pixels(geometry.width,geometry.height,self.request.fps,self.request.bitrate_kbps),geometry})
    }
}
