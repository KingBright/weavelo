//! Runtime-local native camera identities. Device paths never become wire IDs.
use protocol::session::{CameraFacing, CameraInfo, CaptureSource, CaptureSourceInfo};
use std::collections::{HashMap, HashSet};

pub const MAX_CAMERAS: usize = 32;

#[derive(Clone, Debug)]
pub struct NativeCamera {
    pub key: String,
    pub name: String,
    pub facing: CameraFacing,
    pub switch_group: Option<String>,
    pub width: u32,
    pub height: u32,
    pub rotation: u16,
}

pub struct CameraRegistry {
    next: u64,
    devices: HashMap<String, (u64, NativeCamera)>,
    groups: HashMap<String, u64>,
}

impl Default for CameraRegistry {
    fn default() -> Self {
        let bytes = crate::session_crypto::random_bytes_16();
        Self {
            // Remain representable as a positive Android/JSON signed long.
            next: (u64::from_le_bytes(bytes[..8].try_into().unwrap()) & ((1 << 62) - 1)).max(1),
            devices: HashMap::new(), groups: HashMap::new(),
        }
    }
}

impl CameraRegistry {
    fn allocate(&mut self) -> Result<u64, &'static str> {
        let id = self.next;
        if id >= i64::MAX as u64 { return Err("Camera identity space exhausted"); }
        self.next += 1;
        Ok(id)
    }
    /// Validate the complete scan before changing any live bindings.
    pub fn reconcile(&mut self, cameras: Vec<NativeCamera>) -> Result<Vec<CaptureSourceInfo>, &'static str> {
        if cameras.len() > MAX_CAMERAS { return Err("Too many camera devices"); }
        let mut keys = HashSet::new();
        for camera in &cameras {
            if camera.key.is_empty() || camera.key.len() > 4096 || camera.name.is_empty()
                || camera.name.len() > 1024 || !keys.insert(camera.key.as_str())
                || !matches!(camera.rotation, 0 | 90 | 180 | 270)
                || camera.switch_group.as_ref().is_some_and(|group|group.is_empty() || group.len()>128)
            { return Err("Invalid native camera directory"); }
        }
        let needed = cameras.iter().filter(|camera|!self.devices.contains_key(&camera.key)).count()
            + cameras.iter().filter_map(|camera|camera.switch_group.as_ref())
                .filter(|group|!self.groups.contains_key(*group)).collect::<HashSet<_>>().len();
        if self.next.checked_add(needed as u64).is_none_or(|next|next>i64::MAX as u64) {
            return Err("Camera identity space exhausted");
        }
        self.devices.retain(|key,_|keys.contains(key.as_str()));
        let mut live_groups = HashSet::new();
        let mut sources = Vec::with_capacity(cameras.len());
        for camera in cameras {
            let switch_group = if let Some(group) = &camera.switch_group {
                live_groups.insert(group.clone());
                if let Some(id) = self.groups.get(group) { *id }
                else { let id=self.allocate()?;self.groups.insert(group.clone(),id);id }
            } else { 0 };
            let id = if let Some((id,_)) = self.devices.get(&camera.key) { *id }
                else { self.allocate()? };
            sources.push(CaptureSourceInfo {
                source: CaptureSource::Camera(id), title: camera.name.clone(),
                application: String::new(), process_id: None, width: camera.width, height: camera.height,
                supports_input: false,
                camera: Some(CameraInfo { facing: camera.facing, switch_group, rotation: camera.rotation, mirrored: false }),
            });
            self.devices.insert(camera.key.clone(),(id,camera));
        }
        self.groups.retain(|group,_|live_groups.contains(group));
        Ok(sources)
    }
    pub fn resolve(&self, source: CaptureSource) -> Option<&NativeCamera> {
        let CaptureSource::Camera(wanted) = source else { return None; };
        self.devices.values().find_map(|(id,camera)|(*id==wanted).then_some(camera))
    }
    /// Revocation retires identities; a later grant must get fresh ones.
    pub fn revoke(&mut self) { self.devices.clear(); self.groups.clear(); }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn camera(key:&str,facing:CameraFacing)->NativeCamera {
        NativeCamera {key:key.into(),name:key.into(),facing,switch_group:Some("phone".into()),width:1280,height:720,rotation:90}
    }
    #[test]
    fn ordering_does_not_change_native_identity_and_lenses_share_only_their_group() {
        let mut registry=CameraRegistry::default();
        let a=registry.reconcile(vec![camera("back",CameraFacing::Back),camera("front",CameraFacing::Front)]).unwrap();
        let b=registry.reconcile(vec![camera("front",CameraFacing::Front),camera("back",CameraFacing::Back)]).unwrap();
        assert_eq!(a[0].source,b[1].source);assert_eq!(a[1].source,b[0].source);
        assert!(a[0].camera_switch_target(&a[1]));assert!(!a[0].camera_switch_target(&a[0]));
        assert_eq!(registry.resolve(a[0].source).unwrap().key,"back");
        assert!(a.iter().all(|source|source.validate_camera().is_ok()&&!source.supports_input));
    }
    #[test]
    fn unplug_replug_and_revocation_cannot_rebind_an_old_subscription() {
        let mut registry=CameraRegistry::default();
        let a=registry.reconcile(vec![camera("same",CameraFacing::Back)]).unwrap().remove(0);
        registry.reconcile(vec![]).unwrap();assert!(registry.resolve(a.source).is_none());
        let b=registry.reconcile(vec![camera("same",CameraFacing::Back)]).unwrap().remove(0);
        assert_ne!(a.source,b.source);assert!(!a.camera_switch_target(&b));
        registry.revoke();assert!(registry.resolve(b.source).is_none());
        let c=registry.reconcile(vec![camera("same",CameraFacing::Back)]).unwrap().remove(0);
        assert_ne!(b.source,c.source);
    }
    #[test]
    fn invalid_scan_is_atomic_and_cannot_replace_an_existing_camera() {
        let mut registry=CameraRegistry::default();
        let source=registry.reconcile(vec![camera("real",CameraFacing::Back)]).unwrap()[0].source;
        let duplicate=vec![camera("other",CameraFacing::Front);2];
        assert!(registry.reconcile(duplicate).is_err());
        assert_eq!(registry.resolve(source).unwrap().key,"real");
        let mut invalid=camera("real",CameraFacing::Front);invalid.rotation=45;
        assert!(registry.reconcile(vec![invalid]).is_err());
        assert_eq!(registry.resolve(source).unwrap().facing,CameraFacing::Back);
        registry.next=i64::MAX as u64;
        assert!(registry.reconcile(vec![camera("new",CameraFacing::External)]).is_err());
        assert!(registry.resolve(source).is_some());
    }
}
