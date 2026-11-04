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
                let (edge,pixels)=native_pixels(bitmap);
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
}
