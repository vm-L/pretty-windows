use super::{d2d_color_rgb, d2d_rect, LayeredSurface};
use crate::color::Rgb;
use crate::ui::{RectF, DIVIDER_H_DIP};
use windows::Win32::Foundation::{BOOL, RECT};
use windows::Win32::Graphics::Direct2D::Common::{D2D1_COLOR_F, D2D_POINT_2F, D2D_RECT_F};
use windows::Win32::Graphics::Direct2D::{ID2D1DCRenderTarget, ID2D1SolidColorBrush, D2D1_DRAW_TEXT_OPTIONS_CLIP, D2D1_ELLIPSE, D2D1_ROUNDED_RECT};
use windows::Win32::Graphics::DirectWrite::{IDWriteFactory, IDWriteTextFormat, DWRITE_TEXT_METRICS};
use windows::Win32::Graphics::Gdi::{CreateBitmap, DeleteObject};
use windows::Win32::UI::WindowsAndMessaging::{CreateIconIndirect, LoadCursorW, HCURSOR, ICONINFO, IDC_HAND};

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

/// Measures the unconstrained single-line width (physical px) of `text` using DirectWrite.
pub unsafe fn measure_text_width(
    dwrite_factory: &IDWriteFactory,
    text: &str,
    fmt: &IDWriteTextFormat,
    height_px: f32,
) -> Option<f32> {
    if text.is_empty() {
        return Some(0.0);
    }
    let wide: Vec<u16> = text.encode_utf16().collect();
    let layout = dwrite_factory
        .CreateTextLayout(&wide, fmt, 4096.0, height_px.max(12.0))
        .ok()?;
    let mut metrics = DWRITE_TEXT_METRICS::default();
    layout.GetMetrics(&mut metrics).ok()?;
    Some(metrics.widthIncludingTrailingWhitespace.max(metrics.width).max(0.0))
}

/// Draws single-line clipped DirectWrite text inside `rect` and returns the measured width (clamped to `rect.width()`).
pub unsafe fn draw_text_in_rect_measured(
    dwrite_factory: &IDWriteFactory,
    rt: &ID2D1DCRenderTarget,
    text: &str,
    rect: RectF,
    fmt: &IDWriteTextFormat,
    color: D2D1_COLOR_F,
) -> f32 {
    let wide: Vec<u16> = text.encode_utf16().collect();
    let max_w = rect.width().max(1.0);
    let Ok(layout) = dwrite_factory.CreateTextLayout(
        &wide,
        fmt,
        max_w,
        rect.height().max(12.0),
    ) else {
        return 0.0;
    };
    let Ok(brush) = rt.CreateSolidColorBrush(&color, None) else {
        return 0.0;
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
    let mut metrics = DWRITE_TEXT_METRICS::default();
    if layout.GetMetrics(&mut metrics).is_ok() {
        metrics.width.min(max_w).max(0.0)
    } else {
        0.0
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
    let _ = draw_text_in_rect_measured(dwrite_factory, rt, text, rect, fmt, color);
}

/// Draws a sleek rounded Liquid Glass slider track, accent fill, and responsive circular thumb knob.
#[allow(clippy::too_many_arguments)]
pub unsafe fn draw_glass_slider(
    rt: &ID2D1DCRenderTarget,
    track: RectF,
    level: f32,
    dimmed: bool,
    hovered_or_dragging: bool,
    light_glass: bool,
    accent: Rgb,
    scale: f32,
) {
    let rad = track.height() * 0.5;
    let groove_col = if light_glass {
        D2D1_COLOR_F {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 0.14,
        }
    } else {
        D2D1_COLOR_F {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 0.20,
        }
    };
    let groove_rr = D2D1_ROUNDED_RECT {
        rect: d2d_rect(track),
        radiusX: rad,
        radiusY: rad,
    };
    if let Ok(gb) = rt.CreateSolidColorBrush(&groove_col, None) {
        rt.FillRoundedRectangle(&groove_rr, &gb);
    }

    let lv = level.clamp(0.0, 1.0);
    let fill_w = track.width() * lv;
    if fill_w > 0.5 {
        let fill_rect = RectF::new(
            track.left,
            track.top,
            (track.left + fill_w).min(track.right),
            track.bottom,
        );
        let fill_rr = D2D1_ROUNDED_RECT {
            rect: d2d_rect(fill_rect),
            radiusX: rad,
            radiusY: rad,
        };
        let fill_alpha = if dimmed { 0.36 } else { 0.96 };
        if let Ok(fb) = rt.CreateSolidColorBrush(&d2d_color_rgb(accent, fill_alpha), None) {
            rt.FillRoundedRectangle(&fill_rr, &fb);
        }
    }

    let knob_x = track.left + fill_w;
    let knob_y = (track.top + track.bottom) * 0.5;
    let outer_r = if hovered_or_dragging {
        6.2 * scale
    } else {
        5.0 * scale
    };

    // Subtle dark outer halo for crisp legibility on bright wallpapers or accents
    let halo_col = D2D1_COLOR_F {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: if light_glass { 0.18 } else { 0.26 },
    };
    if let Ok(hb) = rt.CreateSolidColorBrush(&halo_col, None) {
        rt.FillEllipse(
            &D2D1_ELLIPSE {
                point: D2D_POINT_2F {
                    x: knob_x,
                    y: knob_y,
                },
                radiusX: outer_r + 0.9 * scale,
                radiusY: outer_r + 0.9 * scale,
            },
            &hb,
        );
    }

    let knob_col = D2D1_COLOR_F {
        r: 1.0,
        g: 1.0,
        b: 1.0,
        a: if dimmed { 0.86 } else { 0.99 },
    };
    if let Ok(kb) = rt.CreateSolidColorBrush(&knob_col, None) {
        rt.FillEllipse(
            &D2D1_ELLIPSE {
                point: D2D_POINT_2F {
                    x: knob_x,
                    y: knob_y,
                },
                radiusX: outer_r,
                radiusY: outer_r,
            },
            &kb,
        );
    }

    let inner_r = if hovered_or_dragging {
        outer_r * 0.48
    } else {
        outer_r * 0.40
    };
    let core_col = if dimmed {
        if light_glass {
            D2D1_COLOR_F {
                r: 0.48,
                g: 0.48,
                b: 0.52,
                a: 0.90,
            }
        } else {
            D2D1_COLOR_F {
                r: 0.62,
                g: 0.64,
                b: 0.68,
                a: 0.92,
            }
        }
    } else {
        d2d_color_rgb(accent, 0.98)
    };
    if let Ok(cb) = rt.CreateSolidColorBrush(&core_col, None) {
        rt.FillEllipse(
            &D2D1_ELLIPSE {
                point: D2D_POINT_2F {
                    x: knob_x,
                    y: knob_y,
                },
                radiusX: inner_r,
                radiusY: inner_r,
            },
            &cb,
        );
    }
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
    self_line(rt, cx - 1.5 * s, cy - 4.2 * s, cx + 4.7 * s, cy - 5.2 * s, brush, 1.8 * s);
}

#[inline]
unsafe fn self_line(
    rt: &ID2D1DCRenderTarget,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    brush: &ID2D1SolidColorBrush,
    stroke_w: f32,
) {
    rt.DrawLine(D2D_POINT_2F { x: x0, y: y0 }, D2D_POINT_2F { x: x1, y: y1 }, brush, stroke_w, None);
}

/// Draws the dual horizontal slider-bars "设置" vector icon centered at `(cx, cy)`.
pub unsafe fn draw_settings_icon(
    rt: &ID2D1DCRenderTarget,
    cx: f32,
    cy: f32,
    s: f32,
    brush: &ID2D1SolidColorBrush,
) {
    self_line(rt, cx - 5.2 * s, cy - 2.6 * s, cx + 5.2 * s, cy - 2.6 * s, brush, 1.3 * s);
    rt.FillEllipse(
        &D2D1_ELLIPSE {
            point: D2D_POINT_2F {
                x: cx - 1.6 * s,
                y: cy - 2.6 * s,
            },
            radiusX: 2.0 * s,
            radiusY: 2.0 * s,
        },
        brush,
    );
    self_line(rt, cx - 5.2 * s, cy + 2.6 * s, cx + 5.2 * s, cy + 2.6 * s, brush, 1.3 * s);
    rt.FillEllipse(
        &D2D1_ELLIPSE {
            point: D2D_POINT_2F {
                x: cx + 1.8 * s,
                y: cy + 2.6 * s,
            },
            radiusX: 2.0 * s,
            radiusY: 2.0 * s,
        },
        brush,
    );
}

/// Draws a Win11-style Liquid Glass pill toggle switch aligned right at `right_x` vertically centered at `cy`.
pub unsafe fn draw_pill_switch(
    rt: &ID2D1DCRenderTarget,
    right_x: f32,
    cy: f32,
    on: bool,
    accent: Rgb,
    s: f32,
    light_glass: bool,
) {
    let w = 28.0 * s;
    let h = 15.0 * s;
    let left = right_x - w;
    let top = cy - h * 0.5;
    let rr = D2D1_ROUNDED_RECT {
        rect: D2D_RECT_F {
            left,
            top,
            right: right_x,
            bottom: top + h,
        },
        radiusX: h * 0.5,
        radiusY: h * 0.5,
    };
    let track_col = if on {
        d2d_color_rgb(accent, 0.92)
    } else if light_glass {
        D2D1_COLOR_F {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 0.18,
        }
    } else {
        D2D1_COLOR_F {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 0.22,
        }
    };
    if let Ok(tb) = rt.CreateSolidColorBrush(&track_col, None) {
        rt.FillRoundedRectangle(&rr, &tb);
    }

    let thumb_r = 5.0 * s;
    let knob_x = if on {
        right_x - 2.5 * s - thumb_r
    } else {
        left + 2.5 * s + thumb_r
    };
    let accent_luma =
        (0.2126 * f32::from(accent.r) + 0.7152 * f32::from(accent.g) + 0.0722 * f32::from(accent.b))
            / 255.0;
    let knob_col = if on && accent_luma > 0.62 {
        D2D1_COLOR_F {
            r: 0.09,
            g: 0.10,
            b: 0.12,
            a: 0.92,
        }
    } else {
        D2D1_COLOR_F {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 0.96,
        }
    };
    if let Ok(kb) = rt.CreateSolidColorBrush(&knob_col, None) {
        rt.FillEllipse(
            &D2D1_ELLIPSE {
                point: D2D_POINT_2F { x: knob_x, y: cy },
                radiusX: thumb_r,
                radiusY: thumb_r,
            },
            &kb,
        );
    }
}

/// Renders a DPI-aware anti-aliased grabbing hand cursor (`HCURSOR`) with dark outline and white fill.
/// Returns `(cursor, is_owned_icon)` where `is_owned_icon` indicates `DestroyIcon` cleanup is required.
pub fn create_grabbing_cursor(
    rt: &ID2D1DCRenderTarget,
    dpi: u32,
) -> windows::core::Result<(HCURSOR, bool)> {
    let scale = (dpi.max(96) as f32) / 96.0;
    let size = ((24.0 * scale).round() as i32).clamp(24, 64);
    let u = (size as f32) / 24.0;

    let mut surf = LayeredSurface::new()?;
    surf.ensure_size(size, size)?;
    let bind_rect = RECT {
        left: 0,
        top: 0,
        right: size,
        bottom: size,
    };

    unsafe {
        rt.BindDC(surf.dc(), &bind_rect)?;
        rt.BeginDraw();
        rt.Clear(Some(&D2D1_COLOR_F {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 0.0,
        }));

        let outline_col = D2D1_COLOR_F {
            r: 0.06,
            g: 0.06,
            b: 0.08,
            a: 0.94,
        };
        let fill_col = D2D1_COLOR_F {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 1.0,
        };

        if let (Ok(outline_brush), Ok(fill_brush)) = (
            rt.CreateSolidColorBrush(&outline_col, None),
            rt.CreateSolidColorBrush(&fill_col, None),
        ) {
            let fingers: [(f32, f32, f32, f32); 4] = [
                (7.0, 5.8, 10.1, 12.2),
                (10.0, 4.7, 13.1, 12.2),
                (13.0, 5.1, 16.1, 12.2),
                (16.0, 6.4, 19.0, 12.6),
            ];
            let palm = (7.0f32, 9.6f32, 19.0f32, 19.4f32);
            let thumb = (3.8f32, 10.6f32, 8.2f32, 16.4f32);
            let pad = 1.15 * u;

            // Dark outline pass
            for &(l, t, r, b) in &fingers {
                rt.FillRoundedRectangle(
                    &D2D1_ROUNDED_RECT {
                        rect: D2D_RECT_F {
                            left: l * u - pad,
                            top: t * u - pad,
                            right: r * u + pad,
                            bottom: b * u + pad,
                        },
                        radiusX: 1.55 * u + pad,
                        radiusY: 1.55 * u + pad,
                    },
                    &outline_brush,
                );
            }
            rt.FillRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F {
                        left: thumb.0 * u - pad,
                        top: thumb.1 * u - pad,
                        right: thumb.2 * u + pad,
                        bottom: thumb.3 * u + pad,
                    },
                    radiusX: 2.0 * u + pad,
                    radiusY: 2.0 * u + pad,
                },
                &outline_brush,
            );
            rt.FillRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F {
                        left: palm.0 * u - pad,
                        top: palm.1 * u - pad,
                        right: palm.2 * u + pad,
                        bottom: palm.3 * u + pad,
                    },
                    radiusX: 3.8 * u + pad,
                    radiusY: 3.8 * u + pad,
                },
                &outline_brush,
            );

            // White fill pass
            for &(l, t, r, b) in &fingers {
                rt.FillRoundedRectangle(
                    &D2D1_ROUNDED_RECT {
                        rect: D2D_RECT_F {
                            left: l * u,
                            top: t * u,
                            right: r * u,
                            bottom: b * u,
                        },
                        radiusX: 1.55 * u,
                        radiusY: 1.55 * u,
                    },
                    &fill_brush,
                );
            }
            rt.FillRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F {
                        left: thumb.0 * u,
                        top: thumb.1 * u,
                        right: thumb.2 * u,
                        bottom: thumb.3 * u,
                    },
                    radiusX: 2.0 * u,
                    radiusY: 2.0 * u,
                },
                &fill_brush,
            );
            rt.FillRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F {
                        left: palm.0 * u,
                        top: palm.1 * u,
                        right: palm.2 * u,
                        bottom: palm.3 * u,
                    },
                    radiusX: 3.8 * u,
                    radiusY: 3.8 * u,
                },
                &fill_brush,
            );

            // Subtle knuckle dividers
            for x in [10.05f32, 13.05, 16.05] {
                self_line(rt, x * u, 6.2 * u, x * u, 11.4 * u, &outline_brush, 0.95 * u);
            }
        }

        rt.EndDraw(None, None)?;

        let row_words = (size as usize).div_ceil(16);
        let mask_bytes = vec![0u8; row_words * 2 * (size as usize)];
        let mask_bmp = CreateBitmap(size, size, 1, 1, Some(mask_bytes.as_ptr().cast()));
        if mask_bmp.is_invalid() {
            return Ok((LoadCursorW(None, IDC_HAND)?, false));
        }

        let info = ICONINFO {
            fIcon: BOOL(0),
            xHotspot: (size / 2) as u32,
            yHotspot: (size / 2) as u32,
            hbmMask: mask_bmp,
            hbmColor: surf.bitmap_handle(),
        };
        let icon_res = CreateIconIndirect(&info);
        let _ = DeleteObject(mask_bmp);

        match icon_res {
            Ok(hicon) if !hicon.is_invalid() => Ok((HCURSOR(hicon.0), true)),
            _ => Ok((LoadCursorW(None, IDC_HAND)?, false)),
        }
    }
}
