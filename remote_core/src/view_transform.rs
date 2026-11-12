//! Receiver-local view geometry. Drawing, viewport demand and input share this
//! inverse transform; native UI adapters own events and GPU presentation.
use protocol::viewport::{FrameGeometry, NORMALIZED_ONE, NormalizedRect};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}
impl ViewRect {
    fn intersect(self, other: Self) -> Option<Self> {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let width = (self.x + self.width).min(other.x + other.width) - x;
        let height = (self.y + self.height).min(other.y + other.height) - y;
        (width > 0. && height > 0.).then_some(Self {
            x,
            y,
            width,
            height,
        })
    }
    fn contains(self, p: [f64; 2]) -> bool {
        p.iter().all(|v| v.is_finite())
            && p[0] >= self.x
            && p[1] >= self.y
            && p[0] <= self.x + self.width
            && p[1] <= self.y + self.height
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewTransform {
    source: [u32; 2],
    viewport: [f64; 2],
    cover: bool,
    zoom: f64,
    center: [f64; 2],
}
impl ViewTransform {
    pub fn new(source: [u32; 2], viewport: [f64; 2], cover: bool) -> Option<Self> {
        let mut view = Self {
            source,
            viewport,
            cover,
            zoom: 1.,
            center: [0.5, 0.5],
        };
        view.layout(source, viewport, cover).then_some(view)
    }
    pub fn layout(&mut self, source: [u32; 2], viewport: [f64; 2], cover: bool) -> bool {
        if source.iter().any(|v| !(2..=32768).contains(v))
            || viewport
                .iter()
                .any(|v| !v.is_finite() || *v < 1. || *v > 32768.)
        {
            return false;
        }
        self.source = source;
        self.viewport = viewport;
        self.cover = cover;
        self.clamp_center();
        true
    }
    pub fn zoom(&self) -> f64 {
        self.zoom
    }
    pub fn is_default(&self) -> bool {
        self.zoom == 1. && self.center == [0.5, 0.5]
    }
    pub fn reset(&mut self) {
        self.zoom = 1.;
        self.center = [0.5, 0.5];
        self.clamp_center();
    }
    fn image_size(&self) -> [f64; 2] {
        let x = self.viewport[0] / f64::from(self.source[0]);
        let y = self.viewport[1] / f64::from(self.source[1]);
        let scale = if self.cover { x.max(y) } else { x.min(y) } * self.zoom;
        [
            f64::from(self.source[0]) * scale,
            f64::from(self.source[1]) * scale,
        ]
    }
    fn clamp_center(&mut self) {
        let size = self.image_size();
        for axis in 0..2 {
            let half = (self.viewport[axis] / size[axis] / 2.).min(0.5);
            self.center[axis] = self.center[axis].clamp(half, 1. - half);
        }
    }
    pub fn source_bounds(&self) -> ViewRect {
        let size = self.image_size();
        ViewRect {
            x: self.viewport[0] / 2. - self.center[0] * size[0],
            y: self.viewport[1] / 2. - self.center[1] * size[1],
            width: size[0],
            height: size[1],
        }
    }
    fn viewport_bounds(&self) -> ViewRect {
        ViewRect {
            x: 0.,
            y: 0.,
            width: self.viewport[0],
            height: self.viewport[1],
        }
    }
    pub fn source_point(&self, point: [f64; 2]) -> Option<[f64; 2]> {
        let image = self.source_bounds();
        if !self.viewport_bounds().contains(point) || !image.contains(point) {
            return None;
        }
        Some([
            ((point[0] - image.x) / image.width).clamp(0., 1.),
            ((point[1] - image.y) / image.height).clamp(0., 1.),
        ])
    }
    pub fn zoom_at(&mut self, factor: f64, anchor: [f64; 2]) -> bool {
        if !factor.is_finite() || factor <= 0. {
            return false;
        }
        let Some(point) = self.source_point(anchor) else {
            return false;
        };
        let zoom = (self.zoom * factor).clamp(1., 4.);
        if zoom == self.zoom {
            return false;
        }
        self.zoom = zoom;
        let size = self.image_size();
        self.center = [
            point[0] - (anchor[0] - self.viewport[0] / 2.) / size[0],
            point[1] - (anchor[1] - self.viewport[1] / 2.) / size[1],
        ];
        self.clamp_center();
        true
    }
    pub fn pan(&mut self, delta: [f64; 2]) -> bool {
        if !delta.iter().all(|v| v.is_finite()) {
            return false;
        }
        let size = self.image_size();
        let old = self.center;
        self.center = [old[0] - delta[0] / size[0], old[1] - delta[1] / size[1]];
        self.clamp_center();
        self.center != old
    }
    pub fn visible(&self) -> NormalizedRect {
        let image = self.source_bounds();
        let view = image
            .intersect(self.viewport_bounds())
            .expect("Validated view covers the source");
        let unit = f64::from(NORMALIZED_ONE);
        let edge = |v: f64, ceil: bool| {
            let v = v.clamp(0., 1.) * unit;
            (if ceil { v.ceil() } else { v.floor() }) as u32
        };
        let x = edge((view.x - image.x) / image.width, false);
        let y = edge((view.y - image.y) / image.height, false);
        let right = edge((view.x + view.width - image.x) / image.width, true);
        let bottom = edge((view.y + view.height - image.y) / image.height, true);
        NormalizedRect {
            x,
            y,
            width: right - x,
            height: bottom - y,
        }
    }
    pub fn physical_size(&self, density: f64) -> Option<[u32; 2]> {
        if !density.is_finite() || density <= 0. {
            return None;
        }
        let view = self.source_bounds().intersect(self.viewport_bounds())?;
        let size = [view.width * density, view.height * density];
        if !size.iter().all(|v| v.is_finite() && *v > 0.) {
            return None;
        }
        let fit = (8192. / size[0])
            .min(8192. / size[1])
            .min((33_554_432. / size[0] / size[1]).sqrt())
            .min(1.);
        Some([
            (size[0] * fit).floor().clamp(16., 8192.) as u32,
            (size[1] * fit).floor().clamp(16., 8192.) as u32,
        ])
    }
    pub fn crop_bounds(&self, geometry: FrameGeometry) -> Option<ViewRect> {
        if geometry.validate().is_err()
            || [geometry.source_width, geometry.source_height] != self.source
        {
            return None;
        }
        let image = self.source_bounds();
        Some(ViewRect {
            x: image.x + f64::from(geometry.crop.x) / f64::from(self.source[0]) * image.width,
            y: image.y + f64::from(geometry.crop.y) / f64::from(self.source[1]) * image.height,
            width: f64::from(geometry.crop.width) / f64::from(self.source[0]) * image.width,
            height: f64::from(geometry.crop.height) / f64::from(self.source[1]) * image.height,
        })
    }
    pub fn input_point(&self, point: [f64; 2], geometry: FrameGeometry) -> Option<[u16; 2]> {
        self.crop_bounds(geometry)?;
        let [x, y] = self.source_point(point)?;
        if !geometry.contains_source_point(x as f32, y as f32) {
            return None;
        }
        let (x, y) = geometry.clamp_source_point((x, y));
        Some([(x * 65535.).round() as u16, (y * 65535.).round() as u16])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::viewport::PixelRect;
    fn geometry() -> FrameGeometry {
        FrameGeometry {
            source_revision: 3,
            demand_revision: 1,
            revision: 1,
            source_width: 4000,
            source_height: 3000,
            crop: PixelRect {
                x: 1000,
                y: 750,
                width: 2000,
                height: 1500,
            },
            width: 1000,
            height: 750,
        }
    }
    #[test]
    fn fit_excludes_letterbox_from_both_input_and_physical_demand() {
        let view = ViewTransform::new([1920, 1080], [1000., 1000.], false).unwrap();
        let rect = view.source_bounds();
        assert!(rect.x.abs() < 1e-9);
        assert!((rect.width - 1000.).abs() < 1e-9);
        assert_eq!((rect.y, rect.height), (218.75, 562.5));
        assert_eq!(view.source_point([500., 500.]), Some([0.5, 0.5]));
        assert_eq!(view.source_point([10., 10.]), None);
        assert_eq!(view.visible(), NormalizedRect::FULL);
        assert_eq!(view.physical_size(2.), Some([2000, 1125]));
    }
    #[test]
    fn zoom_keeps_the_source_under_the_anchor_and_pan_reaches_all_edges() {
        let mut view = ViewTransform::new([4000, 3000], [800., 600.], false).unwrap();
        let anchor = [300., 200.];
        let before = view.source_point(anchor).unwrap();
        assert!(view.zoom_at(2., anchor));
        let after = view.source_point(anchor).unwrap();
        assert!(
            after
                .iter()
                .zip(before)
                .all(|(a, b)| (*a - b).abs() < 1e-12)
        );
        assert!(view.zoom_at(10., [400., 300.]));
        assert_eq!(view.zoom(), 4.);
        view.pan([100_000., 100_000.]);
        assert_eq!(
            view.visible(),
            NormalizedRect {
                x: 0,
                y: 0,
                width: 250_000,
                height: 250_000
            }
        );
        view.pan([-200_000., -200_000.]);
        assert_eq!(
            view.visible(),
            NormalizedRect {
                x: 750_000,
                y: 750_000,
                width: 250_000,
                height: 250_000
            }
        );
        view.reset();
        assert_eq!(view.visible(), NormalizedRect::FULL);
    }
    #[test]
    fn decoded_crop_sits_in_the_full_source_world_and_has_the_same_inverse_input() {
        let mut view = ViewTransform::new([4000, 3000], [800., 600.], false).unwrap();
        view.zoom_at(2., [400., 300.]);
        assert_eq!(
            view.crop_bounds(geometry()),
            Some(ViewRect {
                x: 0.,
                y: 0.,
                width: 800.,
                height: 600.
            })
        );
        assert_eq!(
            view.input_point([400., 300.], geometry()),
            Some([32768, 32768])
        );
        view.reset();
        assert_eq!(
            view.crop_bounds(geometry()),
            Some(ViewRect {
                x: 200.,
                y: 150.,
                width: 400.,
                height: 300.
            })
        );
        assert_eq!(
            view.input_point([100., 100.], geometry()),
            None,
            "Unreceived pixels must not receive input"
        );
    }
    #[test]
    fn cover_requests_only_the_visible_source_and_resize_preserves_center() {
        let mut view = ViewTransform::new([4000, 3000], [800., 400.], true).unwrap();
        let visible = view.visible();
        assert_eq!(visible.x, 0);
        assert_eq!(visible.width, NORMALIZED_ONE);
        assert!(visible.y > 0 && visible.height < NORMALIZED_ONE);
        assert!(view.layout([8000, 6000], [1600., 800.], true));
        assert_eq!(view.visible(), visible);
        assert_eq!(view.source_point([800., 400.]), Some([0.5, 0.5]));
        assert_eq!(
            view.input_point([800., 400.], geometry()),
            None,
            "An older source size cannot use the current layout"
        );
    }
    #[test]
    fn invalid_events_do_not_mutate_the_view_and_demands_obey_wire_bounds() {
        assert!(ViewTransform::new([0, 1080], [800., 600.], false).is_none());
        assert!(ViewTransform::new([1920, 1080], [f64::NAN, 600.], false).is_none());
        assert!(ViewTransform::new([1920, 1080], [f64::MIN_POSITIVE, 600.], false).is_none());
        let mut view = ViewTransform::new([4000, 3000], [32768., 32768.], false).unwrap();
        let old = view;
        assert!(!view.zoom_at(f64::NAN, [10., 10.]));
        assert!(!view.pan([f64::INFINITY, 0.]));
        assert_eq!(view, old);
        let [width, height] = view.physical_size(8.).unwrap();
        protocol::viewport::ViewportDemand {
            id: 7,
            source_revision: 3,
            revision: 1,
            visible: view.visible(),
            width,
            height,
        }
        .validate()
        .unwrap();
        assert_eq!(view.physical_size(f64::INFINITY), None);
    }
}
