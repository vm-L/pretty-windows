use crate::color::{gaussian_blur_bgra, Rgb};
use crate::ui::{
    sample_liquid_silk_field, RectF, DIVIDER_H_DIP, SILK_H, SILK_UPDATE_INTERVAL_MS, SILK_W,
};
use std::mem::size_of;
use std::ptr::null_mut;
use std::time::{Duration, Instant};
use windows::core::w;
use windows::Foundation::Numerics::Matrix3x2;
use windows::Win32::Foundation::{HWND, POINT, SIZE};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_GRADIENT_STOP, D2D1_PIXEL_FORMAT,
    D2D_POINT_2F, D2D_RECT_F, D2D_SIZE_U,
};
use windows::Win32::Graphics::Direct2D::{
    ID2D1Bitmap, ID2D1DCRenderTarget, ID2D1Factory1, ID2D1Geometry, ID2D1SolidColorBrush,
    ID2D1StrokeStyle, D2D1_BITMAP_INTERPOLATION_MODE_LINEAR, D2D1_BITMAP_PROPERTIES,
    D2D1_CAP_STYLE_ROUND, D2D1_DRAW_TEXT_OPTIONS_CLIP, D2D1_ELLIPSE, D2D1_EXTEND_MODE_CLAMP,
    D2D1_GAMMA_2_2, D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES, D2D1_ROUNDED_RECT,
    D2D1_STROKE_STYLE_PROPERTIES1,
};
use windows::Win32::Graphics::DirectWrite::{
    IDWriteFactory, IDWriteInlineObject, IDWriteTextFormat, DWRITE_FONT_STRETCH_NORMAL,
    DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_WEIGHT_SEMI_BOLD,
    DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_LEADING,
    DWRITE_TEXT_ALIGNMENT_TRAILING, DWRITE_TRIMMING, DWRITE_TRIMMING_GRANULARITY_CHARACTER,
    DWRITE_WORD_WRAPPING_NO_WRAP,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC,
    SelectObject, AC_SRC_ALPHA, AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION,
    DIB_RGB_COLORS, HBITMAP, HDC, HGDIOBJ, SRCCOPY,
};
use windows::Win32::UI::WindowsAndMessaging::{UpdateLayeredWindow, ULW_ALPHA};

/// Offscreen 32-bit premultiplied BGRA DIB surface that commits transparently via `UpdateLayeredWindow`.
pub struct LayeredSurface {
    mem_dc: HDC,
    bitmap: HBITMAP,
    old_obj: HGDIOBJ,
    bits: *mut u8,
    width: i32,
    height: i32,
}

impl LayeredSurface {
    pub fn new() -> windows::core::Result<Self> {
        unsafe {
            let screen_dc = GetDC(None);
            let mem_dc = CreateCompatibleDC(screen_dc);
            let _ = ReleaseDC(None, screen_dc);
            if mem_dc.is_invalid() {
                return Err(windows::core::Error::from_win32());
            }
            Ok(Self {
                mem_dc,
                bitmap: HBITMAP::default(),
                old_obj: HGDIOBJ::default(),
                bits: null_mut(),
                width: 0,
                height: 0,
            })
        }
    }

    pub fn ensure_size(&mut self, width: i32, height: i32) -> windows::core::Result<()> {
        let w = width.max(1);
        let h = height.max(1);
        if self.width == w && self.height == h && !self.bitmap.is_invalid() {
            return Ok(());
        }

        self.release_bitmap();

        unsafe {
            let bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w,
                    biHeight: -h, // top-down
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits = null_mut();
            let bmp = CreateDIBSection(self.mem_dc, &bmi, DIB_RGB_COLORS, &mut bits, None, 0)?;
            let old = SelectObject(self.mem_dc, bmp);
            self.bitmap = bmp;
            self.old_obj = old;
            self.bits = bits.cast::<u8>();
            self.width = w;
            self.height = h;
        }
        Ok(())
    }

    pub fn dc(&self) -> HDC {
        self.mem_dc
    }

    pub fn width(&self) -> i32 {
        self.width
    }

    pub fn height(&self) -> i32 {
        self.height
    }

    pub fn bitmap_handle(&self) -> HBITMAP {
        self.bitmap
    }

    #[allow(dead_code)]
    pub unsafe fn pixels_mut(&mut self) -> Option<&mut [u8]> {
        if self.bits.is_null() || self.width <= 0 || self.height <= 0 {
            None
        } else {
            let len = (self.width as usize) * (self.height as usize) * 4;
            Some(std::slice::from_raw_parts_mut(self.bits, len))
        }
    }

    pub fn commit(&self, hwnd: HWND, x: i32, y: i32) {
        self.commit_with_alpha(hwnd, x, y, 255);
    }

    pub fn commit_with_alpha(&self, hwnd: HWND, x: i32, y: i32, alpha: u8) {
        if self.bitmap.is_invalid() {
            return;
        }
        unsafe {
            let pt_dst = POINT { x, y };
            let size = SIZE {
                cx: self.width,
                cy: self.height,
            };
            let pt_src = POINT { x: 0, y: 0 };
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: alpha,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };
            let _ = UpdateLayeredWindow(
                hwnd,
                None,
                Some(&pt_dst),
                Some(&size),
                self.mem_dc,
                Some(&pt_src),
                windows::Win32::Foundation::COLORREF(0),
                Some(&blend),
                ULW_ALPHA,
            );
        }
    }

    fn release_bitmap(&mut self) {
        unsafe {
            if !self.old_obj.is_invalid() {
                let _ = SelectObject(self.mem_dc, self.old_obj);
                self.old_obj = HGDIOBJ::default();
            }
            if !self.bitmap.is_invalid() {
                let _ = DeleteObject(self.bitmap);
                self.bitmap = HBITMAP::default();
            }
            self.bits = null_mut();
        }
    }
}

impl Drop for LayeredSurface {
    fn drop(&mut self) {
        self.release_bitmap();
        unsafe {
            if !self.mem_dc.is_invalid() {
                let _ = DeleteDC(self.mem_dc);
            }
        }
    }
}

/// Converts `RectF` into Direct2D `D2D_RECT_F`.
#[inline]
pub fn d2d_rect(r: RectF) -> D2D_RECT_F {
    D2D_RECT_F {
        left: r.left,
        top: r.top,
        right: r.right,
        bottom: r.bottom,
    }
}

#[inline]
pub fn d2d_color_rgb(rgb: Rgb, a: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: f32::from(rgb.r) / 255.0,
        g: f32::from(rgb.g) / 255.0,
        b: f32::from(rgb.b) / 255.0,
        a: a.clamp(0.0, 1.0),
    }
}

#[inline]
pub fn identity_matrix() -> Matrix3x2 {
    Matrix3x2 {
        M11: 1.0,
        M12: 0.0,
        M21: 0.0,
        M22: 1.0,
        M31: 0.0,
        M32: 0.0,
    }
}

pub fn create_round_cap_stroke(factory: &ID2D1Factory1) -> Option<ID2D1StrokeStyle> {
    let props = D2D1_STROKE_STYLE_PROPERTIES1 {
        startCap: D2D1_CAP_STYLE_ROUND,
        endCap: D2D1_CAP_STYLE_ROUND,
        dashCap: D2D1_CAP_STYLE_ROUND,
        ..Default::default()
    };
    unsafe { factory.CreateStrokeStyle(&props, None).ok().map(Into::into) }
}

pub fn primary_text_color(light_glass: bool) -> D2D1_COLOR_F {
    if light_glass {
        D2D1_COLOR_F {
            r: 0.07,
            g: 0.085,
            b: 0.12,
            a: 0.92,
        }
    } else {
        D2D1_COLOR_F {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 0.95,
        }
    }
}

pub fn muted_text_color(light_glass: bool) -> D2D1_COLOR_F {
    if light_glass {
        D2D1_COLOR_F {
            r: 0.20,
            g: 0.21,
            b: 0.23,
            a: 0.68,
        }
    } else {
        D2D1_COLOR_F {
            r: 0.94,
            g: 0.94,
            b: 0.95,
            a: 0.72,
        }
    }
}

pub fn danger_text_color(light_glass: bool) -> D2D1_COLOR_F {
    if light_glass {
        D2D1_COLOR_F {
            r: 0.78,
            g: 0.16,
            b: 0.18,
            a: 0.94,
        }
    } else {
        D2D1_COLOR_F {
            r: 1.0,
            g: 0.48,
            b: 0.48,
            a: 0.95,
        }
    }
}

pub fn create_text_format(
    dwrite_factory: &IDWriteFactory,
    size: f32,
    semibold: bool,
    trailing: bool,
    with_ellipsis: bool,
) -> windows::core::Result<IDWriteTextFormat> {
    unsafe {
        let fmt = dwrite_factory.CreateTextFormat(
            w!("Segoe UI Variable Text"),
            None,
            if semibold {
                DWRITE_FONT_WEIGHT_SEMI_BOLD
            } else {
                DWRITE_FONT_WEIGHT_NORMAL
            },
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            size,
            w!("zh-CN"),
        )?;
        let _ = fmt.SetTextAlignment(if trailing {
            DWRITE_TEXT_ALIGNMENT_TRAILING
        } else {
            DWRITE_TEXT_ALIGNMENT_LEADING
        });
        let _ = fmt.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER);
        let _ = fmt.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP);
        if with_ellipsis {
            let trimming = DWRITE_TRIMMING {
                granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER,
                delimiter: 0,
                delimiterCount: 0,
            };
            let sign: IDWriteInlineObject = dwrite_factory.CreateEllipsisTrimmingSign(&fmt)?;
            let _ = fmt.SetTrimming(&trimming, &sign);
        }
        Ok(fmt)
    }
}

/// Captures the desktop region directly behind a popup and applies soft frosted blur.
pub fn capture_blurred_desktop_bitmap(
    rt: &ID2D1DCRenderTarget,
    screen_x: i32,
    screen_y: i32,
    w: i32,
    h: i32,
) -> Option<ID2D1Bitmap> {
    if w <= 0 || h <= 0 {
        return None;
    }

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
            return None;
        };
        let old = SelectObject(mem_dc, bmp);
        let _ = BitBlt(mem_dc, 0, 0, w, h, screen_dc, screen_x, screen_y, SRCCOPY);
        let _ = ReleaseDC(None, screen_dc);

        let result = if !bits_ptr.is_null() {
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
            let blurred = gaussian_blur_bgra(&down, dw, dh, 6);
            let props = D2D1_BITMAP_PROPERTIES {
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: 96.0,
                dpiY: 96.0,
            };
            rt.CreateBitmap(
                D2D_SIZE_U {
                    width: dw as u32,
                    height: dh as u32,
                },
                Some(blurred.as_ptr().cast()),
                (dw * 4) as u32,
                &props,
            )
            .ok()
        } else {
            None
        };

        let _ = SelectObject(mem_dc, old);
        let _ = DeleteObject(bmp);
        let _ = DeleteDC(mem_dc);
        result
    }
}

/// Regenerates the 64x48 offscreen liquid-silk bitmap at ~30 FPS when needed.
#[allow(clippy::too_many_arguments)]
pub fn update_silk_bitmap(
    rt: &ID2D1DCRenderTarget,
    silk_bmp: &mut Option<ID2D1Bitmap>,
    silk_buf: &mut Vec<u8>,
    last_silk_tick: &mut Option<Instant>,
    force: bool,
    win_w: i32,
    win_h: i32,
    origin_left: f32,
    origin_bottom: f32,
    s: f32,
    flow_phase: f64,
    halo_colors: &[(f32, f32, f32); 2],
    light_glass: bool,
) {
    let now = Instant::now();
    let need_rebuild = force
        || silk_bmp.is_none()
        || last_silk_tick
            .is_none_or(|t| now.duration_since(t) >= Duration::from_millis(SILK_UPDATE_INTERVAL_MS));
    if !need_rebuild {
        return;
    }
    *last_silk_tick = Some(now);

    let expected_len = (SILK_W * SILK_H * 4) as usize;
    if silk_buf.len() != expected_len {
        silk_buf.resize(expected_len, 0);
    }

    let scale_len = (220.0 * s).max(1.0);
    let phase = (flow_phase % 10_000.0) as f32;
    let w = win_w.max(1) as f32;
    let h = win_h.max(1) as f32;

    for gy in 0..SILK_H {
        let py = (((gy as f32) + 0.5) / (SILK_H as f32)) * h;
        let y = (origin_bottom - py) / scale_len;
        for gx in 0..SILK_W {
            let px = (((gx as f32) + 0.5) / (SILK_W as f32)) * w;
            let x = (px - origin_left) / scale_len;
            let (r, g, b, a) =
                sample_liquid_silk_field(x, y, phase, halo_colors, light_glass);
            let idx = ((gy * SILK_W + gx) * 4) as usize;
            silk_buf[idx] = (b * a * 255.0).round().clamp(0.0, 255.0) as u8;
            silk_buf[idx + 1] = (g * a * 255.0).round().clamp(0.0, 255.0) as u8;
            silk_buf[idx + 2] = (r * a * 255.0).round().clamp(0.0, 255.0) as u8;
            silk_buf[idx + 3] = (a * 255.0).round().clamp(0.0, 255.0) as u8;
        }
    }

    unsafe {
        if let Some(ref bmp) = *silk_bmp {
            let _ = bmp.CopyFromMemory(None, silk_buf.as_ptr().cast(), SILK_W * 4);
        } else {
            let props = D2D1_BITMAP_PROPERTIES {
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: 96.0,
                dpiY: 96.0,
            };
            *silk_bmp = rt
                .CreateBitmap(
                    D2D_SIZE_U {
                        width: SILK_W,
                        height: SILK_H,
                    },
                    Some(silk_buf.as_ptr().cast()),
                    SILK_W * 4,
                    &props,
                )
                .ok();
        }
    }
}

/// Draws the 4 interior layers of Liquid Glass (blurred backdrop, neutral base, liquid silk, diagonal sheen).
#[allow(clippy::too_many_arguments)]
pub unsafe fn draw_glass_interior(
    rt: &ID2D1DCRenderTarget,
    bounds: RectF,
    win_w: i32,
    win_h: i32,
    backdrop_bmp: Option<&ID2D1Bitmap>,
    silk_bmp: Option<&ID2D1Bitmap>,
    light_glass: bool,
) {
    let full_win = D2D_RECT_F {
        left: 0.0,
        top: 0.0,
        right: win_w as f32,
        bottom: win_h as f32,
    };

    if let Some(bg_bmp) = backdrop_bmp {
        rt.DrawBitmap(
            bg_bmp,
            Some(&full_win),
            0.88,
            D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
            None,
        );
    } else {
        let fallback_bg = if light_glass {
            D2D1_COLOR_F {
                r: 246.0 / 255.0,
                g: 246.0 / 255.0,
                b: 248.0 / 255.0,
                a: 0.90,
            }
        } else {
            D2D1_COLOR_F {
                r: 26.0 / 255.0,
                g: 26.0 / 255.0,
                b: 28.0 / 255.0,
                a: 0.88,
            }
        };
        if let Ok(fb_brush) = rt.CreateSolidColorBrush(&fallback_bg, None) {
            rt.FillRectangle(&d2d_rect(bounds), &fb_brush);
        }
    }

    let glass_base = if light_glass {
        D2D1_COLOR_F {
            r: 246.0 / 255.0,
            g: 246.0 / 255.0,
            b: 248.0 / 255.0,
            a: 0.74,
        }
    } else {
        D2D1_COLOR_F {
            r: 24.0 / 255.0,
            g: 24.0 / 255.0,
            b: 26.0 / 255.0,
            a: 0.72,
        }
    };
    if let Ok(base_brush) = rt.CreateSolidColorBrush(&glass_base, None) {
        rt.FillRectangle(&d2d_rect(bounds), &base_brush);
    }

    if let Some(silk) = silk_bmp {
        rt.DrawBitmap(
            silk,
            Some(&full_win),
            1.0,
            D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
            None,
        );
    }

    let sheen_stops = [
        D2D1_GRADIENT_STOP {
            position: 0.0,
            color: D2D1_COLOR_F {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: if light_glass { 0.26 } else { 0.14 },
            },
        },
        D2D1_GRADIENT_STOP {
            position: 0.45,
            color: D2D1_COLOR_F {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: if light_glass { 0.03 } else { 0.02 },
            },
        },
        D2D1_GRADIENT_STOP {
            position: 1.0,
            color: D2D1_COLOR_F {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: if light_glass { 0.10 } else { 0.06 },
            },
        },
    ];
    if let Ok(sc) = rt.CreateGradientStopCollection(&sheen_stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP)
    {
        let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
            startPoint: D2D_POINT_2F {
                x: bounds.left,
                y: bounds.top,
            },
            endPoint: D2D_POINT_2F {
                x: bounds.right,
                y: bounds.bottom,
            },
        };
        if let Ok(sheen_brush) = rt.CreateLinearGradientBrush(&props, None, &sc) {
            rt.FillRectangle(&d2d_rect(bounds), &sheen_brush);
        }
    }
}

/// Draws 3-pass ambient shadows behind a standalone closed rounded Liquid Glass card.
#[allow(dead_code)]
pub unsafe fn draw_rounded_card_shadows(
    rt: &ID2D1DCRenderTarget,
    card_rect: RectF,
    radius: f32,
    win_w: f32,
    win_h: f32,
    s: f32,
    light_glass: bool,
) {
    let shadow_layers = if light_glass {
        [(7.5 * s, 0.022), (4.5 * s, 0.036), (2.0 * s, 0.058)]
    } else {
        [(7.5 * s, 0.035), (4.5 * s, 0.055), (2.0 * s, 0.09)]
    };
    for (expand, alpha) in shadow_layers {
        let side_exp = expand * 0.55;
        let top_exp = expand * 0.45;
        let bot_exp = expand * 0.65;
        let sh_rect = RectF::new(
            (card_rect.left - side_exp).max(0.0),
            (card_rect.top - top_exp).max(0.0),
            (card_rect.right + side_exp).min(win_w),
            (card_rect.bottom + bot_exp).min(win_h),
        );
        let sh_rr = D2D1_ROUNDED_RECT {
            rect: d2d_rect(sh_rect),
            radiusX: radius + side_exp * 0.45,
            radiusY: radius + side_exp * 0.45,
        };
        if let Ok(sh_brush) = rt.CreateSolidColorBrush(
            &D2D1_COLOR_F {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: alpha,
            },
            None,
        ) {
            rt.FillRoundedRectangle(&sh_rr, &sh_brush);
        }
    }
}

fn rim_gradient_stops(light_glass: bool) -> [D2D1_GRADIENT_STOP; 3] {
    if light_glass {
        [
            D2D1_GRADIENT_STOP {
                position: 0.0,
                color: D2D1_COLOR_F {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: 0.82,
                },
            },
            D2D1_GRADIENT_STOP {
                position: 0.55,
                color: D2D1_COLOR_F {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: 0.42,
                },
            },
            D2D1_GRADIENT_STOP {
                position: 1.0,
                color: D2D1_COLOR_F {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: 0.22,
                },
            },
        ]
    } else {
        [
            D2D1_GRADIENT_STOP {
                position: 0.0,
                color: D2D1_COLOR_F {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: 0.48,
                },
            },
            D2D1_GRADIENT_STOP {
                position: 0.55,
                color: D2D1_COLOR_F {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: 0.18,
                },
            },
            D2D1_GRADIENT_STOP {
                position: 1.0,
                color: D2D1_COLOR_F {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: 0.13,
                },
            },
        ]
    }
}

/// Draws the crisp vertical-gradient glass rim highlight around a standalone closed rounded card.
#[allow(dead_code)]
pub unsafe fn draw_rounded_card_rim(
    rt: &ID2D1DCRenderTarget,
    card_rect: RectF,
    radius: f32,
    s: f32,
    light_glass: bool,
) {
    let rr = D2D1_ROUNDED_RECT {
        rect: d2d_rect(card_rect),
        radiusX: radius,
        radiusY: radius,
    };
    if light_glass {
        let def_col = D2D1_COLOR_F {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 0.09,
        };
        if let Ok(def_brush) = rt.CreateSolidColorBrush(&def_col, None) {
            rt.DrawRoundedRectangle(&rr, &def_brush, 1.3 * s, None);
        }
    }

    let rim_stops = rim_gradient_stops(light_glass);
    let stroke_w = if light_glass { 1.0 * s } else { 1.15 * s };
    if let Ok(sc) =
        rt.CreateGradientStopCollection(&rim_stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP)
    {
        let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
            startPoint: D2D_POINT_2F {
                x: card_rect.left,
                y: card_rect.top,
            },
            endPoint: D2D_POINT_2F {
                x: card_rect.left,
                y: card_rect.bottom,
            },
        };
        if let Ok(rim_brush) = rt.CreateLinearGradientBrush(&props, None, &sc) {
            rt.DrawRoundedRectangle(&rr, &rim_brush, stroke_w, None);
        }
    }
}

/// Draws the standard Liquid Glass specular rim highlight along an arbitrary closed/unified geometry contour.
pub unsafe fn draw_geometry_rim(
    rt: &ID2D1DCRenderTarget,
    geo: &ID2D1Geometry,
    left_x: f32,
    top_y: f32,
    bottom_y: f32,
    s: f32,
    light_glass: bool,
) {
    if light_glass {
        let def_col = D2D1_COLOR_F {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 0.09,
        };
        if let Ok(def_brush) = rt.CreateSolidColorBrush(&def_col, None) {
            rt.DrawGeometry(geo, &def_brush, 1.3 * s, None);
        }
    }

    let rim_stops = rim_gradient_stops(light_glass);
    let stroke_w = if light_glass { 1.0 * s } else { 1.15 * s };
    if let Ok(sc) =
        rt.CreateGradientStopCollection(&rim_stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP)
    {
        let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
            startPoint: D2D_POINT_2F {
                x: left_x,
                y: top_y,
            },
            endPoint: D2D_POINT_2F {
                x: left_x,
                y: bottom_y,
            },
        };
        if let Ok(rim_brush) = rt.CreateLinearGradientBrush(&props, None, &sc) {
            rt.DrawGeometry(geo, &rim_brush, stroke_w, None);
        }
    }
}

/// Draws standard menu row hover/active highlight with left accent capsule indicator when selected.
#[allow(clippy::too_many_arguments)]
pub unsafe fn draw_row_highlight(
    rt: &ID2D1DCRenderTarget,
    rect: RectF,
    hovered: bool,
    selected: bool,
    accent: Rgb,
    s: f32,
    light_glass: bool,
) {
    if !hovered && !selected {
        return;
    }
    let rr = D2D1_ROUNDED_RECT {
        rect: d2d_rect(rect),
        radiusX: 8.5 * s,
        radiusY: 8.5 * s,
    };

    if selected {
        let fill_alpha = if light_glass {
            if hovered { 0.26 } else { 0.20 }
        } else if hovered {
            0.34
        } else {
            0.26
        };
        let fill = d2d_color_rgb(accent, fill_alpha);
        if let Ok(b) = rt.CreateSolidColorBrush(&fill, None) {
            rt.FillRoundedRectangle(&rr, &b);
        }
        let border_alpha = if light_glass { 0.48 } else { 0.58 };
        let border = d2d_color_rgb(accent, border_alpha);
        if let Ok(bb) = rt.CreateSolidColorBrush(&border, None) {
            rt.DrawRoundedRectangle(&rr, &bb, 1.1 * s, None);
        }

        let bar_w = 3.0 * s;
        let bar_h = (16.0 * s).min(rect.height() - 10.0 * s);
        let bar_left = rect.left + 3.0 * s;
        let bar_top = (rect.top + rect.bottom - bar_h) * 0.5;
        let bar_rr = D2D1_ROUNDED_RECT {
            rect: D2D_RECT_F {
                left: bar_left,
                top: bar_top,
                right: bar_left + bar_w,
                bottom: bar_top + bar_h,
            },
            radiusX: bar_w * 0.5,
            radiusY: bar_w * 0.5,
        };
        let bar_col = d2d_color_rgb(accent, 0.98);
        if let Ok(ab) = rt.CreateSolidColorBrush(&bar_col, None) {
            rt.FillRoundedRectangle(&bar_rr, &ab);
        }
    } else {
        let fill = if light_glass {
            D2D1_COLOR_F {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.06,
            }
        } else {
            D2D1_COLOR_F {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 0.10,
            }
        };
        if let Ok(b) = rt.CreateSolidColorBrush(&fill, None) {
            rt.FillRoundedRectangle(&rr, &b);
        }
    }
}

/// Draws a standard 1 DIP inset horizontal divider below `upper_row`.
pub unsafe fn draw_divider(
    rt: &ID2D1DCRenderTarget,
    upper_row: RectF,
    s: f32,
    light_glass: bool,
) {
    let div_y = upper_row.bottom + (DIVIDER_H_DIP * s) * 0.5;
    let div_col = if light_glass {
        D2D1_COLOR_F {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 0.10,
        }
    } else {
        D2D1_COLOR_F {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 0.14,
        }
    };
    if let Ok(div_brush) = rt.CreateSolidColorBrush(&div_col, None) {
        rt.DrawLine(
            D2D_POINT_2F {
                x: upper_row.left + 6.0 * s,
                y: div_y,
            },
            D2D_POINT_2F {
                x: upper_row.right - 6.0 * s,
                y: div_y,
            },
            &div_brush,
            1.0 * s,
            None,
        );
    }
}

/// Draws single-line clipped DirectWrite text inside `rect`.
pub unsafe fn draw_text_in_rect(
    dwrite_factory: &IDWriteFactory,
    rt: &ID2D1DCRenderTarget,
    text: &str,
    rect: RectF,
    fmt: &IDWriteTextFormat,
    color: D2D1_COLOR_F,
) {
    let wide: Vec<u16> = text.encode_utf16().collect();
    let Ok(layout) = dwrite_factory.CreateTextLayout(
        &wide,
        fmt,
        rect.width().max(1.0),
        rect.height().max(12.0),
    ) else {
        return;
    };
    let Ok(brush) = rt.CreateSolidColorBrush(&color, None) else {
        return;
    };
    rt.DrawTextLayout(
        D2D_POINT_2F {
            x: rect.left,
            y: rect.top,
        },
        &layout,
        &brush,
        D2D1_DRAW_TEXT_OPTIONS_CLIP,
    );
}

/// Draws compact beamed eighth-notes vector icon centered at `(cx, cy)`.
pub unsafe fn draw_music_note_icon(
    rt: &ID2D1DCRenderTarget,
    cx: f32,
    cy: f32,
    s: f32,
    brush: &ID2D1SolidColorBrush,
) {
    rt.FillEllipse(
        &D2D1_ELLIPSE {
            point: D2D_POINT_2F {
                x: cx - 2.6 * s,
                y: cy + 3.0 * s,
            },
            radiusX: 1.9 * s,
            radiusY: 1.5 * s,
        },
        brush,
    );
    rt.FillEllipse(
        &D2D1_ELLIPSE {
            point: D2D_POINT_2F {
                x: cx + 2.8 * s,
                y: cy + 2.1 * s,
            },
            radiusX: 1.9 * s,
            radiusY: 1.5 * s,
        },
        brush,
    );
    rt.DrawLine(
        D2D_POINT_2F {
            x: cx - 1.1 * s,
            y: cy + 2.8 * s,
        },
        D2D_POINT_2F {
            x: cx - 1.1 * s,
            y: cy - 4.2 * s,
        },
        brush,
        1.3 * s,
        None,
    );
    rt.DrawLine(
        D2D_POINT_2F {
            x: cx + 4.3 * s,
            y: cy + 1.9 * s,
        },
        D2D_POINT_2F {
            x: cx + 4.3 * s,
            y: cy - 5.2 * s,
        },
        brush,
        1.3 * s,
        None,
    );
    rt.DrawLine(
        D2D_POINT_2F {
            x: cx - 1.5 * s,
            y: cy - 4.2 * s,
        },
        D2D_POINT_2F {
            x: cx + 4.7 * s,
            y: cy - 5.2 * s,
        },
        brush,
        1.8 * s,
        None,
    );
}
