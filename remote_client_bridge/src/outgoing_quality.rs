use protocol::session::CaptureSource;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct InitialQuality {
    pub short_edge: u32,
    pub fps: u32,
    pub bitrate_kbps: u32,
}
impl InitialQuality {
    pub fn for_id(self, id:u64)->QualityRequest {
        QualityRequest{id,short_edge:self.short_edge,fps:self.fps,bitrate_kbps:self.bitrate_kbps}
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct QualityRequest {
    pub id: u64,
    pub short_edge: u32,
    pub fps: u32,
    pub bitrate_kbps: u32,
}

impl QualityRequest {
    pub fn validate(self) -> Result<(), String> {
        if !(self.short_edge == 0 || (240..=4320).contains(&self.short_edge))
            || !(1..=120).contains(&self.fps) || !(100..=100_000).contains(&self.bitrate_kbps) {
            return Err("Invalid outgoing screen quality".into());
        }
        Ok(())
    }
    pub fn cap(self, source: [u32; 2]) -> [u32; 2] {
        let short = source[0].min(source[1]).max(1);
        let edge = if self.short_edge == 0 { short } else { self.short_edge.min(short) };
        source.map(|n| ((u64::from(n) * u64::from(edge) / u64::from(short)) as u32).clamp(2,8192))
    }
    pub fn receiver_cap(self, source:[u32;2], receiver:[u32;2])->[u32;2] {
        let chosen=self.cap(source);
        let bounds=if source[0]<=source[1] {[receiver[0].min(receiver[1]),receiver[0].max(receiver[1])]}
            else {[receiver[0].max(receiver[1]),receiver[0].min(receiver[1])]};
        [chosen[0].min(bounds[0]),chosen[1].min(bounds[1])]
    }
}

#[derive(Clone)]
pub(crate) struct DeliveryQuality {
    pub ticket: [u8; 32],
    pub source: CaptureSource,
    pub grant: u64,
    pub settings: QualityRequest,
    pub revision: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn receiver_cap_tracks_source_orientation() {
        let q=QualityRequest{id:1,short_edge:1080,fps:60,bitrate_kbps:20_000};
        assert_eq!(q.receiver_cap([1080,1920],[1920,1080]),[1080,1920]);
        assert_eq!(q.receiver_cap([1080,1920],[1280,720]),[720,1280]);
        assert_eq!(q.receiver_cap([1920,1080],[720,1280]),[1280,720]);
    }
}
