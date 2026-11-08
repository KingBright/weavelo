//! Local navigation and the geometry of the last painted screen/window frame.
//! Paint submission is not a measurement of physical display presentation.
use super::*;
use crate::desktop::original_owner::ViewportTarget;
use protocol::viewport::{FrameGeometry,NormalizedRect,ViewportDemand};
use remote_core::view_transform::ViewTransform;

#[derive(Clone,Copy,PartialEq)]
struct Wanted {visible:NormalizedRect,width:u32,height:u32}

#[derive(Clone)]
pub(super) struct Painted {
    pub decoded_at:Instant,
    pub bounds:Bounds<Pixels>,
    pub geometry:FrameGeometry,
    pub view:ViewTransform,
    pub target:ViewportTarget,
}

pub(super) struct State {
    pub ready:Arc<tokio::sync::Notify>,
    target:Option<ViewportTarget>,
    source:Option<(u32,Instant)>,
    view:Option<ViewTransform>,
    layout:Option<([u32;2],Bounds<Pixels>,f32,bool)>,
    painted:Option<Painted>,
    wanted:Option<Wanted>,
    pending:Option<(ViewportDemand,u64)>,
    last_attempt:Option<Instant>,
    revision:u64,
    epoch:u64,
    cancel:bool,
    navigation:bool,
    pan_point:Option<Point<Pixels>>,
}
impl Default for State {
    fn default()->Self {Self{ready:Arc::new(tokio::sync::Notify::new()),target:None,source:None,view:None,layout:None,
        painted:None,wanted:None,pending:None,last_attempt:None,revision:0,epoch:0,cancel:false,navigation:false,pan_point:None}}
}
impl State {
    pub fn target(&mut self,source:Option<(u32,Instant)>,target:Option<ViewportTarget>) {
        let same=match (&self.target,&target) {
            (Some(old),Some(new))=>old.id==new.id&&old.source_revision==new.source_revision&&old.binding.same_connection(&new.binding),
            (None,None)=>true,
            _=>false,
        };
        if !same {
            self.view=None;self.layout=None;self.painted=None;self.wanted=None;self.pending=None;
            self.last_attempt=None;self.revision=0;self.navigation=false;self.cancel_gesture();
        }else if self.source!=source{self.cancel_gesture();}
        self.source=source;self.target=target;
    }
    fn cancel_gesture(&mut self) {
        self.epoch=self.epoch.checked_add(1).expect("Local view epoch exhausted");self.cancel=true;self.pan_point=None;self.ready.notify_one();
    }
    pub fn take_cancel(&mut self)->bool {std::mem::take(&mut self.cancel)}
    pub fn navigation(&self)->bool {self.navigation}
    pub fn epoch(&self)->u64 {self.epoch}
    pub fn painted(&self)->Option<Painted> {if self.cancel{None}else{self.painted.clone()}}
    pub fn acknowledges(&self,frame:&MacDecodedVideoFrame)->bool {
        self.painted.as_ref().is_some_and(|painted|painted.decoded_at==frame.decoded_at&&Some(painted.geometry)==frame.frame_geometry)
    }
    pub fn cursor_position(&self,x:u16,y:u16,geometry:Option<FrameGeometry>)->Option<Point<Pixels>> {
        let painted=self.painted()?;let geometry=geometry?;
        if geometry.source_revision!=painted.geometry.source_revision{return None;}
        let crop=painted.view.crop_bounds(geometry)?;
        let point=[crop.x+f64::from(x)/65535.*crop.width,crop.y+f64::from(y)/65535.*crop.height];
        painted.view.source_point(point)?;
        Some(gpui::point(painted.bounds.origin.x+px(point[0] as f32),painted.bounds.origin.y+px(point[1] as f32)))
    }
    pub fn zoom(&mut self,factor:f64,anchor:Option<Point<Pixels>>)->bool {
        let Some((_,bounds,_,_))=self.layout else{return false;};
        let anchor=anchor.map(|point|[f64::from(f32::from(point.x-bounds.origin.x)),f64::from(f32::from(point.y-bounds.origin.y))])
            .unwrap_or([f64::from(f32::from(bounds.size.width))/2.,f64::from(f32::from(bounds.size.height))/2.]);
        self.view.as_mut().is_some_and(|view|view.zoom_at(factor,anchor))
    }
    pub fn reset(&mut self) {if let Some(view)=&mut self.view{view.reset();}self.cancel_gesture();}
    pub fn toggle_navigation(&mut self) {
        if self.view.is_some()&&self.target.is_some(){self.navigation=!self.navigation;self.cancel_gesture();}
    }
    pub fn begin_pan(&mut self,point:Point<Pixels>) {if self.navigation{self.pan_point=Some(point);}}
    pub fn end_pan(&mut self) {self.pan_point=None;}
    pub fn pan_to(&mut self,epoch:u64,point:Point<Pixels>)->bool {
        if !self.navigation||self.epoch!=epoch{return false;}
        let Some(previous)=self.pan_point.replace(point)else{return false;};
        self.view.as_mut().is_some_and(|view|view.pan([f64::from(f32::from(point.x-previous.x)),f64::from(f32::from(point.y-previous.y))]))
    }
    fn plan(&mut self,g:FrameGeometry,decoded_at:Instant,bounds:Bounds<Pixels>,density:f32,cover:bool)->Option<(Painted,u64)> {
        if g.validate().is_err(){return None;}
        let target=self.target.as_ref()?.clone();if target.source_revision!=g.source_revision{return None;}
        let source=[g.source_width,g.source_height];
        let size=[f64::from(f32::from(bounds.size.width)),f64::from(f32::from(bounds.size.height))];
        let mut view=self.view.or_else(||ViewTransform::new(source,size,cover))?;
        if !view.layout(source,size,cover){return None;}
        let key=(source,bounds,density,cover);
        if self.layout.as_ref().is_some_and(|old|old!=&key){self.cancel_gesture();}
        self.layout=Some(key);self.view=Some(view);
        let [width,height]=view.physical_size(f64::from(density))?;
        let wanted=Wanted{visible:view.visible(),width,height};
        if self.wanted!=Some(wanted) {
            self.wanted=Some(wanted);self.revision=self.revision.checked_add(1).expect("Viewport demand revision exhausted");
            self.pending=Some((ViewportDemand{id:target.id,source_revision:target.source_revision,revision:self.revision,
                visible:wanted.visible,width,height},self.epoch));self.ready.notify_one();
        }
        Some((Painted{decoded_at,bounds,geometry:g,view,target},self.epoch))
    }
    fn request(&self,now:Instant)->Option<(ViewportTarget,ViewportDemand,u64)> {
        if self.last_attempt.is_some_and(|time|now.saturating_duration_since(time)<remote_core::viewport::DEMAND_INTERVAL){return None;}
        let (request,epoch)=self.pending?;Some((self.target.as_ref()?.clone(),request,epoch))
    }
    fn sent(&mut self,request:ViewportDemand,epoch:u64,accepted:Option<bool>,now:Instant) {
        if self.pending.as_ref().is_none_or(|(current,current_epoch)|current.revision!=request.revision||*current_epoch!=epoch){return;}
        match accepted {
            Some(true)=>{self.last_attempt=Some(now);self.pending=None;}
            Some(false)=>{self.last_attempt=Some(now);},
            None=>{self.pending=None;self.painted=None;self.wanted=None;},
        }
    }
    pub fn retry_in(&self,now:Instant)->Option<Duration> {
        self.pending.map(|_|self.last_attempt.map_or(Duration::ZERO,|time|remote_core::viewport::DEMAND_INTERVAL.saturating_sub(now.saturating_duration_since(time))).max(Duration::from_millis(1)))
    }
    fn did_paint(&mut self,painted:Painted,epoch:u64) {
        if self.epoch==epoch {
            if let Some(connection)=painted.target.binding.scoped_connection(remote_core::shared_files::current_share_scope()) {
                connection.presented_geometry(painted.target.id,painted.target.source_revision,painted.geometry.revision);
            }
            self.painted=Some(painted);self.ready.notify_one();
        }
    }
}

fn native_frame(frame:&MacDecodedVideoFrame)->bool {
    #[cfg(target_os="macos")] {return !frame.cv_pixel_buffer.is_null();}
    #[cfg(all(target_os="linux",feature="native-linux-video"))] {return frame.native_linux.is_some();}
    #[cfg(all(target_os="windows",feature="native-windows-video"))] {return frame.native.is_some();}
    #[cfg(not(any(target_os="macos",all(target_os="linux",feature="native-linux-video"),all(target_os="windows",feature="native-windows-video"))))]
    {let _=frame;false}
}

pub(super) fn surface(frame:Arc<MacDecodedVideoFrame>,state:Arc<Mutex<State>>,mode:ViewportScaleMode)->AnyElement {
    let geometry=frame.frame_geometry.expect("Viewport surface requires frame geometry");
    let paint_state=state.clone();
    canvas(move|bounds,window,cx| {
        if !native_frame(&frame){return None;}
        let plan=state.lock().unwrap().plan(geometry,frame.decoded_at,bounds,window.scale_factor(),mode==ViewportScaleMode::Fill)?;
        let crop=plan.0.view.crop_bounds(geometry)?;
        if crop.x+crop.width<=0. || crop.y+crop.height<=0.
            ||crop.x>=f64::from(f32::from(bounds.size.width)) ||crop.y>=f64::from(f32::from(bounds.size.height)){return None;}
        let mut element=decoded_video_frame_surface_with_fit(&frame,ObjectFit::Fill);
        let origin=point(bounds.origin.x+px(crop.x as f32),bounds.origin.y+px(crop.y as f32));
        window.with_content_mask(Some(ContentMask{bounds}),|window| {
            element.prepaint_as_root(origin,size(AvailableSpace::Definite(px(crop.width as f32)),AvailableSpace::Definite(px(crop.height as f32))),window,cx);
        });
        Some((element,plan))
    },move|bounds,prepared,window,cx| {
        if let Some((mut element,(painted,epoch)))=prepared {
            window.with_content_mask(Some(ContentMask{bounds}),|window|element.paint(window,cx));
            paint_state.lock().unwrap().did_paint(painted,epoch);
        }else{paint_state.lock().unwrap().painted=None;}
    }).size_full().into_any_element()
}

impl RestoredDashboard {
    pub(super) fn poll_viewport(&mut self) {
        let target=self.owner.viewport_target();
        let (request,cancel)={
            let mut state=self.viewport.lock().unwrap();
            state.target(self.bound_source,target);
            let cancel=state.take_cancel();(state.request(Instant::now()),cancel)
        };
        if cancel{self.release_pointer_input();}
        if let Some((target,request,epoch))=request {
            let accepted=self.owner.queue_viewport(&target,request);
            self.viewport.lock().unwrap().sent(request,epoch,accepted,Instant::now());
        }
    }
    pub(super) fn queue_view_pointer(&self,event:protocol::InputEvent)->bool {
        let counter=match &event {protocol::InputEvent::MouseMoveAbsolute{..}=>Some(0),protocol::InputEvent::MouseDown(_)=>Some(1),protocol::InputEvent::MouseUp(_)=>Some(2),_=>None};
        let queued=if self.current_frame.as_ref().is_some_and(|frame|frame.frame_geometry.is_some()) {
            let target=self.viewport.lock().unwrap().painted().map(|painted|painted.target);
            target.is_some_and(|target|self.owner.queue_viewport_input(&target,event))
        }else{self.owner.queue_viewing_input(event)};
        if queued && let Some(index)=counter {let mut counts=self.pointer_dispatch_counts.get();counts[index]=counts[index].saturating_add(1);self.pointer_dispatch_counts.set(counts);}
        queued
    }
    pub(super) fn scroll_target(&self)->Option<ViewportTarget> {
        let frame=self.current_frame.as_ref()?;
        if let Some(geometry)=frame.frame_geometry {
            let painted=self.viewport.lock().unwrap().painted()?;
            if geometry.source_revision!=painted.geometry.source_revision
                || [geometry.source_width,geometry.source_height]!=[painted.geometry.source_width,painted.geometry.source_height]
                || !self.owner.viewport_target_is_current(&painted.target) {return None;}
            Some(painted.target)
        } else {
            self.owner.viewport_target_for_frame(&self.presentation_frame,frame)
        }
    }
    pub(super) fn queue_scroll_pointer(&self,target:&ViewportTarget,event:protocol::InputEvent)->bool {
        let pointer=matches!(event,protocol::InputEvent::MouseMoveAbsolute{..});
        let queued=self.owner.queue_viewport_input(target,event);
        if queued && pointer {let mut counts=self.pointer_dispatch_counts.get();counts[0]=counts[0].saturating_add(1);self.pointer_dispatch_counts.set(counts);}
        queued
    }
    pub(super) fn change_view_zoom(&mut self,factor:f64,anchor:Option<Point<Pixels>>,cx:&mut Context<Self>) {
        if self.viewport.lock().unwrap().zoom(factor,anchor){cx.notify();}
    }
    pub(super) fn viewport_controls(&self,cx:&mut Context<Self>)->Option<Div> {
        let (zoom,navigation,at_default)={let state=self.viewport.lock().unwrap();state.target.as_ref()?;state.layout?;
            let view=state.view?;(view.zoom(),state.navigation,view.is_default())};
        Some(div().flex().items_center().gap_1()
            .child(command_button("view_zoom_out",ActionVariantKind::Neutral,cx).size(px(28.)).p_0().child("−")
                .disabled(zoom<=1.).on_click(cx.listener(|this,_,_,cx|this.change_view_zoom(1./1.25,None,cx))))
            .child(div().w(px(42.)).text_size(px(11.)).text_center().child(format!("{zoom:.1}×")))
            .child(command_button("view_zoom_in",ActionVariantKind::Neutral,cx).size(px(28.)).p_0().child("+")
                .disabled(zoom>=4.).on_click(cx.listener(|this,_,_,cx|this.change_view_zoom(1.25,None,cx))))
            .child(command_button("view_navigation",if navigation{ActionVariantKind::Primary}else{ActionVariantKind::Neutral},cx).child(crate::i18n::tr("Move view"))
                .tooltip(tooltip(crate::i18n::tr("Drag to pan · wheel to zoom. Turn off to use the remote pointer.")).build())
                .on_click(cx.listener(|this,_,_,cx|{this.viewport.lock().unwrap().toggle_navigation();cx.notify();})))
            .child(command_button("view_zoom_reset",ActionVariantKind::Neutral,cx).child(crate::i18n::tr("Reset"))
                .disabled(at_default).on_click(cx.listener(|this,_,_,cx|{this.viewport.lock().unwrap().reset();cx.notify();}))))
    }
}

pub(super) struct PanDrag {pub owner:EntityId,pub epoch:u64}
pub(super) struct PanImage;
impl Render for PanImage {fn render(&mut self,_:&mut Window,_:&mut Context<Self>)->impl IntoElement {div().size(px(0.))}}

impl RestoredDashboard {
    pub(super) fn begin_view_pan(&mut self,point:Point<Pixels>) {self.viewport.lock().unwrap().begin_pan(point);}
    pub(super) fn end_view_pan(&mut self) {self.viewport.lock().unwrap().end_pan();}
    pub(super) fn move_view_pan(&mut self,event:&DragMoveEvent<PanDrag>,cx:&mut Context<Self>) {
        let drag=event.drag(cx);
        if drag.owner==cx.entity_id() && self.viewport.lock().unwrap().pan_to(drag.epoch,event.event.position){cx.notify();}
    }
    pub(super) fn view_scroll(&mut self,position:Point<Pixels>,delta:ScrollDelta,cx:&mut Context<Self>)->bool {
        if !self.viewport.lock().unwrap().navigation(){return false;}
        let delta=delta.pixel_delta(px(40.));let factor=(f64::from(f32::from(delta.y))*0.005).clamp(-0.7,0.7).exp();
        self.change_view_zoom(factor,Some(position),cx);true
    }
}

#[cfg(test)]
mod tests {
    use super::{State,ViewportTarget,FrameGeometry,SourceViewBinding};
    use super::super::PointerInputTracker;
    use gpui::{Bounds,Pixels,point,px,size};
    use std::{sync::Arc,time::{Instant,Duration}};
    use remote_core::{net::{UdpMultiplexer,MultiplexedPacket},workspace_session::WorkspaceConnection};
    struct Fixture {
        target:ViewportTarget,
        _connection:Arc<WorkspaceConnection>,
        server:tokio::task::JoinHandle<()>,
        _runtime:tokio::runtime::Runtime,
    }
    impl Drop for Fixture {fn drop(&mut self){self.server.abort();}}
    fn fixture()->Fixture {
        assert!(remote_core::session_crypto::paired_file_session_key().is_none(),"Model fixture must not use a real device group");
        remote_core::init_crypto_provider();
        let runtime=tokio::runtime::Runtime::new().unwrap();
        let (connection,server)=runtime.block_on(async {
            let mux=UdpMultiplexer::bind("127.0.0.1:0").await.unwrap();let address=mux.local_addr().unwrap();
            let (sender,receiver)=mux.split();
            let server=tokio::spawn(async move {
                while let Ok(MultiplexedPacket::Control(protocol::ControlMessage::Session(command),from))=receiver.recv().await {
                    if let protocol::session::SessionCommand::Open{connection_id,..}=*command {
                        sender.send_control(&protocol::ControlMessage::Session(Box::new(protocol::session::SessionCommand::Opened{
                            connection_id,version:protocol::session::SESSION_VERSION,max_subscriptions:8,
                            files:false,clipboard:false,window_capture:true,
                        })),from).await.unwrap();
                    }
                }
            });
            let (connection,_)=WorkspaceConnection::connect_media(address).await.unwrap();(Arc::new(connection),server)
        });
        let target=ViewportTarget{binding:SourceViewBinding::new(&connection,remote_core::shared_files::current_share_scope()),id:7,source_revision:0};
        Fixture{target,_connection:connection,server,_runtime:runtime}
    }

    #[test]
    fn scroll_gesture_is_bound_to_accepted_source_and_focus_lifetime() {
        use protocol::{InputEvent,ScrollGesture,ScrollPhase as P,MomentumPhase as M};
        let fixture=fixture();
        let target=fixture.target.clone();
        let newer=ViewportTarget{source_revision:target.source_revision+1,..target.clone()};
        let began=ScrollGesture{phase:P::Began,momentum:M::None};
        let changed=ScrollGesture{phase:P::Changed,momentum:M::None};
        let ended=ScrollGesture{phase:P::Ended,momentum:M::None};
        let momentum=ScrollGesture{phase:P::None,momentum:M::Began};
        let momentum_end=ScrollGesture{phase:P::None,momentum:M::Ended};
        let mut tracker=PointerInputTracker::default();
        assert!(tracker.accepts_scroll(began,&target));
        tracker.scrolled((0.,0.),began);
        assert!(!tracker.accepts_scroll(changed,&target),"an unqueued begin must not arm the gesture");
        tracker.scroll_queued(began,target.clone());
        assert!(tracker.accepts_scroll(changed,&target));
        assert!(!tracker.accepts_scroll(changed,&newer),"late input must not follow a source switch");
        tracker.scroll_queued(changed,target.clone());
        let (cancel_target,cancel)=tracker.cancel_scroll().unwrap();
        assert!(cancel_target.same_target(&target));
        assert_eq!(cancel,InputEvent::MouseScroll{delta_x:0,delta_y:0,gesture:ScrollGesture{phase:P::Cancelled,momentum:M::None}});
        assert!(!tracker.accepts_scroll(changed,&target),"focus cancellation disarms continuation");
        tracker.scroll_queued(began,target.clone());
        tracker.scroll_queued(ended,target.clone());
        assert!(tracker.accepts_scroll(momentum,&target),"momentum belongs to the preceding touch gesture");
        tracker.scroll_queued(momentum,target.clone());
        assert!(tracker.accepts_scroll(momentum_end,&target));
        tracker.scroll_queued(momentum_end,target.clone());
        assert!(!tracker.accepts_scroll(momentum,&target));
        assert!(tracker.cancel_scroll().is_none());
    }
    fn geometry()->FrameGeometry {FrameGeometry{source_revision:0,demand_revision:0,revision:1,source_width:4000,source_height:3000,
        crop:protocol::viewport::PixelRect{x:0,y:0,width:4000,height:3000},width:128,height:72}}
    fn bounds()->Bounds<Pixels> {Bounds::new(point(px(0.),px(0.)),size(px(800.),px(600.)))}
    #[test]
    fn latest_region_replaces_pending_work_and_busy_control_queue_has_a_bounded_retry() {
        let fixture=fixture();let now=Instant::now();let mut state=State::default();
        state.target(Some((1,now)),Some(fixture.target.clone()));state.take_cancel();
        let (painted,epoch)=state.plan(geometry(),now,bounds(),2.,false).unwrap();state.did_paint(painted,epoch);
        let (_,first,first_epoch)=state.request(now).unwrap();state.sent(first,first_epoch,Some(false),now);
        assert!(state.request(now).is_none());assert_eq!(state.retry_in(now),Some(remote_core::viewport::DEMAND_INTERVAL));
        assert!(state.zoom(4.,None));let (painted,epoch)=state.plan(geometry(),now,bounds(),2.,false).unwrap();state.did_paint(painted,epoch);
        assert!(state.request(now+Duration::from_millis(49)).is_none());
        let (_,latest,latest_epoch)=state.request(now+Duration::from_millis(50)).unwrap();
        assert!(latest.revision>first.revision);assert_eq!(latest.visible.width,250_000);
        state.sent(first,first_epoch,Some(true),now);assert!(state.request(now+Duration::from_millis(50)).is_some(),"Late older result discarded the latest demand");
        state.sent(latest,latest_epoch,Some(true),now+Duration::from_millis(50));assert_eq!(state.retry_in(now),None);
        state.plan(geometry(),now,bounds(),2.,false).unwrap();assert!(state.request(now+Duration::from_secs(1)).is_none(),"Idle view emitted duplicate demands");
    }
    #[test]
    fn source_changes_reject_late_paint_and_late_queue_results_even_when_revisions_restart() {
        let fixture=fixture();let now=Instant::now();let mut state=State::default();
        state.target(Some((1,now)),Some(fixture.target.clone()));state.take_cancel();
        let (old,old_epoch)=state.plan(geometry(),now,bounds(),2.,false).unwrap();let (_,request,_)=state.request(now).unwrap();
        state.target(Some((1,now+Duration::from_secs(1))),Some(ViewportTarget{id:8,..fixture.target.clone()}));state.take_cancel();
        state.did_paint(old,old_epoch);assert!(state.painted().is_none());
        state.plan(geometry(),now,bounds(),2.,false).unwrap();state.sent(request,old_epoch,Some(true),now);
        assert!(state.request(now).is_some(),"A previous source retired a current request");
        assert!(state.plan(FrameGeometry{source_revision:1,..geometry()},now,bounds(),2.,false).is_none());
    }
    #[test]
    fn resize_cancels_the_old_pan_and_navigation_remains_available_without_visible_roi_pixels() {
        let fixture=fixture();let now=Instant::now();let mut state=State::default();
        state.target(Some((1,now)),Some(fixture.target.clone()));state.take_cancel();
        let (painted,epoch)=state.plan(geometry(),now,bounds(),2.,false).unwrap();state.did_paint(painted,epoch);
        state.toggle_navigation();state.take_cancel();let old_epoch=state.epoch();state.begin_pan(point(px(400.),px(300.)));
        state.plan(geometry(),now,bounds(),1.5,false).unwrap();assert!(state.take_cancel());
        assert!(!state.pan_to(old_epoch,point(px(450.),px(300.))),"A geometry change kept an old drag active");
        state.painted=None;assert!(state.zoom(2.,None),"Zooming out of unreceived pixels must remain possible");
        state.toggle_navigation();assert!(!state.navigation());
    }
    #[test]
    fn decoder_recovery_keeps_zoom_and_monotonic_demand_revisions_for_the_same_subscription() {
        let fixture=fixture();let now=Instant::now();let mut state=State::default();
        state.target(Some((1,now)),Some(fixture.target.clone()));state.take_cancel();
        let (painted,epoch)=state.plan(geometry(),now,bounds(),2.,false).unwrap();state.did_paint(painted,epoch);
        state.zoom(2.,None);state.plan(geometry(),now,bounds(),2.,false).unwrap();let (_,before,_)=state.request(now).unwrap();
        state.target(Some((1,now+Duration::from_secs(1))),Some(fixture.target.clone()));
        assert!(state.take_cancel());assert_eq!(state.view.unwrap().zoom(),2.);
        state.zoom(2.,None);state.plan(geometry(),now,bounds(),2.,false).unwrap();let (_,after,_)=state.request(now).unwrap();
        assert!(after.revision>before.revision,"A decoder recovery restarted the host's viewport revision counter");
    }
}
