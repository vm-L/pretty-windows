use super::TrayMenuInner;
use super::super::layout::TrayMenuItem;
use crate::taskbar::TaskbarStyle;
use crate::ui::glass::draw_music_note_icon;
use windows::Win32::Graphics::Direct2D::Common::{D2D1_FIGURE_BEGIN_FILLED, D2D1_FIGURE_BEGIN_HOLLOW, D2D1_FIGURE_END_CLOSED, D2D1_FIGURE_END_OPEN, D2D_POINT_2F, D2D_RECT_F, D2D_SIZE_F};
use windows::Win32::Graphics::Direct2D::{ID2D1SolidColorBrush, D2D1_ARC_SEGMENT, D2D1_ARC_SIZE_LARGE, D2D1_ROUNDED_RECT, D2D1_SWEEP_DIRECTION_CLOCKWISE};

impl TrayMenuInner {
    pub(super) unsafe fn draw_primary_icon(
        &self,
        item: TrayMenuItem,
        cx: f32,
        cy: f32,
        s: f32,
        brush: &ID2D1SolidColorBrush,
    ) {
        let stroke = self.round_stroke.as_ref();
        match item {
            TrayMenuItem::MediaBar => {
                draw_music_note_icon(&self.rt, cx, cy, s, brush);
            }
            TrayMenuItem::MonitorBar => {
                let pts = [
                    D2D_POINT_2F {
                        x: cx - 5.2 * s,
                        y: cy + 0.6 * s,
                    },
                    D2D_POINT_2F {
                        x: cx - 2.4 * s,
                        y: cy + 0.6 * s,
                    },
                    D2D_POINT_2F {
                        x: cx - 0.8 * s,
                        y: cy - 4.0 * s,
                    },
                    D2D_POINT_2F {
                        x: cx + 1.2 * s,
                        y: cy + 3.8 * s,
                    },
                    D2D_POINT_2F {
                        x: cx + 2.8 * s,
                        y: cy - 0.8 * s,
                    },
                    D2D_POINT_2F {
                        x: cx + 5.2 * s,
                        y: cy - 0.8 * s,
                    },
                ];
                for seg in pts.windows(2) {
                    self.rt.DrawLine(seg[0], seg[1], brush, 1.35 * s, stroke);
                }
            }
            TrayMenuItem::TaskbarStyle => {
                // Minimal window monitor silhouette + accented bottom taskbar strip
                let outer = D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F {
                        left: cx - 5.2 * s,
                        top: cy - 4.4 * s,
                        right: cx + 5.2 * s,
                        bottom: cy + 4.4 * s,
                    },
                    radiusX: 1.8 * s,
                    radiusY: 1.8 * s,
                };
                self.rt.DrawRoundedRectangle(&outer, brush, 1.25 * s, stroke);
                let strip = D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F {
                        left: cx - 3.6 * s,
                        top: cy + 1.4 * s,
                        right: cx + 3.6 * s,
                        bottom: cy + 3.1 * s,
                    },
                    radiusX: 0.85 * s,
                    radiusY: 0.85 * s,
                };
                self.rt.FillRoundedRectangle(&strip, brush);
            }
            TrayMenuItem::Autostart => {
                // Upward launch arrow above a minimal tray bracket
                self.rt.DrawLine(
                    D2D_POINT_2F {
                        x: cx,
                        y: cy + 2.2 * s,
                    },
                    D2D_POINT_2F {
                        x: cx,
                        y: cy - 4.7 * s,
                    },
                    brush,
                    1.35 * s,
                    stroke,
                );
                self.rt.DrawLine(
                    D2D_POINT_2F {
                        x: cx - 3.0 * s,
                        y: cy - 1.7 * s,
                    },
                    D2D_POINT_2F {
                        x: cx,
                        y: cy - 4.8 * s,
                    },
                    brush,
                    1.35 * s,
                    stroke,
                );
                self.rt.DrawLine(
                    D2D_POINT_2F {
                        x: cx + 3.0 * s,
                        y: cy - 1.7 * s,
                    },
                    D2D_POINT_2F {
                        x: cx,
                        y: cy - 4.8 * s,
                    },
                    brush,
                    1.35 * s,
                    stroke,
                );
                self.rt.DrawLine(
                    D2D_POINT_2F {
                        x: cx - 4.4 * s,
                        y: cy + 2.7 * s,
                    },
                    D2D_POINT_2F {
                        x: cx - 4.4 * s,
                        y: cy + 4.7 * s,
                    },
                    brush,
                    1.3 * s,
                    stroke,
                );
                self.rt.DrawLine(
                    D2D_POINT_2F {
                        x: cx - 4.4 * s,
                        y: cy + 4.7 * s,
                    },
                    D2D_POINT_2F {
                        x: cx + 4.4 * s,
                        y: cy + 4.7 * s,
                    },
                    brush,
                    1.3 * s,
                    stroke,
                );
                self.rt.DrawLine(
                    D2D_POINT_2F {
                        x: cx + 4.4 * s,
                        y: cy + 4.7 * s,
                    },
                    D2D_POINT_2F {
                        x: cx + 4.4 * s,
                        y: cy + 2.7 * s,
                    },
                    brush,
                    1.3 * s,
                    stroke,
                );
            }
            TrayMenuItem::Exit => {
                // IEC power symbol: open top arc + vertical stem
                if let Ok(arc) = self.d2d_factory.CreatePathGeometry() {
                    if let Ok(sink) = arc.Open() {
                        sink.BeginFigure(
                            D2D_POINT_2F {
                                x: cx + 3.1 * s,
                                y: cy - 3.2 * s,
                            },
                            D2D1_FIGURE_BEGIN_HOLLOW,
                        );
                        sink.AddArc(&D2D1_ARC_SEGMENT {
                            point: D2D_POINT_2F {
                                x: cx - 3.1 * s,
                                y: cy - 3.2 * s,
                            },
                            size: D2D_SIZE_F {
                                width: 4.5 * s,
                                height: 4.5 * s,
                            },
                            rotationAngle: 0.0,
                            sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                            arcSize: D2D1_ARC_SIZE_LARGE,
                        });
                        sink.EndFigure(D2D1_FIGURE_END_OPEN);
                        let _ = sink.Close();
                        self.rt.DrawGeometry(&arc, brush, 1.35 * s, stroke);
                    }
                }
                self.rt.DrawLine(
                    D2D_POINT_2F {
                        x: cx,
                        y: cy - 5.2 * s,
                    },
                    D2D_POINT_2F {
                        x: cx,
                        y: cy - 0.3 * s,
                    },
                    brush,
                    1.35 * s,
                    stroke,
                );
            }
        }
    }

    pub(super) unsafe fn draw_style_icon(
        &self,
        style: TaskbarStyle,
        cx: f32,
        cy: f32,
        s: f32,
        brush: &ID2D1SolidColorBrush,
    ) {
        let stroke = self.round_stroke.as_ref();
        match style {
            TaskbarStyle::Default => {
                // Classic opaque taskbar bar inside rounded frame
                let frame = D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F {
                        left: cx - 5.0 * s,
                        top: cy - 4.2 * s,
                        right: cx + 5.0 * s,
                        bottom: cy + 4.2 * s,
                    },
                    radiusX: 1.8 * s,
                    radiusY: 1.8 * s,
                };
                self.rt.DrawRoundedRectangle(&frame, brush, 1.25 * s, stroke);
                let fill = D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F {
                        left: cx - 3.4 * s,
                        top: cy + 0.9 * s,
                        right: cx + 3.4 * s,
                        bottom: cy + 2.8 * s,
                    },
                    radiusX: 0.8 * s,
                    radiusY: 0.8 * s,
                };
                self.rt.FillRoundedRectangle(&fill, brush);
            }
            TaskbarStyle::Transparent => {
                // Crystal-clear hollow frame with light diagonal refraction glint
                let frame = D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F {
                        left: cx - 5.0 * s,
                        top: cy - 4.2 * s,
                        right: cx + 5.0 * s,
                        bottom: cy + 4.2 * s,
                    },
                    radiusX: 2.0 * s,
                    radiusY: 2.0 * s,
                };
                self.rt.DrawRoundedRectangle(&frame, brush, 1.25 * s, stroke);
                self.rt.DrawLine(
                    D2D_POINT_2F {
                        x: cx - 2.4 * s,
                        y: cy - 1.6 * s,
                    },
                    D2D_POINT_2F {
                        x: cx + 0.3 * s,
                        y: cy + 1.1 * s,
                    },
                    brush,
                    1.1 * s,
                    stroke,
                );
            }
            TaskbarStyle::Blur => {
                // Three softly staggered horizontal blur bands
                for (dy, half_w) in [(-3.0 * s, 4.4 * s), (0.0, 5.2 * s), (3.0 * s, 3.6 * s)] {
                    self.rt.DrawLine(
                        D2D_POINT_2F {
                            x: cx - half_w,
                            y: cy + dy,
                        },
                        D2D_POINT_2F {
                            x: cx + half_w,
                            y: cy + dy,
                        },
                        brush,
                        1.4 * s,
                        stroke,
                    );
                }
            }
            TaskbarStyle::Acrylic => {
                // Frosted rounded tile with micro 2x2 noise grain dots
                let frame = D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F {
                        left: cx - 4.8 * s,
                        top: cy - 4.4 * s,
                        right: cx + 4.8 * s,
                        bottom: cy + 4.4 * s,
                    },
                    radiusX: 2.0 * s,
                    radiusY: 2.0 * s,
                };
                self.rt.DrawRoundedRectangle(&frame, brush, 1.2 * s, stroke);
                for (dx, dy) in [(-1.8, -1.6), (1.8, -1.6), (-1.8, 1.6), (1.8, 1.6)] {
                    self.rt.FillEllipse(
                        &windows::Win32::Graphics::Direct2D::D2D1_ELLIPSE {
                            point: D2D_POINT_2F {
                                x: cx + dx * s,
                                y: cy + dy * s,
                            },
                            radiusX: 0.9 * s,
                            radiusY: 0.9 * s,
                        },
                        brush,
                    );
                }
            }
            TaskbarStyle::LiquidGlass => {
                // Signature 4-point Liquid Glass sparkle star + secondary droplet glint
                if let Ok(star_geo) = self.d2d_factory.CreatePathGeometry() {
                    if let Ok(sink) = star_geo.Open() {
                        let scx = cx - 0.6 * s;
                        let scy = cy + 0.4 * s;
                        let outer = 4.6 * s;
                        let inner = 1.45 * s;
                        sink.BeginFigure(
                            D2D_POINT_2F {
                                x: scx,
                                y: scy - outer,
                            },
                            D2D1_FIGURE_BEGIN_FILLED,
                        );
                        sink.AddLine(D2D_POINT_2F {
                            x: scx + inner,
                            y: scy - inner,
                        });
                        sink.AddLine(D2D_POINT_2F {
                            x: scx + outer,
                            y: scy,
                        });
                        sink.AddLine(D2D_POINT_2F {
                            x: scx + inner,
                            y: scy + inner,
                        });
                        sink.AddLine(D2D_POINT_2F {
                            x: scx,
                            y: scy + outer,
                        });
                        sink.AddLine(D2D_POINT_2F {
                            x: scx - inner,
                            y: scy + inner,
                        });
                        sink.AddLine(D2D_POINT_2F {
                            x: scx - outer,
                            y: scy,
                        });
                        sink.AddLine(D2D_POINT_2F {
                            x: scx - inner,
                            y: scy - inner,
                        });
                        sink.EndFigure(D2D1_FIGURE_END_CLOSED);
                        let _ = sink.Close();
                        self.rt.FillGeometry(&star_geo, brush, None);
                    }
                }
                self.rt.FillEllipse(
                    &windows::Win32::Graphics::Direct2D::D2D1_ELLIPSE {
                        point: D2D_POINT_2F {
                            x: cx + 4.1 * s,
                            y: cy - 3.6 * s,
                        },
                        radiusX: 1.05 * s,
                        radiusY: 1.05 * s,
                    },
                    brush,
                );
            }
        }
    }
}
