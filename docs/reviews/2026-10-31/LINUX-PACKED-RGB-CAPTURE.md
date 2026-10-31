# Linux mapped RGB capture repair

The current PipeWire consumer advertised only NV12/I420 while requesting mapped CPU buffers. KWin exposes BGRA/BGRx for its shared-memory stream. Those capabilities had no common format, so a successful system selection could still produce an empty source list.

Primary implementation reference: [KWin ScreenCastStream::buildFormats/buildFormat](https://github.com/KDE/kwin/blob/master/src/plugins/screencast/screencaststream.cpp). This is source evidence; HO5 first-frame acceptance still requires its real system selection.

The existing PipeWire worker now negotiates mapped BGRA/BGRx/RGBA/RGBx with explicit full-range RGB, sRGB transfer and BT.709 primaries. The actual negotiated metadata must match that contract before encoding. Unspecified source metadata remains an error; YUV metadata keeps its existing exact handling.

The existing FFmpeg scaling pass converts that declared RGB to limited-range BT.709 YUV while preserving sRGB transfer and BT.709 primaries in HEVC. This adds no Rust pixel conversion loop or additional image staging. The one pending raw frame and compressed-frame ordering remain the same.

Packed rows retain their real stride, checked chunk bounds and crop. Odd RGB source dimensions are valid; the existing encoder output sizing supplies even HEVC dimensions.

Validation:

- Real FFmpeg/libx265 encoding, independent ffprobe SPS metadata and RGB decode for all four packed channel orders passed. Black/white and colored quadrant samples stayed within 6 code values after lossy YUV420 encoding.
- Existing six-frame sequence/timing and compressed access-unit checks run for each RGB format.
- Unknown RGB color descriptions remain rejected; existing unknown YUV matrix preservation remains covered.
- Padded/offset/cropped odd RGB rows and real HO5 native compilation are checked separately.

The current Linux encoder still uses the established libx265 implementation. This fix does not establish GPU encoding performance or physical presentation. Those require measured device acceptance.
