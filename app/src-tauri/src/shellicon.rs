//! Icons of files that carry their own (exe, lnk, ico, url…): the image
//! Explorer shows, as small PNG data URLs. Other file types keep Twinyard's
//! own icons; the panel asks only for the visible rows.

use base64::Engine;
use std::ffi::c_void;
use windows::Win32::Foundation::SIZE;
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDIBits, GetObjectW,
    HBITMAP, HGDIOBJ,
};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
use windows::Win32::UI::Shell::{IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_ICONONLY};
use windows::core::HSTRING;

/// Icons for local files, in order; None where Windows has none to give.
/// `size` in physical pixels (16 × the display scale).
#[tauri::command]
pub async fn file_icons(paths: Vec<String>, size: u32) -> Result<Vec<Option<String>>, String> {
    let size = size.clamp(16, 64) as i32;
    crate::blocking(move || {
        // SAFETY: COM on this worker thread; if it is already initialised
        // (any mode) the call fails harmlessly and COM stays usable.
        let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        Ok(paths.iter().map(|p| icon(p, size).ok()).collect())
    })
    .await
}

fn icon(path: &str, size: i32) -> Result<String, String> {
    let e = |e: windows::core::Error| e.to_string();
    // SAFETY: plain shell / GDI calls; the bitmap is deleted below.
    let hbm = unsafe {
        let f: IShellItemImageFactory = SHCreateItemFromParsingName(&HSTRING::from(path), None).map_err(e)?;
        f.GetImage(SIZE { cx: size, cy: size }, SIIGBF_ICONONLY).map_err(e)?
    };
    let px = pixels(hbm);
    // SAFETY: we own the bitmap GetImage returned.
    let _ = unsafe { DeleteObject(HGDIOBJ(hbm.0)) };
    let (w, h, rgba) = px?;
    let mut png = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut png, w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut wr = enc.write_header().map_err(|e| e.to_string())?;
        wr.write_image_data(&rgba).map_err(|e| e.to_string())?;
    }
    Ok(format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(png)))
}

/// 32-bit BGRA (maybe premultiplied) -> straight RGBA, top-down.
fn pixels(hbm: HBITMAP) -> Result<(u32, u32, Vec<u8>), String> {
    let mut bm = BITMAP::default();
    // SAFETY: BITMAP-sized out buffer for GetObjectW; 32bpp top-down DIB copy.
    unsafe {
        if GetObjectW(HGDIOBJ(hbm.0), size_of::<BITMAP>() as i32, Some(&mut bm as *mut _ as *mut c_void)) == 0 {
            return Err("GetObject".into());
        }
    }
    let (w, h) = (bm.bmWidth, bm.bmHeight.abs());
    if w <= 0 || h <= 0 || w > 256 || h > 256 {
        return Err("bad bitmap size".into());
    }
    let mut bi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut buf = vec![0u8; (w * h * 4) as usize];
    let lines = unsafe {
        let dc = CreateCompatibleDC(None);
        let n = GetDIBits(dc, hbm, 0, h as u32, Some(buf.as_mut_ptr() as *mut c_void), &mut bi, DIB_RGB_COLORS);
        let _ = DeleteDC(dc);
        n
    };
    if lines != h {
        return Err("GetDIBits".into());
    }
    let has_alpha = buf.as_chunks::<4>().0.iter().any(|p| p[3] != 0);
    // Premultiplied when no colour channel exceeds its alpha.
    let premul = has_alpha && buf.as_chunks::<4>().0.iter().all(|p| p[0] <= p[3] && p[1] <= p[3] && p[2] <= p[3]);
    for p in buf.as_chunks_mut::<4>().0 {
        let (b, g, r, a) = (p[0], p[1], p[2], if has_alpha { p[3] } else { 255 });
        let un = |c: u8| if premul && a > 0 && a < 255 { ((c as u32 * 255 + a as u32 / 2) / a as u32).min(255) as u8 } else { c };
        *p = [un(r), un(g), un(b), a];
    }
    Ok((w as u32, h as u32, buf))
}

#[cfg(test)]
mod tests {
    #[test]
    fn notepad_has_an_icon() {
        let _ = unsafe { super::CoInitializeEx(None, super::COINIT_APARTMENTTHREADED) };
        let url = super::icon(r"C:\Windows\notepad.exe", 32).expect("icon");
        assert!(url.starts_with("data:image/png;base64,") && url.len() > 200);
        assert!(super::icon(r"C:\nope\missing.exe", 32).is_err());
    }
}
