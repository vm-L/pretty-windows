use crate::ui::{RectF, PANEL_RADIUS_DIP, STEP_MENISCUS_DIP};
use windows::Win32::Graphics::Direct2D::Common::{D2D1_BEZIER_SEGMENT, D2D1_COLOR_F, D2D1_FIGURE_BEGIN_FILLED, D2D1_FIGURE_END_CLOSED, D2D_POINT_2F, D2D_SIZE_F};
use windows::Win32::Graphics::Direct2D::{ID2D1DCRenderTarget, ID2D1Factory1, ID2D1GeometrySink, ID2D1PathGeometry1, D2D1_ARC_SEGMENT, D2D1_ARC_SIZE_SMALL, D2D1_SWEEP_DIRECTION_CLOCKWISE, D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE};

#[allow(clippy::too_many_arguments)]
unsafe fn emit_top_seam_transition(
    sink: &ID2D1GeometrySink,
    seam_x: f32,
    lt: f32,
    rt: f32,
    _left_w: f32,
    _left_tl: f32,
    _right_w: f32,
    _right_tr: f32,
    r_c: f32,
    step_r: f32,
    s: f32,
) {
    let dy = rt - lt;

    if dy.abs() < 1.0 * s {
        sink.AddLine(D2D_POINT_2F {
            x: seam_x,
            y: (lt + rt) * 0.5,
        });
    } else if dy >= 1.0 * s && dy < r_c + step_r {
        sink.AddLine(D2D_POINT_2F {
            x: seam_x - 10.0 * s,
            y: lt,
        });
        sink.AddBezier(&D2D1_BEZIER_SEGMENT {
            point1: D2D_POINT_2F {
                x: seam_x + 1.0 * s,
                y: lt,
            },
            point2: D2D_POINT_2F {
                x: seam_x + 1.0 * s,
                y: rt,
            },
            point3: D2D_POINT_2F {
                x: seam_x + 14.0 * s,
                y: rt,
            },
        });
    } else if dy <= -s && -dy < r_c + step_r {
        sink.AddLine(D2D_POINT_2F {
            x: seam_x - 14.0 * s,
            y: lt,
        });
        sink.AddBezier(&D2D1_BEZIER_SEGMENT {
            point1: D2D_POINT_2F {
                x: seam_x - 1.0 * s,
                y: lt,
            },
            point2: D2D_POINT_2F {
                x: seam_x - 1.0 * s,
                y: rt,
            },
            point3: D2D_POINT_2F {
                x: seam_x + 10.0 * s,
                y: rt,
            },
        });
    } else if dy >= r_c + step_r {
        sink.AddLine(D2D_POINT_2F {
            x: seam_x - r_c,
            y: lt,
        });
        sink.AddArc(&D2D1_ARC_SEGMENT {
            point: D2D_POINT_2F {
                x: seam_x,
                y: lt + r_c,
            },
            size: D2D_SIZE_F {
                width: r_c,
                height: r_c,
            },
            rotationAngle: 0.0,
            sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
            arcSize: D2D1_ARC_SIZE_SMALL,
        });
        sink.AddLine(D2D_POINT_2F {
            x: seam_x,
            y: rt - step_r,
        });
        sink.AddArc(&D2D1_ARC_SEGMENT {
            point: D2D_POINT_2F {
                x: seam_x + step_r,
                y: rt,
            },
            size: D2D_SIZE_F {
                width: step_r,
                height: step_r,
            },
            rotationAngle: 0.0,
            sweepDirection: D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE,
            arcSize: D2D1_ARC_SIZE_SMALL,
        });
    } else {
        sink.AddLine(D2D_POINT_2F {
            x: seam_x - step_r,
            y: lt,
        });
        sink.AddArc(&D2D1_ARC_SEGMENT {
            point: D2D_POINT_2F {
                x: seam_x,
                y: lt - step_r,
            },
            size: D2D_SIZE_F {
                width: step_r,
                height: step_r,
            },
            rotationAngle: 0.0,
            sweepDirection: D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE,
            arcSize: D2D1_ARC_SIZE_SMALL,
        });
        sink.AddLine(D2D_POINT_2F {
            x: seam_x,
            y: rt + r_c,
        });
        sink.AddArc(&D2D1_ARC_SEGMENT {
            point: D2D_POINT_2F {
                x: seam_x + r_c,
                y: rt,
            },
            size: D2D_SIZE_F {
                width: r_c,
                height: r_c,
            },
            rotationAngle: 0.0,
            sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
            arcSize: D2D1_ARC_SIZE_SMALL,
        });
    }
}

#[allow(clippy::too_many_arguments)]
unsafe fn emit_bottom_seam_transition(
    sink: &ID2D1GeometrySink,
    seam_x: f32,
    lb: f32,
    rb: f32,
    left_w: f32,
    left_bl: f32,
    right_w: f32,
    right_br: f32,
    r_c: f32,
    step_r: f32,
    s: f32,
) {
    let dy = lb - rb;
    if dy.abs() < 1.0 * s {
        sink.AddLine(D2D_POINT_2F {
            x: seam_x,
            y: (lb + rb) * 0.5,
        });
        return;
    }
    let max_left = (left_w - left_bl).max(2.0 * s);
    let max_right = (right_w - right_br).max(2.0 * s);
    if dy.abs() < r_c + step_r {
        let span_r = (14.0 * s).min(max_right);
        let span_l = (14.0 * s).min(max_left);
        sink.AddLine(D2D_POINT_2F {
            x: seam_x + span_r,
            y: rb,
        });
        sink.AddBezier(&D2D1_BEZIER_SEGMENT {
            point1: D2D_POINT_2F { x: seam_x, y: rb },
            point2: D2D_POINT_2F { x: seam_x, y: lb },
            point3: D2D_POINT_2F {
                x: seam_x - span_l,
                y: lb,
            },
        });
    } else if dy > 0.0 {
        let sr = step_r.min(max_right);
        let cr = r_c.min(max_left);
        sink.AddLine(D2D_POINT_2F {
            x: seam_x + sr,
            y: rb,
        });
        sink.AddArc(&D2D1_ARC_SEGMENT {
            point: D2D_POINT_2F {
                x: seam_x,
                y: rb + sr,
            },
            size: D2D_SIZE_F {
                width: sr,
                height: sr,
            },
            rotationAngle: 0.0,
            sweepDirection: D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE,
            arcSize: D2D1_ARC_SIZE_SMALL,
        });
        sink.AddLine(D2D_POINT_2F {
            x: seam_x,
            y: lb - cr,
        });
        sink.AddArc(&D2D1_ARC_SEGMENT {
            point: D2D_POINT_2F {
                x: seam_x - cr,
                y: lb,
            },
            size: D2D_SIZE_F {
                width: cr,
                height: cr,
            },
            rotationAngle: 0.0,
            sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
            arcSize: D2D1_ARC_SIZE_SMALL,
        });
    } else {
        let cr = r_c.min(max_right);
        let sr = step_r.min(max_left);
        sink.AddLine(D2D_POINT_2F {
            x: seam_x + cr,
            y: rb,
        });
        sink.AddArc(&D2D1_ARC_SEGMENT {
            point: D2D_POINT_2F {
                x: seam_x,
                y: rb - cr,
            },
            size: D2D_SIZE_F {
                width: cr,
                height: cr,
            },
            rotationAngle: 0.0,
            sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
            arcSize: D2D1_ARC_SIZE_SMALL,
        });
        sink.AddLine(D2D_POINT_2F {
            x: seam_x,
            y: lb + sr,
        });
        sink.AddArc(&D2D1_ARC_SEGMENT {
            point: D2D_POINT_2F {
                x: seam_x - sr,
                y: lb,
            },
            size: D2D_SIZE_F {
                width: sr,
                height: sr,
            },
            rotationAngle: 0.0,
            sweepDirection: D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE,
            arcSize: D2D1_ARC_SIZE_SMALL,
        });
    }
}

unsafe fn emit_unified_contour(
    sink: &ID2D1GeometrySink,
    primary: RectF,
    sub: Option<RectF>,
    open_left: bool,
    s: f32,
) {
    let r_c = PANEL_RADIUS_DIP * s;
    let step_r = STEP_MENISCUS_DIP * s;
    let eff_p = r_c
        .min((primary.height() * 0.45).max(4.0 * s))
        .min((primary.width() * 0.45).max(4.0 * s));

    match sub {
        None => {
            sink.BeginFigure(
                D2D_POINT_2F {
                    x: primary.left,
                    y: primary.bottom - eff_p,
                },
                D2D1_FIGURE_BEGIN_FILLED,
            );
            sink.AddLine(D2D_POINT_2F {
                x: primary.left,
                y: primary.top + eff_p,
            });
            sink.AddArc(&D2D1_ARC_SEGMENT {
                point: D2D_POINT_2F {
                    x: primary.left + eff_p,
                    y: primary.top,
                },
                size: D2D_SIZE_F {
                    width: eff_p,
                    height: eff_p,
                },
                rotationAngle: 0.0,
                sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                arcSize: D2D1_ARC_SIZE_SMALL,
            });
            sink.AddLine(D2D_POINT_2F {
                x: primary.right - eff_p,
                y: primary.top,
            });
            sink.AddArc(&D2D1_ARC_SEGMENT {
                point: D2D_POINT_2F {
                    x: primary.right,
                    y: primary.top + eff_p,
                },
                size: D2D_SIZE_F {
                    width: eff_p,
                    height: eff_p,
                },
                rotationAngle: 0.0,
                sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                arcSize: D2D1_ARC_SIZE_SMALL,
            });
            sink.AddLine(D2D_POINT_2F {
                x: primary.right,
                y: primary.bottom - eff_p,
            });
            sink.AddArc(&D2D1_ARC_SEGMENT {
                point: D2D_POINT_2F {
                    x: primary.right - eff_p,
                    y: primary.bottom,
                },
                size: D2D_SIZE_F {
                    width: eff_p,
                    height: eff_p,
                },
                rotationAngle: 0.0,
                sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                arcSize: D2D1_ARC_SIZE_SMALL,
            });
            sink.AddLine(D2D_POINT_2F {
                x: primary.left + eff_p,
                y: primary.bottom,
            });
            sink.AddArc(&D2D1_ARC_SEGMENT {
                point: D2D_POINT_2F {
                    x: primary.left,
                    y: primary.bottom - eff_p,
                },
                size: D2D_SIZE_F {
                    width: eff_p,
                    height: eff_p,
                },
                rotationAngle: 0.0,
                sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                arcSize: D2D1_ARC_SIZE_SMALL,
            });
        }
        Some(sub) if !open_left => {
            let seam_x = primary.right;
            let sub_r = r_c
                .min((sub.height() * 0.45).max(4.0 * s))
                .min((sub.width() * 0.45).max(4.0 * s));

            sink.BeginFigure(
                D2D_POINT_2F {
                    x: primary.left,
                    y: primary.bottom - eff_p,
                },
                D2D1_FIGURE_BEGIN_FILLED,
            );
            sink.AddLine(D2D_POINT_2F {
                x: primary.left,
                y: primary.top + eff_p,
            });
            sink.AddArc(&D2D1_ARC_SEGMENT {
                point: D2D_POINT_2F {
                    x: primary.left + eff_p,
                    y: primary.top,
                },
                size: D2D_SIZE_F {
                    width: eff_p,
                    height: eff_p,
                },
                rotationAngle: 0.0,
                sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                arcSize: D2D1_ARC_SIZE_SMALL,
            });

            emit_top_seam_transition(
                sink,
                seam_x,
                primary.top,
                sub.top,
                primary.width(),
                eff_p,
                sub.width(),
                sub_r,
                r_c,
                step_r,
                s,
            );

            sink.AddLine(D2D_POINT_2F {
                x: sub.right - sub_r,
                y: sub.top,
            });
            sink.AddArc(&D2D1_ARC_SEGMENT {
                point: D2D_POINT_2F {
                    x: sub.right,
                    y: sub.top + sub_r,
                },
                size: D2D_SIZE_F {
                    width: sub_r,
                    height: sub_r,
                },
                rotationAngle: 0.0,
                sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                arcSize: D2D1_ARC_SIZE_SMALL,
            });
            sink.AddLine(D2D_POINT_2F {
                x: sub.right,
                y: sub.bottom - sub_r,
            });
            sink.AddArc(&D2D1_ARC_SEGMENT {
                point: D2D_POINT_2F {
                    x: sub.right - sub_r,
                    y: sub.bottom,
                },
                size: D2D_SIZE_F {
                    width: sub_r,
                    height: sub_r,
                },
                rotationAngle: 0.0,
                sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                arcSize: D2D1_ARC_SIZE_SMALL,
            });

            emit_bottom_seam_transition(
                sink,
                seam_x,
                primary.bottom,
                sub.bottom,
                primary.width(),
                eff_p,
                sub.width(),
                sub_r,
                r_c,
                step_r,
                s,
            );

            sink.AddLine(D2D_POINT_2F {
                x: primary.left + eff_p,
                y: primary.bottom,
            });
            sink.AddArc(&D2D1_ARC_SEGMENT {
                point: D2D_POINT_2F {
                    x: primary.left,
                    y: primary.bottom - eff_p,
                },
                size: D2D_SIZE_F {
                    width: eff_p,
                    height: eff_p,
                },
                rotationAngle: 0.0,
                sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                arcSize: D2D1_ARC_SIZE_SMALL,
            });
        }
        Some(sub) => {
            let seam_x = primary.left;
            let sub_r = r_c
                .min((sub.height() * 0.45).max(4.0 * s))
                .min((sub.width() * 0.45).max(4.0 * s));

            sink.BeginFigure(
                D2D_POINT_2F {
                    x: sub.left,
                    y: sub.bottom - sub_r,
                },
                D2D1_FIGURE_BEGIN_FILLED,
            );
            sink.AddLine(D2D_POINT_2F {
                x: sub.left,
                y: sub.top + sub_r,
            });
            sink.AddArc(&D2D1_ARC_SEGMENT {
                point: D2D_POINT_2F {
                    x: sub.left + sub_r,
                    y: sub.top,
                },
                size: D2D_SIZE_F {
                    width: sub_r,
                    height: sub_r,
                },
                rotationAngle: 0.0,
                sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                arcSize: D2D1_ARC_SIZE_SMALL,
            });

            emit_top_seam_transition(
                sink,
                seam_x,
                sub.top,
                primary.top,
                sub.width(),
                sub_r,
                primary.width(),
                eff_p,
                r_c,
                step_r,
                s,
            );

            sink.AddLine(D2D_POINT_2F {
                x: primary.right - eff_p,
                y: primary.top,
            });
            sink.AddArc(&D2D1_ARC_SEGMENT {
                point: D2D_POINT_2F {
                    x: primary.right,
                    y: primary.top + eff_p,
                },
                size: D2D_SIZE_F {
                    width: eff_p,
                    height: eff_p,
                },
                rotationAngle: 0.0,
                sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                arcSize: D2D1_ARC_SIZE_SMALL,
            });
            sink.AddLine(D2D_POINT_2F {
                x: primary.right,
                y: primary.bottom - eff_p,
            });
            sink.AddArc(&D2D1_ARC_SEGMENT {
                point: D2D_POINT_2F {
                    x: primary.right - eff_p,
                    y: primary.bottom,
                },
                size: D2D_SIZE_F {
                    width: eff_p,
                    height: eff_p,
                },
                rotationAngle: 0.0,
                sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                arcSize: D2D1_ARC_SIZE_SMALL,
            });

            emit_bottom_seam_transition(
                sink,
                seam_x,
                sub.bottom,
                primary.bottom,
                sub.width(),
                sub_r,
                primary.width(),
                eff_p,
                r_c,
                step_r,
                s,
            );

            sink.AddLine(D2D_POINT_2F {
                x: sub.left + sub_r,
                y: sub.bottom,
            });
            sink.AddArc(&D2D1_ARC_SEGMENT {
                point: D2D_POINT_2F {
                    x: sub.left,
                    y: sub.bottom - sub_r,
                },
                size: D2D_SIZE_F {
                    width: sub_r,
                    height: sub_r,
                },
                rotationAngle: 0.0,
                sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                arcSize: D2D1_ARC_SIZE_SMALL,
            });
        }
    }

    sink.EndFigure(D2D1_FIGURE_END_CLOSED);
}

/// Builds a single fused Liquid Glass path silhouette for a primary card and optional cascading submenu.
pub fn build_unified_menu_geometry(
    d2d_factory: &ID2D1Factory1,
    primary: RectF,
    sub: Option<RectF>,
    open_left: bool,
    s: f32,
) -> windows::core::Result<ID2D1PathGeometry1> {
    unsafe {
        let geo = d2d_factory.CreatePathGeometry()?;
        let sink = geo.Open()?;
        emit_unified_contour(&sink, primary, sub, open_left, s);
        sink.Close()?;
        Ok(geo)
    }
}

/// Draws 3-pass ambient shadows behind a fused cascading menu silhouette.
#[allow(clippy::too_many_arguments)]
pub unsafe fn draw_unified_shadows(
    d2d_factory: &ID2D1Factory1,
    rt: &ID2D1DCRenderTarget,
    primary_rect: RectF,
    sub_rect: Option<RectF>,
    open_left: bool,
    win_w: i32,
    win_h: i32,
    light_glass: bool,
    s: f32,
) {
    let max_w = win_w as f32;
    let max_h = win_h as f32;
    let shadow_layers = if light_glass {
        [(7.5 * s, 0.022), (4.5 * s, 0.036), (2.0 * s, 0.058)]
    } else {
        [(7.5 * s, 0.035), (4.5 * s, 0.055), (2.0 * s, 0.09)]
    };
    for (expand, alpha) in shadow_layers {
        let side_exp = expand * 0.55;
        let top_exp = expand * 0.45;
        let bot_exp = expand * 0.65;
        let sh_primary = RectF::new(
            if open_left && sub_rect.is_some() {
                primary_rect.left
            } else {
                (primary_rect.left - side_exp).max(0.0)
            },
            (primary_rect.top - top_exp).max(0.0),
            if !open_left && sub_rect.is_some() {
                primary_rect.right
            } else {
                (primary_rect.right + side_exp).min(max_w)
            },
            (primary_rect.bottom + bot_exp).min(max_h),
        );
        let sh_sub = sub_rect.map(|sr| {
            RectF::new(
                if !open_left {
                    sr.left
                } else {
                    (sr.left - side_exp).max(0.0)
                },
                (sr.top - top_exp).max(0.0),
                if open_left {
                    sr.right
                } else {
                    (sr.right + side_exp).min(max_w)
                },
                (sr.bottom + bot_exp).min(max_h),
            )
        });

        if let (Ok(geo), Ok(sh_brush)) = (
            build_unified_menu_geometry(d2d_factory, sh_primary, sh_sub, open_left, s),
            rt.CreateSolidColorBrush(
                &D2D1_COLOR_F {
                    r: 0.0,
                    g: 0.0,
                    b: 0.0,
                    a: alpha,
                },
                None,
            ),
        ) {
            rt.FillGeometry(&geo, &sh_brush, None);
        }
    }
}
