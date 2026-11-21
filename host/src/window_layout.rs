//! Pure logical-geometry helpers for window-scoped coordinate mapping and host window tiling.

use protocol::session::WindowArrangeLayout;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct LogicalRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl LogicalRect {
    pub(crate) fn is_valid(self) -> bool {
        self.x.is_finite()
            && self.y.is_finite()
            && self.width.is_finite()
            && self.height.is_finite()
            && self.width >= 2.0
            && self.height >= 2.0
    }

    pub(crate) fn center(self) -> (f64, f64) {
        (self.x + self.width * 0.5, self.y + self.height * 0.5)
    }

    pub(crate) fn overlap_area(self, other: Self) -> f64 {
        let x0 = self.x.max(other.x);
        let y0 = self.y.max(other.y);
        let x1 = (self.x + self.width).min(other.x + other.width);
        let y1 = (self.y + self.height).min(other.y + other.height);
        (x1 - x0).max(0.0) * (y1 - y0).max(0.0)
    }
}

/// Map window-normalized coordinates (`0..=65535`) into monitor-normalized coordinates (`0..=65535`).
/// Both `win` and `(monitor_pos, monitor_size)` use the compositor's logical coordinate space.
#[allow(dead_code)]
pub(crate) fn map_window_point_to_monitor(
    nx: u16,
    ny: u16,
    win: LogicalRect,
    monitor_pos: (i32, i32),
    monitor_size: (i32, i32),
) -> Option<(u16, u16)> {
    if !win.is_valid() || monitor_size.0 <= 0 || monitor_size.1 <= 0 {
        return None;
    }
    let mon = LogicalRect {
        x: f64::from(monitor_pos.0),
        y: f64::from(monitor_pos.1),
        width: f64::from(monitor_size.0),
        height: f64::from(monitor_size.1),
    };
    if win.overlap_area(mon) <= 0.0 {
        return None;
    }
    let abs_x = win.x + (f64::from(nx) / f64::from(u16::MAX)) * (win.width - 1.0).max(0.0);
    let abs_y = win.y + (f64::from(ny) / f64::from(u16::MAX)) * (win.height - 1.0).max(0.0);
    let max_rel_x = (mon.width - 1.0).max(0.0);
    let max_rel_y = (mon.height - 1.0).max(0.0);
    let rel_x = (abs_x - mon.x).clamp(0.0, max_rel_x);
    let rel_y = (abs_y - mon.y).clamp(0.0, max_rel_y);
    let denom_x = max_rel_x.max(1.0);
    let denom_y = max_rel_y.max(1.0);
    let mx = ((rel_x / denom_x) * f64::from(u16::MAX))
        .round()
        .clamp(0.0, f64::from(u16::MAX)) as u16;
    let my = ((rel_y / denom_y) * f64::from(u16::MAX))
        .round()
        .clamp(0.0, f64::from(u16::MAX)) as u16;
    Some((mx, my))
}

#[allow(dead_code)]
pub(crate) fn pick_monitor_index(
    win: LogicalRect,
    monitors: &[((i32, i32), (i32, i32))],
) -> Option<usize> {
    if !win.is_valid() || monitors.is_empty() {
        return None;
    }
    let (cx, cy) = win.center();
    if let Some((idx, _)) = monitors.iter().enumerate().find(|(_, (pos, size))| {
        size.0 > 0
            && size.1 > 0
            && cx >= f64::from(pos.0)
            && cx < f64::from(pos.0) + f64::from(size.0)
            && cy >= f64::from(pos.1)
            && cy < f64::from(pos.1) + f64::from(size.1)
    }) {
        return Some(idx);
    }
    monitors
        .iter()
        .enumerate()
        .filter(|(_, (_, size))| size.0 > 0 && size.1 > 0)
        .max_by(|(_, (p0, s0)), (_, (p1, s1))| {
            let a0 = win.overlap_area(LogicalRect {
                x: f64::from(p0.0),
                y: f64::from(p0.1),
                width: f64::from(s0.0),
                height: f64::from(s0.1),
            });
            let a1 = win.overlap_area(LogicalRect {
                x: f64::from(p1.0),
                y: f64::from(p1.1),
                width: f64::from(s1.0),
                height: f64::from(s1.1),
            });
            a0.partial_cmp(&a1).unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(idx, _)| idx)
}

/// Compute non-overlapping window target rectangles inside `work_area`.
pub(crate) fn compute_tile_rects(
    work_area: LogicalRect,
    count: usize,
    layout: WindowArrangeLayout,
) -> Vec<LogicalRect> {
    if count == 0 || !work_area.is_valid() || layout == WindowArrangeLayout::Restore {
        return Vec::new();
    }
    let gap = 8.0_f64;
    let margin = 8.0_f64;
    let area = LogicalRect {
        x: work_area.x + margin,
        y: work_area.y + margin,
        width: (work_area.width - margin * 2.0).max(240.0),
        height: (work_area.height - margin * 2.0).max(240.0),
    };
    let (cols, rows) = match layout {
        WindowArrangeLayout::TileHorizontal => (count.max(1), 1usize),
        WindowArrangeLayout::TileGrid => {
            if count <= 1 {
                (1, 1)
            } else if count == 2 {
                (2, 1)
            } else {
                let c = (count as f64).sqrt().ceil() as usize;
                let r = count.div_ceil(c);
                (c, r)
            }
        }
        WindowArrangeLayout::Restore => return Vec::new(),
    };
    let row_h = ((area.height - gap * (rows.saturating_sub(1) as f64)) / rows as f64).max(160.0);
    let mut rects = Vec::with_capacity(count);
    let mut placed = 0usize;
    for r in 0..rows {
        let remaining = count - placed;
        let row_cols = remaining.min(cols);
        let col_w = ((area.width - gap * (row_cols.saturating_sub(1) as f64)) / row_cols as f64)
            .max(200.0);
        let y = area.y + (r as f64) * (row_h + gap);
        for c in 0..row_cols {
            let x = area.x + (c as f64) * (col_w + gap);
            rects.push(LogicalRect {
                x: x.round(),
                y: y.round(),
                width: col_w.round(),
                height: row_h.round(),
            });
            placed += 1;
        }
    }
    rects
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_monitor_window_maps_identically() {
        let win = LogicalRect {
            x: 0.0,
            y: 0.0,
            width: 2648.0,
            height: 1490.0,
        };
        assert_eq!(
            map_window_point_to_monitor(0, 0, win, (0, 0), (2648, 1490)),
            Some((0, 0))
        );
        assert_eq!(
            map_window_point_to_monitor(u16::MAX, u16::MAX, win, (0, 0), (2648, 1490)),
            Some((u16::MAX, u16::MAX))
        );
        let center = map_window_point_to_monitor(32768, 32768, win, (0, 0), (2648, 1490)).unwrap();
        assert!((i32::from(center.0) - 32768).abs() <= 2);
        assert!((i32::from(center.1) - 32768).abs() <= 2);
    }

    #[test]
    fn right_half_kwin_window_under_fractional_scaling_maps_to_right_half_of_monitor() {
        let win = LogicalRect {
            x: 1324.0,
            y: 0.0,
            width: 1324.2758,
            height: 1444.1379,
        };
        let mon_pos = (0, 0);
        let mon_size = (2648, 1490);
        let left_edge = map_window_point_to_monitor(0, 0, win, mon_pos, mon_size).unwrap();
        let right_bottom =
            map_window_point_to_monitor(u16::MAX, u16::MAX, win, mon_pos, mon_size).unwrap();
        assert!((i32::from(left_edge.0) - 32780).abs() <= 30);
        assert_eq!(left_edge.1, 0);
        assert_eq!(right_bottom.0, u16::MAX);
        assert!(right_bottom.1 > 60000 && right_bottom.1 < u16::MAX);
    }

    #[test]
    fn negative_monitor_offset_and_offscreen_rejection_work() {
        let win_on_left = LogicalRect {
            x: -960.0,
            y: 100.0,
            width: 640.0,
            height: 480.0,
        };
        let mapped =
            map_window_point_to_monitor(32768, 32768, win_on_left, (-1920, 0), (1920, 1080))
                .unwrap();
        // Window covers x in [-960..-320], midpoint = -640 -> rel = 1280 -> 1280/1919 ≈ 2/3
        assert!((i32::from(mapped.0) - 43712).abs() <= 50);

        let offscreen = LogicalRect {
            x: 3000.0,
            y: 0.0,
            width: 800.0,
            height: 600.0,
        };
        assert_eq!(
            map_window_point_to_monitor(1000, 1000, offscreen, (0, 0), (1920, 1080)),
            None
        );
    }

    #[test]
    fn monitor_picker_selects_monitor_containing_window_center() {
        let monitors = [((0, 0), (1920, 1080)), ((1920, 0), (2560, 1440))];
        let win = LogicalRect {
            x: 2000.0,
            y: 100.0,
            width: 1000.0,
            height: 800.0,
        };
        assert_eq!(pick_monitor_index(win, &monitors), Some(1));
    }

    #[test]
    fn compute_tile_rects_creates_non_overlapping_horizontal_and_grid_layouts() {
        let work = LogicalRect {
            x: 0.0,
            y: 25.0,
            width: 1920.0,
            height: 1055.0,
        };
        let horiz = compute_tile_rects(work, 2, WindowArrangeLayout::TileHorizontal);
        assert_eq!(horiz.len(), 2);
        assert!(horiz[0].overlap_area(horiz[1]) == 0.0);
        assert!(horiz[0].x + horiz[0].width < horiz[1].x);

        let grid3 = compute_tile_rects(work, 3, WindowArrangeLayout::TileGrid);
        assert_eq!(grid3.len(), 3);
        assert_eq!(grid3[0].overlap_area(grid3[1]), 0.0);
        assert_eq!(grid3[0].overlap_area(grid3[2]), 0.0);
        assert_eq!(grid3[1].overlap_area(grid3[2]), 0.0);
        // Third window spans the bottom row
        assert!(grid3[2].width > grid3[0].width);
    }
}
