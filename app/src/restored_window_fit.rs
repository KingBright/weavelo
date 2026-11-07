//! Sizing policy for a stream window. Source dimensions are independent of
//! the encoded crop and demand size, so changing the window cannot feed back.
use std::time::{Duration, Instant};
const RESIZE_SETTLE: Duration = Duration::from_millis(750);
type FitKey = ([u32; 2], [u32; 2], u32, u64);
#[derive(Debug)]
pub(super) struct WindowFit {
    auto: bool,
    explicit: bool,
    last_size: [f32; 2],
    pending: Option<([f32; 2], Instant)>,
    fitted_source: Option<FitKey>,
    special: bool,
}

impl WindowFit {
    pub(super) fn new(size: [f32; 2]) -> Self {
        Self { auto: true, explicit: false, last_size: size, pending: None, fitted_source: None, special: false }
    }

    pub(super) fn observe(&mut self, size: [f32; 2], special: bool) {
        let changed = !near(size, self.last_size);
        if special || self.special {
            self.pending = None;
            // Refit after returning from fullscreen/maximized.
            self.fitted_source = None;
        } else if changed {
            if !self.pending.is_some_and(|(target, _)| near(size, target)) {
                self.auto = false;
            }
            self.pending = None;
        }
        self.last_size = size;
        self.special = special;
    }

    pub(super) fn request_fit(&mut self) {
        self.auto = true;
        self.pending = None;
        self.explicit = true;
        self.fitted_source = None;
    }

    fn key(source: [u32; 2], display_size: [f32; 2], display_scale: f32, revision: u64) -> FitKey {
        (source, [display_size[0].round() as u32, display_size[1].round() as u32],
            (display_scale * 1000.).round() as u32, revision)
    }

    fn needs_fit(&mut self, key: FitKey, special: bool, eligible: bool, now: Instant) -> bool {
        if self.pending.is_some_and(|(_, started)| now.duration_since(started) >= RESIZE_SETTLE) {
            self.pending = None;
        }
        !special && self.auto && (eligible || self.explicit) && self.pending.is_none()
            && !key.0.contains(&0) && self.fitted_source != Some(key)
    }

    fn unavailable(&mut self, key: FitKey) {
        // Do not query or retry repeatedly when the compositor supplies no work area.
        self.fitted_source = Some(key);
        self.explicit = false;
    }

    fn target(&mut self, key: FitKey, available: [f32; 2], display_scale: f32, now: Instant) -> Option<[f32; 2]> {
        if available.iter().any(|n| !n.is_finite() || *n < 120.)
            || !display_scale.is_finite() || display_scale <= 0. {
            self.unavailable(key);
            return None;
        }
        let source = key.0;
        let max_scale = (available[0] / source[0] as f32).min(available[1] / source[1] as f32);
        let min_scale = (120. / source[0] as f32).max(120. / source[1] as f32);
        let scale = (1. / display_scale).max(min_scale).min(max_scale);
        let target = [
            (source[0] as f32 * scale).round().max(120.).min(available[0]),
            (source[1] as f32 * scale).round().max(120.).min(available[1]),
        ];
        self.fitted_source = Some(key);
        self.explicit = false;
        if near(target, self.last_size) { return None; }
        self.pending = Some((target, now));
        Some(target)
    }

}

fn near(a: [f32; 2], b: [f32; 2]) -> bool {
    (a[0] - b[0]).abs() <= 2. && (a[1] - b[1]).abs() <= 2.
}

pub(super) fn source_size(frame: &super::MacDecodedVideoFrame) -> Option<[u32; 2]> {
    let geometry = frame.frame_geometry?;
    Some(rotated([geometry.source_width, geometry.source_height], frame.display_rotation))
}

fn rotated(mut source: [u32; 2], rotation: u16) -> [u32; 2] {
    if matches!(rotation, 90 | 270) { source.swap(0, 1); }
    source
}

pub(super) fn fit(window: &mut gpui::Window, cx: &gpui::App, state: &mut WindowFit, source: [u32; 2], eligible: bool) {
    let Some(display) = window.display(cx) else { return; };
    let display_bounds = display.bounds();
    let scale = window.scale_factor();
    let key = WindowFit::key(source, [f32::from(display_bounds.size.width), f32::from(display_bounds.size.height)], scale, window.fit_bounds_revision());
    let now = Instant::now();
    if !state.needs_fit(key, window.is_fullscreen() || window.is_maximized(), eligible, now) { return; }
    #[cfg(target_os = "linux")]
    let work = window.fit_work_area();
    #[cfg(not(target_os = "linux"))]
    let work = Some(display.visible_bounds());
    let Some(work) = work else { state.unavailable(key); return; };
    let viewport = window.viewport_size();
    let bounds = window.bounds();
    let width = f32::from(work.size.width) - (f32::from(bounds.origin.x) - f32::from(work.origin.x)).max(0.);
    let height = f32::from(work.size.height) - (f32::from(bounds.origin.y) - f32::from(work.origin.y)).max(0.);
    let available = [
        width - (f32::from(bounds.size.width) - f32::from(viewport.width)).max(0.) - 32.,
        height - (f32::from(bounds.size.height) - f32::from(viewport.height)).max(0.) - 48.,
    ];
    if let Some(target) = state.target(key, available, scale, now) {
        window.resize(gpui::size(gpui::px(target[0]), gpui::px(target[1])));
    }
}

pub(super) fn observe(window: &gpui::Window, state: &mut WindowFit) {
    let size = window.viewport_size();
    state.observe([f32::from(size.width), f32::from(size.height)], window.is_fullscreen() || window.is_maximized());
}

#[cfg(test)]
mod tests {
    use super::*;
    fn attempt(fit: &mut WindowFit, source: [u32;2], area: [f32;2], scale: f32, special: bool, eligible: bool, now: Instant) -> Option<[f32;2]> {
        let key = WindowFit::key(source, [1440.,900.], scale, 0);
        fit.needs_fit(key, special, eligible, now).then(|| fit.target(key, area, scale, now)).flatten()
    }
    #[test]
    fn follows_rotation_and_manual_resize_until_fit() {
        let now=Instant::now(); let mut fit=WindowFit::new([1000.,700.]);
        assert_eq!(attempt(&mut fit,[1080,2400],[1400.,800.],1.,false,true,now),Some([360.,800.]));
        fit.observe([360.,800.],false);
        fit.observe([360.,800.],false);
        assert_eq!(attempt(&mut fit,[1080,2400],[900.,700.],1.,false,true,now),None);
        assert_eq!(attempt(&mut fit,[2400,1080],[1400.,800.],1.,false,true,now),Some([1400.,630.]));
        fit.observe([1400.,630.],false); fit.observe([1000.,450.],false);
        assert_eq!(attempt(&mut fit,[1080,2400],[1400.,800.],1.,false,true,now),None);
        fit.request_fit(); assert_eq!(attempt(&mut fit,[1080,2400],[1400.,800.],1.,false,true,now),Some([360.,800.]));
    }
    #[test]
    fn ignored_and_clamped_resize_can_be_retried_without_auto_loop() {
        let now=Instant::now(); let mut fit=WindowFit::new([1000.,700.]);
        assert!(attempt(&mut fit,[800,600],[1400.,800.],1.,false,true,now).is_some());
        fit.observe([1000.,700.],false);
        assert_eq!(attempt(&mut fit,[800,600],[1400.,800.],1.,false,true,now+RESIZE_SETTLE),None);
        fit.request_fit();
        assert!(attempt(&mut fit,[800,600],[1400.,800.],1.,false,true,now+RESIZE_SETTLE).is_some());
        fit.observe([790.,590.],false); // Conservatively treat a clamp as manual.
        assert_eq!(attempt(&mut fit,[800,600],[1400.,800.],1.,false,true,now+RESIZE_SETTLE),None);
        fit.request_fit();
        assert!(attempt(&mut fit,[800,600],[1400.,800.],1.,false,true,now+RESIZE_SETTLE).is_some());
    }
    #[test]
    fn manual_resize_during_pending_disables_following_source_geometry() {
        let now=Instant::now(); let mut fit=WindowFit::new([1000.,700.]);
        assert_eq!(attempt(&mut fit,[800,600],[1400.,800.],1.,false,true,now),Some([800.,600.]));
        fit.observe([640.,480.],false);
        assert_eq!(attempt(&mut fit,[600,800],[1400.,800.],1.,false,true,now),None);
        fit.request_fit();
        assert_eq!(attempt(&mut fit,[600,800],[1400.,800.],1.,false,true,now),Some([600.,800.]));
    }
    #[test]
    fn unavailable_work_area_waits_for_a_new_compositor_revision_or_fit() {
        let now=Instant::now(); let mut fit=WindowFit::new([1000.,700.]);
        let key=WindowFit::key([800,600],[1440.,900.],1.,0);
        assert!(fit.needs_fit(key,false,true,now)); fit.unavailable(key);
        assert!(!fit.needs_fit(key,false,true,now));
        let updated=WindowFit::key([800,600],[1440.,900.],1.,1);
        assert!(fit.needs_fit(updated,false,true,now));
        fit.unavailable(updated); fit.request_fit();
        assert!(fit.needs_fit(updated,false,true,now));
    }
    #[test]
    fn newer_source_can_fit_after_ignored_resize_settles() {
        let now=Instant::now(); let mut fit=WindowFit::new([1000.,700.]);
        assert!(attempt(&mut fit,[800,600],[1400.,800.],1.,false,true,now).is_some());
        assert_eq!(attempt(&mut fit,[600,800],[1400.,800.],1.,false,true,now),None);
        assert!(attempt(&mut fit,[600,800],[1400.,800.],1.,false,true,now+RESIZE_SETTLE).is_some());
    }
    #[test]
    fn minimum_and_retina_and_special_modes() {
        let now=Instant::now(); let mut fit=WindowFit::new([1000.,700.]);
        assert_eq!(attempt(&mut fit,[800,600],[119.,800.],1.,false,true,now),None);
        fit.request_fit();
        assert_eq!(attempt(&mut fit,[800,600],[1400.,800.],2.,false,true,now),Some([400.,300.]));
        assert_eq!(rotated([1080,2400],90),[2400,1080]);
        let mut special=WindowFit::new([1000.,700.]);
        assert_eq!(attempt(&mut special,[800,600],[1400.,800.],1.,true,true,now),None);
        special.observe([1400.,800.],true); special.observe([1000.,700.],false);
        assert!(attempt(&mut special,[800,600],[1400.,800.],1.,false,true,now).is_some());
    }
    #[test]
    fn desktop_display_needs_explicit_fit() {
        let now=Instant::now();let mut fit=WindowFit::new([1000.,700.]);
        assert_eq!(attempt(&mut fit,[1920,1080],[1400.,800.],1.,false,false,now),None);
        fit.request_fit();assert!(attempt(&mut fit,[1920,1080],[1400.,800.],1.,false,false,now).is_some());
    }
}
