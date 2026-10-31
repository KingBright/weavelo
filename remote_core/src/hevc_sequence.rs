//! Shared bounded HEVC SPS reader and explicit source-color metadata repair. This does not decode
//! pixels, guess a GUI size, or replace the decoder's authoritative output metadata.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SequenceInfo {
    pub coded: [u32; 2],
    pub visible: [u32; 4],
    pub bit_depth: u8,
    pub chroma_format: u8,
    pub signal: Option<VideoSignal>,
    pub pixel_aspect: Option<[u32; 2]>,
    pub chroma_location: Option<u8>,
}
/// Values carried by HEVC VUI, not guessed from frame dimensions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VideoSignal {
    pub full_range: bool,
    pub primaries: Option<u8>,
    pub transfer: Option<u8>,
    pub matrix: Option<u8>,
}
struct Bits<'a> {
    data: &'a [u8],
    at: usize,
    transfer_at: Option<usize>,
}
impl Bits<'_> {
    fn read(&mut self, n: usize) -> Result<u32, &'static str> {
        if n > 32
            || self
                .at
                .checked_add(n)
                .is_none_or(|end| end > self.data.len() * 8)
        {
            return Err("truncated SPS");
        }
        let mut v = 0;
        for _ in 0..n {
            v = (v << 1) | u32::from((self.data[self.at / 8] >> (7 - self.at % 8)) & 1);
            self.at += 1;
        }
        Ok(v)
    }
    fn skip(&mut self, n: usize) -> Result<(), &'static str> {
        if self
            .at
            .checked_add(n)
            .is_none_or(|end| end > self.data.len() * 8)
        {
            return Err("truncated SPS profile");
        }
        self.at += n;
        Ok(())
    }
    fn ue(&mut self) -> Result<u32, &'static str> {
        let mut zeros = 0;
        while self.read(1)? == 0 {
            zeros += 1;
            if zeros > 30 {
                return Err("unbounded SPS Exp-Golomb value");
            }
        }
        Ok(((1u32 << zeros) - 1) + self.read(zeros)?)
    }
}
fn start_code(bytes: &[u8], from: usize) -> Option<(usize, usize)> {
    let mut i = from;
    while i + 3 <= bytes.len() {
        if bytes[i..].starts_with(&[0, 0, 0, 1]) {
            return Some((i, 4));
        }
        if bytes[i..].starts_with(&[0, 0, 1]) {
            return Some((i, 3));
        }
        i += 1;
    }
    None
}
/// Parse the first SPS in one bounded Annex B access unit. No SPS is a normal None.
pub fn sequence_info(bytes: &[u8]) -> Result<Option<SequenceInfo>, &'static str> {
    Ok(sequence_details(bytes)?.map(|sps| sps.info))
}

struct ParsedSps {
    info: SequenceInfo,
    header: usize,
    end: usize,
    transfer_at: Option<usize>,
    rbsp: Vec<u8>,
}

/// The caller must have independently verified an SDR sRGB source. Repair only
/// an explicit BT.709 transfer tag; preserve range, matrix, geometry and VCL data.
pub fn set_srgb_transfer(bytes: &mut Vec<u8>) -> Result<bool, &'static str> {
    let mut cursor=0;
    let mut changed=false;
    while let Some(mut sps)=sequence_details(&bytes[cursor..])? {
        let signal=sps.info.signal.ok_or("SPS color metadata is missing")?;
        if signal.primaries!=Some(1) || signal.matrix!=Some(1) || !matches!(signal.transfer,Some(1|13)) {
            return Err("SPS does not describe the verified SDR color space");
        }
        if signal.transfer==Some(13) {cursor+=sps.end;continue;}
        let at=sps.transfer_at.ok_or("SPS transfer offset is missing")?;
        for bit in 0..8 {
            let position=at+bit;
            let mask=1<<(7-position%8);
            sps.rbsp[position/8]=(sps.rbsp[position/8]&!mask)|((((13>>(7-bit))&1)as u8)<<(7-position%8));
        }
        let mut escaped=Vec::with_capacity(sps.rbsp.len()+8);
        let mut zeros=0;
        for byte in sps.rbsp {
            if zeros>=2 && byte<=3 {escaped.push(3);zeros=0;}
            escaped.push(byte);zeros=if byte==0 {zeros+1}else{0};
        }
        let start=cursor+sps.header+2;
        let next=start+escaped.len();
        bytes.splice(start..cursor+sps.end,escaped);
        cursor=next;
        changed=true;
    }
    Ok(changed)
}

fn sequence_details(bytes: &[u8]) -> Result<Option<ParsedSps>, &'static str> {
    if bytes.len() > 16 * 1024 * 1024 {
        return Err("access unit too large");
    }
    let mut cursor = 0;
    while let Some((offset, prefix)) = start_code(bytes, cursor) {
        let header = offset + prefix;
        let end = start_code(bytes, header)
            .map(|v| v.0)
            .unwrap_or(bytes.len());
        cursor = end;
        if header + 2 > end {
            return Err("truncated HEVC NAL header");
        }
        if (bytes[header] >> 1) & 63 != 33 {
            continue;
        }
        if bytes[header] & 0x80 != 0 || bytes[header + 1] & 7 == 0 {
            return Err("invalid SPS NAL header");
        }
        if end - header > 65536 {
            return Err("SPS exceeds bounded syntax budget");
        }
        let mut rbsp = Vec::with_capacity(end - header - 2);
        let mut zeros = 0;
        for i in header + 2..end {
            let v = bytes[i];
            if zeros >= 2 && v == 3 {
                if i + 1 >= end || bytes[i + 1] > 3 {
                    return Err("invalid emulation prevention byte");
                }
                zeros = 0;
                continue;
            }
            rbsp.push(v);
            zeros = if v == 0 { zeros + 1 } else { 0 };
        }
        let mut b = Bits { data: &rbsp, at: 0, transfer_at: None };
        b.skip(4)?;
        let layers = b.read(3)? as usize;
        if layers > 6 {
            return Err("unsupported HEVC sub-layer count");
        }
        b.skip(1)?;
        b.skip(96)?;
        let mut present = [(false, false); 7];
        for p in present.iter_mut().take(layers) {
            *p = (b.read(1)? != 0, b.read(1)? != 0);
        }
        if layers > 0 {
            b.skip((8 - layers) * 2)?;
        }
        for (profile, level) in present.into_iter().take(layers) {
            if profile {
                b.skip(88)?;
            }
            if level {
                b.skip(8)?;
            }
        }
        if b.ue()? > 15 {
            return Err("invalid SPS identifier");
        }
        let chroma = b.ue()?;
        if chroma > 3 {
            return Err("invalid chroma format");
        }
        let separate = chroma == 3 && b.read(1)? != 0;
        let w = b.ue()?;
        let h = b.ue()?;
        if w < 48 || h < 48 || w > 8192 || h > 8192 {
            return Err("unsupported sequence extent");
        }
        let mut crop = [0u32; 4];
        if b.read(1)? != 0 {
            for c in &mut crop {
                *c = b.ue()?;
            }
        }
        let luma = b.ue()?;
        let chroma_depth = b.ue()?;
        if luma != chroma_depth || !matches!(luma, 0 | 2) {
            return Err("native decoder accepts equal 8 or 10 bit planes");
        }
        if chroma != 1 || separate {
            return Err("native decoder requires 4:2:0");
        }
        let x = crop[0].checked_mul(2).ok_or("crop overflow")?;
        let right = crop[1].checked_mul(2).ok_or("crop overflow")?;
        let y = crop[2].checked_mul(2).ok_or("crop overflow")?;
        let bottom = crop[3].checked_mul(2).ok_or("crop overflow")?;
        let vw = w
            .checked_sub(x.checked_add(right).ok_or("crop overflow")?)
            .filter(|v| *v > 0)
            .ok_or("invalid horizontal crop")?;
        let vh = h
            .checked_sub(y.checked_add(bottom).ok_or("crop overflow")?)
            .filter(|v| *v > 0)
            .ok_or("invalid vertical crop")?;
        let (signal, pixel_aspect, chroma_location) = read_signal_tail(&mut b, layers)?;
        let info=SequenceInfo {
            signal,
            pixel_aspect,
            chroma_location,
            coded: [w, h],
            visible: [x, y, vw, vh],
            bit_depth: (8 + luma) as u8,
            chroma_format: chroma as u8,
        };
        let transfer_at=b.transfer_at;
        return Ok(Some(ParsedSps {info,header,end,transfer_at,rbsp}));
    }
    Ok(None)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reads_existing_encoded_fixture() {
        let info = sequence_info(include_bytes!("../../client/tests/fixtures/desktop-test.h265"))
            .unwrap()
            .unwrap();
        assert_eq!(&info.visible[2..], &[128, 72]);
        assert_eq!(info.bit_depth, 8);
    }
    fn fixture(w: u32, h: u32, crop: [u32; 4], depth: u32) -> Vec<u8> {
        fixture_transfer(w,h,crop,depth,1)
    }
    fn fixture_transfer(w: u32, h: u32, crop: [u32; 4], depth: u32, transfer:u32) -> Vec<u8> {
        let mut bits = Vec::new();
        let put = |bits: &mut Vec<bool>, v: u32, n: usize| {
            for i in (0..n).rev() {
                bits.push(v & (1u32 << i) != 0);
            }
        };
        let ue = |bits: &mut Vec<bool>, v: u32| {
            let q = v + 1;
            let n = 32 - q.leading_zeros();
            bits.extend(std::iter::repeat_n(false, (n - 1) as usize));
            for i in (0..n).rev() {
                bits.push(q & (1 << i) != 0);
            }
        };
        put(&mut bits, 1, 8);
        bits.extend(std::iter::repeat_n(false, 96));
        ue(&mut bits, 0);
        ue(&mut bits, 1);
        ue(&mut bits, w);
        ue(&mut bits, h);
        put(&mut bits, 1, 1);
        for c in crop {
            ue(&mut bits, c);
        }
        ue(&mut bits, depth);
        ue(&mut bits, depth);
        ue(&mut bits, 4);
        put(&mut bits, 0, 1);
        for value in [1, 0, 0, 0, 3, 0, 3, 0, 0] {
            ue(&mut bits, value);
        }
        put(&mut bits, 0, 1);
        put(&mut bits, 1, 1);
        put(&mut bits, 1, 1);
        put(&mut bits, 0, 1);
        ue(&mut bits, 0);
        put(&mut bits, 0, 1);
        put(&mut bits, 1, 1);
        put(&mut bits, 1, 1);
        put(&mut bits, 1, 1); // VUI present
        put(&mut bits, 1, 1);
        put(&mut bits, 1, 8); // square samples
        put(&mut bits, 0, 1); // overscan absent
        put(&mut bits, 1, 1);
        put(&mut bits, 5, 3);
        put(&mut bits, 0, 1);
        put(&mut bits, 1, 1);
        for value in [1, transfer, 1] {
            put(&mut bits, value, 8);
        } // limited BT.709
        put(&mut bits, 1, 1);
        ue(&mut bits, 0);
        ue(&mut bits, 0); // left-sited progressive chroma
        bits.push(true);
        while bits.len() % 8 != 0 {
            bits.push(false);
        }
        let raw: Vec<u8> = bits
            .chunks_exact(8)
            .map(|b| b.iter().fold(0u8, |v, b| (v << 1) | u8::from(*b)))
            .collect();
        let mut out = vec![0, 0, 0, 1, 0x42, 1];
        let mut zero = 0;
        for v in raw {
            if zero >= 2 && v <= 3 {
                out.push(3);
                zero = 0;
            }
            out.push(v);
            zero = if v == 0 { zero + 1 } else { 0 };
        }
        out
    }
    #[test]
    fn coded_padding_and_visible_crop_are_not_conflated() {
        let s = sequence_info(&fixture(1920, 1088, [0, 0, 0, 4], 2))
            .unwrap()
            .unwrap();
        assert_eq!(s.coded, [1920, 1088]);
        assert_eq!(s.visible, [0, 0, 1920, 1080]);
        assert_eq!(s.bit_depth, 10);
    }
    #[test]
    fn crop_and_dimension_overflow_are_rejected() {
        for clip in [
            fixture(1920, 1080, [1000, 0, 0, 0], 0),
            fixture(8193, 1080, [0; 4], 0),
            fixture(1920, 1080, [0; 4], 4),
        ] {
            assert!(sequence_info(&clip).is_err());
        }
    }
    #[test]
    fn truncated_data_never_reads_outside_input() {
        let bytes = fixture(1920, 1088, [0, 0, 0, 4], 0);
        for end in 0..bytes.len() {
            let _ = sequence_info(&bytes[..end]);
        }
        assert!(sequence_info(&bytes).unwrap().is_some());
    }
    #[test]
    fn signalled_color_survives_independently_of_decoder_media_type() {
        let info = sequence_info(&fixture(1920, 1088, [0, 0, 0, 4], 2))
            .unwrap()
            .unwrap();
        assert_eq!(
            info.signal,
            Some(VideoSignal {
                full_range: false,
                primaries: Some(1),
                transfer: Some(1),
                matrix: Some(1)
            })
        );
        assert_eq!(info.pixel_aspect, Some([1, 1]));
        assert_eq!(info.chroma_location, Some(0));
    }
    #[test]
    fn non_parameter_units_are_not_used_as_dimensions() {
        assert_eq!(sequence_info(&[0, 0, 1, 2, 1, 0x80]).unwrap(), None);
    }
    #[test]
    fn srgb_repair_preserves_geometry_range_and_compressed_picture() {
        let mut bytes=fixture(1920,1088,[0,0,0,4],2);
        let picture=[0,0,0,1,0x26,1,0xaa,0xbb,0xff];
        bytes.extend(picture);
        let before=sequence_info(&bytes).unwrap().unwrap();
        assert!(set_srgb_transfer(&mut bytes).unwrap());
        let mut after=sequence_info(&bytes).unwrap().unwrap();
        assert_eq!(after.signal.as_ref().unwrap().transfer,Some(13));
        after.signal.as_mut().unwrap().transfer=Some(1);
        assert_eq!(before,after);
        assert!(bytes.ends_with(&picture));
        assert!(!set_srgb_transfer(&mut bytes).unwrap());
    }
    #[test]
    fn srgb_repair_handles_each_sps_and_rejects_unspecified_color() {
        let first=fixture(640,480,[0;4],0);
        let second=fixture(1920,1088,[0,0,0,4],2);
        let mut bytes=[first,second].concat();
        assert!(set_srgb_transfer(&mut bytes).unwrap());
        let first=sequence_details(&bytes).unwrap().unwrap();
        assert_eq!(first.info.signal.unwrap().transfer,Some(13));
        assert_eq!(sequence_info(&bytes[first.end..]).unwrap().unwrap().signal.unwrap().transfer,Some(13));
        let mut unknown=fixture_transfer(640,480,[0;4],0,2);
        let original=unknown.clone();
        assert!(set_srgb_transfer(&mut unknown).is_err());
        assert_eq!(unknown,original);
    }
    #[test]
    fn native_amf_sps_needs_no_encoder_extradata_to_preserve_full_range_srgb() {
        // Actual owned 578x384 WGC/AMF fixture. AMF's separate initialization
        // extradata omits VPS; the on-wire SPS is sufficient for this repair.
        let mut bytes=vec![0,0,0,1,66,1,1,1,96,0,0,3,0,176,0,0,3,0,0,3,0,156,160,5,2,1,129,193,7,203,146,66,149,146,227,255,0,1,0,1,110,2,2,2,8,0,0,3,0,8,0,0,3,0,246,32];
        assert_eq!(sequence_info(&bytes).unwrap().unwrap().visible,[0,0,578,384]);
        assert!(set_srgb_transfer(&mut bytes).unwrap());
        assert_eq!(sequence_info(&bytes).unwrap().unwrap().signal,Some(VideoSignal {
            full_range:true,primaries:Some(1),transfer:Some(13),matrix:Some(1),
        }));
    }
}

fn bounded_ue(b: &mut Bits<'_>, max: u32) -> Result<u32, &'static str> {
    let value = b.ue()?;
    if value > max {
        return Err("SPS syntax exceeds supported bound");
    }
    Ok(value)
}
fn read_signal_tail(
    b: &mut Bits<'_>,
    layers: usize,
) -> Result<(Option<VideoSignal>, Option<[u32; 2]>, Option<u8>), &'static str> {
    let poc_bits = bounded_ue(b, 12)? + 4;
    let all_layers = b.read(1)? != 0;
    for _ in if all_layers { 0 } else { layers }..=layers {
        bounded_ue(b, 16)?;
        bounded_ue(b, 16)?;
        b.ue()?;
    }
    for limit in [3, 6, 3, 6, 8, 8] {
        bounded_ue(b, limit)?;
    }
    if b.read(1)? != 0 && b.read(1)? != 0 {
        for size in 0..4 {
            for _ in (0..6).step_by(if size == 3 { 3 } else { 1 }) {
                if b.read(1)? == 0 {
                    bounded_ue(b, 6)?;
                } else {
                    if size > 1 {
                        b.ue()?;
                    }
                    for _ in 0..usize::min(64, 1usize << (4 + 2 * size)) {
                        b.ue()?;
                    }
                }
            }
        }
    }
    b.skip(2)?; // asymmetric partition and sample-adaptive offset flags
    if b.read(1)? != 0 {
        b.skip(8)?;
        bounded_ue(b, 3)?;
        bounded_ue(b, 6)?;
        b.skip(1)?;
    }
    let sets = bounded_ue(b, 64)?;
    let mut delta_counts = Vec::<u32>::with_capacity(sets as usize);
    for index in 0..sets {
        let count = if index > 0 && b.read(1)? != 0 {
            b.skip(1)?;
            bounded_ue(b, 32767)?;
            let mut retained = 0;
            for _ in 0..=delta_counts[(index - 1) as usize] {
                let used = b.read(1)? != 0;
                let keep = if used { true } else { b.read(1)? != 0 };
                if keep {
                    retained += 1;
                }
            }
            retained
        } else {
            let negative = bounded_ue(b, 16)?;
            let positive = bounded_ue(b, 16)?;
            for _ in 0..negative + positive {
                bounded_ue(b, 32767)?;
                b.skip(1)?;
            }
            negative + positive
        };
        if count > 32 {
            return Err("too many HEVC reference deltas");
        }
        delta_counts.push(count);
    }
    if b.read(1)? != 0 {
        let refs = bounded_ue(b, 32)?;
        b.skip(refs as usize * (poc_bits as usize + 1))?;
    }
    b.skip(2)?; // temporal MVP and strong intra smoothing flags
    if b.read(1)? == 0 {
        return Ok((None, None, None));
    }
    let sar = if b.read(1)? != 0 {
        let id = b.read(8)?;
        let values = match id {
            0 => None,
            1 => Some([1, 1]),
            2 => Some([12, 11]),
            3 => Some([10, 11]),
            4 => Some([16, 11]),
            5 => Some([40, 33]),
            6 => Some([24, 11]),
            7 => Some([20, 11]),
            8 => Some([32, 11]),
            9 => Some([80, 33]),
            10 => Some([18, 11]),
            11 => Some([15, 11]),
            12 => Some([64, 33]),
            13 => Some([160, 99]),
            14 => Some([4, 3]),
            15 => Some([3, 2]),
            16 => Some([2, 1]),
            255 => Some([b.read(16)?, b.read(16)?]),
            _ => return Err("reserved sample aspect ratio"),
        };
        if values.is_some_and(|r| r[0] == 0 || r[1] == 0) {
            return Err("zero sample aspect ratio");
        }
        values
    } else {
        None
    };
    if b.read(1)? != 0 {
        b.skip(1)?;
    }
    let signal = if b.read(1)? != 0 {
        b.skip(3)?;
        let full_range = b.read(1)? != 0;
        let described = b.read(1)? != 0;
        let (primaries, transfer, matrix) = if described {
            let primaries=Some(b.read(8)? as u8);
            b.transfer_at=Some(b.at);
            (primaries,Some(b.read(8)? as u8),Some(b.read(8)? as u8))
        } else {
            (None, None, None)
        };
        Some(VideoSignal {
            full_range,
            primaries,
            transfer,
            matrix,
        })
    } else {
        None
    };
    let chroma = if b.read(1)? != 0 {
        let top = bounded_ue(b, 5)? as u8;
        let bottom = bounded_ue(b, 5)? as u8;
        if top != bottom {
            return Err("interlaced chroma sites require a separate adapter");
        }
        Some(top)
    } else {
        None
    };
    Ok((signal, sar, chroma))
}
