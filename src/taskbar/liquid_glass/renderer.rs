use super::super::config::TaskbarStyle;
use super::super::swca::generate_acrylic_noise_bgra;
use super::{MonitorGlassBackdrop, ShellCardGlassBackdrop};
use crate::ui::glass::{
    d2d_rect, draw_rounded_card_rim, identity_matrix, update_silk_bitmap,
};
use crate::ui::{
    lift_backdrop_alpha, popup_glass_recipe, sample_liquid_silk_field, GlassSurfaceStyle, RectF,
    SILK_UPDATE_INTERVAL_MS,
};
use std::mem::ManuallyDrop;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_GRADIENT_STOP, D2D1_PIXEL_FORMAT,
    D2D_POINT_2F, D2D_RECT_F, D2D_SIZE_U,
};
use windows::Win32::Graphics::Direct2D::{
    ID2D1Bitmap, ID2D1BitmapBrush, ID2D1DCRenderTarget, ID2D1Factory1, ID2D1Geometry,
    D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_BITMAP_BRUSH_PROPERTIES,
    D2D1_BITMAP_INTERPOLATION_MODE_LINEAR, D2D1_BITMAP_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
    D2D1_BITMAP_PROPERTIES, D2D1_COMBINE_MODE_EXCLUDE, D2D1_EXTEND_MODE_CLAMP,
    D2D1_EXTEND_MODE_WRAP, D2D1_GAMMA_2_2, D2D1_LAYER_OPTIONS_NONE, D2D1_LAYER_PARAMETERS,
    D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES, D2D1_ROUNDED_RECT,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;

/// Wide-aspect Liquid Silk bitmap dimensions matching full taskbar proportions (`192 × 16 = 3072` px)
/// to avoid horizontal aliasing stripes without increasing per-frame CPU work.
pub(super) const BAR_SILK_W: u32 = 192;
pub(super) const BAR_SILK_H: u32 = 16;

#[allow(clippy::too_many_arguments)]
fn update_wide_silk_bitmap(
    rt: &ID2D1DCRenderTarget,
    silk_bmp: &mut Option<ID2D1Bitmap>,
    silk_buf: &mut Vec<u8>,
    last_silk_tick: &mut Option<Instant>,
    win_w: i32,
    win_h: i32,
    s: f32,
    flow_phase: f64,
    halo_colors: &[(f32, f32, f32); 2],
    light_glass: bool,
) {
    let now = Instant::now();
    let need_rebuild = silk_bmp.is_none()
        || last_silk_tick
            .is_none_or(|t| now.duration_since(t) >= Duration::from_millis(SILK_UPDATE_INTERVAL_MS));
    if !need_rebuild {
        return;
    }
    *last_silk_tick = Some(now);

    let expected_len = (BAR_SILK_W * BAR_SILK_H * 4) as usize;
    if silk_buf.len() != expected_len {
        silk_buf.resize(expected_len, 0);
    }

    let scale_len = (420.0 * s).max(1.0);
    let phase = (flow_phase % 10_000.0) as f32;
    let wf = win_w.max(1) as f32;
    let hf = win_h.max(1) as f32;

    for gy in 0..BAR_SILK_H {
        let py = (((gy as f32) + 0.5) / (BAR_SILK_H as f32)) * hf;
        let y = (hf - py) / scale_len;
        for gx in 0..BAR_SILK_W {
            let px = (((gx as f32) + 0.5) / (BAR_SILK_W as f32)) * wf;
            let x = px / scale_len;
            let (r, g, b, a) = sample_liquid_silk_field(x, y, phase, halo_colors, light_glass);
            let idx = ((gy * BAR_SILK_W + gx) * 4) as usize;
            silk_buf[idx] = (b * a * 255.0).round().clamp(0.0, 255.0) as u8;
            silk_buf[idx + 1] = (g * a * 255.0).round().clamp(0.0, 255.0) as u8;
            silk_buf[idx + 2] = (r * a * 255.0).round().clamp(0.0, 255.0) as u8;
            silk_buf[idx + 3] = (a * 255.0).round().clamp(0.0, 255.0) as u8;
        }
    }

    unsafe {
        if let Some(ref bmp) = *silk_bmp {
            let _ = bmp.CopyFromMemory(None, silk_buf.as_ptr().cast(), BAR_SILK_W * 4);
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
                        width: BAR_SILK_W,
                        height: BAR_SILK_H,
                    },
                    Some(silk_buf.as_ptr().cast()),
                    BAR_SILK_W * 4,
                    &props,
                )
                .ok();
        }
    }
}

unsafe fn ensure_acrylic_noise_brush<'a>(
    rt: &ID2D1DCRenderTarget,
    noise_brush: &'a mut Option<ID2D1BitmapBrush>,
    noise_key: &mut Option<(bool, u8)>,
    light_glass: bool,
    max_alpha: u8,
) -> Option<&'a ID2D1BitmapBrush> {
    if noise_brush.is_none() || *noise_key != Some((light_glass, max_alpha)) {
        const NOISE_SIZE: u32 = 128;
        let noise_bytes =
            generate_acrylic_noise_bgra(NOISE_SIZE as usize, 0x51A7_C301, max_alpha);
        let bmp_props = D2D1_BITMAP_PROPERTIES {
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
                    width: NOISE_SIZE,
                    height: NOISE_SIZE,
                },
                Some(noise_bytes.as_ptr().cast()),
                NOISE_SIZE * 4,
                &bmp_props,
            )
            .ok()?;
        let brush_props = D2D1_BITMAP_BRUSH_PROPERTIES {
            extendModeX: D2D1_EXTEND_MODE_WRAP,
            extendModeY: D2D1_EXTEND_MODE_WRAP,
            interpolationMode: D2D1_BITMAP_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
        };
        let brush = rt.CreateBitmapBrush(&bmp, Some(&brush_props), None).ok()?;
        *noise_brush = Some(brush);
        *noise_key = Some((light_glass, max_alpha));
    }
    noise_brush.as_ref()
}

unsafe fn fill_linear_gradient(
    rt: &ID2D1DCRenderTarget,
    rect: D2D_RECT_F,
    start: D2D_POINT_2F,
    end: D2D_POINT_2F,
    stops: &[D2D1_GRADIENT_STOP],
) {
    if let Ok(sc) = rt.CreateGradientStopCollection(stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP) {
        let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
            startPoint: start,
            endPoint: end,
        };
        if let Ok(brush) = rt.CreateLinearGradientBrush(&props, None, &sc) {
            rt.FillRectangle(&rect, &brush);
        }
    }
}

unsafe fn draw_horizontal_gradient_line(
    rt: &ID2D1DCRenderTarget,
    wf: f32,
    y: f32,
    stroke_w: f32,
    stops: &[D2D1_GRADIENT_STOP],
) {
    if let Ok(sc) = rt.CreateGradientStopCollection(stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP) {
        let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
            startPoint: D2D_POINT_2F { x: 0.0, y },
            endPoint: D2D_POINT_2F { x: wf, y },
        };
        if let Ok(eb) = rt.CreateLinearGradientBrush(&props, None, &sc) {
            rt.DrawLine(
                D2D_POINT_2F { x: 0.0, y },
                D2D_POINT_2F { x: wf, y },
                &eb,
                stroke_w,
                None,
            );
        }
    }
}

unsafe fn draw_top_rim_line(
    rt: &ID2D1DCRenderTarget,
    wf: f32,
    s: f32,
    light_glass: bool,
    dark_sep_alpha: Option<f32>,
    edge_alpha: f32,
    mid_alpha: f32,
) {
    let edge_y = 0.6 * s;
    if let Some(sep_a) = dark_sep_alpha {
        let def_col = D2D1_COLOR_F {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: sep_a,
        };
        if let Ok(db) = rt.CreateSolidColorBrush(&def_col, None) {
            rt.DrawLine(
                D2D_POINT_2F { x: 0.0, y: edge_y },
                D2D_POINT_2F { x: wf, y: edge_y },
                &db,
                1.0 * s,
                None,
            );
        }
    }
    let edge_stops = [
        D2D1_GRADIENT_STOP {
            position: 0.0,
            color: D2D1_COLOR_F {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: edge_alpha,
            },
        },
        D2D1_GRADIENT_STOP {
            position: 0.5,
            color: D2D1_COLOR_F {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: mid_alpha,
            },
        },
        D2D1_GRADIENT_STOP {
            position: 1.0,
            color: D2D1_COLOR_F {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: edge_alpha,
            },
        },
    ];
    let stroke_w = if light_glass { 1.0 * s } else { 1.15 * s };
    draw_horizontal_gradient_line(rt, wf, edge_y, stroke_w, &edge_stops);
}

#[allow(clippy::too_many_arguments)]
pub(super) fn render_layered_backdrop(
    entry: &mut MonitorGlassBackdrop,
    style: TaskbarStyle,
    tb_rect: RECT,
    w: i32,
    h: i32,
    dpi: u32,
    flow_phase: f64,
    halo_colors: &[(f32, f32, f32); 2],
    light_glass: bool,
) -> windows::core::Result<()> {
    entry.surface.ensure_size(w, h)?;
    let s = (dpi.max(96) as f32) / 96.0;
    let wf = w as f32;
    let hf = h as f32;
    let has_live_blur = entry.has_live_blur;

    if style == TaskbarStyle::LiquidGlass {
        update_wide_silk_bitmap(
            &entry.rt,
            &mut entry.silk_bmp,
            &mut entry.silk_buf,
            &mut entry.last_silk_tick,
            w,
            h,
            s,
            flow_phase,
            halo_colors,
            light_glass,
        );
    }

    let bind_rect = RECT {
        left: 0,
        top: 0,
        right: entry.surface.width(),
        bottom: entry.surface.height(),
    };
    let full = d2d_rect(RectF::new(0.0, 0.0, wf, hf));

    unsafe {
        entry.rt.BindDC(entry.surface.dc(), &bind_rect)?;
        entry.rt.BeginDraw();
        entry.rt.Clear(Some(&D2D1_COLOR_F {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 0.0,
        }));

        match style {
            TaskbarStyle::Blur => {
                // Tier 1: Frosted Glass (毛玻璃) — creamy uniform frost tint + soft white
                // volumetric scattering veil + fine acid-etched micro-grain + gentle hairline.
                let base_alpha = lift_backdrop_alpha(match (light_glass, has_live_blur) {
                    (false, true) => 0.34,
                    (false, false) => 0.45,
                    (true, true) => 0.41,
                    (true, false) => 0.51,
                });
                let base_col = if light_glass {
                    D2D1_COLOR_F {
                        r: 248.0 / 255.0,
                        g: 250.0 / 255.0,
                        b: 254.0 / 255.0,
                        a: base_alpha,
                    }
                } else {
                    D2D1_COLOR_F {
                        r: 26.0 / 255.0,
                        g: 28.0 / 255.0,
                        b: 34.0 / 255.0,
                        a: base_alpha,
                    }
                };
                if let Ok(base_brush) = entry.rt.CreateSolidColorBrush(&base_col, None) {
                    entry.rt.FillRectangle(&full, &base_brush);
                }

                let frost_veil_stops = if light_glass {
                    [
                        D2D1_GRADIENT_STOP {
                            position: 0.0,
                            color: D2D1_COLOR_F {
                                r: 1.0,
                                g: 1.0,
                                b: 1.0,
                                a: 0.20,
                            },
                        },
                        D2D1_GRADIENT_STOP {
                            position: 0.55,
                            color: D2D1_COLOR_F {
                                r: 1.0,
                                g: 1.0,
                                b: 1.0,
                                a: 0.08,
                            },
                        },
                        D2D1_GRADIENT_STOP {
                            position: 1.0,
                            color: D2D1_COLOR_F {
                                r: 1.0,
                                g: 1.0,
                                b: 1.0,
                                a: 0.03,
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
                                a: 0.11,
                            },
                        },
                        D2D1_GRADIENT_STOP {
                            position: 0.55,
                            color: D2D1_COLOR_F {
                                r: 1.0,
                                g: 1.0,
                                b: 1.0,
                                a: 0.04,
                            },
                        },
                        D2D1_GRADIENT_STOP {
                            position: 1.0,
                            color: D2D1_COLOR_F {
                                r: 1.0,
                                g: 1.0,
                                b: 1.0,
                                a: 0.015,
                            },
                        },
                    ]
                };
                fill_linear_gradient(
                    &entry.rt,
                    full,
                    D2D_POINT_2F { x: 0.0, y: 0.0 },
                    D2D_POINT_2F { x: 0.0, y: hf },
                    &frost_veil_stops,
                );

                let grain_max_alpha = if light_glass { 5 } else { 6 };
                if let Some(noise_brush) = ensure_acrylic_noise_brush(
                    &entry.rt,
                    &mut entry.noise_brush,
                    &mut entry.noise_key,
                    light_glass,
                    grain_max_alpha,
                ) {
                    entry.rt.FillRectangle(&full, noise_brush);
                }

                let edge_y = 0.5 * s;
                let sep_col = if light_glass {
                    D2D1_COLOR_F {
                        r: 0.0,
                        g: 0.0,
                        b: 0.0,
                        a: 0.07,
                    }
                } else {
                    D2D1_COLOR_F {
                        r: 1.0,
                        g: 1.0,
                        b: 1.0,
                        a: 0.16,
                    }
                };
                if let Ok(sb) = entry.rt.CreateSolidColorBrush(&sep_col, None) {
                    entry.rt.DrawLine(
                        D2D_POINT_2F { x: 0.0, y: edge_y },
                        D2D_POINT_2F { x: wf, y: edge_y },
                        &sb,
                        1.0 * s,
                        None,
                    );
                }
            }
            TaskbarStyle::Acrylic => {
                // Tier 2: Classic Win11 dense frosted acrylic with steady tint, vertical finish & 128x128 grain.
                let base_alpha = lift_backdrop_alpha(match (light_glass, has_live_blur) {
                    (false, true) => 0.47,
                    (false, false) => 0.56,
                    (true, true) => 0.54,
                    (true, false) => 0.63,
                });
                let base_col = if light_glass {
                    D2D1_COLOR_F {
                        r: 246.0 / 255.0,
                        g: 247.0 / 255.0,
                        b: 250.0 / 255.0,
                        a: base_alpha,
                    }
                } else {
                    D2D1_COLOR_F {
                        r: 22.0 / 255.0,
                        g: 23.0 / 255.0,
                        b: 27.0 / 255.0,
                        a: base_alpha,
                    }
                };
                if let Ok(base_brush) = entry.rt.CreateSolidColorBrush(&base_col, None) {
                    entry.rt.FillRectangle(&full, &base_brush);
                }

                let frost_stops = [
                    D2D1_GRADIENT_STOP {
                        position: 0.0,
                        color: D2D1_COLOR_F {
                            r: 1.0,
                            g: 1.0,
                            b: 1.0,
                            a: if light_glass { 0.12 } else { 0.05 },
                        },
                    },
                    D2D1_GRADIENT_STOP {
                        position: 1.0,
                        color: D2D1_COLOR_F {
                            r: 1.0,
                            g: 1.0,
                            b: 1.0,
                            a: 0.0,
                        },
                    },
                ];
                fill_linear_gradient(
                    &entry.rt,
                    full,
                    D2D_POINT_2F { x: 0.0, y: 0.0 },
                    D2D_POINT_2F { x: 0.0, y: hf },
                    &frost_stops,
                );

                let acrylic_grain_alpha = if light_glass { 9 } else { 11 };
                if let Some(noise_brush) = ensure_acrylic_noise_brush(
                    &entry.rt,
                    &mut entry.noise_brush,
                    &mut entry.noise_key,
                    light_glass,
                    acrylic_grain_alpha,
                ) {
                    entry.rt.FillRectangle(&full, noise_brush);
                }

                if light_glass {
                    draw_top_rim_line(&entry.rt, wf, s, true, Some(0.08), 0.48, 0.48);
                } else {
                    draw_top_rim_line(&entry.rt, wf, s, false, None, 0.22, 0.22);
                }
            }
            TaskbarStyle::LiquidGlass => {
                // Tier 3: Optical Liquid Glass (无高光棱边 + 强柔焦) — crystal body +
                // soft bottom legibility gradient + wide-aspect liquid silk ribbons + gentle diffuse glow.
                let base_stops = match (light_glass, has_live_blur) {
                    (false, true) => [
                        (0.0, 0.14),
                        (0.52, 0.19),
                        (1.0, 0.31),
                    ],
                    (false, false) => [
                        (0.0, 0.22),
                        (0.52, 0.28),
                        (1.0, 0.40),
                    ],
                    (true, true) => [
                        (0.0, 0.19),
                        (0.55, 0.25),
                        (1.0, 0.34),
                    ],
                    (true, false) => [
                        (0.0, 0.28),
                        (0.55, 0.34),
                        (1.0, 0.44),
                    ],
                };
                let (br, bg, bb) = if light_glass {
                    (248.0 / 255.0, 250.0 / 255.0, 1.0)
                } else {
                    (14.0 / 255.0, 16.0 / 255.0, 22.0 / 255.0)
                };
                let body_grad = [
                    D2D1_GRADIENT_STOP {
                        position: base_stops[0].0,
                        color: D2D1_COLOR_F {
                            r: br,
                            g: bg,
                            b: bb,
                            a: lift_backdrop_alpha(base_stops[0].1),
                        },
                    },
                    D2D1_GRADIENT_STOP {
                        position: base_stops[1].0,
                        color: D2D1_COLOR_F {
                            r: br,
                            g: bg,
                            b: bb,
                            a: lift_backdrop_alpha(base_stops[1].1),
                        },
                    },
                    D2D1_GRADIENT_STOP {
                        position: base_stops[2].0,
                        color: D2D1_COLOR_F {
                            r: br,
                            g: bg,
                            b: bb,
                            a: lift_backdrop_alpha(base_stops[2].1),
                        },
                    },
                ];
                fill_linear_gradient(
                    &entry.rt,
                    full,
                    D2D_POINT_2F { x: 0.0, y: 0.0 },
                    D2D_POINT_2F { x: 0.0, y: hf },
                    &body_grad,
                );

                // 2. Wide-aspect domain-warped 3-ribbon Liquid Silk wisps (adapted to unified theme accent).
                if let Some(ref silk) = entry.silk_bmp {
                    let silk_alpha = if light_glass { 0.96 } else { 1.0 };
                    entry.rt.DrawBitmap(
                        silk,
                        Some(&full),
                        silk_alpha,
                        D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                        None,
                    );
                }

                // 3. Soft diagonal optical diffuse glow without bright glass edge rims.
                let sheen_stops = if light_glass {
                    [
                        D2D1_GRADIENT_STOP {
                            position: 0.0,
                            color: D2D1_COLOR_F {
                                r: 1.0,
                                g: 1.0,
                                b: 1.0,
                                a: 0.20,
                            },
                        },
                        D2D1_GRADIENT_STOP {
                            position: 0.42,
                            color: D2D1_COLOR_F {
                                r: 1.0,
                                g: 1.0,
                                b: 1.0,
                                a: 0.03,
                            },
                        },
                        D2D1_GRADIENT_STOP {
                            position: 0.75,
                            color: D2D1_COLOR_F {
                                r: 1.0,
                                g: 1.0,
                                b: 1.0,
                                a: 0.06,
                            },
                        },
                        D2D1_GRADIENT_STOP {
                            position: 1.0,
                            color: D2D1_COLOR_F {
                                r: 1.0,
                                g: 1.0,
                                b: 1.0,
                                a: 0.11,
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
                                a: 0.12,
                            },
                        },
                        D2D1_GRADIENT_STOP {
                            position: 0.42,
                            color: D2D1_COLOR_F {
                                r: 1.0,
                                g: 1.0,
                                b: 1.0,
                                a: 0.015,
                            },
                        },
                        D2D1_GRADIENT_STOP {
                            position: 0.75,
                            color: D2D1_COLOR_F {
                                r: 1.0,
                                g: 1.0,
                                b: 1.0,
                                a: 0.035,
                            },
                        },
                        D2D1_GRADIENT_STOP {
                            position: 1.0,
                            color: D2D1_COLOR_F {
                                r: 1.0,
                                g: 1.0,
                                b: 1.0,
                                a: 0.07,
                            },
                        },
                    ]
                };
                fill_linear_gradient(
                    &entry.rt,
                    full,
                    D2D_POINT_2F { x: 0.0, y: 0.0 },
                    D2D_POINT_2F { x: wf, y: hf },
                    &sheen_stops,
                );
            }
            TaskbarStyle::Default | TaskbarStyle::Transparent => {}
        }

        entry.rt.EndDraw(None, None)?;
    }

    entry
        .surface
        .commit(entry.glass_hwnd, tb_rect.left, tb_rect.top);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn render_shell_card_layered_backdrop(
    d2d_factory: &ID2D1Factory1,
    ov: &mut ShellCardGlassBackdrop,
    style: TaskbarStyle,
    glass_rect: RECT,
    card_rect: RECT,
    pad_px: i32,
    dpi: u32,
    flow_phase: f64,
    halo_colors: &[(f32, f32, f32); 2],
    light_glass: bool,
) -> windows::core::Result<()> {
    let glass_w = (glass_rect.right - glass_rect.left).max(1);
    let glass_h = (glass_rect.bottom - glass_rect.top).max(1);
    let card_w = (card_rect.right - card_rect.left).max(1);
    let card_h = (card_rect.bottom - card_rect.top).max(1);
    ov.surface.ensure_size(glass_w, glass_h)?;

    let s = (dpi.max(96) as f32) / 96.0;
    let pad_f = pad_px.max(0) as f32;
    let wf = card_w as f32;
    let hf = card_h as f32;
    let radius = (8.0 * s).min((wf.min(hf) * 0.45).max(2.0));
    let card_local = RectF::new(pad_f, pad_f, pad_f + wf, pad_f + hf);
    let card_d2d = d2d_rect(card_local);
    let has_live_blur = ov.has_live_blur;
    let recipe = popup_glass_recipe(GlassSurfaceStyle::from(style), light_glass, 0.5);

    if style == TaskbarStyle::LiquidGlass {
        let force_silk = ov.silk_bmp.is_none();
        update_silk_bitmap(
            &ov.rt,
            &mut ov.silk_bmp,
            &mut ov.silk_buf,
            &mut ov.last_silk_tick,
            force_silk,
            card_w,
            card_h,
            0.0,
            hf,
            s,
            flow_phase,
            halo_colors,
            light_glass,
        );
    }

    let bind_rect = RECT {
        left: 0,
        top: 0,
        right: ov.surface.width(),
        bottom: ov.surface.height(),
    };

    unsafe {
        ov.rt.BindDC(ov.surface.dc(), &bind_rect)?;
        ov.rt.BeginDraw();
        ov.rt.Clear(Some(&D2D1_COLOR_F {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 0.0,
        }));

        let shadow_layers: [(f32, f32); 3] = if light_glass {
            [(7.5, 0.022), (4.5, 0.036), (2.0, 0.058)]
        } else {
            [(7.5, 0.035), (4.5, 0.055), (2.0, 0.090)]
        };
        let geo_key = (glass_w, glass_h, card_w, card_h, pad_px, dpi);
        if ov.shadow_geo_key != Some(geo_key) || ov.cached_clip_geo.is_none() {
            ov.cached_clip_geo = None;
            ov.cached_shadow_rings = [None, None, None];
            let rr = D2D1_ROUNDED_RECT {
                rect: card_d2d,
                radiusX: radius,
                radiusY: radius,
            };
            if let Ok(clip_raw) = d2d_factory.CreateRoundedRectangleGeometry(&rr) {
                let clip_geo: ID2D1Geometry = clip_raw.into();
                if pad_px > 0 {
                    let shadow_scale = (pad_f / (10.0 * s)).clamp(0.55, 1.0);
                    let gf_w = glass_w as f32;
                    let gf_h = glass_h as f32;
                    for (idx, (expand_dip, _)) in shadow_layers.iter().enumerate() {
                        let expand = expand_dip * shadow_scale * s;
                        let side_exp = expand * 0.55;
                        let top_exp = expand * 0.45;
                        let bot_exp = expand * 0.65;
                        let sh_rect = RectF::new(
                            (card_local.left - side_exp).max(0.0),
                            (card_local.top - top_exp).max(0.0),
                            (card_local.right + side_exp).min(gf_w),
                            (card_local.bottom + bot_exp).min(gf_h),
                        );
                        let sh_rr = D2D1_ROUNDED_RECT {
                            rect: d2d_rect(sh_rect),
                            radiusX: radius + side_exp * 0.45,
                            radiusY: radius + side_exp * 0.45,
                        };
                        if let Ok(outer_raw) = d2d_factory.CreateRoundedRectangleGeometry(&sh_rr) {
                            let outer_geo: ID2D1Geometry = outer_raw.into();
                            if let Ok(ring) = d2d_factory.CreatePathGeometry() {
                                if let Ok(sink) = ring.Open() {
                                    if outer_geo
                                        .CombineWithGeometry(
                                            &clip_geo,
                                            D2D1_COMBINE_MODE_EXCLUDE,
                                            None,
                                            0.25,
                                            &sink,
                                        )
                                        .is_ok()
                                        && sink.Close().is_ok()
                                    {
                                        ov.cached_shadow_rings[idx] = Some(ring.into());
                                    }
                                }
                            }
                        }
                    }
                }
                ov.cached_clip_geo = Some(clip_geo);
                ov.shadow_geo_key = Some(geo_key);
            } else {
                ov.shadow_geo_key = None;
            }
        }

        if let Some(clip_geo) = ov.cached_clip_geo.clone() {
            if pad_px > 0 {
                let mult = if style == TaskbarStyle::Transparent {
                    0.65
                } else {
                    1.0
                };
                for (idx, &(_, base_alpha)) in shadow_layers.iter().enumerate() {
                    if let Some(ref ring_geo) = ov.cached_shadow_rings[idx] {
                        let sh_col = D2D1_COLOR_F {
                            r: 0.0,
                            g: 0.0,
                            b: 0.0,
                            a: base_alpha * mult,
                        };
                        if let Ok(sh_brush) = ov.rt.CreateSolidColorBrush(&sh_col, None) {
                            ov.rt.FillGeometry(ring_geo, &sh_brush, None);
                        }
                    }
                }
            }

            let outer_params = D2D1_LAYER_PARAMETERS {
                contentBounds: card_d2d,
                geometricMask: ManuallyDrop::new(Some(clip_geo)),
                maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                maskTransform: identity_matrix(),
                opacity: 1.0,
                opacityBrush: ManuallyDrop::new(None),
                layerOptions: D2D1_LAYER_OPTIONS_NONE,
            };
            ov.rt.PushLayer(&outer_params, &ov.layer);

            let (tr, tg, tb) = recipe.tint_rgb;
            let base_a = match (style, light_glass, has_live_blur) {
                (TaskbarStyle::Transparent, false, _) => lift_backdrop_alpha(0.31),
                (TaskbarStyle::Transparent, true, _) => lift_backdrop_alpha(0.34),
                (TaskbarStyle::Blur, false, true) => lift_backdrop_alpha(0.36),
                (TaskbarStyle::Blur, false, false) => lift_backdrop_alpha(0.47),
                (TaskbarStyle::Blur, true, true) => lift_backdrop_alpha(0.42),
                (TaskbarStyle::Blur, true, false) => lift_backdrop_alpha(0.54),
                (TaskbarStyle::Acrylic, false, true) => lift_backdrop_alpha(0.47),
                (TaskbarStyle::Acrylic, false, false) => lift_backdrop_alpha(0.57),
                (TaskbarStyle::Acrylic, true, true) => lift_backdrop_alpha(0.54),
                (TaskbarStyle::Acrylic, true, false) => lift_backdrop_alpha(0.65),
                (TaskbarStyle::LiquidGlass, false, true) => lift_backdrop_alpha(0.26),
                (TaskbarStyle::LiquidGlass, false, false) => lift_backdrop_alpha(0.38),
                (TaskbarStyle::LiquidGlass, true, true) => lift_backdrop_alpha(0.31),
                (TaskbarStyle::LiquidGlass, true, false) => lift_backdrop_alpha(0.43),
                (TaskbarStyle::Default, _, _) => 0.0,
            };
            let base_col = D2D1_COLOR_F {
                r: tr,
                g: tg,
                b: tb,
                a: base_a,
            };
            if let Ok(base_brush) = ov.rt.CreateSolidColorBrush(&base_col, None) {
                ov.rt.FillRectangle(&card_d2d, &base_brush);
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
                fill_linear_gradient(
                    &ov.rt,
                    card_d2d,
                    D2D_POINT_2F {
                        x: card_local.left,
                        y: card_local.top,
                    },
                    D2D_POINT_2F {
                        x: card_local.left,
                        y: card_local.bottom,
                    },
                    &veil_stops,
                );
            }

            if recipe.grain_max_alpha > 0 {
                if let Some(noise_brush) = ov.noise_cache.ensure_brush(&ov.rt, recipe.grain_max_alpha)
                {
                    ov.rt.FillRectangle(&card_d2d, noise_brush);
                }
            }

            if style == TaskbarStyle::LiquidGlass && recipe.silk_alpha > 0.002 {
                if let Some(ref silk) = ov.silk_bmp {
                    ov.rt.DrawBitmap(
                        silk,
                        Some(&card_d2d),
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
                            a: (if light_glass { 0.24 } else { 0.14 }) * ss,
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
                            a: (if light_glass { 0.09 } else { 0.06 }) * ss,
                        },
                    },
                ];
                fill_linear_gradient(
                    &ov.rt,
                    card_d2d,
                    D2D_POINT_2F {
                        x: card_local.left,
                        y: card_local.top,
                    },
                    D2D_POINT_2F {
                        x: card_local.right,
                        y: card_local.bottom,
                    },
                    &sheen_stops,
                );
            }

            ov.rt.PopLayer();
            let _ = ManuallyDrop::into_inner(outer_params.geometricMask);

            let inset = 0.5 * s;
            let rim_rect = RectF::new(
                card_local.left + inset,
                card_local.top + inset,
                (card_local.right - inset).max(card_local.left + inset + 1.0),
                (card_local.bottom - inset).max(card_local.top + inset + 1.0),
            );
            let rim_r = (radius - inset).max(1.0);
            draw_rounded_card_rim(&ov.rt, rim_rect, rim_r, s, &recipe);
        }

        ov.rt.EndDraw(None, None)?;
    }

    ov.surface.commit(ov.glass_hwnd, glass_rect.left, glass_rect.top);
    Ok(())
}
