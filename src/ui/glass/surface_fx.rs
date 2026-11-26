use super::d2d_rect;
use crate::ui::{
    fill_liquid_silk_bgra, generate_acrylic_noise_bgra, PopupGlassRecipe, RectF, SILK_H,
    SILK_UPDATE_INTERVAL_MS, SILK_W,
};
use std::time::{Duration, Instant};
use windows::Win32::Graphics::Direct2D::Common::{D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_GRADIENT_STOP, D2D1_PIXEL_FORMAT, D2D_POINT_2F, D2D_RECT_F, D2D_SIZE_U};
use windows::Win32::Graphics::Direct2D::{ID2D1Bitmap, ID2D1BitmapBrush, ID2D1DCRenderTarget, ID2D1Geometry, D2D1_BITMAP_BRUSH_PROPERTIES, D2D1_BITMAP_INTERPOLATION_MODE_LINEAR, D2D1_BITMAP_INTERPOLATION_MODE_NEAREST_NEIGHBOR, D2D1_BITMAP_PROPERTIES, D2D1_EXTEND_MODE_CLAMP, D2D1_EXTEND_MODE_WRAP, D2D1_GAMMA_2_2, D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES, D2D1_ROUNDED_RECT};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;

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

    fill_liquid_silk_bgra(
        silk_buf,
        win_w,
        win_h,
        origin_left,
        origin_bottom,
        s,
        flow_phase,
        halo_colors,
        light_glass,
    );

    unsafe {
        if let Some(ref bmp) = *silk_bmp {
            if bmp
                .CopyFromMemory(None, silk_buf.as_ptr().cast(), SILK_W * 4)
                .is_err()
            {
                *silk_bmp = None;
            }
        }
        if silk_bmp.is_none() {
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

/// Lazily cached 128×128 tiled premultiplied BGRA micro-grain brush shared across frosted/acrylic popups.
#[derive(Default)]
pub struct PopupNoiseCache {
    brush: Option<ID2D1BitmapBrush>,
    max_alpha: u8,
}

impl PopupNoiseCache {
    pub fn clear(&mut self) {
        self.brush = None;
        self.max_alpha = 0;
    }

    pub fn ensure_brush(
        &mut self,
        rt: &ID2D1DCRenderTarget,
        max_alpha: u8,
    ) -> Option<&ID2D1BitmapBrush> {
        if max_alpha == 0 {
            return None;
        }
        if self.brush.is_none() || self.max_alpha != max_alpha {
            const NOISE_SIZE: u32 = 128;
            let bytes = generate_acrylic_noise_bgra(NOISE_SIZE as usize, 0x51A7_C301, max_alpha);
            let bmp_props = D2D1_BITMAP_PROPERTIES {
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: 96.0,
                dpiY: 96.0,
            };
            unsafe {
                let bmp = rt
                    .CreateBitmap(
                        D2D_SIZE_U {
                            width: NOISE_SIZE,
                            height: NOISE_SIZE,
                        },
                        Some(bytes.as_ptr().cast()),
                        NOISE_SIZE * 4,
                        &bmp_props,
                    )
                    .ok()?;
                let brush_props = D2D1_BITMAP_BRUSH_PROPERTIES {
                    extendModeX: D2D1_EXTEND_MODE_WRAP,
                    extendModeY: D2D1_EXTEND_MODE_WRAP,
                    interpolationMode: D2D1_BITMAP_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
                };
                self.brush = rt.CreateBitmapBrush(&bmp, Some(&brush_props), None).ok();
                self.max_alpha = max_alpha;
            }
        }
        self.brush.as_ref()
    }
}

/// Draws the style-harmonized glass interior layers (blurred backdrop, neutral base, frosted veil, grain, silk, diagonal sheen).
#[allow(clippy::too_many_arguments)]
pub unsafe fn draw_glass_interior(
    rt: &ID2D1DCRenderTarget,
    bounds: RectF,
    win_w: i32,
    win_h: i32,
    backdrop_bmp: Option<&ID2D1Bitmap>,
    silk_bmp: Option<&ID2D1Bitmap>,
    light_glass: bool,
    recipe: &PopupGlassRecipe,
    noise_cache: &mut PopupNoiseCache,
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
            recipe.backdrop_alpha,
            D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
            None,
        );
    } else {
        let fallback_bg = D2D1_COLOR_F {
            r: recipe.tint_rgb.0,
            g: recipe.tint_rgb.1,
            b: recipe.tint_rgb.2,
            a: (recipe.tint_alpha + 0.28).clamp(0.45, 0.94),
        };
        if let Ok(fb_brush) = rt.CreateSolidColorBrush(&fallback_bg, None) {
            rt.FillRectangle(&d2d_rect(bounds), &fb_brush);
        }
    }

    let glass_base = D2D1_COLOR_F {
        r: recipe.tint_rgb.0,
        g: recipe.tint_rgb.1,
        b: recipe.tint_rgb.2,
        a: recipe.tint_alpha,
    };
    if let Ok(base_brush) = rt.CreateSolidColorBrush(&glass_base, None) {
        rt.FillRectangle(&d2d_rect(bounds), &base_brush);
    }

    if recipe.frost_veil_top_alpha > 0.002 {
        let veil_stops = [
            D2D1_GRADIENT_STOP {
                position: 0.0,
                color: D2D1_COLOR_F {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: recipe.frost_veil_top_alpha,
                },
            },
            D2D1_GRADIENT_STOP {
                position: 0.55,
                color: D2D1_COLOR_F {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: recipe.frost_veil_mid_alpha,
                },
            },
            D2D1_GRADIENT_STOP {
                position: 1.0,
                color: D2D1_COLOR_F {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: recipe.frost_veil_bot_alpha,
                },
            },
        ];
        if let Ok(sc) =
            rt.CreateGradientStopCollection(&veil_stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP)
        {
            let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                startPoint: D2D_POINT_2F {
                    x: bounds.left,
                    y: bounds.top,
                },
                endPoint: D2D_POINT_2F {
                    x: bounds.left,
                    y: bounds.bottom,
                },
            };
            if let Ok(veil_brush) = rt.CreateLinearGradientBrush(&props, None, &sc) {
                rt.FillRectangle(&d2d_rect(bounds), &veil_brush);
            }
        }
    }

    if recipe.grain_max_alpha > 0 {
        if let Some(noise_brush) = noise_cache.ensure_brush(rt, recipe.grain_max_alpha) {
            rt.FillRectangle(&d2d_rect(bounds), noise_brush);
        }
    }

    if recipe.silk_alpha > 0.002 {
        if let Some(silk) = silk_bmp {
            rt.DrawBitmap(
                silk,
                Some(&full_win),
                recipe.silk_alpha,
                D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                None,
            );
        }
    }

    if recipe.sheen_scale > 0.01 {
        let ss = recipe.sheen_scale;
        let sheen_stops = [
            D2D1_GRADIENT_STOP {
                position: 0.0,
                color: D2D1_COLOR_F {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: (if light_glass { 0.26 } else { 0.14 }) * ss,
                },
            },
            D2D1_GRADIENT_STOP {
                position: 0.45,
                color: D2D1_COLOR_F {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: (if light_glass { 0.03 } else { 0.02 }) * ss,
                },
            },
            D2D1_GRADIENT_STOP {
                position: 1.0,
                color: D2D1_COLOR_F {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: (if light_glass { 0.10 } else { 0.06 }) * ss,
                },
            },
        ];
        if let Ok(sc) =
            rt.CreateGradientStopCollection(&sheen_stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP)
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

fn rim_gradient_stops(recipe: &PopupGlassRecipe) -> [D2D1_GRADIENT_STOP; 3] {
    [
        D2D1_GRADIENT_STOP {
            position: 0.0,
            color: D2D1_COLOR_F {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: recipe.rim_top_alpha,
            },
        },
        D2D1_GRADIENT_STOP {
            position: 0.55,
            color: D2D1_COLOR_F {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: recipe.rim_mid_alpha,
            },
        },
        D2D1_GRADIENT_STOP {
            position: 1.0,
            color: D2D1_COLOR_F {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: recipe.rim_bot_alpha,
            },
        },
    ]
}

/// Draws the restrained vertical-gradient glass rim highlight around a standalone closed rounded card.
#[allow(dead_code)]
pub unsafe fn draw_rounded_card_rim(
    rt: &ID2D1DCRenderTarget,
    card_rect: RectF,
    radius: f32,
    s: f32,
    recipe: &PopupGlassRecipe,
) {
    let rr = D2D1_ROUNDED_RECT {
        rect: d2d_rect(card_rect),
        radiusX: radius,
        radiusY: radius,
    };
    if recipe.rim_dark_sep_alpha > 0.004 {
        let def_col = D2D1_COLOR_F {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: recipe.rim_dark_sep_alpha,
        };
        if let Ok(def_brush) = rt.CreateSolidColorBrush(&def_col, None) {
            rt.DrawRoundedRectangle(&rr, &def_brush, 1.25 * s, None);
        }
    }

    let rim_stops = rim_gradient_stops(recipe);
    let stroke_w = recipe.rim_stroke_dip * s;
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

/// Draws the restrained glass rim highlight along an arbitrary closed/unified geometry contour.
pub unsafe fn draw_geometry_rim(
    rt: &ID2D1DCRenderTarget,
    geo: &ID2D1Geometry,
    left_x: f32,
    top_y: f32,
    bottom_y: f32,
    s: f32,
    recipe: &PopupGlassRecipe,
) {
    if recipe.rim_dark_sep_alpha > 0.004 {
        let def_col = D2D1_COLOR_F {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: recipe.rim_dark_sep_alpha,
        };
        if let Ok(def_brush) = rt.CreateSolidColorBrush(&def_col, None) {
            rt.DrawGeometry(geo, &def_brush, 1.25 * s, None);
        }
    }

    let rim_stops = rim_gradient_stops(recipe);
    let stroke_w = recipe.rim_stroke_dip * s;
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
