use crate::color::{extract_backdrop_base_rgb, gaussian_blur_bgra, inpaint_occluded_rects_bgra, Rgb};
use std::mem::size_of;
use std::ptr::null_mut;
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Direct2D::Common::{D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_PIXEL_FORMAT, D2D_SIZE_U};
use windows::Win32::Graphics::Direct2D::{ID2D1Bitmap, ID2D1DCRenderTarget, D2D1_BITMAP_PROPERTIES};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::{BitBlt, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, SRCCOPY};

/// Captures the desktop region directly behind a popup while inpaints out any transient
/// occluding windows (e.g. dismissing overflow flyouts/tooltips), returning the blurred bitmap and mean RGB.
pub fn capture_blurred_desktop_backdrop_excluding(
    rt: &ID2D1DCRenderTarget,
    screen_x: i32,
    screen_y: i32,
    w: i32,
    h: i32,
    blur_radius: usize,
    exclude_screen_rects: &[RECT],
) -> (Option<ID2D1Bitmap>, Option<Rgb>) {
    if w <= 0 || h <= 0 {
        return (None, None);
    }

    unsafe {
        let screen_dc = GetDC(None);
        if screen_dc.is_invalid() {
            return (None, None);
        }
        let mem_dc = CreateCompatibleDC(screen_dc);
        if mem_dc.is_invalid() {
            let _ = ReleaseDC(None, screen_dc);
            return (None, None);
        }

        let bmi = BITMAPINFO {
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
        let mut bits_ptr = null_mut();
        let Ok(bmp) = CreateDIBSection(mem_dc, &bmi, DIB_RGB_COLORS, &mut bits_ptr, None, 0) else {
            let _ = DeleteDC(mem_dc);
            let _ = ReleaseDC(None, screen_dc);
            return (None, None);
        };
        let old = SelectObject(mem_dc, bmp);
        let _ = BitBlt(mem_dc, 0, 0, w, h, screen_dc, screen_x, screen_y, SRCCOPY);
        let _ = ReleaseDC(None, screen_dc);

        let (result, mean_rgb) = if !bits_ptr.is_null() {
            let raw =
                std::slice::from_raw_parts(bits_ptr.cast::<u8>(), (w as usize) * (h as usize) * 4);
            let dw = (w as usize / 4).max(1);
            let dh = (h as usize / 4).max(1);
            let mut down = Vec::with_capacity(dw * dh * 4);
            for by in 0..dh {
                let y0 = (by * 4).min((h as usize) - 1);
                let y1 = ((by + 1) * 4).min(h as usize).max(y0 + 1);
                for bx in 0..dw {
                    let x0 = (bx * 4).min((w as usize) - 1);
                    let x1 = ((bx + 1) * 4).min(w as usize).max(x0 + 1);
                    let mut sum_b = 0u32;
                    let mut sum_g = 0u32;
                    let mut sum_r = 0u32;
                    let mut count = 0u32;
                    for sy in y0..y1 {
                        for sx in x0..x1 {
                            let idx = (sy * (w as usize) + sx) * 4;
                            sum_b += u32::from(raw[idx]);
                            sum_g += u32::from(raw[idx + 1]);
                            sum_r += u32::from(raw[idx + 2]);
                            count += 1;
                        }
                    }
                    let count = count.max(1);
                    down.push((sum_b / count) as u8);
                    down.push((sum_g / count) as u8);
                    down.push((sum_r / count) as u8);
                    down.push(255);
                }
            }

            let mut fully_occluded = false;
            if !exclude_screen_rects.is_empty() {
                let mut down_rects = Vec::with_capacity(exclude_screen_rects.len());
                for r in exclude_screen_rects {
                    let rx0 = (r.left - screen_x).max(0) as usize;
                    let ry0 = (r.top - screen_y).max(0) as usize;
                    let rx1 = (r.right - screen_x).max(0) as usize;
                    let ry1 = (r.bottom - screen_y).max(0) as usize;
                    let x0 = (rx0 / 4).min(dw);
                    let y0 = (ry0 / 4).min(dh);
                    let x1 = (rx1.saturating_add(3) / 4).min(dw);
                    let y1 = (ry1.saturating_add(3) / 4).min(dh);
                    if x0 < x1 && y0 < y1 {
                        down_rects.push((x0, y0, x1, y1));
                    }
                }
                if !down_rects.is_empty()
                    && !inpaint_occluded_rects_bgra(&mut down, dw, dh, &down_rects)
                {
                    fully_occluded = true;
                }
            }

            if fully_occluded {
                (None, None)
            } else {
                let mean = extract_backdrop_base_rgb(&down, dw, dh);
                let blurred = if blur_radius == 0 {
                    down
                } else {
                    gaussian_blur_bgra(&down, dw, dh, blur_radius)
                };
                let props = D2D1_BITMAP_PROPERTIES {
                    pixelFormat: D2D1_PIXEL_FORMAT {
                        format: DXGI_FORMAT_B8G8R8A8_UNORM,
                        alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                    },
                    dpiX: 96.0,
                    dpiY: 96.0,
                };
                let bmp = rt
                    .CreateBitmap(
                        D2D_SIZE_U {
                            width: dw as u32,
                            height: dh as u32,
                        },
                        Some(blurred.as_ptr().cast()),
                        (dw * 4) as u32,
                        &props,
                    )
                    .ok();
                (bmp, mean)
            }
        } else {
            (None, None)
        };

        let _ = SelectObject(mem_dc, old);
        let _ = DeleteObject(bmp);
        let _ = DeleteDC(mem_dc);
        (result, mean_rgb)
    }
}

/// Captures the desktop region directly behind a popup, returning both the blurred bitmap and its mean RGB.
#[allow(dead_code)]
pub fn capture_blurred_desktop_backdrop(
    rt: &ID2D1DCRenderTarget,
    screen_x: i32,
    screen_y: i32,
    w: i32,
    h: i32,
    blur_radius: usize,
) -> (Option<ID2D1Bitmap>, Option<Rgb>) {
    capture_blurred_desktop_backdrop_excluding(rt, screen_x, screen_y, w, h, blur_radius, &[])
}

/// Captures only the blurred desktop bitmap behind a popup (compatibility helper).
#[allow(dead_code)]
pub fn capture_blurred_desktop_bitmap(
    rt: &ID2D1DCRenderTarget,
    screen_x: i32,
    screen_y: i32,
    w: i32,
    h: i32,
) -> Option<ID2D1Bitmap> {
    capture_blurred_desktop_backdrop(rt, screen_x, screen_y, w, h, 6).0
}

/// Samples the mean opaque RGB color of a small screen region (e.g. taskbar background next to a widget).
pub fn sample_screen_region_rgb(screen_x: i32, screen_y: i32, w: i32, h: i32) -> Option<Rgb> {
    if w <= 0 || h <= 0 {
        return None;
    }
    let sw = w.clamp(1, 128);
    let sh = h.clamp(1, 64);

    unsafe {
        let screen_dc = GetDC(None);
        if screen_dc.is_invalid() {
            return None;
        }
        let mem_dc = CreateCompatibleDC(screen_dc);
        if mem_dc.is_invalid() {
            let _ = ReleaseDC(None, screen_dc);
            return None;
        }

        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: sw,
                biHeight: -sh,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits_ptr = null_mut();
        let Ok(bmp) = CreateDIBSection(mem_dc, &bmi, DIB_RGB_COLORS, &mut bits_ptr, None, 0) else {
            let _ = DeleteDC(mem_dc);
            let _ = ReleaseDC(None, screen_dc);
            return None;
        };
        let old = SelectObject(mem_dc, bmp);
        let _ = BitBlt(mem_dc, 0, 0, sw, sh, screen_dc, screen_x, screen_y, SRCCOPY);
        let _ = ReleaseDC(None, screen_dc);

        let rgb = if !bits_ptr.is_null() {
            let raw = std::slice::from_raw_parts(
                bits_ptr.cast::<u8>(),
                (sw as usize) * (sh as usize) * 4,
            );
            // Force alpha >= 255 in case desktop DC leaves zero alpha bytes.
            let mut sum_r = 0u64;
            let mut sum_g = 0u64;
            let mut sum_b = 0u64;
            let count = (sw as u64) * (sh as u64);
            for px in raw.chunks_exact(4) {
                sum_b += u64::from(px[0]);
                sum_g += u64::from(px[1]);
                sum_r += u64::from(px[2]);
            }
            if let (Some(r), Some(g), Some(b)) = (
                sum_r.checked_div(count),
                sum_g.checked_div(count),
                sum_b.checked_div(count),
            ) {
                Some(Rgb::new(
                    r.min(255) as u8,
                    g.min(255) as u8,
                    b.min(255) as u8,
                ))
            } else {
                None
            }
        } else {
            None
        };

        let _ = SelectObject(mem_dc, old);
        let _ = DeleteObject(bmp);
        let _ = DeleteDC(mem_dc);
        rgb
    }
}
