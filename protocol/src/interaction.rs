//! Shared bounds for application artwork and committed IME text.
use serde::{Deserialize, Deserializer, Serialize};

pub const MAX_ICON_EDGE: u32 = 64;
pub const MAX_ICON_BYTES: usize = 16 * 1024;
pub const MAX_DIRECTORY_ICON_BYTES: usize = 64 * 1024;
pub const MAX_COMMITTED_TEXT_BYTES: usize = 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct CommittedText(String);

impl CommittedText {
    pub fn new(text: String) -> Result<Self, &'static str> {
        validate_text_contents(&text)?;
        if text.len() > MAX_COMMITTED_TEXT_BYTES {
            return Err("committed text exceeds 1 KiB");
        }
        Ok(Self(text))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl<'de> Deserialize<'de> for CommittedText {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::new(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

fn validate_text_contents(text: &str) -> Result<(), &'static str> {
    if text.is_empty() {
        return Err("committed text is empty");
    }
    if text
        .chars()
        .any(|ch| ch.is_control() && !matches!(ch, '\n' | '\r' | '\t'))
    {
        return Err("committed text contains a control character");
    }
    Ok(())
}

/// Split larger IME commits without copying or dividing a UTF-8 scalar.
pub fn text_chunks(text: &str) -> Result<impl Iterator<Item = &str>, &'static str> {
    validate_text_contents(text)?;
    let mut remaining = text;
    Ok(std::iter::from_fn(move || {
        if remaining.is_empty() {
            return None;
        }
        let mut end = remaining.len().min(MAX_COMMITTED_TEXT_BYTES);
        while !remaining.is_char_boundary(end) {
            end -= 1;
        }
        let (next, rest) = remaining.split_at(end);
        remaining = rest;
        Some(next)
    }))
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApplicationIcon {
    /// Content digest; consumers verify it before sharing a render cache entry.
    pub id: [u8; 32],
    pub png: Vec<u8>,
}

const fn crc_table() -> [u32; 256] {
    let mut table = [0; 256];
    let mut index = 0;
    while index < 256 {
        let mut value = index as u32;
        let mut bit = 0;
        while bit < 8 {
            value = (value >> 1) ^ if value & 1 != 0 { 0xedb88320 } else { 0 };
            bit += 1;
        }
        table[index] = value;
        index += 1;
    }
    table
}
const CRC: [u32; 256] = crc_table();
fn crc32(bytes: &[u8]) -> u32 {
    !bytes.iter().fold(!0_u32, |crc, byte| {
        CRC[((crc ^ u32::from(*byte)) & 255) as usize] ^ (crc >> 8)
    })
}

/// Validate the bounded, normalized RGBA PNG container before any image decoder.
/// Decoders must additionally bound pixel allocation; this does not inflate IDAT.
pub fn validate_icon_container(png: &[u8]) -> Result<(u32, u32), &'static str> {
    if png.len() > MAX_ICON_BYTES || !png.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err("invalid application PNG size or signature");
    }
    let mut offset = 8_usize;
    let mut dimensions = None;
    let mut image_data = false;
    while offset < png.len() {
        let header = png.get(offset..offset + 8).ok_or("truncated PNG chunk")?;
        let size = u32::from_be_bytes(header[..4].try_into().unwrap()) as usize;
        let end = offset
            .checked_add(12)
            .and_then(|n| n.checked_add(size))
            .ok_or("PNG chunk overflow")?;
        let chunk = png.get(offset..end).ok_or("truncated PNG chunk")?;
        if crc32(&chunk[4..8 + size]) != u32::from_be_bytes(chunk[8 + size..].try_into().unwrap()) {
            return Err("application PNG checksum differs");
        }
        match &header[4..8] {
            b"IHDR" if offset == 8 && size == 13 => {
                let width = u32::from_be_bytes(chunk[8..12].try_into().unwrap());
                let height = u32::from_be_bytes(chunk[12..16].try_into().unwrap());
                if !(1..=MAX_ICON_EDGE).contains(&width)
                    || !(1..=MAX_ICON_EDGE).contains(&height)
                    || chunk[16..21] != [8, 6, 0, 0, 0]
                {
                    return Err("application PNG must be at most 64 x 64 RGBA8 pixels");
                }
                dimensions = Some((width, height));
            }
            b"IDAT" if dimensions.is_some() && size != 0 => image_data = true,
            b"IEND" if dimensions.is_some() && image_data && size == 0 && end == png.len() => {
                return Ok(dimensions.unwrap());
            }
            _ => return Err("unsupported application PNG chunk or order"),
        }
        offset = end;
    }
    Err("application PNG is incomplete")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(width: u32, extra: bool) -> Vec<u8> {
        fn chunk(out: &mut Vec<u8>, tag: &[u8; 4], data: &[u8]) {
            out.extend_from_slice(&(data.len() as u32).to_be_bytes());
            let start = out.len();
            out.extend_from_slice(tag);
            out.extend_from_slice(data);
            out.extend_from_slice(&crc32(&out[start..]).to_be_bytes());
        }
        let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut ihdr = width.to_be_bytes().to_vec();
        ihdr.extend_from_slice(&1_u32.to_be_bytes());
        ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
        chunk(&mut out, b"IHDR", &ihdr);
        if extra {
            chunk(&mut out, b"iCCP", b"untrusted compressed metadata");
        }
        // A container fixture only; real pixel decoding is checked by the codec consumer.
        chunk(&mut out, b"IDAT", &[1]);
        chunk(&mut out, b"IEND", &[]);
        out
    }
    #[test]
    fn text_uses_byte_limits_and_rejects_terminal_control_characters() {
        assert!(CommittedText::new("a".repeat(1024)).is_ok());
        assert!(CommittedText::new("中".repeat(342)).is_err());
        assert!(CommittedText::new("中😀e\u{301}\n\t".into()).is_ok());
        for text in ["", "a\0b", "\u{1b}[1m"] {
            assert!(CommittedText::new(text.into()).is_err());
        }
    }
    #[test]
    fn text_chunks_preserve_utf8_without_dropping_chinese_emoji_or_marks() {
        let original = format!("{}{}{}", "a".repeat(1023), "中😀".repeat(300), "e\u{301}");
        let chunks: Vec<_> = text_chunks(&original).unwrap().collect();
        assert_eq!(chunks.concat(), original);
        assert!(
            chunks
                .iter()
                .all(|text| !text.is_empty() && text.len() <= 1024)
        );
        assert!(
            chunks
                .iter()
                .all(|text| CommittedText::new((*text).into()).is_ok())
        );
    }
    #[test]
    fn deserialization_cannot_create_oversized_committed_text() {
        let bytes = bincode::serialize(&"a".repeat(1025)).unwrap();
        assert!(bincode::deserialize::<CommittedText>(&bytes).is_err());
        let text = CommittedText::new("中文😀".into()).unwrap();
        assert_eq!(
            bincode::deserialize::<CommittedText>(&bincode::serialize(&text).unwrap()).unwrap(),
            text
        );
    }
    #[test]
    fn icons_reject_oversize_metadata_bad_checksums_and_trailing_bytes() {
        let good = png(64, false);
        assert_eq!(validate_icon_container(&good), Ok((64, 1)));
        assert!(validate_icon_container(&png(65, false)).is_err());
        assert!(validate_icon_container(&png(64, true)).is_err());
        let mut corrupt = good.clone();
        corrupt[17] ^= 1;
        assert!(validate_icon_container(&corrupt).is_err());
        let mut trailing = good;
        trailing.push(0);
        assert!(validate_icon_container(&trailing).is_err());
        assert!(validate_icon_container(&vec![0; MAX_ICON_BYTES + 1]).is_err());
    }
}
