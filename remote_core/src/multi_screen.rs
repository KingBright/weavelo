//! Multi-screen-to-multi-screen and Super-Screen topology coordinator.
//!
//! Coordinates three multi-display workflows:
//! 1. **1:1 Multi-Monitor Pairing (`OneToOneMulti`)**: deterministically pairs
//!    local monitors to remote displays (primary-to-primary first, then
//!    remaining displays by aspect ratio and spatial position order), handling
//!    mismatched monitor counts cleanly.
//! 2. **Super-Screen Virtual Canvas (`SuperScreen`)**: stitches multiple remote
//!    displays (physical and virtual) into a single normalized `[0.0, 1.0]`
//!    virtual canvas with seamless cross-display cursor mapping.
//! 3. **Wireless Secondary Display (`WirelessSecondary`)**: computes an
//!    even-aligned, HiDPI-appropriate virtual display resolution and adjacent
//!    placement next to the host's primary display.

use serde::{Deserialize, Serialize};

/// Axis-aligned display rectangle in virtual desktop pixel coordinates.
///
/// Origins (`x`, `y`) may be negative when a monitor is positioned to the left
/// of or above the primary display.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct ScreenRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl ScreenRect {
    pub const fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// Exclusive right edge in virtual desktop coordinates.
    pub const fn right(self) -> i64 {
        self.x as i64 + self.width as i64
    }

    /// Exclusive bottom edge in virtual desktop coordinates.
    pub const fn bottom(self) -> i64 {
        self.y as i64 + self.height as i64
    }

    /// Returns true if the rectangle has non-zero area.
    pub const fn is_valid(self) -> bool {
        self.width > 0 && self.height > 0
    }

    /// Width-to-height ratio, or `0.0` if height is zero.
    pub fn aspect_ratio(self) -> f64 {
        if self.height == 0 {
            0.0
        } else {
            f64::from(self.width) / f64::from(self.height)
        }
    }

    /// Center point in virtual desktop coordinates.
    pub fn center(self) -> (f64, f64) {
        (
            f64::from(self.x) + f64::from(self.width) * 0.5,
            f64::from(self.y) + f64::from(self.height) * 0.5,
        )
    }

    /// Half-open containment test: `[x, right) x [y, bottom)`.
    pub fn contains_half_open(self, px: f64, py: f64) -> bool {
        self.is_valid()
            && px >= f64::from(self.x)
            && px < self.right() as f64
            && py >= f64::from(self.y)
            && py < self.bottom() as f64
    }

    /// Inclusive containment test: `[x, right] x [y, bottom]`.
    pub fn contains_inclusive(self, px: f64, py: f64) -> bool {
        self.is_valid()
            && px >= f64::from(self.x)
            && px <= self.right() as f64
            && py >= f64::from(self.y)
            && py <= self.bottom() as f64
    }

    /// Returns true if two rectangles have overlapping area.
    pub const fn intersects(self, other: Self) -> bool {
        self.is_valid()
            && other.is_valid()
            && (self.x as i64) < other.right()
            && self.right() > (other.x as i64)
            && (self.y as i64) < other.bottom()
            && self.bottom() > (other.y as i64)
    }
}

/// Description of a physical monitor attached to the local client device.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LocalMonitorInfo {
    pub id: u32,
    pub name: String,
    pub rect: ScreenRect,
    pub refresh_hz: u32,
    pub scale: f32,
    pub primary: bool,
}

/// Description of a physical or virtual display on the remote host.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RemoteDisplayInfo {
    pub id: u32,
    pub name: String,
    pub rect: ScreenRect,
    pub refresh_hz: u32,
    pub scale: f32,
    pub is_virtual: bool,
    pub primary: bool,
}

/// Active multi-screen presentation/topology mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum MultiScreenLayoutMode {
    /// Single remote display rendered in one client window.
    #[default]
    Single,
    /// Same remote display mirrored across multiple local monitors.
    Mirror,
    /// Dedicated 1:1 pairing between local monitors and remote displays.
    OneToOneMulti,
    /// Client acts as a wireless secondary virtual display attached to the host.
    WirelessSecondary,
    /// Multiple remote displays stitched into one unified virtual canvas.
    SuperScreen,
}

/// Binding between a local client monitor and a remote host display.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ScreenBinding {
    pub local_monitor_id: u32,
    pub remote_display_id: u32,
    pub viewport_rect: ScreenRect,
}

/// Deterministic result of 1:1 multi-monitor pairing.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct OneToOnePlan {
    pub bindings: Vec<ScreenBinding>,
    pub unmatched_local_ids: Vec<u32>,
    pub unmatched_remote_ids: Vec<u32>,
}

/// Normalized `[0.0, 1.0]` rectangle inside a stitched Super-Screen canvas.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct NormalizedCanvasRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// One remote display placed inside a [`SuperScreenCanvas`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SuperScreenTile {
    pub remote_display_id: u32,
    pub is_virtual: bool,
    pub primary: bool,
    pub rect: ScreenRect,
    pub normalized_rect: NormalizedCanvasRect,
}

/// Unified virtual canvas stitching multiple remote displays (physical + virtual).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct SuperScreenCanvas {
    bounds: ScreenRect,
    tiles: Vec<SuperScreenTile>,
}

impl SuperScreenCanvas {
    /// Bounding rectangle covering all stitched remote displays in virtual
    /// desktop coordinates.
    pub const fn canvas_bounds(&self) -> ScreenRect {
        self.bounds
    }

    /// All stitched display tiles in deterministic spatial order.
    pub fn tiles(&self) -> &[SuperScreenTile] {
        &self.tiles
    }

    /// Returns true when the canvas contains no valid displays.
    pub fn is_empty(&self) -> bool {
        self.tiles.is_empty() || !self.bounds.is_valid()
    }

    /// Maps a normalized canvas cursor coordinate `(norm_x, norm_y)` in
    /// `[0.0, 1.0]` to `(remote_display_id, local_norm_x, local_norm_y)`.
    ///
    /// Returns `None` if the canvas is empty, the input is non-finite or
    /// outside `[0.0, 1.0]`, or the point falls in empty dead space between
    /// unevenly sized monitors.
    pub fn map_cursor(&self, norm_x: f64, norm_y: f64) -> Option<(u32, f64, f64)> {
        if self.is_empty()
            || !norm_x.is_finite()
            || !norm_y.is_finite()
            || !(0.0..=1.0).contains(&norm_x)
            || !(0.0..=1.0).contains(&norm_y)
        {
            return None;
        }

        let vx = f64::from(self.bounds.x) + norm_x * f64::from(self.bounds.width);
        let vy = f64::from(self.bounds.y) + norm_y * f64::from(self.bounds.height);

        // First pass: half-open containment avoids double-matching shared edges
        // between adjacent displays (e.g. x == 1920 belongs to the right tile).
        let tile = self
            .tiles
            .iter()
            .find(|tile| tile.rect.contains_half_open(vx, vy))
            // Second pass: inclusive containment handles the exact right/bottom
            // boundary of the canvas (norm_x == 1.0 or norm_y == 1.0).
            .or_else(|| {
                self.tiles
                    .iter()
                    .find(|tile| tile.rect.contains_inclusive(vx, vy))
            })?;

        let local_norm_x =
            ((vx - f64::from(tile.rect.x)) / f64::from(tile.rect.width)).clamp(0.0, 1.0);
        let local_norm_y =
            ((vy - f64::from(tile.rect.y)) / f64::from(tile.rect.height)).clamp(0.0, 1.0);

        Some((tile.remote_display_id, local_norm_x, local_norm_y))
    }

    /// Inverse mapping from a specific remote display's local normalized
    /// coordinate `(local_norm_x, local_norm_y)` back into the Super-Screen
    /// normalized canvas `[0.0, 1.0]`.
    pub fn map_display_to_canvas(
        &self,
        remote_display_id: u32,
        local_norm_x: f64,
        local_norm_y: f64,
    ) -> Option<(f64, f64)> {
        if self.is_empty()
            || !local_norm_x.is_finite()
            || !local_norm_y.is_finite()
            || !(0.0..=1.0).contains(&local_norm_x)
            || !(0.0..=1.0).contains(&local_norm_y)
        {
            return None;
        }
        let tile = self
            .tiles
            .iter()
            .find(|t| t.remote_display_id == remote_display_id)?;

        let canvas_x = (tile.normalized_rect.x + local_norm_x * tile.normalized_rect.width)
            .clamp(0.0, 1.0);
        let canvas_y = (tile.normalized_rect.y + local_norm_y * tile.normalized_rect.height)
            .clamp(0.0, 1.0);
        Some((canvas_x, canvas_y))
    }
}

/// Target geometry and HiDPI settings for a wireless secondary virtual display.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WirelessSecondaryGeometry {
    /// Even-aligned pixel resolution and placement adjacent to the host's
    /// primary display.
    pub rect: ScreenRect,
    /// Logical width in device-independent pixels (`rect.width / scale`).
    pub logical_width: u32,
    /// Logical height in device-independent pixels (`rect.height / scale`).
    pub logical_height: u32,
    /// Quantized HiDPI scale factor (e.g. `1.0`, `1.25`, `1.5`, `1.75`, `2.0`).
    pub scale: f32,
    /// Clamped target refresh rate in Hz.
    pub refresh_hz: u32,
}

const STANDARD_DPI: f32 = 96.0;
const MIN_VIRTUAL_EDGE: u32 = 320;
const MAX_VIRTUAL_LONG_EDGE: u32 = 3840;
const MAX_VIRTUAL_SHORT_EDGE: u32 = 2160;
const MIN_LOGICAL_WIDTH: f32 = 640.0;
const MIN_LOGICAL_HEIGHT: f32 = 480.0;
const DEFAULT_REFRESH_HZ: u32 = 60;
const MIN_REFRESH_HZ: u32 = 30;
const MAX_REFRESH_HZ: u32 = 240;
const DEFAULT_HOST_PRIMARY: ScreenRect = ScreenRect::new(0, 0, 1920, 1080);

/// Coordinates multi-screen topologies across local monitors and remote displays.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MultiScreenCoordinator {
    layout_mode: MultiScreenLayoutMode,
    local_monitors: Vec<LocalMonitorInfo>,
    remote_displays: Vec<RemoteDisplayInfo>,
}

impl MultiScreenCoordinator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_topology(
        local_monitors: Vec<LocalMonitorInfo>,
        remote_displays: Vec<RemoteDisplayInfo>,
    ) -> Self {
        Self {
            layout_mode: MultiScreenLayoutMode::Single,
            local_monitors,
            remote_displays,
        }
    }

    pub fn layout_mode(&self) -> MultiScreenLayoutMode {
        self.layout_mode
    }

    pub fn set_layout_mode(&mut self, mode: MultiScreenLayoutMode) {
        self.layout_mode = mode;
    }

    pub fn local_monitors(&self) -> &[LocalMonitorInfo] {
        &self.local_monitors
    }

    pub fn set_local_monitors(&mut self, local_monitors: Vec<LocalMonitorInfo>) {
        self.local_monitors = local_monitors;
    }

    pub fn remote_displays(&self) -> &[RemoteDisplayInfo] {
        &self.remote_displays
    }

    pub fn set_remote_displays(&mut self, remote_displays: Vec<RemoteDisplayInfo>) {
        self.remote_displays = remote_displays;
    }

    /// Plans a deterministic 1:1 pairing between local monitors and remote
    /// displays:
    /// - Pairs primary-to-primary first (falling back to top-leftmost display
    ///   when no primary flag is set).
    /// - Pairs remaining monitors by combined aspect-ratio similarity and
    ///   spatial left-to-right / top-to-bottom order.
    /// - Handles mismatched counts deterministically, reporting leftover IDs.
    pub fn plan_one_to_one(
        local_monitors: &[LocalMonitorInfo],
        remote_displays: &[RemoteDisplayInfo],
    ) -> OneToOnePlan {
        let mut valid_local: Vec<&LocalMonitorInfo> = Vec::new();
        let mut unmatched_local_ids: Vec<u32> = Vec::new();
        for m in local_monitors {
            if m.rect.is_valid() {
                valid_local.push(m);
            } else {
                unmatched_local_ids.push(m.id);
            }
        }

        let mut valid_remote: Vec<&RemoteDisplayInfo> = Vec::new();
        let mut unmatched_remote_ids: Vec<u32> = Vec::new();
        for d in remote_displays {
            if d.rect.is_valid() {
                valid_remote.push(d);
            } else {
                unmatched_remote_ids.push(d.id);
            }
        }

        if valid_local.is_empty() || valid_remote.is_empty() {
            valid_local.sort_by_key(|m| (m.rect.x, m.rect.y, m.id));
            valid_remote.sort_by_key(|d| (d.rect.x, d.rect.y, d.id));
            unmatched_local_ids.extend(valid_local.iter().map(|m| m.id));
            unmatched_remote_ids.extend(valid_remote.iter().map(|d| d.id));
            return OneToOnePlan {
                bindings: Vec::new(),
                unmatched_local_ids,
                unmatched_remote_ids,
            };
        }

        let primary_local_idx = select_primary_index(
            &valid_local,
            |m| m.primary,
            |m| (m.rect.x, m.rect.y, m.id),
        );
        let primary_remote_idx = select_primary_index(
            &valid_remote,
            |d| d.primary,
            |d| (d.rect.x, d.rect.y, d.id),
        );

        let primary_local = valid_local.remove(primary_local_idx);
        let primary_remote = valid_remote.remove(primary_remote_idx);

        let mut bindings = vec![ScreenBinding {
            local_monitor_id: primary_local.id,
            remote_display_id: primary_remote.id,
            viewport_rect: primary_local.rect,
        }];

        // Sort remaining displays in deterministic spatial order: left-to-right,
        // top-to-bottom, then ID.
        valid_local.sort_by_key(|m| (m.rect.x, m.rect.y, m.id));
        valid_remote.sort_by_key(|d| (d.rect.x, d.rect.y, d.id));

        let pair_count = valid_local.len().min(valid_remote.len());
        if pair_count > 0 {
            let matched_pairs = match_remaining_monitors(&valid_local, &valid_remote);
            let mut used_local = vec![false; valid_local.len()];
            let mut used_remote = vec![false; valid_remote.len()];

            let mut remaining_bindings: Vec<(usize, ScreenBinding)> =
                Vec::with_capacity(matched_pairs.len());
            for (li, ri) in matched_pairs {
                used_local[li] = true;
                used_remote[ri] = true;
                let local = valid_local[li];
                let remote = valid_remote[ri];
                remaining_bindings.push((
                    li,
                    ScreenBinding {
                        local_monitor_id: local.id,
                        remote_display_id: remote.id,
                        viewport_rect: local.rect,
                    },
                ));
            }
            // Emit remaining bindings in local spatial order.
            remaining_bindings.sort_by_key(|(li, _)| *li);
            bindings.extend(remaining_bindings.into_iter().map(|(_, b)| b));

            for (idx, m) in valid_local.iter().enumerate() {
                if !used_local[idx] {
                    unmatched_local_ids.push(m.id);
                }
            }
            for (idx, d) in valid_remote.iter().enumerate() {
                if !used_remote[idx] {
                    unmatched_remote_ids.push(d.id);
                }
            }
        } else {
            unmatched_local_ids.extend(valid_local.iter().map(|m| m.id));
            unmatched_remote_ids.extend(valid_remote.iter().map(|d| d.id));
        }

        OneToOnePlan {
            bindings,
            unmatched_local_ids,
            unmatched_remote_ids,
        }
    }

    /// Convenience wrapper using the coordinator's stored topology.
    pub fn plan_current_one_to_one(&self) -> OneToOnePlan {
        Self::plan_one_to_one(&self.local_monitors, &self.remote_displays)
    }

    /// Stitches multiple remote displays (physical and virtual) into a single
    /// normalized Super-Screen virtual canvas.
    pub fn plan_super_screen(remote_displays: &[RemoteDisplayInfo]) -> SuperScreenCanvas {
        let mut valid: Vec<&RemoteDisplayInfo> = remote_displays
            .iter()
            .filter(|d| d.rect.is_valid())
            .collect();
        if valid.is_empty() {
            return SuperScreenCanvas::default();
        }

        // Deterministic ordering: spatial left-to-right, top-to-bottom, primary
        // first on identical origin, then display ID.
        valid.sort_by_key(|d| (d.rect.x, d.rect.y, !d.primary, d.id));

        let min_x = valid.iter().map(|d| d.rect.x as i64).min().unwrap_or(0);
        let min_y = valid.iter().map(|d| d.rect.y as i64).min().unwrap_or(0);
        let max_x = valid.iter().map(|d| d.rect.right()).max().unwrap_or(0);
        let max_y = valid.iter().map(|d| d.rect.bottom()).max().unwrap_or(0);

        let canvas_width = (max_x - min_x).max(0) as u32;
        let canvas_height = (max_y - min_y).max(0) as u32;
        let bounds = ScreenRect::new(min_x as i32, min_y as i32, canvas_width, canvas_height);

        let tiles = valid
            .into_iter()
            .map(|d| {
                let normalized_rect = NormalizedCanvasRect {
                    x: (d.rect.x as i64 - min_x) as f64 / f64::from(canvas_width),
                    y: (d.rect.y as i64 - min_y) as f64 / f64::from(canvas_height),
                    width: f64::from(d.rect.width) / f64::from(canvas_width),
                    height: f64::from(d.rect.height) / f64::from(canvas_height),
                };
                SuperScreenTile {
                    remote_display_id: d.id,
                    is_virtual: d.is_virtual,
                    primary: d.primary,
                    rect: d.rect,
                    normalized_rect,
                }
            })
            .collect();

        SuperScreenCanvas { bounds, tiles }
    }

    /// Convenience wrapper using the coordinator's stored remote displays.
    pub fn plan_current_super_screen(&self) -> SuperScreenCanvas {
        Self::plan_super_screen(&self.remote_displays)
    }

    /// Computes an even-aligned, HiDPI-appropriate target resolution and
    /// placement for a wireless secondary virtual display attached next to the
    /// host's primary display.
    pub fn compute_wireless_secondary_geometry(
        &self,
        client_width: u32,
        client_height: u32,
        client_dpi: f32,
        client_refresh_hz: u32,
    ) -> WirelessSecondaryGeometry {
        Self::compute_wireless_secondary_geometry_for_host(
            &self.remote_displays,
            client_width,
            client_height,
            client_dpi,
            client_refresh_hz,
        )
    }

    /// Pure calculation of wireless secondary display geometry relative to an
    /// explicit list of existing host displays.
    pub fn compute_wireless_secondary_geometry_for_host(
        host_displays: &[RemoteDisplayInfo],
        client_width: u32,
        client_height: u32,
        client_dpi: f32,
        client_refresh_hz: u32,
    ) -> WirelessSecondaryGeometry {
        let valid_hosts: Vec<&RemoteDisplayInfo> =
            host_displays.iter().filter(|d| d.rect.is_valid()).collect();
        let host_primary = if valid_hosts.is_empty() {
            DEFAULT_HOST_PRIMARY
        } else {
            let idx = select_primary_index(
                &valid_hosts,
                |d| d.primary,
                |d| (d.rect.x, d.rect.y, d.id),
            );
            valid_hosts[idx].rect
        };

        let raw_w = client_width.max(MIN_VIRTUAL_EDGE);
        let raw_h = client_height.max(MIN_VIRTUAL_EDGE);

        // Cap dimensions to encoder-friendly maximums while preserving aspect
        // ratio and orientation (landscape vs portrait).
        let (max_w, max_h) = if raw_w >= raw_h {
            (MAX_VIRTUAL_LONG_EDGE, MAX_VIRTUAL_SHORT_EDGE)
        } else {
            (MAX_VIRTUAL_SHORT_EDGE, MAX_VIRTUAL_LONG_EDGE)
        };
        let downscale = (f64::from(max_w) / f64::from(raw_w))
            .min(f64::from(max_h) / f64::from(raw_h))
            .min(1.0);

        let target_width = align_even((f64::from(raw_w) * downscale).round() as u32)
            .max(MIN_VIRTUAL_EDGE);
        let target_height = align_even((f64::from(raw_h) * downscale).round() as u32)
            .max(MIN_VIRTUAL_EDGE);

        // Compute HiDPI scale from client DPI, adjusted if we had to downscale
        // the virtual surface so UI elements stay comfortable in physical size.
        let effective_dpi = if client_dpi.is_finite() && client_dpi > 0.0 {
            client_dpi * (downscale as f32)
        } else {
            STANDARD_DPI
        };
        let raw_scale = (effective_dpi / STANDARD_DPI).clamp(1.0, 3.0);

        // Ensure logical workspace remains usable (not shrunk below minimum
        // comfortable dimensions when the physical resolution allows it).
        let max_usable_scale = (target_width as f32 / MIN_LOGICAL_WIDTH)
            .min(target_height as f32 / MIN_LOGICAL_HEIGHT)
            .max(1.0);
        let scale = quantize_hidpi_scale(raw_scale.min(max_usable_scale));

        let logical_width = ((target_width as f32) / scale).round().max(1.0) as u32;
        let logical_height = ((target_height as f32) / scale).round().max(1.0) as u32;

        // Place directly to the right of the host's primary display, top-aligned.
        // If another existing host display already occupies that band, shift
        // further right until collision-free.
        let mut place_x = host_primary.right();
        let place_y = host_primary.y;
        loop {
            let candidate = ScreenRect::new(
                place_x.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
                place_y,
                target_width,
                target_height,
            );
            let colliding_right = valid_hosts
                .iter()
                .filter(|d| d.rect.intersects(candidate))
                .map(|d| d.rect.right())
                .max();
            match colliding_right {
                Some(next_x) if next_x > place_x => place_x = next_x,
                _ => break,
            }
        }

        let refresh_hz = if client_refresh_hz == 0 {
            DEFAULT_REFRESH_HZ
        } else {
            client_refresh_hz.clamp(MIN_REFRESH_HZ, MAX_REFRESH_HZ)
        };

        WirelessSecondaryGeometry {
            rect: ScreenRect::new(
                place_x.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
                place_y,
                target_width,
                target_height,
            ),
            logical_width,
            logical_height,
            scale,
            refresh_hz,
        }
    }
}

/// Aligns a pixel dimension to an even integer (`>= 2`) for NV12 / H.264 / HEVC.
#[inline]
const fn align_even(value: u32) -> u32 {
    let even = value & !1;
    if even < 2 { 2 } else { even }
}

/// Quantizes a raw HiDPI scale factor to standard OS scale steps.
fn quantize_hidpi_scale(raw: f32) -> f32 {
    const STEPS: [f32; 7] = [1.0, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0];
    let clamped = raw.clamp(STEPS[0], STEPS[STEPS.len() - 1]);
    let mut best = STEPS[0];
    let mut best_dist = (clamped - best).abs();
    for &step in &STEPS[1..] {
        let dist = (clamped - step).abs();
        if dist < best_dist {
            best = step;
            best_dist = dist;
        }
    }
    best
}

fn select_primary_index<T, F, K, OrdKey>(items: &[T], is_primary: F, tie_key: K) -> usize
where
    F: Fn(&T) -> bool,
    K: Fn(&T) -> OrdKey,
    OrdKey: Ord,
{
    items
        .iter()
        .enumerate()
        .min_by_key(|(_, item)| (!is_primary(item), tie_key(item)))
        .map(|(idx, _)| idx)
        .unwrap_or(0)
}

/// Pairwise matching cost between remaining local monitor `l_idx` and remote
/// display `r_idx`, balancing aspect-ratio similarity with normalized spatial
/// position order.
fn pair_cost(
    local: &[&LocalMonitorInfo],
    remote: &[&RemoteDisplayInfo],
    l_idx: usize,
    r_idx: usize,
) -> f64 {
    let ar_l = local[l_idx].rect.aspect_ratio().max(1e-6);
    let ar_r = remote[r_idx].rect.aspect_ratio().max(1e-6);
    let aspect_diff = (ar_l / ar_r).ln().abs();

    let rank_l = if local.len() <= 1 {
        0.0
    } else {
        l_idx as f64 / (local.len() - 1) as f64
    };
    let rank_r = if remote.len() <= 1 {
        0.0
    } else {
        r_idx as f64 / (remote.len() - 1) as f64
    };
    let pos_diff = (rank_l - rank_r).abs();

    aspect_diff + 0.35 * pos_diff
}

/// Deterministically matches `min(local.len(), remote.len())` remaining
/// monitors. Uses exact minimum-cost bipartite matching for up to 10 displays
/// per side and deterministic greedy matching beyond that.
fn match_remaining_monitors(
    local: &[&LocalMonitorInfo],
    remote: &[&RemoteDisplayInfo],
) -> Vec<(usize, usize)> {
    let m = local.len();
    let n = remote.len();
    let k = m.min(n);
    if k == 0 {
        return Vec::new();
    }

    if m <= 10 && n <= 10 {
        // Assign the smaller side into the larger side using bitmask DP over
        // the larger side so every element of the smaller side is paired.
        let swap = m > n;
        let (rows, cols) = if swap { (n, m) } else { (m, n) };
        let cost = |r: usize, c: usize| -> f64 {
            if swap {
                pair_cost(local, remote, c, r)
            } else {
                pair_cost(local, remote, r, c)
            }
        };

        let states = 1usize << cols;
        let mut dp = vec![f64::INFINITY; states];
        let mut choice = vec![usize::MAX; states];
        dp[0] = 0.0;

        for mask in 0..states {
            let row = mask.count_ones() as usize;
            if row >= rows || !dp[mask].is_finite() {
                continue;
            }
            for col in 0..cols {
                if (mask & (1 << col)) != 0 {
                    continue;
                }
                let next_mask = mask | (1 << col);
                let cand = dp[mask] + cost(row, col);
                // Strictly less (with small epsilon) preserves deterministic
                // lower-index spatial order on ties.
                if cand + 1e-12 < dp[next_mask] {
                    dp[next_mask] = cand;
                    choice[next_mask] = col;
                }
            }
        }

        // Find the best final mask with popcount == rows.
        let mut best_mask = 0usize;
        let mut best_cost = f64::INFINITY;
        for (mask, &val) in dp.iter().enumerate() {
            if mask.count_ones() as usize == rows && val + 1e-12 < best_cost {
                best_cost = val;
                best_mask = mask;
            }
        }

        let mut pairs = Vec::with_capacity(rows);
        let mut cur = best_mask;
        for row in (0..rows).rev() {
            let col = choice[cur];
            debug_assert!(col < cols);
            cur ^= 1 << col;
            if swap {
                pairs.push((col, row));
            } else {
                pairs.push((row, col));
            }
        }
        pairs.reverse();
        return pairs;
    }

    // Deterministic greedy fallback for > 10 displays.
    let mut candidates: Vec<(f64, usize, usize)> = Vec::with_capacity(m * n);
    for li in 0..m {
        for ri in 0..n {
            candidates.push((pair_cost(local, remote, li, ri), li, ri));
        }
    }
    candidates.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));

    let mut used_l = vec![false; m];
    let mut used_r = vec![false; n];
    let mut pairs = Vec::with_capacity(k);
    for (_, li, ri) in candidates {
        if !used_l[li] && !used_r[ri] {
            used_l[li] = true;
            used_r[ri] = true;
            pairs.push((li, ri));
            if pairs.len() == k {
                break;
            }
        }
    }
    pairs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local(id: u32, x: i32, y: i32, w: u32, h: u32, primary: bool) -> LocalMonitorInfo {
        LocalMonitorInfo {
            id,
            name: format!("Local-{id}"),
            rect: ScreenRect::new(x, y, w, h),
            refresh_hz: 60,
            scale: 1.0,
            primary,
        }
    }

    fn remote(
        id: u32,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        primary: bool,
        is_virtual: bool,
    ) -> RemoteDisplayInfo {
        RemoteDisplayInfo {
            id,
            name: format!("Remote-{id}"),
            rect: ScreenRect::new(x, y, w, h),
            refresh_hz: 60,
            scale: 1.0,
            is_virtual,
            primary,
        }
    }

    #[test]
    fn one_to_one_equal_counts_matches_primary_and_spatial_order() {
        let locals = vec![
            local(10, -1920, 0, 1920, 1080, false),
            local(20, 0, 0, 2560, 1440, true),
            local(30, 2560, 0, 1920, 1080, false),
        ];
        let remotes = vec![
            remote(3, 1920, 0, 1920, 1080, false, false),
            remote(1, 0, 0, 2560, 1440, true, false),
            remote(2, -1920, 0, 1920, 1080, false, true),
        ];

        let plan = MultiScreenCoordinator::plan_one_to_one(&locals, &remotes);
        assert!(plan.unmatched_local_ids.is_empty());
        assert!(plan.unmatched_remote_ids.is_empty());
        assert_eq!(
            plan.bindings,
            vec![
                ScreenBinding {
                    local_monitor_id: 20,
                    remote_display_id: 1,
                    viewport_rect: ScreenRect::new(0, 0, 2560, 1440),
                },
                ScreenBinding {
                    local_monitor_id: 10,
                    remote_display_id: 2,
                    viewport_rect: ScreenRect::new(-1920, 0, 1920, 1080),
                },
                ScreenBinding {
                    local_monitor_id: 30,
                    remote_display_id: 3,
                    viewport_rect: ScreenRect::new(2560, 0, 1920, 1080),
                },
            ]
        );
    }

    #[test]
    fn one_to_one_prefers_orientation_aspect_match_over_inverted_order() {
        // Local has primary + portrait on left + landscape on right.
        // Remote has primary + landscape on left + portrait on right.
        let locals = vec![
            local(1, 0, 0, 2560, 1440, true),
            local(2, -1080, 0, 1080, 1920, false),
            local(3, 2560, 0, 1920, 1080, false),
        ];
        let remotes = vec![
            remote(100, 0, 0, 2560, 1440, true, false),
            remote(200, -1920, 0, 1920, 1080, false, false),
            remote(300, 2560, 0, 1080, 1920, false, false),
        ];

        let plan = MultiScreenCoordinator::plan_one_to_one(&locals, &remotes);
        assert_eq!(plan.bindings.len(), 3);
        // Primary -> Primary
        assert_eq!(plan.bindings[0].local_monitor_id, 1);
        assert_eq!(plan.bindings[0].remote_display_id, 100);
        // Portrait local (2) -> Portrait remote (300)
        assert_eq!(plan.bindings[1].local_monitor_id, 2);
        assert_eq!(plan.bindings[1].remote_display_id, 300);
        // Landscape local (3) -> Landscape remote (200)
        assert_eq!(plan.bindings[2].local_monitor_id, 3);
        assert_eq!(plan.bindings[2].remote_display_id, 200);
    }

    #[test]
    fn one_to_one_handles_more_local_than_remote_deterministically() {
        let locals = vec![
            local(1, 0, 0, 1920, 1080, true),
            local(2, 1920, 0, 1920, 1080, false),
            local(3, 3840, 0, 1080, 1920, false),
        ];
        let remotes = vec![
            remote(10, 0, 0, 1920, 1080, true, false),
            remote(20, 1920, 0, 1920, 1080, false, false),
        ];

        let plan = MultiScreenCoordinator::plan_one_to_one(&locals, &remotes);
        assert_eq!(plan.bindings.len(), 2);
        assert_eq!(plan.bindings[0].local_monitor_id, 1);
        assert_eq!(plan.bindings[0].remote_display_id, 10);
        assert_eq!(plan.bindings[1].local_monitor_id, 2);
        assert_eq!(plan.bindings[1].remote_display_id, 20);
        assert_eq!(plan.unmatched_local_ids, vec![3]);
        assert!(plan.unmatched_remote_ids.is_empty());

        // Reversed input order produces the exact same plan.
        let mut locals_rev = locals.clone();
        let mut remotes_rev = remotes.clone();
        locals_rev.reverse();
        remotes_rev.reverse();
        assert_eq!(
            MultiScreenCoordinator::plan_one_to_one(&locals_rev, &remotes_rev),
            plan
        );
    }

    #[test]
    fn one_to_one_handles_more_remote_than_local_and_empty_inputs() {
        let locals = vec![local(1, 0, 0, 2560, 1440, true)];
        let remotes = vec![
            remote(10, 0, 0, 2560, 1440, true, false),
            remote(20, 2560, 0, 1920, 1080, false, false),
            remote(30, 4480, 0, 1920, 1080, false, true),
        ];

        let plan = MultiScreenCoordinator::plan_one_to_one(&locals, &remotes);
        assert_eq!(plan.bindings.len(), 1);
        assert_eq!(plan.bindings[0].local_monitor_id, 1);
        assert_eq!(plan.bindings[0].remote_display_id, 10);
        assert!(plan.unmatched_local_ids.is_empty());
        assert_eq!(plan.unmatched_remote_ids, vec![20, 30]);

        let empty_plan = MultiScreenCoordinator::plan_one_to_one(&[], &remotes);
        assert!(empty_plan.bindings.is_empty());
        assert_eq!(empty_plan.unmatched_remote_ids, vec![10, 20, 30]);
    }

    #[test]
    fn super_screen_stitches_physical_and_virtual_displays_and_maps_cursor() {
        // Left physical display at x = -1920, center primary at x = 0, right
        // virtual display at x = 1920. Total width = 5760, height = 1080.
        let displays = vec![
            remote(2, 0, 0, 1920, 1080, true, false),
            remote(3, 1920, 0, 1920, 1080, false, true),
            remote(1, -1920, 0, 1920, 1080, false, false),
        ];

        let canvas = MultiScreenCoordinator::plan_super_screen(&displays);
        assert_eq!(canvas.canvas_bounds(), ScreenRect::new(-1920, 0, 5760, 1080));
        assert_eq!(canvas.tiles().len(), 3);
        assert_eq!(
            canvas.tiles().iter().map(|t| t.remote_display_id).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        assert!(canvas.tiles()[2].is_virtual);

        // Center of left display (1/6 of total width)
        let (id, lx, ly) = canvas.map_cursor(1.0 / 6.0, 0.5).unwrap();
        assert_eq!(id, 1);
        assert!((lx - 0.5).abs() < 1e-6);
        assert!((ly - 0.5).abs() < 1e-6);

        // Exact seam between display 1 and display 2 transitions to display 2 at local x = 0.0
        let (id, lx, ly) = canvas.map_cursor(1.0 / 3.0, 0.25).unwrap();
        assert_eq!(id, 2);
        assert!(lx.abs() < 1e-6);
        assert!((ly - 0.25).abs() < 1e-6);

        // Center of right virtual display (5/6 of total width)
        let (id, lx, ly) = canvas.map_cursor(5.0 / 6.0, 0.75).unwrap();
        assert_eq!(id, 3);
        assert!((lx - 0.5).abs() < 1e-6);
        assert!((ly - 0.75).abs() < 1e-6);

        // Exact far-right/bottom corner maps to display 3 at (1.0, 1.0)
        let (id, lx, ly) = canvas.map_cursor(1.0, 1.0).unwrap();
        assert_eq!(id, 3);
        assert!((lx - 1.0).abs() < 1e-6);
        assert!((ly - 1.0).abs() < 1e-6);

        // Round-trip inverse mapping
        let (cx, cy) = canvas.map_display_to_canvas(3, 0.5, 0.75).unwrap();
        assert!((cx - 5.0 / 6.0).abs() < 1e-6);
        assert!((cy - 0.75).abs() < 1e-6);

        // Out-of-bounds and NaN return None
        assert_eq!(canvas.map_cursor(-0.01, 0.5), None);
        assert_eq!(canvas.map_cursor(1.01, 0.5), None);
        assert_eq!(canvas.map_cursor(f64::NAN, 0.5), None);
    }

    #[test]
    fn super_screen_returns_none_in_dead_space_for_uneven_heights() {
        // Left display: 1920x1080 at (0, 0)
        // Right display: 1920x1440 at (1920, 0)
        // Canvas bounds: (0, 0, 3840, 1440) -> bottom-left [0..1920) x [1080..1440) is dead space.
        let displays = vec![
            remote(1, 0, 0, 1920, 1080, true, false),
            remote(2, 1920, 0, 1920, 1440, false, false),
        ];
        let canvas = MultiScreenCoordinator::plan_super_screen(&displays);
        assert_eq!(canvas.canvas_bounds(), ScreenRect::new(0, 0, 3840, 1440));

        // Inside left display
        let (id, _, _) = canvas.map_cursor(0.25, 0.5).unwrap();
        assert_eq!(id, 1);

        // In dead space below left display (y = 0.9 * 1440 = 1296 > 1080)
        assert_eq!(canvas.map_cursor(0.25, 0.9), None);

        // Inside bottom portion of right display
        let (id, lx, ly) = canvas.map_cursor(0.75, 0.9).unwrap();
        assert_eq!(id, 2);
        assert!((lx - 0.5).abs() < 1e-6);
        assert!((ly - 0.9).abs() < 1e-6);
    }

    #[test]
    fn wireless_secondary_geometry_even_aligns_scales_hidpi_and_places_adjacent() {
        let mut coordinator = MultiScreenCoordinator::new();
        coordinator.set_remote_displays(vec![remote(1, 0, 0, 2560, 1440, true, false)]);

        // Odd-sized HiDPI tablet (e.g. iPad-like 2733x2049 @ 264 DPI, 120 Hz)
        let geom = coordinator.compute_wireless_secondary_geometry(2733, 2049, 264.0, 120);
        assert_eq!(geom.rect.width % 2, 0);
        assert_eq!(geom.rect.height % 2, 0);
        assert_eq!(geom.rect.width, 2732);
        assert_eq!(geom.rect.height, 2048);
        assert_eq!(geom.rect.x, 2560);
        assert_eq!(geom.rect.y, 0);
        assert!((geom.scale - 3.0).abs() < 1e-6 || (geom.scale - 2.5).abs() < 1e-6);
        assert_eq!(geom.refresh_hz, 120);
        assert!(geom.logical_width >= 640);
        assert!(geom.logical_height >= 480);

        // Standard 1080p @ 96 DPI, 0 Hz (defaults to 60 Hz)
        let std_geom = coordinator.compute_wireless_secondary_geometry(1921, 1081, 96.0, 0);
        assert_eq!(std_geom.rect, ScreenRect::new(2560, 0, 1920, 1080));
        assert_eq!(std_geom.scale, 1.0);
        assert_eq!(std_geom.logical_width, 1920);
        assert_eq!(std_geom.logical_height, 1080);
        assert_eq!(std_geom.refresh_hz, 60);
    }

    #[test]
    fn wireless_secondary_geometry_caps_oversized_resolution_and_avoids_overlap() {
        let mut coordinator = MultiScreenCoordinator::new();
        coordinator.set_remote_displays(vec![
            remote(1, 0, 0, 1920, 1080, true, false),
            // Existing secondary display already occupies x = 1920..3840
            remote(2, 1920, 0, 1920, 1080, false, false),
        ]);

        // 5K client should be capped within 3840x2160 and placed past display 2
        let geom = coordinator.compute_wireless_secondary_geometry(5120, 2880, 218.0, 144);
        assert_eq!(geom.rect.width, 3840);
        assert_eq!(geom.rect.height, 2160);
        assert_eq!(geom.rect.width % 2, 0);
        assert_eq!(geom.rect.height % 2, 0);
        assert_eq!(geom.rect.x, 3840);
        assert_eq!(geom.rect.y, 0);
        assert!((geom.scale - 1.75).abs() < 1e-6);
        assert_eq!(geom.refresh_hz, 144);
    }
}
