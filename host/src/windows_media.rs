//! SPS verification shared by the native Windows screen and camera encoders.
use remote_core::hevc_sequence::VideoSignal;
pub(crate) fn validate_keyframe(nalu:&mut Vec<u8>,size:[u32;2],signal:VideoSignal)->Result<(),String> {
    // This is a correction of the known hardware MFT's SPS declaration for an
    // explicitly sRGB source, never a guess based on its dimensions.
    if signal.transfer==Some(13) {
        remote_core::hevc_sequence::set_srgb_transfer_for_matrix(nalu,signal.matrix.ok_or("Native YUV matrix is unavailable")?)
            .map_err(str::to_owned)?;
    }
    let sequence=remote_core::hevc_sequence::sequence_info(nalu).map_err(str::to_owned)?
        .ok_or("Native hardware keyframe has no SPS")?;
    if sequence.bit_depth!=8||sequence.visible[2..]!=size||sequence.signal!=Some(signal) {
        return Err("Native hardware HEVC dimensions/color differ from the actual GPU surface".into());
    }
    Ok(())
}
