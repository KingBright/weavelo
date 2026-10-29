//! Small immutable application assets, outside the media/frame path.
use protocol::interaction::{
    ApplicationIcon, MAX_APPLICATION_ICONS, MAX_DIRECTORY_ICON_BYTES, MAX_ICON_BYTES,
    MAX_ICON_EDGE, validate_icon_container,
};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    io::Cursor,
    sync::Arc,
};

pub type IconId = [u8; 32];
pub type IconAssets = Arc<HashMap<IconId, Arc<IconBitmap>>>;
pub type SourceIconAssets = Arc<HashMap<protocol::session::CaptureSource, Arc<IconBitmap>>>;

#[derive(Debug)]
pub struct IconBitmap {
    pub id: IconId,
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<[u8]>,
    pub png: Arc<[u8]>,
}

#[derive(Default, Clone)]
pub struct IconCache {
    assets: IconAssets,
}

#[derive(Default, Clone)]
pub struct SourceIconCache {
    icons: IconCache,
    sources: SourceIconAssets,
}
impl SourceIconCache {
    pub fn assets(&self) -> SourceIconAssets {
        self.sources.clone()
    }
    pub fn known_ids(&self) -> Vec<IconId> {
        self.icons.known_ids()
    }
    pub fn update(
        &mut self,
        advertised: impl IntoIterator<Item = protocol::session::CaptureSource>,
        bindings: &[protocol::interaction::SourceIconBinding],
        icons: &[ApplicationIcon],
    ) -> Result<(), String> {
        let mut unique = HashSet::new();
        for source in advertised {
            if unique.len() >= MAX_APPLICATION_ICONS || !unique.insert(source) {
                return Err(
                    "source directory is oversized or contains duplicate identities".into(),
                );
            }
        }
        let advertised = unique;
        if advertised.len() > MAX_APPLICATION_ICONS || bindings.len() > advertised.len() {
            return Err("invalid source icon directory size".into());
        }
        let mut seen = HashSet::new();
        for binding in bindings {
            if !advertised.contains(&binding.source) || !seen.insert(binding.source) {
                return Err(
                    "application artwork is not bound to a unique advertised source".into(),
                );
            }
        }
        let mut next = self.icons.clone();
        next.update(icons, bindings.iter().map(|b| b.icon_id))?;
        let source_assets = bindings
            .iter()
            .map(|b| (b.source, next.assets[&b.icon_id].clone()))
            .collect();
        self.icons = next;
        self.sources = Arc::new(source_assets);
        Ok(())
    }
}
impl IconCache {
    pub fn assets(&self) -> IconAssets {
        self.assets.clone()
    }
    pub fn known_ids(&self) -> Vec<IconId> {
        self.assets.keys().copied().collect()
    }

    /// Resolve a whole directory atomically. References can reuse unchanged assets.
    pub fn update(
        &mut self,
        icons: &[ApplicationIcon],
        references: impl IntoIterator<Item = IconId>,
    ) -> Result<(), String> {
        let referenced: HashSet<_> = references.into_iter().collect();
        if icons.len() > MAX_APPLICATION_ICONS || referenced.len() > MAX_APPLICATION_ICONS {
            return Err("too many application icons".into());
        }
        let mut supplied = HashMap::new();
        let mut bytes = 0_usize;
        for icon in icons {
            bytes = bytes
                .checked_add(icon.png.len())
                .ok_or("application icon budget overflow")?;
            if bytes > MAX_DIRECTORY_ICON_BYTES
                || !referenced.contains(&icon.id)
                || supplied.insert(icon.id, icon).is_some()
            {
                return Err("invalid application icon directory".into());
            }
            validate_icon_container(&icon.png)?;
            if <[u8; 32]>::from(Sha256::digest(&icon.png)) != icon.id {
                return Err("application icon digest differs".into());
            }
        }
        let mut next = HashMap::new();
        for id in referenced {
            let bitmap = match supplied.get(&id) {
                Some(icon) => match self.assets.get(&id) {
                    Some(cached) if cached.png.as_ref() == icon.png.as_slice() => cached.clone(),
                    _ => Arc::new(decode(&icon.png, id)?),
                },
                None => self
                    .assets
                    .get(&id)
                    .cloned()
                    .ok_or("application icon reference is unknown")?,
            };
            next.insert(id, bitmap);
        }
        self.assets = Arc::new(next);
        Ok(())
    }
}

fn decode(bytes: &[u8], id: IconId) -> Result<IconBitmap, String> {
    let (width, height) = validate_icon_container(bytes)?;
    let decoder =
        png::Decoder::new_with_limits(Cursor::new(bytes), png::Limits { bytes: 128 * 1024 });
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let size = reader
        .output_buffer_size()
        .ok_or("application icon pixel size overflow")?;
    let expected = width as usize * height as usize * 4;
    if size != expected {
        return Err("application icon pixel format differs".into());
    }
    let mut pixels = vec![0; expected];
    let info = reader.next_frame(&mut pixels).map_err(|e| e.to_string())?;
    if info.width != width
        || info.height != height
        || info.color_type != png::ColorType::Rgba
        || info.bit_depth != png::BitDepth::Eight
        || info.buffer_size() != expected
    {
        return Err("application icon decoded format differs".into());
    }
    reader.finish().map_err(|e| e.to_string())?;
    Ok(IconBitmap {
        id,
        width,
        height,
        rgba: pixels.into(),
        png: bytes.into(),
    })
}

/// Native adapters provide straight RGBA. Encoding, scaling and hashing are shared.
pub fn encode_rgba(width: u32, height: u32, rgba: &[u8]) -> Result<ApplicationIcon, String> {
    if width == 0
        || height == 0
        || width > 512
        || height > 512
        || rgba.len() != width as usize * height as usize * 4
    {
        return Err("invalid native application icon pixels".into());
    }
    let mut edge = MAX_ICON_EDGE;
    loop {
        let (w, h, pixels) = fit_rgba(width, height, rgba, edge);
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, w, h);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.set_compression(png::Compression::Fast);
            let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
            writer
                .write_image_data(&pixels)
                .map_err(|e| e.to_string())?;
            writer.finish().map_err(|e| e.to_string())?;
        }
        if bytes.len() <= MAX_ICON_BYTES {
            validate_icon_container(&bytes)?;
            return Ok(ApplicationIcon {
                id: Sha256::digest(&bytes).into(),
                png: bytes,
            });
        }
        // Even incompressible pixels fit after this bounded reduction.
        edge /= 2;
        if edge == 0 {
            return Err("application icon cannot fit its byte budget".into());
        }
    }
}

fn fit_rgba(width: u32, height: u32, pixels: &[u8], edge: u32) -> (u32, u32, Vec<u8>) {
    let largest = width.max(height);
    if largest <= edge {
        return (width, height, pixels.to_vec());
    }
    let w = (width * edge / largest).max(1);
    let h = (height * edge / largest).max(1);
    let mut out = Vec::with_capacity(w as usize * h as usize * 4);
    for y in 0..h {
        for x in 0..w {
            let mut sum = [0_u64; 4];
            let mut count = 0_u64;
            for sy in y * height / h..(y + 1) * height / h {
                for sx in x * width / w..(x + 1) * width / w {
                    let i = (sy as usize * width as usize + sx as usize) * 4;
                    let alpha = u64::from(pixels[i + 3]);
                    for c in 0..3 {
                        sum[c] += u64::from(pixels[i + c]) * alpha;
                    }
                    sum[3] += alpha;
                    count += 1;
                }
            }
            for channel in &sum[..3] {
                out.push(if sum[3] == 0 {
                    0
                } else {
                    (channel / sum[3]) as u8
                });
            }
            out.push((sum[3] / count) as u8);
        }
    }
    (w, h, out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_artwork_is_scoped_to_the_advertised_directory() {
        use protocol::{interaction::SourceIconBinding, session::CaptureSource};
        let source = CaptureSource::Window(7);
        let other = CaptureSource::Window(8);
        let icon = encode_rgba(1, 1, &[1, 2, 3, 255]).unwrap();
        let binding = SourceIconBinding {
            source,
            icon_id: icon.id,
        };
        let mut cache = SourceIconCache::default();
        cache
            .update([source], &[binding.clone()], &[icon.clone()])
            .unwrap();
        let first = cache.assets();
        assert!(cache.update([source, source], &[], &[]).is_err());
        assert!(
            cache
                .update((0..257).map(CaptureSource::Window), &[], &[])
                .is_err()
        );
        assert!(cache.update([other], &[binding.clone()], &[]).is_err());
        assert!(
            cache
                .update([source], &[binding.clone(), binding.clone()], &[])
                .is_err()
        );
        assert!(Arc::ptr_eq(&first, &cache.assets()));
        cache.update([source], &[binding], &[]).unwrap();
        assert!(Arc::ptr_eq(&first[&source], &cache.assets()[&source]));
        cache.update([], &[], &[]).unwrap();
        assert!(cache.assets().is_empty());
        assert!(cache.known_ids().is_empty());
    }
    #[test]
    fn rgba_icons_round_trip_and_cache_reuses_unchanged_assets() {
        let pixels = [23, 57, 121, 128].repeat(32 * 32);
        let icon = encode_rgba(32, 32, &pixels).unwrap();
        let mut cache = IconCache::default();
        cache
            .update(std::slice::from_ref(&icon), [icon.id])
            .unwrap();
        let first = cache.assets()[&icon.id].clone();
        assert_eq!(&*first.rgba, pixels.as_slice());
        cache.update(&[], [icon.id]).unwrap();
        assert!(Arc::ptr_eq(&first, &cache.assets()[&icon.id]));
        cache.update(&[icon.clone()], [icon.id]).unwrap();
        assert!(Arc::ptr_eq(&first, &cache.assets()[&icon.id]));
    }
    #[test]
    fn invalid_updates_leave_the_previous_directory_intact() {
        let icon = encode_rgba(1, 1, &[1, 2, 3, 255]).unwrap();
        let mut cache = IconCache::default();
        cache.update(&[icon.clone()], [icon.id]).unwrap();
        let before = cache.assets();
        let mut changed = icon.clone();
        changed.id[0] ^= 1;
        let changed_id = changed.id;
        assert!(cache.update(&[changed], [changed_id]).is_err());
        assert!(
            cache
                .update(&[icon.clone(), icon.clone()], [icon.id])
                .is_err()
        );
        assert!(cache.update(&[], [[0; 32]]).is_err());
        assert!(Arc::ptr_eq(&before, &cache.assets()));
    }
    #[test]
    fn incompressible_native_icons_remain_within_both_budgets() {
        let mut state = 1_u32;
        let pixels: Vec<_> = (0..64 * 64 * 4)
            .map(|_| {
                state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                (state >> 24) as u8
            })
            .collect();
        let icon = encode_rgba(64, 64, &pixels).unwrap();
        assert!(icon.png.len() <= MAX_ICON_BYTES);
        let mut cache = IconCache::default();
        cache.update(&[icon.clone()], [icon.id]).unwrap();
        assert!(cache.assets()[&icon.id].width <= 64);
    }
    #[test]
    fn downscaling_preserves_alpha_without_dark_color_fringes() {
        let (_, _, pixel) = fit_rgba(
            2,
            2,
            &[255, 0, 0, 255, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            1,
        );
        assert_eq!(pixel, [255, 0, 0, 63]);
    }
}
