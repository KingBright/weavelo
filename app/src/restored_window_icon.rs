use gpui::{App,Window,WindowIcon};
use std::sync::OnceLock;
use remote_core::application_icons::IconBitmap;

#[derive(Default)]
pub(super) struct Artwork { current:Option<[u8;32]> }
impl Artwork {
    pub fn sync(&mut self,window:&mut Window,cx:&App,bitmap:Option<&IconBitmap>) {
        let next=bitmap.map(|bitmap|bitmap.id);
        if self.current==next {return;}
        let result=(||->Result<(),String> {
            if let Some(bitmap)=bitmap {
                let (edge,pixels)=remote_application_pixels(bitmap)?;
                let icon=WindowIcon::new(edge,pixels).map_err(|e|e.to_string())?;
                window.set_window_icon(&icon).map_err(|e|e.to_string())?;
            } else {apply(window,cx);}
            Ok(())
        })();
        match result {
            Ok(())=>self.current=next,
            Err(error)=>eprintln!("Weavelo application window icon: {error}"),
        }
    }
}

fn remote_application_pixels(bitmap:&IconBitmap)->Result<(u32,Vec<u8>),String> {
    let (edge,pixels)=native_pixels(bitmap);
    let image=image::RgbaImage::from_raw(edge,edge,pixels).ok_or("Invalid application icon")?;
    let mut image=image::imageops::resize(&image,64,64,image::imageops::FilterType::Triangle);
    static BADGE:OnceLock<image::RgbaImage>=OnceLock::new();
    let badge=if let Some(badge)=BADGE.get() {badge}else {
        let mut badge=image::load_from_memory(include_bytes!("../../assets/brand/weavelo/icon-24.png"))
            .map_err(|error|error.to_string())?.into_rgba8();
        for pixel in badge.pixels_mut() {
            for channel in 0..3 {pixel[channel]=((u16::from(pixel[channel])*u16::from(pixel[3])+127)/255) as u8;}
        }
        BADGE.get_or_init(||badge)
    };
    // Source-over in premultiplied space: keep the application's identity and
    // add the product mark once per asset change, never in the video path.
    for (x,y,foreground) in badge.enumerate_pixels() {
        let background=image.get_pixel_mut(x+40,y+40);
        let remaining=255-u16::from(foreground[3]);
        for channel in 0..4 {
            background[channel]=(u16::from(foreground[channel])+(u16::from(background[channel])*remaining+127)/255).min(255) as u8;
        }
    }
    Ok((64,image.into_raw()))
}

fn native_pixels(bitmap:&IconBitmap)->(u32,Vec<u8>) {
    // Catalog PNGs contain straight RGBA; native window icons require square,
    // premultiplied RGBA. Preserve aspect and pad instead of stretching artwork.
    let edge=bitmap.width.max(bitmap.height);
    let mut pixels=vec![0;edge as usize*edge as usize*4];
    let left=(edge-bitmap.width)/2;
    let top=(edge-bitmap.height)/2;
    for y in 0..bitmap.height {
        for x in 0..bitmap.width {
            let from=((y*bitmap.width+x)*4) as usize;
            let to=(((y+top)*edge+x+left)*4) as usize;
            let alpha=u16::from(bitmap.rgba[from+3]);
            for channel in 0..3 {pixels[to+channel]=((u16::from(bitmap.rgba[from+channel])*alpha+127)/255) as u8;}
            pixels[to+3]=alpha as u8;
        }
    }
    (edge,pixels)
}

pub(super) fn apply(window:&mut Window,cx:&App) {
    let result=(||->Result<bool,String> {
        static ICON:OnceLock<WindowIcon>=OnceLock::new();
        let icon=if let Some(icon)=ICON.get() {icon.clone()}else{
            let image=cx.svg_renderer().render_single_frame(
                include_bytes!("product_components/icons/remote-window.svg"),1.,false).map_err(|e|e.to_string())?;
            let size=image.size(0);
            let rgba=image::RgbaImage::from_raw(size.width.0 as u32,size.height.0 as u32,
                image.as_bytes(0).ok_or("Missing native window icon pixels")?.to_vec()).ok_or("Invalid window icon dimensions")?;
            let rgba=image::imageops::resize(&rgba,64,64,image::imageops::FilterType::Triangle).into_raw();
            let icon=WindowIcon::new(64,rgba).map_err(|e|e.to_string())?;
            ICON.get_or_init(||icon).clone()
        };
        window.set_window_icon(&icon).map_err(|e|e.to_string())
    })();
    if let Err(error)=result {eprintln!("Weavelo native window icon: {error}");}
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn application_artwork_keeps_aspect_and_premultiplies_transparent_pixels() {
        let bitmap=IconBitmap{id:[1;32],width:2,height:1,
            rgba:vec![255,128,64,128, 255,255,255,0].into(),png:vec![].into()};
        let (edge,pixels)=native_pixels(&bitmap);
        assert_eq!(edge,2);
        assert_eq!(pixels,vec![128,64,32,128, 0,0,0,0, 0,0,0,0, 0,0,0,0]);
        assert!(WindowIcon::new(edge,pixels).is_ok());
    }
    #[test]
    fn remote_badge_keeps_the_application_and_has_valid_premultiplied_alpha() {
        let bitmap=IconBitmap{id:[2;32],width:64,height:64,
            rgba:[240,80,20,255].repeat(64*64).into(),png:vec![].into()};
        let (edge,pixels)=remote_application_pixels(&bitmap).unwrap();
        assert_eq!(edge,64);
        assert_eq!(&pixels[..4],&[240,80,20,255]);
        assert_ne!(&pixels[(54*64+54)*4..(54*64+54)*4+4],&[240,80,20,255]);
        assert!(pixels.chunks_exact(4).all(|pixel|pixel[..3].iter().all(|channel|*channel<=pixel[3])));
    }
}
