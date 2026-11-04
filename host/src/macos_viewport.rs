//! NV12 IOSurface -> Metal crop/scale -> NV12 IOSurface. Luma/chroma remain in
//! their actual source range and matrix; no RGB conversion or CPU pixel readback.
use core_foundation::{base::{CFRelease,TCFType},boolean::CFBoolean,dictionary::CFDictionary,number::CFNumber,string::CFString};
use metal::{foreign_types::{ForeignType,ForeignTypeRef},Device,CommandQueue,ComputePipelineState,MTLCommandBufferStatus,MTLPixelFormat,MTLSize,TextureRef};
use protocol::viewport::FrameGeometry;
use std::{error::Error,ffi::c_void,ptr};
type Result<T> = std::result::Result<T,Box<dyn Error+Send+Sync>>;

#[link(name="CoreVideo",kind="framework")]
unsafe extern "C" {
    fn CVPixelBufferPoolCreate(a:*const c_void,attrs:*const c_void,pixels:*const c_void,out:*mut *mut c_void)->i32;
    fn CVPixelBufferPoolCreatePixelBufferWithAuxAttributes(a:*const c_void,pool:*mut c_void,attrs:*const c_void,out:*mut *mut c_void)->i32;
    fn CVPixelBufferGetPixelFormatType(buffer:*mut c_void)->u32;
    fn CVPixelBufferGetWidthOfPlane(buffer:*mut c_void,plane:usize)->usize;
    fn CVPixelBufferGetHeightOfPlane(buffer:*mut c_void,plane:usize)->usize;
    fn CVBufferRemoveAllAttachments(buffer:*mut c_void);
    fn CVBufferPropagateAttachments(source:*mut c_void,target:*mut c_void);
    fn CVBufferRemoveAttachment(buffer:*mut c_void,key:*const c_void);
    fn CVMetalTextureCacheCreate(a:*const c_void,attrs:*const c_void,device:*mut c_void,texture_attrs:*const c_void,out:*mut *mut c_void)->i32;
    fn CVMetalTextureCacheCreateTextureFromImage(a:*const c_void,cache:*mut c_void,image:*mut c_void,attrs:*const c_void,format:u64,width:usize,height:usize,plane:usize,out:*mut *mut c_void)->i32;
    fn CVMetalTextureGetTexture(texture:*mut c_void)->*mut metal::MTLTexture;
    static kCVPixelBufferWidthKey:*const c_void;
    static kCVPixelBufferHeightKey:*const c_void;
    static kCVPixelBufferPixelFormatTypeKey:*const c_void;
    static kCVPixelBufferIOSurfacePropertiesKey:*const c_void;
    static kCVPixelBufferMetalCompatibilityKey:*const c_void;
    static kCVPixelBufferPoolAllocationThresholdKey:*const c_void;
    static kCVMetalTextureUsage:*const c_void;
    static kCVImageBufferCleanApertureKey:*const c_void;
    static kCVImageBufferPreferredCleanApertureKey:*const c_void;
    static kCVImageBufferDisplayDimensionsKey:*const c_void;
}
pub(crate) struct PixelBuffer(pub(crate) *mut c_void);
impl Drop for PixelBuffer {fn drop(&mut self){unsafe{CFRelease(self.0);}}}
struct NativeRef(*mut c_void);
impl Drop for NativeRef {fn drop(&mut self){if !self.0.is_null(){unsafe{CFRelease(self.0);}}}}
impl NativeRef {fn empty()->Self {Self(ptr::null_mut())}}

pub(crate) struct Cropper {
    device:Device,
    queue:CommandQueue,
    pipeline:ComputePipelineState,
    cache:NativeRef,
    pool:NativeRef,
    format:Option<(u32,u32,u32)>,
}
unsafe impl Send for Cropper {}
unsafe impl Sync for Cropper {}
const SHADER:&str=r#"
#include <metal_stdlib>
using namespace metal;
kernel void viewport_nv12(texture2d<float,access::sample> sy [[texture(0)]],
                          texture2d<float,access::sample> sc [[texture(1)]],
                          texture2d<float,access::write> dy [[texture(2)]],
                          texture2d<float,access::write> dc [[texture(3)]],
                          constant float4 &crop [[buffer(0)]], uint2 p [[thread_position_in_grid]]) {
    constexpr sampler s(coord::normalized,address::clamp_to_edge,filter::linear);
    if (p.x<dy.get_width() && p.y<dy.get_height()) {
        float2 uv=(float2(p)+0.5)/float2(dy.get_width(),dy.get_height());
        dy.write(sy.sample(s,(crop.xy+uv*crop.zw)/float2(sy.get_width(),sy.get_height())),p);
    }
    if (p.x<dc.get_width() && p.y<dc.get_height()) {
        float2 uv=(float2(p)+0.5)/float2(dc.get_width(),dc.get_height());
        dc.write(sc.sample(s,(crop.xy+uv*crop.zw)*0.5/float2(sc.get_width(),sc.get_height())),p);
    }
}
"#;
impl Cropper {
    pub(crate) fn new()->Result<Self> {
        metal::objc::rc::autoreleasepool(||{
            let device=Device::system_default().ok_or("No native Metal device for viewport capture")?;
            let queue=device.new_command_queue();
            let library=device.new_library_with_source(SHADER,&metal::CompileOptions::new())?;
            let function=library.get_function("viewport_nv12",None)?;
            let pipeline=device.new_compute_pipeline_state_with_function(&function)?;
            let mut cache=NativeRef::empty();
            let key=unsafe{CFString::wrap_under_get_rule(kCVMetalTextureUsage.cast())};
            let usage=CFNumber::from(3_i64); // MTLTextureUsageShaderRead | ShaderWrite
            let texture_attrs=CFDictionary::from_CFType_pairs(&[(key.as_CFType(),usage.as_CFType())]);
            let status=unsafe{CVMetalTextureCacheCreate(ptr::null(),ptr::null(),device.as_ptr().cast(),texture_attrs.as_concrete_TypeRef().cast(),&mut cache.0)};
            if status!=0||cache.0.is_null(){return Err(format!("Cannot create Metal viewport texture cache: {status}").into());}
            Ok(Self{device,queue,pipeline,cache,pool:NativeRef::empty(),format:None})
        })
    }
    fn pool(&mut self,width:u32,height:u32,format:u32)->Result<()> {
        if self.format==Some((width,height,format)){return Ok(());}
        let width_value=CFNumber::from(i64::from(width));let height_value=CFNumber::from(i64::from(height));let pixel=CFNumber::from(i64::from(format));
        let surface=CFDictionary::<CFString,CFBoolean>::from_CFType_pairs(&[]);
        let keys=unsafe{[kCVPixelBufferWidthKey,kCVPixelBufferHeightKey,kCVPixelBufferPixelFormatTypeKey,kCVPixelBufferIOSurfacePropertiesKey,kCVPixelBufferMetalCompatibilityKey]}
            .map(|key|unsafe{CFString::wrap_under_get_rule(key.cast())});
        let values=[width_value.as_CFType(),height_value.as_CFType(),pixel.as_CFType(),surface.as_CFType(),CFBoolean::true_value().as_CFType()];
        let attrs=CFDictionary::from_CFType_pairs(&keys.iter().zip(values).map(|(k,v)|(k.as_CFType(),v)).collect::<Vec<_>>());
        let mut pool=NativeRef::empty();let status=unsafe{CVPixelBufferPoolCreate(ptr::null(),ptr::null(),attrs.as_concrete_TypeRef().cast(),&mut pool.0)};
        if status!=0||pool.0.is_null(){return Err(format!("Cannot allocate native viewport pool: {status}").into());}
        self.pool=pool;self.format=Some((width,height,format));Ok(())
    }
    fn plane(&self,buffer:*mut c_void,size:[u32;2],plane:usize)->Result<NativeRef> {
        let mut texture=NativeRef::empty();
        let width=unsafe{CVPixelBufferGetWidthOfPlane(buffer,plane)};
        let height=unsafe{CVPixelBufferGetHeightOfPlane(buffer,plane)};
        if width==0||height==0||plane==0&&(width!=size[0] as usize||height!=size[1] as usize) {
            return Err("NV12 plane dimensions differ from the captured source".into());
        }
        let format=if plane==0{MTLPixelFormat::R8Unorm}else{MTLPixelFormat::RG8Unorm};
        let status=unsafe{CVMetalTextureCacheCreateTextureFromImage(ptr::null(),self.cache.0,buffer,ptr::null(),format as u64,
            width,height,plane,&mut texture.0)};
        if status!=0||texture.0.is_null(){return Err(format!("Cannot import NV12 plane into Metal: {status}").into());}
        Ok(texture)
    }
    pub(crate) fn crop(&mut self,source:*mut c_void,g:FrameGeometry)->Result<PixelBuffer> {
        g.validate()?;
        metal::objc::rc::autoreleasepool(||{
            let format=unsafe{CVPixelBufferGetPixelFormatType(source)};
            if ![u32::from_be_bytes(*b"420v"),u32::from_be_bytes(*b"420f")].contains(&format) {
                return Err("Native viewport capture requires the actual NV12 source planes".into());
            }
            self.pool(g.width,g.height,format)?;
            let threshold=CFNumber::from(5_i64);
            let key=unsafe{CFString::wrap_under_get_rule(kCVPixelBufferPoolAllocationThresholdKey.cast())};
            let aux=CFDictionary::from_CFType_pairs(&[(key.as_CFType(),threshold.as_CFType())]);
            let mut output=ptr::null_mut();
            let status=unsafe{CVPixelBufferPoolCreatePixelBufferWithAuxAttributes(ptr::null(),self.pool.0,aux.as_concrete_TypeRef().cast(),&mut output)};
            if status!=0||output.is_null(){return Err(format!("Native viewport pool is unavailable: {status}").into());}
            let output=PixelBuffer(output);
            unsafe{
                CVBufferRemoveAllAttachments(output.0);CVBufferPropagateAttachments(source,output.0);
                // Retain real color metadata, but an aperture belonging to the
                // larger source cannot describe the new encoded pixel buffer.
                for key in [kCVImageBufferCleanApertureKey,kCVImageBufferPreferredCleanApertureKey,kCVImageBufferDisplayDimensionsKey] {
                    CVBufferRemoveAttachment(output.0,key);
                }
            }
            let planes=[self.plane(source,[g.source_width,g.source_height],0)?,self.plane(source,[g.source_width,g.source_height],1)?,
                self.plane(output.0,[g.width,g.height],0)?,self.plane(output.0,[g.width,g.height],1)?];
            let command=self.queue.new_command_buffer();
            let encoder=command.new_compute_command_encoder();encoder.set_compute_pipeline_state(&self.pipeline);
            for (index,plane) in planes.iter().enumerate() {
                let texture=unsafe{CVMetalTextureGetTexture(plane.0)};
                if texture.is_null(){return Err("CoreVideo supplied no Metal viewport texture".into());}
                encoder.set_texture(index as u64,Some(unsafe{TextureRef::from_ptr(texture)}));
            }
            let crop=[g.crop.x as f32,g.crop.y as f32,g.crop.width as f32,g.crop.height as f32];
            encoder.set_bytes(0,16,crop.as_ptr().cast());
            encoder.dispatch_threads(MTLSize{width:g.width as u64,height:g.height as u64,depth:1},MTLSize{width:16,height:16,depth:1});
            encoder.end_encoding();command.commit();command.wait_until_completed();
            if command.status()!=MTLCommandBufferStatus::Completed {return Err("Metal viewport crop did not complete".into());}
            // Command completion owns synchronization with VideoToolbox. The
            // encoder retains this pixel buffer after submission; no CPU map.
            let _=&self.device;
            Ok(output)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[link(name="CoreVideo",kind="framework")]
    unsafe extern "C" {
        fn CVPixelBufferCreate(a:*const c_void,width:usize,height:usize,format:u32,attrs:*const c_void,out:*mut *mut c_void)->i32;
        fn CVPixelBufferLockBaseAddress(buffer:*mut c_void,flags:u64)->i32;
        fn CVPixelBufferUnlockBaseAddress(buffer:*mut c_void,flags:u64)->i32;
        fn CVPixelBufferGetBaseAddressOfPlane(buffer:*mut c_void,plane:usize)->*mut u8;
        fn CVPixelBufferGetBytesPerRowOfPlane(buffer:*mut c_void,plane:usize)->usize;
    }
    #[test]
    fn gpu_crop_preserves_native_luma_chroma_and_aligned_coordinates() {
        // Synthetic pixels only. No screen/camera access, TCC or running app.
        let surface=CFDictionary::<CFString,CFBoolean>::from_CFType_pairs(&[]);
        let keys=unsafe{[kCVPixelBufferIOSurfacePropertiesKey,kCVPixelBufferMetalCompatibilityKey]}
            .map(|p|unsafe{CFString::wrap_under_get_rule(p.cast())});
        let attrs=CFDictionary::from_CFType_pairs(&[(keys[0].as_CFType(),surface.as_CFType()),(keys[1].as_CFType(),CFBoolean::true_value().as_CFType())]);
        let mut buffer=ptr::null_mut();assert_eq!(unsafe{CVPixelBufferCreate(ptr::null(),1280,720,u32::from_be_bytes(*b"420f"),attrs.as_concrete_TypeRef().cast(),&mut buffer)},0);
        let buffer=PixelBuffer(buffer);
        unsafe {
            assert_eq!(CVPixelBufferLockBaseAddress(buffer.0,0),0);
            let y=CVPixelBufferGetBaseAddressOfPlane(buffer.0,0);let ys=CVPixelBufferGetBytesPerRowOfPlane(buffer.0,0);
            let uv=CVPixelBufferGetBaseAddressOfPlane(buffer.0,1);let uvs=CVPixelBufferGetBytesPerRowOfPlane(buffer.0,1);
            for row in 0..720 {for column in 0..1280 {*y.add(row*ys+column)=((column/8+row/8)%256) as u8;}}
            for row in 0..360 {for column in 0..640 {*uv.add(row*uvs+column*2)=96;*uv.add(row*uvs+column*2+1)=160;}}
            assert_eq!(CVPixelBufferUnlockBaseAddress(buffer.0,0),0);
        }
        let geometry=FrameGeometry{source_revision:0,demand_revision:1,revision:1,source_width:1280,source_height:720,
            crop:protocol::viewport::PixelRect{x:320,y:180,width:640,height:360},width:640,height:360};
        let mut cropper=Cropper::new().unwrap();
        for offset in [0,32,128,256] {
            // Move the crop without rebuilding the device, pool, pipeline or
            // encoded dimensions. Check actual GPU output, not shader text.
            let g=FrameGeometry{crop:protocol::viewport::PixelRect{x:offset,..geometry.crop},..geometry};
            let output=cropper.crop(buffer.0,g).unwrap();
            unsafe {
                assert_eq!(CVPixelBufferLockBaseAddress(output.0,1),0);
                let y=CVPixelBufferGetBaseAddressOfPlane(output.0,0);let stride=CVPixelBufferGetBytesPerRowOfPlane(output.0,0);
                for (x,row) in [(0,0),(300,120),(639,359)] {
                    let actual=*y.add(row*stride+x);let expected=(((x+offset as usize)/8+(row+180)/8)%256) as u8;
                    assert!(actual.abs_diff(expected)<=1,"GPU crop at {x},{row}: {actual} != {expected}");
                }
                let uv=CVPixelBufferGetBaseAddressOfPlane(output.0,1);assert_eq!(*uv,96);assert_eq!(*uv.add(1),160);
                assert_eq!(CVPixelBufferUnlockBaseAddress(output.0,1),0);
            }
        }
    }
}
