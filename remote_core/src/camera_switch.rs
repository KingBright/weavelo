//! Shared camera-view binding and single-camera occupancy rules. This never
//! changes a sending ticket, source identity, or another viewer's subscription.
use protocol::session::{CameraSwitchRequest, CaptureSource, CaptureSourceInfo};

pub struct CameraView {
    pub source: CaptureSource,
    pub revision: u32,
    pub delivery_request: Option<u64>,
}

pub fn validate(request: &CameraSwitchRequest, view: CameraView, catalog: &[CaptureSourceInfo]) -> Result<(), &'static str> {
    if request.request_id == 0 || request.request_id > i64::MAX as u64
        || request.from != view.source || request.source_revision != view.revision
        || request.delivery_request_id != view.delivery_request {
        return Err("The camera view changed; refresh before switching");
    }
    if request.from == request.target { return Err("Choose another camera"); }
    let info = |source| catalog.iter().find(|info| info.source == source)
        .filter(|info| info.validate_camera().is_ok()).and_then(|info| info.camera.as_ref());
    let from = info(request.from).ok_or("The current camera is no longer shared")?;
    let target = info(request.target).ok_or("The requested camera is no longer shared")?;
    if from.switch_group == 0 || from.switch_group != target.switch_group {
        return Err("These cameras do not belong to the same switching group");
    }
    Ok(())
}

/// Exclude the requesting view. A different physical camera still in use by
/// another viewer cannot be closed implicitly to satisfy this request.
pub fn exclusive_camera_busy(target: CaptureSource, others: impl IntoIterator<Item=CaptureSource>) -> bool {
    others.into_iter().any(|source| matches!(source, CaptureSource::Camera(_)) && source != target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::session::{CameraFacing, CameraInfo};
    fn catalog() -> Vec<CaptureSourceInfo> {
        [1, 2, 3].into_iter().map(|id| CaptureSourceInfo {source:CaptureSource::Camera(id),
            title:format!("Camera {id}"),application:String::new(),process_id:None,width:1280,height:720,supports_input:false,
            camera:Some(CameraInfo {facing:CameraFacing::Unspecified,switch_group:if id==3 {8}else{7},rotation:90,mirrored:false})}).collect()
    }
    fn request() -> CameraSwitchRequest {CameraSwitchRequest {id:256,request_id:17,source_revision:4,
        from:CaptureSource::Camera(1),target:CaptureSource::Camera(2),delivery_request_id:Some(9)}}
    fn view() -> CameraView {CameraView {source:CaptureSource::Camera(1),revision:4,delivery_request:Some(9)}}
    #[test]
    fn switch_requires_current_view_and_exact_delivery_intent() {
        let request=request();assert!(validate(&request,view(),&catalog()).is_ok());
        let mut stale=request.clone();stale.source_revision=3;assert!(validate(&stale,view(),&catalog()).is_err());
        stale=request.clone();stale.delivery_request_id=Some(10);assert!(validate(&stale,view(),&catalog()).is_err());
        stale=request;stale.from=CaptureSource::MainDisplay;assert!(validate(&stale,view(),&catalog()).is_err());
    }
    #[test]
    fn retired_or_unrelated_lens_never_replaces_current_camera() {
        let mut request=request();let mut sources=catalog();sources.remove(1);
        assert!(validate(&request,view(),&sources).is_err());
        request.target=CaptureSource::Camera(3);assert!(validate(&request,view(),&catalog()).is_err());
        request.target=CaptureSource::Camera(2);sources=catalog();sources[1].supports_input=true;
        assert!(validate(&request,view(),&sources).is_err());
    }
    #[test]
    fn occupancy_preserves_other_camera_viewers_but_not_unrelated_screens() {
        assert!(!exclusive_camera_busy(CaptureSource::Camera(2),[CaptureSource::MainDisplay,CaptureSource::Camera(2)]));
        assert!(exclusive_camera_busy(CaptureSource::Camera(2),[CaptureSource::Camera(1)]));
    }
}
