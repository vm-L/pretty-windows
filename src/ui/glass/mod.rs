mod backdrop;
mod composition;
mod contour;
mod controls;
mod surface_fx;

pub use backdrop::*;
pub use composition::CompositionEngine;
pub use contour::*;
pub use controls::*;
pub use surface_fx::*;

use crate::color::Rgb;
use crate::ui::RectF;
use std::mem::size_of;
use std::ptr::null_mut;
use windows::core::w;
use windows::Foundation::Numerics::Matrix3x2;
use windows::Win32::Foundation::{HWND, POINT, SIZE};
use windows::Win32::Graphics::Direct2D::Common::{D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT, D2D_RECT_F};
use windows::Win32::Graphics::Direct2D::{ID2D1DCRenderTarget, ID2D1Factory1, ID2D1StrokeStyle, D2D1_CAP_STYLE_ROUND, D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE_DEFAULT, D2D1_RENDER_TARGET_USAGE_NONE, D2D1_STROKE_STYLE_PROPERTIES1};
use windows::Win32::Graphics::DirectWrite::{IDWriteFactory, IDWriteInlineObject, IDWriteTextFormat, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_TEXT_ALIGNMENT_TRAILING, DWRITE_TRIMMING, DWRITE_TRIMMING_GRANULARITY_CHARACTER, DWRITE_WORD_WRAPPING_NO_WRAP};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::{CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC, SelectObject, AC_SRC_ALPHA, AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION, DIB_RGB_COLORS, HBITMAP, HDC, HGDIOBJ};
use windows::Win32::UI::WindowsAndMessaging::{UpdateLayeredWindow, ULW_ALPHA};

/// Offscreen 32-bit premultiplied BGRA DIB surface that commits transparently via `UpdateLayeredWindow`.
pub struct LayeredSurface {
    mem_dc: HDC,
    bitmap: HBITMAP,
    old_obj: HGDIOBJ,
    bits: *mut u8,
    width: i32,
    height: i32,
    cap_width: i32,
    cap_height: i32,
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
                cap_width: 0,
                cap_height: 0,
            })
        }
    }

    pub fn ensure_size(&mut self, width: i32, height: i32) -> windows::core::Result<()> {
        let w = width.max(1);
        let h = height.max(1);
        // Reuse an existing top-down DIB whenever width matches and allocated height fits `h`
        // (up to 8x capped headroom), eliminating `CreateDIBSection` churn when collapsed boxes retract.
        if !self.bitmap.is_invalid()
            && w == self.cap_width
            && h <= self.cap_height
            && self.cap_height <= h.max(256).saturating_mul(8)
        {
            self.width = w;
            self.height = h;
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
            self.cap_width = w;
            self.cap_height = h;
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

    pub fn release_bitmap(&mut self) {
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
            self.width = 0;
            self.height = 0;
            self.cap_width = 0;
            self.cap_height = 0;
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

/// Creates a standard 96-DPI premultiplied BGRA Direct2D DC render target.
pub fn create_dc_render_target(
    d2d_factory: &ID2D1Factory1,
) -> windows::core::Result<ID2D1DCRenderTarget> {
    let rt_props = D2D1_RENDER_TARGET_PROPERTIES {
        r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
        pixelFormat: D2D1_PIXEL_FORMAT {
            format: DXGI_FORMAT_B8G8R8A8_UNORM,
            alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
        },
        dpiX: 96.0,
        dpiY: 96.0,
        usage: D2D1_RENDER_TARGET_USAGE_NONE,
        minLevel: Default::default(),
    };
    unsafe { d2d_factory.CreateDCRenderTarget(&rt_props) }
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
