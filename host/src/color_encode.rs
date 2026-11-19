//! Host color profile negotiation, platform HEVC profile mapping, and HDR-to-SDR tone mapping.
//!
//! Bridges [`protocol::color::ColorProfile`] with host hardware encoder capabilities
//! (macOS VideoToolbox, Windows NVENC/AMF/QSV, Linux VAAPI/NVENC & PipeWire SPA) and
//! viewer display capabilities, with deterministic fallbacks (`Yuv444 -> Yuv420`, `HDR -> SDR`).

use protocol::color::{
    ChromaSubsampling, ColorBitDepth, ColorPrimaries, ColorProfile, HdrTransfer,
};
use serde::{Deserialize, Serialize};

/// Hardware/software encoder color capabilities reported by the host platform.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub struct EncoderColorCapabilities {
    pub supports_yuv444: bool,
    pub supports_10bit: bool,
    pub supports_444_10bit: bool,
    pub supports_pq_hdr10: bool,
    pub supports_hlg: bool,
    pub supports_bt2020: bool,
}

impl Default for EncoderColorCapabilities {
    fn default() -> Self {
        Self::sdr_baseline()
    }
}

impl EncoderColorCapabilities {
    /// Universal baseline HEVC Main (8-bit 4:2:0 SDR BT.709).
    pub const fn sdr_baseline() -> Self {
        Self {
            supports_yuv444: false,
            supports_10bit: false,
            supports_444_10bit: false,
            supports_pq_hdr10: false,
            supports_hlg: false,
            supports_bt2020: false,
        }
    }

    /// Apple Silicon VideoToolbox HEVC capabilities (Main, Main10, Main444, Main44410, HDR10/HLG).
    pub const fn apple_silicon_videotoolbox() -> Self {
        Self {
            supports_yuv444: true,
            supports_10bit: true,
            supports_444_10bit: true,
            supports_pq_hdr10: true,
            supports_hlg: true,
            supports_bt2020: true,
        }
    }

    /// NVIDIA NVENC (Turing / RTX 20-series and newer) HEVC RExt + Main10 capabilities.
    pub const fn nvenc_turing_plus() -> Self {
        Self {
            supports_yuv444: true,
            supports_10bit: true,
            supports_444_10bit: true,
            supports_pq_hdr10: true,
            supports_hlg: true,
            supports_bt2020: true,
        }
    }

    /// AMD AMF (RDNA 2/3) HEVC capabilities (10-bit HDR10/HLG, 4:2:0 only on hardware).
    pub const fn amf_rdna() -> Self {
        Self {
            supports_yuv444: false,
            supports_10bit: true,
            supports_444_10bit: false,
            supports_pq_hdr10: true,
            supports_hlg: true,
            supports_bt2020: true,
        }
    }

    /// Intel Quick Sync Video (Gen 12 / Xe and newer) HEVC RExt + Main10 capabilities.
    pub const fn qsv_gen12_plus() -> Self {
        Self {
            supports_yuv444: true,
            supports_10bit: true,
            supports_444_10bit: true,
            supports_pq_hdr10: true,
            supports_hlg: true,
            supports_bt2020: true,
        }
    }

    /// Modern Linux VAAPI (Intel Xe / AMD RDNA) typical capabilities (10-bit 4:2:0 HDR).
    pub const fn vaapi_hdr10() -> Self {
        Self {
            supports_yuv444: false,
            supports_10bit: true,
            supports_444_10bit: false,
            supports_pq_hdr10: true,
            supports_hlg: true,
            supports_bt2020: true,
        }
    }

    /// Returns `true` if this encoder can natively emit `profile` without downgrading.
    pub fn supports_profile(&self, profile: &ColorProfile) -> bool {
        if profile.validate().is_err() {
            return false;
        }
        if profile.chroma.is_444() {
            if !self.supports_yuv444 {
                return false;
            }
            if profile.bit_depth.is_10bit() && !self.supports_444_10bit {
                return false;
            }
        }
        if profile.bit_depth.is_10bit() && !self.supports_10bit {
            return false;
        }
        if profile.primaries == ColorPrimaries::Bt2020 && !self.supports_bt2020 {
            return false;
        }
        match profile.transfer {
            HdrTransfer::Sdr => true,
            HdrTransfer::PqSt2084 => self.supports_pq_hdr10,
            HdrTransfer::Hlg => self.supports_hlg,
        }
    }
}

/// Viewer display and decoder color capabilities advertised by the client.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientDisplayCapabilities {
    pub supports_yuv444_decode: bool,
    pub supports_10bit_decode: bool,
    pub supports_pq_hdr10: bool,
    pub supports_hlg: bool,
    pub supports_bt2020: bool,
    /// Client display peak luminance in nits (e.g. 100 for SDR, 600–1600 for HDR).
    pub max_luminance_nits: u16,
}

impl Default for ClientDisplayCapabilities {
    fn default() -> Self {
        Self::sdr_8bit()
    }
}

impl ClientDisplayCapabilities {
    /// Standard 8-bit SDR 4:2:0 client display.
    pub const fn sdr_8bit() -> Self {
        Self {
            supports_yuv444_decode: false,
            supports_10bit_decode: false,
            supports_pq_hdr10: false,
            supports_hlg: false,
            supports_bt2020: false,
            max_luminance_nits: 100,
        }
    }

    /// SDR client with hardware/software YUV 4:4:4 decoding support.
    pub const fn sdr_444() -> Self {
        Self {
            supports_yuv444_decode: true,
            supports_10bit_decode: false,
            supports_pq_hdr10: false,
            supports_hlg: false,
            supports_bt2020: false,
            max_luminance_nits: 100,
        }
    }

    /// HDR10-capable display with 10-bit 4:2:0 decoding.
    pub const fn hdr10_display(max_luminance_nits: u16) -> Self {
        Self {
            supports_yuv444_decode: false,
            supports_10bit_decode: true,
            supports_pq_hdr10: true,
            supports_hlg: true,
            supports_bt2020: true,
            max_luminance_nits,
        }
    }

    /// Full-featured HDR10 + YUV 4:4:4 display/decoder (e.g. Apple Silicon / desktop GPU).
    pub const fn hdr10_444_display(max_luminance_nits: u16) -> Self {
        Self {
            supports_yuv444_decode: true,
            supports_10bit_decode: true,
            supports_pq_hdr10: true,
            supports_hlg: true,
            supports_bt2020: true,
            max_luminance_nits,
        }
    }
}

/// Deterministic fallback step taken during color negotiation.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorFallbackReason {
    /// Requested profile failed standards validation and was reset to baseline SDR 4:2:0.
    InvalidRequestedProfile,
    /// HDR (PQ ST 2084 / HLG) was downgraded to SDR (tone mapping required if source is HDR).
    HdrDowngradedToSdr,
    /// 10-bit depth was downgraded to 8-bit.
    BitDepthDowngradedTo8Bit,
    /// BT.2020 wide gamut was downgraded to BT.709.
    PrimariesDowngradedToBt709,
    /// YUV 4:4:4 chroma was downgraded to YUV 4:2:0.
    ChromaDowngradedTo420,
}

/// Outcome of negotiating a requested [`ColorProfile`] against host encoder and client display.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ColorNegotiation {
    pub requested: ColorProfile,
    pub negotiated: ColorProfile,
    pub fallbacks: Vec<ColorFallbackReason>,
    /// `true` when the requested/source stream is HDR but the negotiated wire stream is SDR,
    /// indicating the host should apply PQ/HLG-to-SDR tone mapping before encoding.
    pub requires_hdr_to_sdr_tonemap: bool,
}

impl ColorNegotiation {
    /// Deterministically negotiate `requested` against `encoder` and `client` capabilities.
    ///
    /// Fallback order:
    /// 1. Invalid requested profile -> baseline SDR 4:2:0.
    /// 2. HDR transfer (`PqSt2084` / `Hlg`) -> `Sdr` when either endpoint lacks HDR/10-bit/BT.2020.
    /// 3. Bit depth `Bit10` -> `Bit8` when either endpoint lacks 10-bit support.
    /// 4. Primaries `Bt2020` -> `Bt709` when either endpoint lacks BT.2020 support.
    /// 5. Chroma `Yuv444` -> `Yuv420` when either endpoint lacks 4:4:4 (or 4:4:4 10-bit) support.
    pub fn negotiate(
        requested: &ColorProfile,
        encoder: &EncoderColorCapabilities,
        client: &ClientDisplayCapabilities,
    ) -> Self {
        if requested.validate().is_err() {
            return Self {
                requested: *requested,
                negotiated: ColorProfile::sdr_420(),
                fallbacks: vec![ColorFallbackReason::InvalidRequestedProfile],
                requires_hdr_to_sdr_tonemap: false,
            };
        }

        let mut fallbacks = Vec::new();

        let can_444 = encoder.supports_yuv444 && client.supports_yuv444_decode;
        let can_10bit = encoder.supports_10bit && client.supports_10bit_decode;
        let can_bt2020 = encoder.supports_bt2020 && client.supports_bt2020;
        let can_pq = encoder.supports_pq_hdr10 && client.supports_pq_hdr10 && can_10bit && can_bt2020;
        let can_hlg = encoder.supports_hlg && client.supports_hlg && can_10bit && can_bt2020;
        let can_444_10bit = can_444 && can_10bit && encoder.supports_444_10bit;

        // 1. Transfer (HDR -> SDR fallback)
        let mut requires_hdr_to_sdr_tonemap = false;
        let transfer = match requested.transfer {
            HdrTransfer::Sdr => HdrTransfer::Sdr,
            HdrTransfer::PqSt2084 => {
                if can_pq {
                    HdrTransfer::PqSt2084
                } else {
                    fallbacks.push(ColorFallbackReason::HdrDowngradedToSdr);
                    requires_hdr_to_sdr_tonemap = true;
                    HdrTransfer::Sdr
                }
            }
            HdrTransfer::Hlg => {
                if can_hlg {
                    HdrTransfer::Hlg
                } else {
                    fallbacks.push(ColorFallbackReason::HdrDowngradedToSdr);
                    requires_hdr_to_sdr_tonemap = true;
                    HdrTransfer::Sdr
                }
            }
        };

        // 2. Bit depth
        let bit_depth = if transfer.is_hdr() {
            ColorBitDepth::Bit10
        } else if requested.bit_depth.is_10bit() {
            if can_10bit {
                ColorBitDepth::Bit10
            } else {
                fallbacks.push(ColorFallbackReason::BitDepthDowngradedTo8Bit);
                ColorBitDepth::Bit8
            }
        } else {
            ColorBitDepth::Bit8
        };

        // 3. Color primaries
        let primaries = if transfer.is_hdr() {
            ColorPrimaries::Bt2020
        } else if requested.primaries == ColorPrimaries::Bt2020 {
            if can_bt2020 {
                ColorPrimaries::Bt2020
            } else {
                fallbacks.push(ColorFallbackReason::PrimariesDowngradedToBt709);
                ColorPrimaries::Bt709
            }
        } else {
            ColorPrimaries::Bt709
        };

        // 4. Chroma subsampling (Yuv444 -> Yuv420 fallback)
        let chroma = if requested.chroma.is_444() {
            let ok = if bit_depth.is_10bit() {
                can_444_10bit
            } else {
                can_444
            };
            if ok {
                ChromaSubsampling::Yuv444
            } else {
                fallbacks.push(ColorFallbackReason::ChromaDowngradedTo420);
                ChromaSubsampling::Yuv420
            }
        } else {
            ChromaSubsampling::Yuv420
        };

        // 5. HDR mastering metadata is stripped whenever transfer is SDR
        let hdr_metadata = if transfer.is_hdr() {
            requested.hdr_metadata
        } else {
            None
        };

        let negotiated = ColorProfile {
            chroma,
            bit_depth,
            transfer,
            primaries,
            hdr_metadata,
        };
        debug_assert!(negotiated.validate().is_ok());

        Self {
            requested: *requested,
            negotiated,
            fallbacks,
            requires_hdr_to_sdr_tonemap,
        }
    }

    pub fn is_exact_match(&self) -> bool {
        self.fallbacks.is_empty() && self.negotiated == self.requested
    }

    pub fn was_downgraded(&self) -> bool {
        !self.fallbacks.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Platform Encoder Profile Mappings
// ---------------------------------------------------------------------------

/// macOS VideoToolbox HEVC compression session profile and colorimetry parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VideoToolboxHevcProfile {
    /// `kVTProfileLevel_HEVC_*` constant name.
    pub profile_level: &'static str,
    /// CoreVideo pixel format FourCC (`420v`, `x420`, `444v`, `x444`).
    pub pixel_format_fourcc: &'static str,
    /// `kCVImageBufferColorPrimaries_*` constant name.
    pub color_primaries: &'static str,
    /// `kCVImageBufferTransferFunction_*` constant name.
    pub transfer_function: &'static str,
    /// `kCVImageBufferYCbCrMatrix_*` constant name.
    pub ycbcr_matrix: &'static str,
}

impl VideoToolboxHevcProfile {
    pub const fn from_color_profile(profile: &ColorProfile) -> Self {
        let (profile_level, pixel_format_fourcc) = match (profile.chroma, profile.bit_depth) {
            (ChromaSubsampling::Yuv420, ColorBitDepth::Bit8) => ("HEVC_Main_AutoLevel", "420v"),
            (ChromaSubsampling::Yuv420, ColorBitDepth::Bit10) => ("HEVC_Main10_AutoLevel", "x420"),
            (ChromaSubsampling::Yuv444, ColorBitDepth::Bit8) => ("HEVC_Main444_AutoLevel", "444v"),
            (ChromaSubsampling::Yuv444, ColorBitDepth::Bit10) => {
                ("HEVC_Main44410_AutoLevel", "x444")
            }
        };

        let (color_primaries, ycbcr_matrix) = match profile.primaries {
            ColorPrimaries::Bt709 => ("ITU_R_709_2", "ITU_R_709_2"),
            ColorPrimaries::Bt2020 => ("ITU_R_2020", "ITU_R_2020"),
        };

        let transfer_function = match profile.transfer {
            HdrTransfer::Sdr => "ITU_R_709_2",
            HdrTransfer::PqSt2084 => "SMPTE_ST_2084_PQ",
            HdrTransfer::Hlg => "ITU_R_2100_HLG",
        };

        Self {
            profile_level,
            pixel_format_fourcc,
            color_primaries,
            transfer_function,
            ycbcr_matrix,
        }
    }
}

/// Hardware encoder vendor on Windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowsEncoderVendor {
    Nvenc,
    Amf,
    Qsv,
}

/// Windows NVENC / AMF / QSV HEVC encoder profile and colorimetry mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowsHevcProfile {
    pub vendor: WindowsEncoderVendor,
    /// FFmpeg encoder name (`hevc_nvenc`, `hevc_amf`, `hevc_qsv`).
    pub ffmpeg_encoder: &'static str,
    /// HEVC profile string (`main`, `main10`, `rext`).
    pub hevc_profile: &'static str,
    /// FFmpeg input pixel format (`nv12`, `p010le`, `yuv444p`, `yuv444p16le`).
    pub ffmpeg_pix_fmt: &'static str,
    /// Corresponding DXGI surface format name.
    pub dxgi_format: &'static str,
    /// FFmpeg `-color_primaries` value.
    pub color_primaries: &'static str,
    /// FFmpeg `-color_trc` value.
    pub color_trc: &'static str,
    /// FFmpeg `-colorspace` value.
    pub colorspace: &'static str,
}

impl WindowsHevcProfile {
    pub const fn from_color_profile(
        vendor: WindowsEncoderVendor,
        profile: &ColorProfile,
    ) -> Self {
        let ffmpeg_encoder = match vendor {
            WindowsEncoderVendor::Nvenc => "hevc_nvenc",
            WindowsEncoderVendor::Amf => "hevc_amf",
            WindowsEncoderVendor::Qsv => "hevc_qsv",
        };

        let (hevc_profile, ffmpeg_pix_fmt, dxgi_format) = match (profile.chroma, profile.bit_depth)
        {
            (ChromaSubsampling::Yuv420, ColorBitDepth::Bit8) => ("main", "nv12", "DXGI_FORMAT_NV12"),
            (ChromaSubsampling::Yuv420, ColorBitDepth::Bit10) => {
                ("main10", "p010le", "DXGI_FORMAT_P010")
            }
            (ChromaSubsampling::Yuv444, ColorBitDepth::Bit8) => {
                ("rext", "yuv444p", "DXGI_FORMAT_AYUV")
            }
            (ChromaSubsampling::Yuv444, ColorBitDepth::Bit10) => {
                ("rext", "yuv444p16le", "DXGI_FORMAT_Y410")
            }
        };

        let (color_primaries, colorspace) = match profile.primaries {
            ColorPrimaries::Bt709 => ("bt709", "bt709"),
            ColorPrimaries::Bt2020 => ("bt2020", "bt2020nc"),
        };

        let color_trc = match profile.transfer {
            HdrTransfer::Sdr => "bt709",
            HdrTransfer::PqSt2084 => "smpte2084",
            HdrTransfer::Hlg => "arib-std-b67",
        };

        Self {
            vendor,
            ffmpeg_encoder,
            hevc_profile,
            ffmpeg_pix_fmt,
            dxgi_format,
            color_primaries,
            color_trc,
            colorspace,
        }
    }
}

/// Hardware encoder backend on Linux.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinuxEncoderBackend {
    Vaapi,
    Nvenc,
}

/// PipeWire SPA video colorimetry identifiers (`spa/param/video/color.h`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PipeWireSpaColorMapping {
    pub transfer_name: &'static str,
    /// `enum spa_video_transfer_function` numeric value.
    pub transfer_id: u32,
    pub primaries_name: &'static str,
    /// `enum spa_video_color_primaries` numeric value.
    pub primaries_id: u32,
    pub matrix_name: &'static str,
    /// `enum spa_video_color_matrix` numeric value.
    pub matrix_id: u32,
}

impl PipeWireSpaColorMapping {
    pub const fn from_color_profile(profile: &ColorProfile) -> Self {
        let (transfer_name, transfer_id) = match profile.transfer {
            HdrTransfer::Sdr => ("SPA_VIDEO_TRANSFER_BT709", 5),
            HdrTransfer::PqSt2084 => ("SPA_VIDEO_TRANSFER_SMPTE2084", 12),
            HdrTransfer::Hlg => ("SPA_VIDEO_TRANSFER_ARIB_STD_B67", 13),
        };

        let (primaries_name, primaries_id, matrix_name, matrix_id) = match profile.primaries {
            ColorPrimaries::Bt709 => (
                "SPA_VIDEO_COLOR_PRIMARIES_BT709",
                1,
                "SPA_VIDEO_COLOR_MATRIX_BT709",
                3,
            ),
            ColorPrimaries::Bt2020 => (
                "SPA_VIDEO_COLOR_PRIMARIES_BT2020",
                7,
                "SPA_VIDEO_COLOR_MATRIX_BT2020",
                6,
            ),
        };

        Self {
            transfer_name,
            transfer_id,
            primaries_name,
            primaries_id,
            matrix_name,
            matrix_id,
        }
    }
}

/// Linux VAAPI / NVENC HEVC encoder profile + PipeWire SPA color mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinuxHevcProfile {
    pub backend: LinuxEncoderBackend,
    pub ffmpeg_encoder: &'static str,
    pub hevc_profile: &'static str,
    /// libva profile symbol name (`VAProfileHEVC*`).
    pub vaapi_profile: &'static str,
    pub pixel_format: &'static str,
    pub pipewire_spa: PipeWireSpaColorMapping,
}

impl LinuxHevcProfile {
    pub const fn from_color_profile(
        backend: LinuxEncoderBackend,
        profile: &ColorProfile,
    ) -> Self {
        let ffmpeg_encoder = match backend {
            LinuxEncoderBackend::Vaapi => "hevc_vaapi",
            LinuxEncoderBackend::Nvenc => "hevc_nvenc",
        };

        let (hevc_profile, vaapi_profile, pixel_format) = match (profile.chroma, profile.bit_depth)
        {
            (ChromaSubsampling::Yuv420, ColorBitDepth::Bit8) => {
                ("main", "VAProfileHEVCMain", "nv12")
            }
            (ChromaSubsampling::Yuv420, ColorBitDepth::Bit10) => {
                ("main10", "VAProfileHEVCMain10", "p010le")
            }
            (ChromaSubsampling::Yuv444, ColorBitDepth::Bit8) => {
                ("rext", "VAProfileHEVCMain444", "yuv444p")
            }
            (ChromaSubsampling::Yuv444, ColorBitDepth::Bit10) => {
                ("rext", "VAProfileHEVCMain444_10", "yuv444p10le")
            }
        };

        Self {
            backend,
            ffmpeg_encoder,
            hevc_profile,
            vaapi_profile,
            pixel_format,
            pipewire_spa: PipeWireSpaColorMapping::from_color_profile(profile),
        }
    }
}

// ---------------------------------------------------------------------------
// PQ / HLG -> SDR Tone-Mapping Helpers (Reinhard & ACES Filmic)
// ---------------------------------------------------------------------------

// SMPTE ST 2084 (PQ) constants
const PQ_M1: f32 = 2610.0 / 16384.0;
const PQ_M2: f32 = (2523.0 / 4096.0) * 128.0;
const PQ_C1: f32 = 3424.0 / 4096.0;
const PQ_C2: f32 = (2413.0 / 4096.0) * 32.0;
const PQ_C3: f32 = (2392.0 / 4096.0) * 32.0;
const PQ_PEAK_NITS: f32 = 10_000.0;

// ARIB STD-B67 (HLG) constants
const HLG_A: f32 = 0.17883277;
const HLG_B: f32 = 1.0 - 4.0 * HLG_A;
const HLG_C: f32 = 0.55991073;

/// Tone-mapping operator used to compress HDR linear luminance into SDR `[0, 1]`.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToneMapOperator {
    /// Simple global Reinhard curve: `x / (1 + x)`.
    Reinhard,
    /// Narkowicz (2015) ACES Filmic approximation with smooth highlight roll-off.
    AcesFilmic,
}

/// Configuration for PQ/HLG-to-SDR tone mapping.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ToneMapConfig {
    pub operator: ToneMapOperator,
    pub source_transfer: HdrTransfer,
    /// Peak mastering luminance of the HDR source in nits (used by HLG system gamma).
    pub source_peak_nits: f32,
    /// Reference diffuse white in nits (ITU-R BT.2408 recommends 203.0 nits).
    pub sdr_white_nits: f32,
    /// Linear exposure multiplier applied prior to tone compression.
    pub exposure: f32,
}

impl Default for ToneMapConfig {
    fn default() -> Self {
        Self {
            operator: ToneMapOperator::AcesFilmic,
            source_transfer: HdrTransfer::PqSt2084,
            source_peak_nits: 1000.0,
            sdr_white_nits: 203.0,
            exposure: 1.0,
        }
    }
}

/// SMPTE ST 2084 EOTF: maps normalized PQ signal `[0.0, 1.0]` to linear luminance in nits `[0.0, 10000.0]`.
pub fn pq_eotf_nits(pq_signal: f32) -> f32 {
    let e = pq_signal.clamp(0.0, 1.0);
    if e <= 0.0 {
        return 0.0;
    }
    let p = e.powf(1.0 / PQ_M2);
    let num = (p - PQ_C1).max(0.0);
    let den = (PQ_C2 - PQ_C3 * p).max(1e-12);
    (num / den).powf(1.0 / PQ_M1) * PQ_PEAK_NITS
}

/// SMPTE ST 2084 OETF: maps linear luminance in nits `[0.0, 10000.0]` to normalized PQ signal `[0.0, 1.0]`.
pub fn pq_oetf_from_nits(nits: f32) -> f32 {
    let y = (nits / PQ_PEAK_NITS).clamp(0.0, 1.0);
    if y <= 0.0 {
        return 0.0;
    }
    let ym1 = y.powf(PQ_M1);
    ((PQ_C1 + PQ_C2 * ym1) / (1.0 + PQ_C3 * ym1)).powf(PQ_M2)
}

/// ARIB STD-B67 inverse OETF: maps normalized HLG signal `[0.0, 1.0]` to scene-referred linear `[0.0, 1.0]`.
pub fn hlg_inverse_oetf(hlg_signal: f32) -> f32 {
    let e = hlg_signal.clamp(0.0, 1.0);
    if e <= 0.5 {
        (e * e) / 3.0
    } else {
        (((e - HLG_C) / HLG_A).exp() + HLG_B) / 12.0
    }
}

/// ITU-R BT.2100 HLG system gamma as a function of display peak luminance `peak_nits`.
pub fn hlg_system_gamma(peak_nits: f32) -> f32 {
    let peak = peak_nits.max(100.0);
    (1.2 + 0.42 * (peak / 1000.0).log10()).clamp(1.0, 1.8)
}

/// ARIB STD-B67 / BT.2100 HLG scalar EOTF: maps HLG signal `[0.0, 1.0]` to display nits.
pub fn hlg_eotf_nits(hlg_signal: f32, peak_nits: f32) -> f32 {
    let scene = hlg_inverse_oetf(hlg_signal);
    if scene <= 0.0 {
        return 0.0;
    }
    let peak = peak_nits.max(100.0);
    peak * scene.powf(hlg_system_gamma(peak))
}

/// Global Reinhard tone-mapping curve on non-negative linear light.
pub fn reinhard_tonemap(linear: f32) -> f32 {
    let x = linear.max(0.0);
    x / (1.0 + x)
}

/// Narkowicz (2015) ACES Filmic tone-mapping curve on non-negative linear light.
pub fn aces_filmic_tonemap(linear: f32) -> f32 {
    let x = linear.max(0.0);
    let num = x * (2.51 * x + 0.03);
    let den = x * (2.43 * x + 0.59) + 0.14;
    (num / den).clamp(0.0, 1.0)
}

/// sRGB OETF: encodes linear light `[0.0, 1.0]` to gamma-encoded SDR `[0.0, 1.0]`.
pub fn srgb_oetf(linear: f32) -> f32 {
    let v = linear.clamp(0.0, 1.0);
    if v <= 0.003_130_8 {
        12.92 * v
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    }
}

/// Stateful helper that maps PQ or HLG HDR signals to SDR gamma-encoded values.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ToneMapper {
    config: ToneMapConfig,
}

impl ToneMapper {
    pub const fn new(config: ToneMapConfig) -> Self {
        Self { config }
    }

    pub const fn config(&self) -> ToneMapConfig {
        self.config
    }

    fn apply_operator(&self, linear_scaled: f32) -> f32 {
        match self.config.operator {
            ToneMapOperator::Reinhard => reinhard_tonemap(linear_scaled),
            ToneMapOperator::AcesFilmic => aces_filmic_tonemap(linear_scaled),
        }
    }

    /// Map a single normalized HDR signal channel `[0.0, 1.0]` to SDR `[0.0, 1.0]`.
    pub fn map_scalar(&self, signal: f32) -> f32 {
        let clamped = signal.clamp(0.0, 1.0);
        let nits = match self.config.source_transfer {
            HdrTransfer::Sdr => return clamped,
            HdrTransfer::PqSt2084 => pq_eotf_nits(clamped),
            HdrTransfer::Hlg => hlg_eotf_nits(clamped, self.config.source_peak_nits),
        };
        let white = self.config.sdr_white_nits.max(1.0);
        let exposure = self.config.exposure.max(0.0);
        let linear_sdr = self.apply_operator((nits / white) * exposure);
        srgb_oetf(linear_sdr)
    }

    /// Map an HDR RGB triplet `[r, g, b]` in `[0.0, 1.0]` to SDR gamma-encoded RGB in `[0.0, 1.0]`.
    pub fn map_rgb(&self, rgb: [f32; 3]) -> [f32; 3] {
        let [r, g, b] = [
            rgb[0].clamp(0.0, 1.0),
            rgb[1].clamp(0.0, 1.0),
            rgb[2].clamp(0.0, 1.0),
        ];
        let white = self.config.sdr_white_nits.max(1.0);
        let exposure = self.config.exposure.max(0.0);

        let linear_nits = match self.config.source_transfer {
            HdrTransfer::Sdr => return [r, g, b],
            HdrTransfer::PqSt2084 => [pq_eotf_nits(r), pq_eotf_nits(g), pq_eotf_nits(b)],
            HdrTransfer::Hlg => {
                // BT.2100 HLG RGB OOTF preserves chromaticity via BT.2020 luminance Ys.
                let sr = hlg_inverse_oetf(r);
                let sg = hlg_inverse_oetf(g);
                let sb = hlg_inverse_oetf(b);
                let ys = (0.2627 * sr + 0.6780 * sg + 0.0593 * sb).max(0.0);
                let peak = self.config.source_peak_nits.max(100.0);
                let gain = if ys > 0.0 {
                    peak * ys.powf(hlg_system_gamma(peak) - 1.0)
                } else {
                    0.0
                };
                [sr * gain, sg * gain, sb * gain]
            }
        };

        [
            srgb_oetf(self.apply_operator((linear_nits[0] / white) * exposure)),
            srgb_oetf(self.apply_operator((linear_nits[1] / white) * exposure)),
            srgb_oetf(self.apply_operator((linear_nits[2] / white) * exposure)),
        ]
    }

    /// Build a 1D lookup table of `entries` samples spanning `[0.0, 1.0]`.
    pub fn build_lut(&self, entries: usize) -> Vec<f32> {
        if entries == 0 {
            return Vec::new();
        }
        if entries == 1 {
            return vec![self.map_scalar(0.0)];
        }
        let denom = (entries - 1) as f32;
        (0..entries)
            .map(|i| self.map_scalar(i as f32 / denom))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::color::HdrMasteringMetadata;

    #[test]
    fn negotiation_preserves_full_hdr10_444_when_both_endpoints_support_it() {
        let req = ColorProfile::hdr10_444(Some(HdrMasteringMetadata::hdr10_default()));
        let res = ColorNegotiation::negotiate(
            &req,
            &EncoderColorCapabilities::apple_silicon_videotoolbox(),
            &ClientDisplayCapabilities::hdr10_444_display(1000),
        );
        assert!(res.is_exact_match());
        assert!(!res.was_downgraded());
        assert!(!res.requires_hdr_to_sdr_tonemap);
        assert_eq!(res.negotiated, req);
    }

    #[test]
    fn negotiation_falls_back_yuv444_to_yuv420_when_client_or_encoder_lacks_444() {
        let req = ColorProfile::sdr_444();
        let res = ColorNegotiation::negotiate(
            &req,
            &EncoderColorCapabilities::apple_silicon_videotoolbox(),
            &ClientDisplayCapabilities::sdr_8bit(),
        );
        assert_eq!(res.negotiated, ColorProfile::sdr_420());
        assert_eq!(res.fallbacks, vec![ColorFallbackReason::ChromaDowngradedTo420]);
        assert!(!res.requires_hdr_to_sdr_tonemap);

        // AMD AMF supports 10-bit HDR10 but not 4:4:4
        let req_hdr_444 = ColorProfile::hdr10_444(Some(HdrMasteringMetadata::hdr10_default()));
        let res_amf = ColorNegotiation::negotiate(
            &req_hdr_444,
            &EncoderColorCapabilities::amf_rdna(),
            &ClientDisplayCapabilities::hdr10_444_display(1000),
        );
        assert_eq!(
            res_amf.negotiated,
            ColorProfile::hdr10_420(Some(HdrMasteringMetadata::hdr10_default()))
        );
        assert_eq!(
            res_amf.fallbacks,
            vec![ColorFallbackReason::ChromaDowngradedTo420]
        );
        assert!(!res_amf.requires_hdr_to_sdr_tonemap);
    }

    #[test]
    fn negotiation_falls_back_hdr10_to_sdr_and_flags_tonemap_for_sdr_client() {
        let req = ColorProfile::hdr10_444(Some(HdrMasteringMetadata::hdr10_default()));
        let res = ColorNegotiation::negotiate(
            &req,
            &EncoderColorCapabilities::nvenc_turing_plus(),
            &ClientDisplayCapabilities::sdr_444(),
        );
        assert_eq!(res.negotiated, ColorProfile::sdr_444());
        assert!(res.requires_hdr_to_sdr_tonemap);
        assert!(res.fallbacks.contains(&ColorFallbackReason::HdrDowngradedToSdr));
        assert!(res.fallbacks.contains(&ColorFallbackReason::BitDepthDowngradedTo8Bit));
        assert!(res.fallbacks.contains(&ColorFallbackReason::PrimariesDowngradedToBt709));
        assert_eq!(res.negotiated.hdr_metadata, None);
        assert_eq!(res.negotiated.validate(), Ok(()));
    }

    #[test]
    fn negotiation_handles_encoder_without_444_10bit_combo() {
        let partial_encoder = EncoderColorCapabilities {
            supports_yuv444: true,
            supports_10bit: true,
            supports_444_10bit: false,
            supports_pq_hdr10: true,
            supports_hlg: true,
            supports_bt2020: true,
        };
        let req = ColorProfile::hdr10_444(None);
        let res = ColorNegotiation::negotiate(
            &req,
            &partial_encoder,
            &ClientDisplayCapabilities::hdr10_444_display(1000),
        );
        // Keeps HDR10 10-bit, downgrades chroma to 4:2:0
        assert_eq!(res.negotiated, ColorProfile::hdr10_420(None));
        assert_eq!(res.fallbacks, vec![ColorFallbackReason::ChromaDowngradedTo420]);
    }

    #[test]
    fn negotiation_sanitizes_invalid_requested_profile() {
        let invalid = ColorProfile {
            chroma: ChromaSubsampling::Yuv444,
            bit_depth: ColorBitDepth::Bit8,
            transfer: HdrTransfer::PqSt2084,
            primaries: ColorPrimaries::Bt709,
            hdr_metadata: None,
        };
        let res = ColorNegotiation::negotiate(
            &invalid,
            &EncoderColorCapabilities::nvenc_turing_plus(),
            &ClientDisplayCapabilities::hdr10_444_display(1000),
        );
        assert_eq!(res.negotiated, ColorProfile::sdr_420());
        assert_eq!(
            res.fallbacks,
            vec![ColorFallbackReason::InvalidRequestedProfile]
        );
    }

    #[test]
    fn platform_profile_mappings_cover_macos_windows_and_linux() {
        // macOS VideoToolbox
        let vt_444 = VideoToolboxHevcProfile::from_color_profile(&ColorProfile::sdr_444());
        assert_eq!(vt_444.profile_level, "HEVC_Main444_AutoLevel");
        assert_eq!(vt_444.pixel_format_fourcc, "444v");
        assert_eq!(vt_444.transfer_function, "ITU_R_709_2");

        let vt_hdr = VideoToolboxHevcProfile::from_color_profile(&ColorProfile::hdr10_420(None));
        assert_eq!(vt_hdr.profile_level, "HEVC_Main10_AutoLevel");
        assert_eq!(vt_hdr.pixel_format_fourcc, "x420");
        assert_eq!(vt_hdr.color_primaries, "ITU_R_2020");
        assert_eq!(vt_hdr.transfer_function, "SMPTE_ST_2084_PQ");

        let vt_hdr_444 = VideoToolboxHevcProfile::from_color_profile(&ColorProfile::hlg_444(None));
        assert_eq!(vt_hdr_444.profile_level, "HEVC_Main44410_AutoLevel");
        assert_eq!(vt_hdr_444.transfer_function, "ITU_R_2100_HLG");

        // Windows NVENC / AMF / QSV
        let nv_rext = WindowsHevcProfile::from_color_profile(
            WindowsEncoderVendor::Nvenc,
            &ColorProfile::hdr10_444(None),
        );
        assert_eq!(nv_rext.ffmpeg_encoder, "hevc_nvenc");
        assert_eq!(nv_rext.hevc_profile, "rext");
        assert_eq!(nv_rext.dxgi_format, "DXGI_FORMAT_Y410");
        assert_eq!(nv_rext.color_trc, "smpte2084");

        let amf_main10 = WindowsHevcProfile::from_color_profile(
            WindowsEncoderVendor::Amf,
            &ColorProfile::hdr10_420(None),
        );
        assert_eq!(amf_main10.ffmpeg_encoder, "hevc_amf");
        assert_eq!(amf_main10.hevc_profile, "main10");
        assert_eq!(amf_main10.dxgi_format, "DXGI_FORMAT_P010");

        let qsv_444 = WindowsHevcProfile::from_color_profile(
            WindowsEncoderVendor::Qsv,
            &ColorProfile::sdr_444(),
        );
        assert_eq!(qsv_444.ffmpeg_encoder, "hevc_qsv");
        assert_eq!(qsv_444.hevc_profile, "rext");
        assert_eq!(qsv_444.dxgi_format, "DXGI_FORMAT_AYUV");

        // Linux VAAPI / NVENC + PipeWire SPA
        let vaapi_hdr = LinuxHevcProfile::from_color_profile(
            LinuxEncoderBackend::Vaapi,
            &ColorProfile::hdr10_420(None),
        );
        assert_eq!(vaapi_hdr.ffmpeg_encoder, "hevc_vaapi");
        assert_eq!(vaapi_hdr.vaapi_profile, "VAProfileHEVCMain10");
        assert_eq!(vaapi_hdr.pipewire_spa.transfer_name, "SPA_VIDEO_TRANSFER_SMPTE2084");
        assert_eq!(vaapi_hdr.pipewire_spa.transfer_id, 12);
        assert_eq!(vaapi_hdr.pipewire_spa.primaries_id, 7);

        let linux_nv_hlg = LinuxHevcProfile::from_color_profile(
            LinuxEncoderBackend::Nvenc,
            &ColorProfile::hlg_444(None),
        );
        assert_eq!(linux_nv_hlg.ffmpeg_encoder, "hevc_nvenc");
        assert_eq!(linux_nv_hlg.hevc_profile, "rext");
        assert_eq!(linux_nv_hlg.vaapi_profile, "VAProfileHEVCMain444_10");
        assert_eq!(
            linux_nv_hlg.pipewire_spa.transfer_name,
            "SPA_VIDEO_TRANSFER_ARIB_STD_B67"
        );
        assert_eq!(linux_nv_hlg.pipewire_spa.transfer_id, 13);
    }

    #[test]
    fn pq_oetf_and_eotf_roundtrip_reference_points() {
        for &nits in &[0.0_f32, 0.005, 10.0, 100.0, 203.0, 1000.0, 4000.0, 10_000.0] {
            let pq = pq_oetf_from_nits(nits);
            let recovered = pq_eotf_nits(pq);
            let tol = (nits * 1e-3).max(1e-3);
            assert!(
                (recovered - nits).abs() <= tol,
                "PQ roundtrip mismatch at {nits} nits: pq={pq}, recovered={recovered}"
            );
        }
        // Known reference: 100 nits is ~0.5081 in PQ
        assert!((pq_oetf_from_nits(100.0) - 0.5081).abs() < 0.001);
    }

    #[test]
    fn reinhard_and_aces_tonemappers_are_monotonic_and_bounded() {
        for operator in [ToneMapOperator::Reinhard, ToneMapOperator::AcesFilmic] {
            for transfer in [HdrTransfer::PqSt2084, HdrTransfer::Hlg] {
                let mapper = ToneMapper::new(ToneMapConfig {
                    operator,
                    source_transfer: transfer,
                    source_peak_nits: 1000.0,
                    sdr_white_nits: 203.0,
                    exposure: 1.0,
                });
                let lut = mapper.build_lut(257);
                assert_eq!(lut.len(), 257);
                assert_eq!(lut[0], 0.0);
                assert!(lut[256] > 0.85 && lut[256] <= 1.0);
                for pair in lut.windows(2) {
                    assert!(
                        pair[1] >= pair[0],
                        "LUT must be monotonically non-decreasing for {operator:?}/{transfer:?}: {} -> {}",
                        pair[0],
                        pair[1]
                    );
                    assert!(pair[1].is_finite() && (0.0..=1.0).contains(&pair[1]));
                }
            }
        }
    }

    #[test]
    fn tonemap_rgb_preserves_neutral_gray_and_sdr_passthrough() {
        let sdr_mapper = ToneMapper::new(ToneMapConfig {
            source_transfer: HdrTransfer::Sdr,
            ..Default::default()
        });
        assert_eq!(sdr_mapper.map_rgb([0.2, 0.5, 0.8]), [0.2, 0.5, 0.8]);

        let hlg_mapper = ToneMapper::new(ToneMapConfig {
            operator: ToneMapOperator::AcesFilmic,
            source_transfer: HdrTransfer::Hlg,
            source_peak_nits: 1000.0,
            sdr_white_nits: 203.0,
            exposure: 1.0,
        });
        let gray = hlg_mapper.map_rgb([0.6, 0.6, 0.6]);
        assert!((gray[0] - gray[1]).abs() < 1e-6);
        assert!((gray[1] - gray[2]).abs() < 1e-6);
        assert!(gray[0] > 0.0 && gray[0] <= 1.0);
    }
}
