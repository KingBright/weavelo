//! Shared viewport policy. Native adapters crop pixels; this code owns geometry,
//! latest-demand coalescing and output-size hysteresis on every platform.
use protocol::viewport::{
    FrameGeometry, NORMALIZED_ONE, NormalizedRect, PixelRect, ViewportDemand,
};
use std::time::{Duration, Instant};

pub const DEMAND_INTERVAL: Duration = Duration::from_millis(50);
const ZOOM_SETTLE: Duration = Duration::from_millis(150);
const RESIZE_INTERVAL: Duration = Duration::from_millis(400);

pub fn visible_pixels(rect: NormalizedRect, source: [u32; 2]) -> PixelRect {
    let floor =
        |n: u32, total: u32| (u64::from(n) * u64::from(total) / u64::from(NORMALIZED_ONE)) as u32;
    let ceil = |n: u32, total: u32| {
        (u64::from(n) * u64::from(total)).div_ceil(u64::from(NORMALIZED_ONE)) as u32
    };
    let x = floor(rect.x, source[0]);
    let y = floor(rect.y, source[1]);
    PixelRect {
        x,
        y,
        width: ceil(rect.x + rect.width, source[0]) - x,
        height: ceil(rect.y + rect.height, source[1]) - y,
    }
}

fn crop_with_margin(visible: PixelRect, source: [u32; 2], motion: [i64; 2]) -> PixelRect {
    let axis = |origin: u32, size: u32, full: u32, direction: i64| {
        // Preserve crop size at edges. Panning cannot resize the codec merely
        // because a margin reached the source boundary. NV12 is even aligned.
        let extent = (size.saturating_add(size.div_ceil(5)).saturating_add(1) & !1)
            .min(full)
            .max(2);
        let margin = i64::from(extent.saturating_sub(size)) / 2;
        let ahead = direction.clamp(-margin, margin) / 2;
        let start =
            (i64::from(origin) - margin + ahead).clamp(0, i64::from(full - extent)) as u32 & !1;
        // Keep NV12 origins aligned, without dropping the last real source
        // column/row when the native window has an odd extent. GPU scaling
        // still produces an even coded surface.
        let extent = extent.max(origin + size - start).min(full - start);
        (start, extent)
    };
    let (x, width) = axis(visible.x, visible.width, source[0], motion[0]);
    let (y, height) = axis(visible.y, visible.height, source[1], motion[1]);
    PixelRect {
        x,
        y,
        width,
        height,
    }
}

fn fit_pixels(source: [u32; 2], cap: [u32; 2]) -> [u32; 2] {
    let scale = (f64::from(cap[0]) / f64::from(source[0]))
        .min(f64::from(cap[1]) / f64::from(source[1]))
        .min(1.);
    [
        (f64::from(source[0]) * scale) as u32 & !1,
        (f64::from(source[1]) * scale) as u32 & !1,
    ]
}

#[derive(Default)]
pub struct ViewportPlanner {
    native_window: bool,
    demand: Option<ViewportDemand>,
    changed: Option<Instant>,
    last_resize: Option<Instant>,
    last_applied: Option<Instant>,
    last_visible: Option<PixelRect>,
    last_limits: Option<([u32; 2], Option<[u32; 2]>, bool)>,
    resize_pending: bool,
    current: Option<FrameGeometry>,
    next_revision: u64,
}
impl ViewportPlanner {
    pub fn for_source(source: protocol::session::CaptureSource) -> Self {
        Self {
            native_window: matches!(source, protocol::session::CaptureSource::Window(_)),
            ..Self::default()
        }
    }
    pub fn demand(&self) -> Option<ViewportDemand> {
        self.demand
    }
    /// The owner checks subscription/source identity first. Older duplicates do
    /// not change the pending crop or renew the stabilization deadline.
    pub fn update(&mut self, demand: ViewportDemand, now: Instant) -> Result<bool, &'static str> {
        demand.validate()?;
        if self
            .demand
            .is_some_and(|old| old.id != demand.id || old.source_revision != demand.source_revision)
        {
            return Err("Viewport belongs to a different source");
        }
        if self
            .demand
            .is_some_and(|old| demand.revision <= old.revision)
        {
            return Ok(false);
        }
        if self.demand.is_none_or(|old| {
            old.visible.width != demand.visible.width
                || old.visible.height != demand.visible.height
                || (old.width, old.height) != (demand.width, demand.height)
        }) {
            self.changed = Some(now);
        }
        self.demand = Some(demand);
        Ok(true)
    }
    pub fn reset(&mut self) {
        *self = Self {
            native_window: self.native_window,
            ..Self::default()
        };
    }
    /// Capture geometry changed, including a same-sized rotation. Keep the
    /// sequence monotonic so in-flight encoded frames remain distinguishable.
    pub fn invalidate_current(&mut self) -> Option<u64> {
        self.current = None;
        self.last_visible = None;
        self.last_limits = None;
        self.resize_pending = false;
        self.next_revision.checked_add(1)
    }
    pub fn plan(
        &mut self,
        source: [u32; 2],
        cap: [u32; 2],
        source_revision: u32,
        now: Instant,
    ) -> Result<FrameGeometry, &'static str> {
        self.plan_with_receiver_density(source, cap, source_revision, now, true)
    }
    /// Directed send can choose coded density independently from the receiver's
    /// physical window, while retaining the same visible crop and revisions.
    pub fn plan_with_receiver_density(
        &mut self,
        source: [u32; 2],
        cap: [u32; 2],
        source_revision: u32,
        now: Instant,
        receiver_density: bool,
    ) -> Result<FrameGeometry, &'static str> {
        if source.iter().any(|v| !(2..=32768).contains(v))
            || cap.iter().any(|v| !(2..=8192).contains(v))
        {
            return Err("Source or viewport pixel size is unavailable");
        }
        let demand = self.demand.filter(|v| v.source_revision == source_revision);
        let limits = (cap, demand.map(|v| [v.width, v.height]), receiver_density);
        let visible = visible_pixels(demand.map_or(NormalizedRect::FULL, |v| v.visible), source);
        let old = self.current.filter(|g| {
            g.source_revision == source_revision && [g.source_width, g.source_height] == source
        });
        let motion = self.last_visible.map_or([0, 0], |last| {
            [
                i64::from(visible.x) - i64::from(last.x),
                i64::from(visible.y) - i64::from(last.y),
            ]
        });
        let mut crop = crop_with_margin(visible, source, motion);
        if let Some(old) = old {
            let covered = old.crop.contains(visible);
            let same_zoom = self
                .last_visible
                .is_some_and(|v| (v.width, v.height) == (visible.width, visible.height));
            let coalescing = self
                .last_applied
                .is_some_and(|t| now.saturating_duration_since(t) < DEMAND_INTERVAL);
            if covered && same_zoom && coalescing {
                crop = old.crop;
            }
        }
        let target =
            if let Some(demand) = demand.filter(|_| receiver_density && !self.native_window) {
                // Pixel density follows the visible receiver, retaining prefetch
                // margins at that density. Never enlarge beyond captured pixels.
                let w = (u64::from(demand.width) * u64::from(crop.width))
                    .div_ceil(u64::from(visible.width.max(1)));
                let h = (u64::from(demand.height) * u64::from(crop.height))
                    .div_ceil(u64::from(visible.height.max(1)));
                [
                    cap[0].min(w.min(8192) as u32).max(2),
                    cap[1].min(h.min(8192) as u32).max(2),
                ]
            } else {
                cap
            };
        let mut output = fit_pixels([crop.width, crop.height], target);
        if output.iter().any(|v| *v < 2) {
            return Err("Viewport is too narrow to encode");
        }
        let mut resize_pending = false;
        if let Some(old) = old {
            let proposed = [old.width, old.height] != output;
            let settled = self
                .changed
                .is_none_or(|t| now.saturating_duration_since(t) >= ZOOM_SETTLE);
            let interval = self
                .last_resize
                .is_none_or(|t| now.saturating_duration_since(t) >= RESIZE_INTERVAL);
            let old_fits = old.width <= source[0]
                && old.height <= source[1]
                && old.width <= cap[0]
                && old.height <= cap[1];
            // An explicit stream quality or window clarity cap change is a
            // deliberate user transaction, not a continuous zoom gesture:
            // apply the new coded resolution immediately without 400ms lag.
            let explicit_cap_change = self
                .last_limits
                .is_some_and(|(old_cap, _, _)| old_cap != cap);
            let stable_pan = self.last_limits == Some(limits)
                && self
                    .last_visible
                    .is_some_and(|v| (v.width, v.height) == (visible.width, visible.height));
            // Keep the old codec during active navigation. Per-frame geometry
            // restores the crop's true display aspect after GPU scaling, even
            // when integer alignment changes its aspect by a few pixels.
            if proposed
                && old_fits
                && stable_pan
                && !self.resize_pending
                && old.width <= crop.width
                && old.height <= crop.height
            {
                output = [old.width, old.height];
            } else if proposed && old_fits && !explicit_cap_change && (!settled || !interval) {
                // Keep retrying this resize after the gesture settles. The
                // new crop by itself must not turn it into a stable pan.
                resize_pending = true;
                // While zooming in, keep enough real source pixels for the
                // current codec instead of upsampling or recreating it at 20Hz.
                // The local GPU still follows every gesture immediately.
                let expand = |origin: u32, size: u32, minimum: u32, full: u32| {
                    let extent = size.max(minimum).min(full);
                    let start = i64::from(origin) - i64::from(extent - size) / 2;
                    let start = start.clamp(0, i64::from(full - extent)) as u32 & !1;
                    (start, extent.max(origin + size - start).min(full - start))
                };
                let (x, width) = expand(crop.x, crop.width, old.width, source[0]);
                let (y, height) = expand(crop.y, crop.height, old.height, source[1]);
                crop = PixelRect {
                    x,
                    y,
                    width,
                    height,
                };
                output = [old.width, old.height];
            }
        }
        let mut g = FrameGeometry {
            source_revision,
            demand_revision: demand.map_or(0, |v| v.revision),
            revision: 1,
            source_width: source[0],
            source_height: source[1],
            crop,
            width: output[0],
            height: output[1],
        };
        let changed = old.is_none_or(|old| old.crop != g.crop || [old.width, old.height] != output);
        if changed {
            self.next_revision = self
                .next_revision
                .checked_add(1)
                .ok_or("Viewport geometry revision exhausted")?;
            if old.is_none_or(|old| [old.width, old.height] != output) {
                self.last_resize = Some(now);
            }
            self.last_applied = Some(now);
        }
        g.revision = if changed {
            self.next_revision
        } else {
            old.unwrap().revision
        };
        g.validate()?;
        self.resize_pending = resize_pending;
        self.last_visible = Some(visible);
        self.last_limits = Some(limits);
        self.current = Some(g);
        Ok(g)
    }
}

/// Automatic text-friendly target, always below the existing user/network cap.
pub fn bitrate_for_pixels(width: u32, height: u32, fps: u32, cap_kbps: u32) -> u32 {
    ((u64::from(width) * u64::from(height) * u64::from(fps) * 22 / 100_000)
        .max(500)
        .min(u64::from(cap_kbps))) as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    fn demand(revision: u64, x: u32, zoom: u32) -> ViewportDemand {
        ViewportDemand {
            id: 7,
            source_revision: 3,
            revision,
            visible: NormalizedRect {
                x,
                y: 0,
                width: NORMALIZED_ONE / zoom,
                height: NORMALIZED_ONE / zoom,
            },
            width: 1280,
            height: 720,
        }
    }
    #[test]
    fn directed_quality_uses_cap_not_receiver_window_and_keeps_crop() {
        let now = Instant::now();
        let mut planner = ViewportPlanner::default();
        let viewport = ViewportDemand {
            id: 7,
            source_revision: 3,
            revision: 1,
            visible: NormalizedRect::FULL,
            width: 360,
            height: 800,
        };
        planner.update(viewport, now).unwrap();
        let source = [1080, 1920];
        for (index, short) in [540, 720, 1080].into_iter().enumerate() {
            let cap = [short, short * 16 / 9];
            let geometry = planner
                .plan_with_receiver_density(
                    source,
                    cap,
                    3,
                    now + Duration::from_millis(index as u64 * 20),
                    false,
                )
                .unwrap();
            assert_eq!(geometry.width, short);
            assert!(
                geometry
                    .crop
                    .contains(visible_pixels(viewport.visible, source))
            );
        }
        let normal = planner
            .plan(source, [1080, 1920], 3, now + Duration::from_secs(2))
            .unwrap();
        assert!(normal.width <= 360 && normal.height <= 800);
    }
    #[test]
    fn pan_is_latest_only_and_never_changes_coded_size_at_edges() {
        let now = Instant::now();
        let mut planner = ViewportPlanner::default();
        planner.update(demand(1, 0, 4), now).unwrap();
        let first = planner.plan([3840, 2160], [1920, 1080], 3, now).unwrap();
        for (n, x) in [100_000, 300_000, 600_000, 750_000, 0]
            .into_iter()
            .enumerate()
        {
            let time = now + Duration::from_millis(n as u64 * 60 + 60);
            planner.update(demand(n as u64 + 2, x, 4), time).unwrap();
            let g = planner.plan([3840, 2160], [1920, 1080], 3, time).unwrap();
            assert_eq!((g.width, g.height), (first.width, first.height));
            assert!(
                g.crop
                    .contains(visible_pixels(demand(1, x, 4).visible, [3840, 2160]))
            );
        }
    }
    #[test]
    fn odd_native_sources_keep_the_last_column_and_row_in_view() {
        let now = Instant::now();
        let mut planner = ViewportPlanner::default();
        let source = [641, 371];
        let geometry = planner.plan(source, [320, 180], 3, now).unwrap();
        assert!(geometry.crop.contains(PixelRect {
            x: 0,
            y: 0,
            width: source[0],
            height: source[1]
        }));
    }
    #[test]
    fn odd_edge_prefetch_keeps_coverage_and_one_coded_size_during_pan() {
        let now = Instant::now();
        let mut planner = ViewportPlanner::default();
        let source = [641, 371];
        let mut size = None;
        for (n, (x, y)) in [
            (0, 0),
            (500_000, 0),
            (500_000, 500_000),
            (0, 500_000),
            (0, 0),
        ]
        .into_iter()
        .enumerate()
        {
            let demand = ViewportDemand {
                id: 7,
                source_revision: 3,
                revision: n as u64 + 1,
                visible: NormalizedRect {
                    x,
                    y,
                    width: 500_000,
                    height: 500_000,
                },
                width: 320,
                height: 180,
            };
            let time = now + Duration::from_millis(n as u64 * 500);
            planner.update(demand, time).unwrap();
            let geometry = planner.plan(source, [320, 180], 3, time).unwrap();
            assert!(
                geometry
                    .crop
                    .contains(visible_pixels(demand.visible, source)),
                "Visible edge was clipped: {geometry:?}"
            );
            assert_eq!(
                *size.get_or_insert([geometry.width, geometry.height]),
                [geometry.width, geometry.height],
                "Panning must retain one codec size"
            );
        }
    }
    #[test]
    fn fast_zoom_out_restores_coverage_and_wrong_source_cannot_replace_pending() {
        let now = Instant::now();
        let mut planner = ViewportPlanner::default();
        planner.update(demand(1, 400_000, 4), now).unwrap();
        planner.plan([3840, 2160], [1920, 1080], 3, now).unwrap();
        let latest = demand(2, 0, 1);
        planner
            .update(latest, now + Duration::from_millis(1))
            .unwrap();
        let full = planner
            .plan(
                [3840, 2160],
                [1920, 1080],
                3,
                now + Duration::from_millis(2),
            )
            .unwrap();
        assert_eq!(
            full.crop,
            PixelRect {
                x: 0,
                y: 0,
                width: 3840,
                height: 2160
            }
        );
        assert_eq!(full.demand_revision, 2);
        assert!(!planner.update(demand(1, 0, 4), now).unwrap());
        assert!(
            planner
                .update(
                    ViewportDemand {
                        source_revision: 4,
                        ..latest
                    },
                    now
                )
                .is_err()
        );
        assert_eq!(planner.demand(), Some(latest));
    }
    #[test]
    fn repeated_zoom_out_recovers_the_full_view_coded_size_after_settling() {
        let start = Instant::now();
        let source = [3840, 2160];
        let cap = [1920, 1080];
        let mut planner = ViewportPlanner::default();
        planner.update(demand(1, 0, 1), start).unwrap();
        let full = planner.plan(source, cap, 3, start).unwrap();

        for (cycle, zoom) in [4, 6, 8].into_iter().enumerate() {
            let now = start + Duration::from_secs(cycle as u64 * 2 + 1);
            let revision = 2 + cycle as u64 * 2;
            planner.update(demand(revision, 0, zoom), now).unwrap();
            planner.plan(source, cap, 3, now).unwrap();
            let now = now + ZOOM_SETTLE;
            let zoomed = planner.plan(source, cap, 3, now).unwrap();
            assert!(zoomed.width < full.width);

            let now = now + Duration::from_millis(1);
            planner.update(demand(revision + 1, 0, 1), now).unwrap();
            let immediate = planner.plan(source, cap, 3, now).unwrap();
            assert_eq!(immediate.crop, full.crop);
            assert_eq!(
                (immediate.width, immediate.height),
                (zoomed.width, zoomed.height)
            );

            let recovered = planner.plan(source, cap, 3, now + RESIZE_INTERVAL).unwrap();
            assert_eq!(
                (recovered.width, recovered.height),
                (full.width, full.height)
            );
        }
    }
    #[test]
    fn explicit_receiver_cap_switch_updates_window_coded_size_immediately() {
        let now = Instant::now();
        let mut planner = ViewportPlanner::for_source(protocol::session::CaptureSource::Window(77));
        let source = [1600, 1000];
        let native = planner.plan(source, [1600, 1000], 3, now).unwrap();
        assert_eq!((native.width, native.height), (1600, 1000));
        // Even 10ms later (well inside the 400ms zoom-hysteresis window), an
        // explicit user clarity switch (50% -> 800x500) must take effect immediately.
        let half = planner
            .plan(source, [800, 500], 3, now + Duration::from_millis(10))
            .unwrap();
        assert_eq!((half.width, half.height), (800, 500));
        assert_ne!(half.revision, native.revision);
        // Raising clarity back to 75% (1200x750) 20ms later also applies immediately.
        let high = planner
            .plan(source, [1200, 750], 3, now + Duration::from_millis(30))
            .unwrap();
        assert_eq!((high.width, high.height), (1200, 750));
        assert_ne!(high.revision, half.revision);
    }
    #[test]
    fn continuous_window_zoom_keeps_codec_dimensions_until_the_gesture_settles() {
        let now = Instant::now();
        let mut planner = ViewportPlanner::for_source(protocol::session::CaptureSource::Window(77));
        let first = planner.plan([1280, 720], [1920, 1080], 3, now).unwrap();
        for n in 1..=30 {
            let time = now + Duration::from_millis(n * 50);
            let width = 1_000_000 - (n as u32 * 20_000);
            planner
                .update(
                    ViewportDemand {
                        id: 7,
                        source_revision: 3,
                        revision: n,
                        visible: NormalizedRect {
                            x: (1_000_000 - width) / 2,
                            y: (1_000_000 - width) / 2,
                            width,
                            height: width,
                        },
                        width: 1280,
                        height: 720,
                    },
                    time,
                )
                .unwrap();
            let g = planner.plan([1280, 720], [1920, 1080], 3, time).unwrap();
            assert_eq!((g.width, g.height), (first.width, first.height));
            assert!(g.crop.contains(visible_pixels(
                planner.demand().unwrap().visible,
                [1280, 720]
            )));
        }
        let settled = planner
            .plan(
                [1280, 720],
                [1920, 1080],
                3,
                now + Duration::from_millis(1700),
            )
            .unwrap();
        assert!(settled.width < first.width && settled.height < first.height);
    }
    #[test]
    fn actual_window_pixels_are_not_upsampled_and_resize_changes_frame_geometry() {
        let now = Instant::now();
        let mut planner = ViewportPlanner::default();
        let native = planner.plan([1280, 904], [1920, 1080], 0, now).unwrap();
        assert_eq!((native.width, native.height), (1280, 904));
        let resized = planner.plan([1600, 1100], [1920, 1080], 0, now).unwrap();
        assert_ne!(native.revision, resized.revision);
        assert_eq!((resized.source_width, resized.source_height), (1600, 1100));
        assert!(resized.width <= 1600 && resized.height <= 1100);
        assert!(bitrate_for_pixels(640, 360, 30, 5000) < bitrate_for_pixels(1920, 1080, 30, 5000));
        assert_eq!(bitrate_for_pixels(8192, 4096, 120, 2000), 2000);
        assert_eq!(bitrate_for_pixels(640, 360, 30, 100), 100);
        let mut window = ViewportPlanner::for_source(protocol::session::CaptureSource::Window(77));
        window
            .update(
                ViewportDemand {
                    id: 7,
                    source_revision: 0,
                    revision: 1,
                    visible: NormalizedRect::FULL,
                    width: 640,
                    height: 452,
                },
                now,
            )
            .unwrap();
        let g = window.plan([1280, 904], [1920, 1080], 0, now).unwrap();
        assert_eq!(
            (g.width, g.height),
            (1280, 904),
            "small independent windows retain real pixels even on a smaller viewer"
        );
    }
}
