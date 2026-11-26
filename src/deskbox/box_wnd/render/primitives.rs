//! Stateless Direct2D drawing primitives for DeskBox gradients, icons, highlights, text, and scrollbar.

use crate::color::Rgb;
use crate::ui::glass::{d2d_color_rgb, d2d_rect, danger_text_color, primary_text_color};
use crate::ui::RectF;
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_COLOR_F, D2D1_GRADIENT_STOP, D2D_POINT_2F, D2D_RECT_F,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_DRAW_TEXT_OPTIONS_CLIP, D2D1_ELLIPSE, D2D1_EXTEND_MODE_CLAMP, D2D1_GAMMA_2_2,
    D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES, D2D1_ROUNDED_RECT, ID2D1DCRenderTarget,
    ID2D1RenderTarget,
};

pub(super) unsafe fn fill_grad(
    rt: &ID2D1RenderTarget,
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
        if let Ok(b) = rt.CreateLinearGradientBrush(&props, None, &sc) {
            rt.FillRectangle(&rect, &b);
        }
    }
}

pub(super) unsafe fn fill_grad_rounded(
    rt: &ID2D1RenderTarget,
    rr: &D2D1_ROUNDED_RECT,
    start: D2D_POINT_2F,
    end: D2D_POINT_2F,
    stops: &[D2D1_GRADIENT_STOP],
) {
    if let Ok(sc) = rt.CreateGradientStopCollection(stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP) {
        let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
            startPoint: start,
            endPoint: end,
        };
        if let Ok(b) = rt.CreateLinearGradientBrush(&props, None, &sc) {
            rt.FillRoundedRectangle(rr, &b);
        }
    }
}

pub(super) unsafe fn draw_padlock_icon(
    rt: &ID2D1DCRenderTarget,
    cx: f32,
    cy: f32,
    locked: bool,
    col: D2D1_COLOR_F,
    s: f32,
    stroke: Option<&windows::Win32::Graphics::Direct2D::ID2D1StrokeStyle>,
) {
    let Ok(lb) = rt.CreateSolidColorBrush(&col, None) else {
        return;
    };
    let body = D2D1_ROUNDED_RECT {
        rect: D2D_RECT_F {
            left: cx - 3.6 * s,
            top: cy - 0.5 * s,
            right: cx + 3.6 * s,
            bottom: cy + 4.4 * s,
        },
        radiusX: 1.2 * s,
        radiusY: 1.2 * s,
    };
    rt.DrawRoundedRectangle(&body, &lb, 1.2 * s, stroke);
    // Left shackle leg (always anchored)
    rt.DrawLine(
        D2D_POINT_2F {
            x: cx - 2.2 * s,
            y: cy - 0.6 * s,
        },
        D2D_POINT_2F {
            x: cx - 2.2 * s,
            y: cy - 3.3 * s,
        },
        &lb,
        1.2 * s,
        stroke,
    );
    // Shackle top bar
    rt.DrawLine(
        D2D_POINT_2F {
            x: cx - 2.2 * s,
            y: cy - 3.3 * s,
        },
        D2D_POINT_2F {
            x: cx + 2.2 * s,
            y: cy - 3.3 * s,
        },
        &lb,
        1.2 * s,
        stroke,
    );
    // Right shackle leg: closed when locked, slightly lifted/open when unlocked
    let right_end_y = if locked { cy - 0.6 * s } else { cy - 2.0 * s };
    rt.DrawLine(
        D2D_POINT_2F {
            x: cx + 2.2 * s,
            y: cy - 3.3 * s,
        },
        D2D_POINT_2F {
            x: cx + 2.2 * s,
            y: right_end_y,
        },
        &lb,
        1.2 * s,
        stroke,
    );
}

pub(super) unsafe fn draw_icon_btn_hover(rt: &ID2D1DCRenderTarget, rect: RectF, s: f32, light_glass: bool) {
    let fill = if light_glass {
        D2D1_COLOR_F {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 0.08,
        }
    } else {
        D2D1_COLOR_F {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 0.13,
        }
    };
    let rr = D2D1_ROUNDED_RECT {
        rect: d2d_rect(rect),
        radiusX: 6.5 * s,
        radiusY: 6.5 * s,
    };
    if let Ok(b) = rt.CreateSolidColorBrush(&fill, None) {
        rt.FillRoundedRectangle(&rr, &b);
    }
}

pub(super) unsafe fn draw_item_cell_highlight(
    rt: &ID2D1RenderTarget,
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
        radiusX: 8.0 * s,
        radiusY: 8.0 * s,
    };
    if selected {
        let fill = d2d_color_rgb(
            accent,
            if light_glass {
                if hovered { 0.28 } else { 0.22 }
            } else if hovered {
                0.34
            } else {
                0.26
            },
        );
        if let Ok(b) = rt.CreateSolidColorBrush(&fill, None) {
            rt.FillRoundedRectangle(&rr, &b);
        }
        let border = d2d_color_rgb(accent, if light_glass { 0.52 } else { 0.62 });
        if let Ok(bb) = rt.CreateSolidColorBrush(&border, None) {
            rt.DrawRoundedRectangle(&rr, &bb, 1.0 * s, None);
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

pub(super) unsafe fn draw_item_remove_button(
    rt: &ID2D1RenderTarget,
    rect: RectF,
    hovered: bool,
    s: f32,
    light_glass: bool,
    round_stroke: Option<&windows::Win32::Graphics::Direct2D::ID2D1StrokeStyle>,
) {
    let cx = (rect.left + rect.right) * 0.5;
    let cy = (rect.top + rect.bottom) * 0.5;
    let r = (rect.width().min(rect.height()) * 0.5).max(1.0);
    let bg = if hovered {
        let mut danger = danger_text_color(light_glass);
        danger.a = 0.88;
        danger
    } else if light_glass {
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
            a: 0.15,
        }
    };
    if let Ok(bg_brush) = rt.CreateSolidColorBrush(&bg, None) {
        rt.FillEllipse(
            &D2D1_ELLIPSE {
                point: D2D_POINT_2F { x: cx, y: cy },
                radiusX: r,
                radiusY: r,
            },
            &bg_brush,
        );
    }

    let fg = if hovered {
        D2D1_COLOR_F {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 0.98,
        }
    } else {
        let mut p = primary_text_color(light_glass);
        p.a *= 0.82;
        p
    };
    let arm = (3.1 * s).min(r * 0.52);
    let sw = if hovered { 1.35 * s } else { 1.25 * s };
    if let Ok(fg_brush) = rt.CreateSolidColorBrush(&fg, None) {
        rt.DrawLine(
            D2D_POINT_2F {
                x: cx - arm,
                y: cy - arm,
            },
            D2D_POINT_2F {
                x: cx + arm,
                y: cy + arm,
            },
            &fg_brush,
            sw,
            round_stroke,
        );
        rt.DrawLine(
            D2D_POINT_2F {
                x: cx + arm,
                y: cy - arm,
            },
            D2D_POINT_2F {
                x: cx - arm,
                y: cy + arm,
            },
            &fg_brush,
            sw,
            round_stroke,
        );
    }
}

pub(super) unsafe fn draw_placeholder_icon(rt: &ID2D1RenderTarget, icon_rect: RectF, s: f32, light_glass: bool) {
    let col = if light_glass {
        D2D1_COLOR_F {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 0.12,
        }
    } else {
        D2D1_COLOR_F {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 0.16,
        }
    };
    let rr = D2D1_ROUNDED_RECT {
        rect: d2d_rect(icon_rect),
        radiusX: 5.0 * s,
        radiusY: 5.0 * s,
    };
    if let Ok(b) = rt.CreateSolidColorBrush(&col, None) {
        rt.FillRoundedRectangle(&rr, &b);
    }
}

pub(super) unsafe fn draw_missing_badge(rt: &ID2D1RenderTarget, icon_rect: RectF, s: f32) {
    let r = (4.5 * s).min(icon_rect.width() * 0.22);
    let badge_col = D2D1_COLOR_F {
        r: 0.92,
        g: 0.30,
        b: 0.28,
        a: 0.95,
    };
    let cx = icon_rect.right - r * 0.7;
    let cy = icon_rect.bottom - r * 0.7;
    if let Ok(b) = rt.CreateSolidColorBrush(&badge_col, None) {
        rt.FillEllipse(
            &D2D1_ELLIPSE {
                point: D2D_POINT_2F { x: cx, y: cy },
                radiusX: r,
                radiusY: r,
            },
            &b,
        );
    }
}

pub(super) unsafe fn draw_text_with_optional_shadow(
    dwrite_factory: &windows::Win32::Graphics::DirectWrite::IDWriteFactory,
    rt: &ID2D1RenderTarget,
    text: &str,
    rect: RectF,
    fmt: &windows::Win32::Graphics::DirectWrite::IDWriteTextFormat,
    color: D2D1_COLOR_F,
    shadow: Option<(f32, D2D1_COLOR_F)>,
) {
    let wide: Vec<u16> = text.encode_utf16().collect();
    let max_w = rect.width().max(4.0);
    let max_h = rect.height().max(12.0);
    let Ok(layout) = dwrite_factory.CreateTextLayout(&wide, fmt, max_w, max_h) else {
        return;
    };
    if let Some((dy, sh_col)) = shadow {
        if let Ok(sh_brush) = rt.CreateSolidColorBrush(&sh_col, None) {
            rt.DrawTextLayout(
                D2D_POINT_2F {
                    x: rect.left,
                    y: rect.top + dy,
                },
                &layout,
                &sh_brush,
                D2D1_DRAW_TEXT_OPTIONS_CLIP,
            );
        }
    }
    if let Ok(brush) = rt.CreateSolidColorBrush(&color, None) {
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
}

pub(super) unsafe fn draw_wrapped_text(
    dwrite_factory: &windows::Win32::Graphics::DirectWrite::IDWriteFactory,
    rt: &ID2D1RenderTarget,
    text: &str,
    rect: RectF,
    fmt: &windows::Win32::Graphics::DirectWrite::IDWriteTextFormat,
    color: D2D1_COLOR_F,
) {
    draw_text_with_optional_shadow(dwrite_factory, rt, text, rect, fmt, color, None);
}

pub(super) unsafe fn draw_scrollbar(
    rt: &ID2D1RenderTarget,
    thumb: RectF,
    light_glass: bool,
    active: bool,
) {
    if thumb.width() <= 0.5 || thumb.height() <= 0.5 {
        return;
    }
    let col = if light_glass {
        D2D1_COLOR_F {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: if active { 0.36 } else { 0.22 },
        }
    } else {
        D2D1_COLOR_F {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: if active { 0.42 } else { 0.25 },
        }
    };
    let rad = thumb.width() * 0.5;
    let rr = D2D1_ROUNDED_RECT {
        rect: d2d_rect(thumb),
        radiusX: rad,
        radiusY: rad,
    };
    if let Ok(b) = rt.CreateSolidColorBrush(&col, None) {
        rt.FillRoundedRectangle(&rr, &b);
    }
}
