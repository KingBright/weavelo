//! Source-space viewport demands and the geometry of one encoded access unit.
//!
//! Coordinates describe edges in the complete selected source, never desktop
//! coordinates. Integer millionths keep the authenticated wire format bounded
//! and avoid NaN, floating-point drift and JSON integer precision differences.
use serde::{Deserialize, Serialize};

pub const NORMALIZED_ONE: u32 = 1_000_000;
pub const FRAME_HEADER_LEN: usize = 60;
const FRAME_MAGIC: [u8; 8] = *b"WVROI\0\x01\0";

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct NormalizedRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}
impl NormalizedRect {
    pub const FULL: Self = Self { x: 0, y: 0, width: NORMALIZED_ONE, height: NORMALIZED_ONE };
    pub fn validate(self) -> Result<(), &'static str> {
        if self.width == 0 || self.height == 0
            || self.x.checked_add(self.width).is_none_or(|v| v > NORMALIZED_ONE)
            || self.y.checked_add(self.height).is_none_or(|v| v > NORMALIZED_ONE)
        { return Err("Viewport is outside the selected source"); }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ViewportDemand {
    pub id: u32,
    pub source_revision: u32,
    pub revision: u64,
    pub visible: NormalizedRect,
    /// Physical video viewport pixels, excluding letterbox and system insets.
    pub width: u32,
    pub height: u32,
}
impl ViewportDemand {
    pub fn validate(self) -> Result<(), &'static str> {
        self.visible.validate()?;
        if self.id == 0 || self.id >= 0x1000_0000 || self.revision == 0
            || !(16..=8192).contains(&self.width) || !(16..=8192).contains(&self.height)
            || u64::from(self.width) * u64::from(self.height) > 33_554_432
        { return Err("Invalid viewport demand"); }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PixelRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}
impl PixelRect {
    pub fn contains(self, other: Self) -> bool {
        other.x >= self.x && other.y >= self.y
            && u64::from(other.x) + u64::from(other.width) <= u64::from(self.x) + u64::from(self.width)
            && u64::from(other.y) + u64::from(other.height) <= u64::from(self.y) + u64::from(self.height)
    }
}

/// Attached to the access unit itself, so reordered/delayed frames cannot adopt
/// the newest requested crop. No unbounded geometry table or timing overload.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct FrameGeometry {
    pub source_revision: u32,
    pub demand_revision: u64,
    pub revision: u64,
    pub source_width: u32,
    pub source_height: u32,
    pub crop: PixelRect,
    pub width: u32,
    pub height: u32,
}
impl FrameGeometry {
    pub fn validate(self) -> Result<(), &'static str> {
        if self.revision == 0 || !(2..=32768).contains(&self.source_width)
            || !(2..=32768).contains(&self.source_height)
            || !(2..=8192).contains(&self.width) || !(2..=8192).contains(&self.height)
            || u64::from(self.width) * u64::from(self.height) > 33_554_432
            || self.crop.width < 2 || self.crop.height < 2
            || self.width > self.crop.width || self.height > self.crop.height
            || !(PixelRect { x: 0, y: 0, width: self.source_width, height: self.source_height }).contains(self.crop)
        { return Err("Invalid encoded viewport geometry"); }
        Ok(())
    }
    pub fn normalized_crop(self) -> NormalizedRect {
        let scale = |v: u32, total: u32| (u64::from(v) * u64::from(NORMALIZED_ONE) / u64::from(total)) as u32;
        NormalizedRect { x: scale(self.crop.x,self.source_width), y: scale(self.crop.y,self.source_height),
            width: scale(self.crop.width,self.source_width), height: scale(self.crop.height,self.source_height) }
    }
    /// Invert a point on this decoded image into the full selected source.
    pub fn source_point(self, x: u16, y: u16) -> (u16, u16) {
        let map = |v: u16, origin: u32, size: u32, full: u32| {
            ((u64::from(origin) * 65535 + u64::from(v) * u64::from(size) + u64::from(full)/2)
                / u64::from(full)).min(65535) as u16
        };
        (map(x,self.crop.x,self.crop.width,self.source_width),map(y,self.crop.y,self.crop.height,self.source_height))
    }
    pub fn contains_source_point(self, x: f32, y: f32) -> bool {
        x.is_finite() && y.is_finite() && x >= 0. && y >= 0. && x <= 1. && y <= 1.
            && f64::from(x)*f64::from(self.source_width) >= f64::from(self.crop.x)
            && f64::from(y)*f64::from(self.source_height) >= f64::from(self.crop.y)
            && f64::from(x)*f64::from(self.source_width) <= f64::from(self.crop.x+self.crop.width)
            && f64::from(y)*f64::from(self.source_height) <= f64::from(self.crop.y+self.crop.height)
    }
    pub fn clamp_source_point(self,point:(f64,f64))->(f64,f64) {
        let axis=|v:f64,origin:u32,size:u32,full:u32| {
            let inset=0.5_f64.min(f64::from(size)/4.);
            v.clamp((f64::from(origin)+inset)/f64::from(full),
                (f64::from(origin+size)-inset)/f64::from(full))
        };
        (axis(point.0,self.crop.x,self.crop.width,self.source_width),axis(point.1,self.crop.y,self.crop.height,self.source_height))
    }
    pub fn write_header(self, out: &mut Vec<u8>) {
        out.extend_from_slice(&FRAME_MAGIC);
        out.extend_from_slice(&self.source_revision.to_be_bytes());
        out.extend_from_slice(&self.demand_revision.to_be_bytes());
        out.extend_from_slice(&self.revision.to_be_bytes());
        for v in [self.source_width,self.source_height,self.crop.x,self.crop.y,self.crop.width,self.crop.height,self.width,self.height] {
            out.extend_from_slice(&v.to_be_bytes());
        }
    }
}

/// Camera access units keep their normal Annex-B representation. Screen/window
/// units may carry this fixed, validated header. The session version controls
/// protocol compatibility; this is not a legacy-client negotiation path.
pub fn split_frame(data: &[u8]) -> Result<(Option<FrameGeometry>, &[u8]), &'static str> {
    if !data.starts_with(&FRAME_MAGIC) { return Ok((None,data)); }
    if data.len() <= FRAME_HEADER_LEN { return Err("Truncated viewport frame"); }
    let u32_at = |n| u32::from_be_bytes(data[n..n+4].try_into().unwrap());
    let u64_at = |n| u64::from_be_bytes(data[n..n+8].try_into().unwrap());
    let g = FrameGeometry { source_revision:u32_at(8), demand_revision:u64_at(12), revision:u64_at(20),
        source_width:u32_at(28),source_height:u32_at(32),
        crop:PixelRect{x:u32_at(36),y:u32_at(40),width:u32_at(44),height:u32_at(48)},width:u32_at(52),height:u32_at(56) };
    g.validate()?;
    let payload = &data[FRAME_HEADER_LEN..];
    if !payload.starts_with(&[0,0,0,1]) && !payload.starts_with(&[0,0,1]) {
        return Err("Viewport frame has no HEVC access unit");
    }
    Ok((Some(g),payload))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn geometry() -> FrameGeometry {FrameGeometry {source_revision:3,demand_revision:u64::MAX,revision:7,
        source_width:3840,source_height:2160,crop:PixelRect{x:960,y:540,width:1920,height:1080},width:1280,height:720}}
    #[test]
    fn access_unit_keeps_its_own_geometry_and_exact_revision() {
        let g=geometry();let mut bytes=Vec::new();g.write_header(&mut bytes);assert_eq!(bytes.len(),FRAME_HEADER_LEN);
        bytes.extend_from_slice(&[0,0,0,1,0x26,1,4]);
        assert_eq!(split_frame(&bytes),Ok((Some(g),&bytes[FRAME_HEADER_LEN..])));
        assert_eq!(g.source_point(0,0),(16384,16384));
        assert_eq!(g.source_point(65535,65535),(49151,49151));
        assert!(g.contains_source_point(0.5,0.5));assert!(!g.contains_source_point(0.1,0.5));
    }
    #[test]
    fn malformed_or_overflowed_geometry_is_not_guessed() {
        let mut g=geometry();g.crop.x=u32::MAX;assert!(g.validate().is_err());
        let mut bytes=Vec::new();g.write_header(&mut bytes);bytes.extend_from_slice(&[0,0,1,0x26,1]);assert!(split_frame(&bytes).is_err());
        assert!(split_frame(&FRAME_MAGIC).is_err());
        assert!(NormalizedRect{x:u32::MAX,y:0,width:2,height:2}.validate().is_err());
        assert!(NormalizedRect{x:0,y:0,width:0,height:1}.validate().is_err());
    }
    #[test]
    fn full_frame_mapping_is_identity_and_camera_is_not_reinterpreted() {
        let mut g=geometry();g.crop=PixelRect{x:0,y:0,width:3840,height:2160};
        for v in [0,1,32767,65534,65535] {assert_eq!(g.source_point(v,v),(v,v));}
        let nal=[0,0,0,1,0x26,1];assert_eq!(split_frame(&nal),Ok((None,nal.as_slice())));
    }
}
