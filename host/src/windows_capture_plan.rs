//! Geometry and command construction shared by Windows capture and input.
use std::process::Command;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}
impl Rect {
    pub fn valid(self) -> bool {
        self.width > 1
            && self.height > 1
            && i64::from(self.x) + i64::from(self.width) <= i64::from(i32::MAX)
            && i64::from(self.y) + i64::from(self.height) <= i64::from(i32::MAX)
    }
    pub fn contains(self, other: Self) -> bool {
        self.valid()
            && other.valid()
            && other.x >= self.x
            && other.y >= self.y
            && i64::from(other.x) + i64::from(other.width)
                <= i64::from(self.x) + i64::from(self.width)
            && i64::from(other.y) + i64::from(other.height)
                <= i64::from(self.y) + i64::from(self.height)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Encoder {
    Amd,
    Nvidia,
    Intel,
}
impl Encoder {
    pub fn from_vendor(vendor: u32) -> Result<Self, String> {
        match vendor {
            0x1002 => Ok(Self::Amd),
            0x10de => Ok(Self::Nvidia),
            0x8086 => Ok(Self::Intel),
            _ => Err(format!(
                "No native HEVC encoder is implemented for GPU vendor {vendor:04x}"
            )),
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Amd => "hevc_amf",
            Self::Nvidia => "hevc_nvenc",
            Self::Intel => "hevc_qsv",
        }
    }
}

pub(crate) fn fit(source: Rect, width: u32, height: u32) -> Result<(u32, u32), String> {
    if !source.valid() || width < 2 || height < 2 {
        return Err("Capture dimensions are unavailable".into());
    }
    let scale = (f64::from(width) / f64::from(source.width))
        .min(f64::from(height) / f64::from(source.height))
        .min(1.0);
    let w = (f64::from(source.width) * scale) as u32 / 2 * 2;
    let h = (f64::from(source.height) * scale) as u32 / 2 * 2;
    if w < 2 || h < 2 {
        return Err("Capture dimensions are too small".into());
    }
    Ok((w, h))
}

/// Invert GPU aspect-fit padding, then map physical source pixels into SendInput's virtual desktop.
pub(crate) fn absolute(
    source: Rect,
    desktop: Rect,
    canvas: (u32, u32),
    x: u16,
    y: u16,
) -> Result<(i32, i32), String> {
    if !source.valid() || !desktop.contains(source) || canvas.0 < 2 || canvas.1 < 2 {
        return Err("Input geometry is unavailable".into());
    }
    let (w, h) = (f64::from(canvas.0), f64::from(canvas.1));
    let scale = (w / f64::from(source.width)).min(h / f64::from(source.height));
    let (draw_w, draw_h) = (
        f64::from(source.width) * scale,
        f64::from(source.height) * scale,
    );
    let (px, py) = (f64::from(x) / 65535.0 * w, f64::from(y) / 65535.0 * h);
    let (left, top) = ((w - draw_w) / 2.0, (h - draw_h) / 2.0);
    if px < left || px > left + draw_w || py < top || py > top + draw_h {
        return Err("Pointer is outside the captured content".into());
    }
    let sx = f64::from(source.x) + (px - left) / draw_w * f64::from(source.width - 1);
    let sy = f64::from(source.y) + (py - top) / draw_h * f64::from(source.height - 1);
    Ok((
        ((sx - f64::from(desktop.x)) / f64::from(desktop.width - 1) * 65535.0).round() as i32,
        ((sy - f64::from(desktop.y)) / f64::from(desktop.height - 1) * 65535.0).round() as i32,
    ))
}

pub(crate) fn command(
    program: &std::path::Path,
    handle: u64,
    window: bool,
    canvas: (u32, u32),
    fps: u32,
    kbps: u32,
    encoder: Encoder,
) -> Command {
    let mut command = Command::new(program);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let selector = if window { "hwnd" } else { "hmonitor" };
    let filter = format!(
        "gfxcapture={selector}={handle}:capture_border=0:display_border=1:max_framerate={fps}:width={}:height={}:resize_mode=scale_aspect:output_fmt=bgra",
        canvas.0, canvas.1
    );
    let vendor = match encoder {
        Encoder::Amd => "0x1002",
        Encoder::Nvidia => "0x10de",
        Encoder::Intel => "0x8086",
    };
    command.args([
        "-hide_banner",
        "-nostdin",
        "-loglevel",
        "error",
        "-init_hw_device",
        &format!("d3d11va=weavelo:,vendor_id={vendor}"),
        "-filter_hw_device",
        "weavelo",
        "-filter_complex",
        &filter,
        "-an",
        "-c:v",
        encoder.name(),
    ]);
    match encoder {
        Encoder::Amd => {
            command.args([
                "-usage",
                "ultralowlatency",
                "-quality",
                "speed",
                "-async_depth",
                "1",
                "-rc",
                "cbr",
                "-header_insertion_mode",
                "idr",
            ]);
        }
        Encoder::Nvidia => {
            command.args([
                "-preset",
                "p1",
                "-tune",
                "ull",
                "-zerolatency",
                "1",
                "-rc",
                "cbr",
                "-rc-lookahead",
                "0",
            ]);
        }
        Encoder::Intel => {
            command.args([
                "-preset",
                "veryfast",
                "-async_depth",
                "1",
                "-low_delay_brc",
                "1",
                "-look_ahead",
                "0",
            ]);
        }
    }
    let bitrate = format!("{kbps}k");
    command.args([
        "-g",
        &fps.max(1).to_string(),
        "-bf",
        "0",
        "-b:v",
        &bitrate,
        "-maxrate",
        &bitrate,
        "-bufsize",
        &bitrate,
        "-color_range",
        "tv",
        "-colorspace",
        "bt709",
        "-color_primaries",
        "bt709",
        "-color_trc",
        "iec61966-2-1",
        "-f",
        "hevc",
        "pipe:1",
    ]);
    command
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hardware_command_keeps_full_handle_and_gpu_frames() {
        let command = command(
            std::path::Path::new("owned-ffmpeg"),
            0x1234_5678_abcdef,
            true,
            (1280, 720),
            60,
            8000,
            Encoder::Amd,
        );
        let args = command
            .get_args()
            .map(|a| a.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(args.contains("hwnd=5124095577148911"));
        assert!(args.contains("hevc_amf"));
        for forbidden in ["hwdownload", "gdigrab", "libx265", "format=yuv420p"] {
            assert!(!args.contains(forbidden));
        }
    }
    #[test]
    fn native_bounds_fit_without_upscale_or_stretch() {
        assert_eq!(
            fit(
                Rect {
                    x: 0,
                    y: 0,
                    width: 1081,
                    height: 801
                },
                1920,
                1080
            )
            .unwrap(),
            (1080, 800)
        );
        assert_eq!(
            fit(
                Rect {
                    x: 0,
                    y: 0,
                    width: 3840,
                    height: 2160
                },
                1920,
                1080
            )
            .unwrap(),
            (1920, 1080)
        );
    }
    #[test]
    fn physical_display_is_not_assumed_to_be_virtual_desktop_origin() {
        let source = Rect {
            x: -1920,
            y: 0,
            width: 1920,
            height: 1080,
        };
        let desktop = Rect {
            x: -1920,
            y: 0,
            width: 4480,
            height: 1440,
        };
        assert_eq!(
            absolute(source, desktop, (1920, 1080), 0, 0).unwrap(),
            (0, 0)
        );
        let (x, y) = absolute(source, desktop, (1920, 1080), 65535, 65535).unwrap();
        assert!((x - 28078).abs() < 2 && (y - 49141).abs() < 2);
    }
    #[test]
    fn padded_video_edges_cannot_click_another_window() {
        let source = Rect {
            x: 100,
            y: 100,
            width: 1000,
            height: 500,
        };
        let desktop = Rect {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
        };
        assert!(absolute(source, desktop, (1000, 1000), 32768, 0).is_err());
        assert!(absolute(source, desktop, (1000, 1000), 32768, 32768).is_ok());
    }
}
