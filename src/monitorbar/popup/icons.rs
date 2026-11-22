use super::{MonitorPopupInner, PrimaryItem};
use crate::ui::glass::draw_settings_icon;
use windows::Win32::Graphics::Direct2D::Common::{D2D1_COLOR_F, D2D1_FIGURE_BEGIN_HOLLOW, D2D1_FIGURE_END_OPEN, D2D_POINT_2F, D2D_RECT_F, D2D_SIZE_F};
use windows::Win32::Graphics::Direct2D::{ID2D1SolidColorBrush, D2D1_ARC_SEGMENT, D2D1_ARC_SIZE_SMALL, D2D1_ELLIPSE, D2D1_ROUNDED_RECT, D2D1_SWEEP_DIRECTION_CLOCKWISE};

impl MonitorPopupInner {
    pub(super) unsafe fn draw_chevron(&self, cx: f32, cy: f32, s: f32, brush: &ID2D1SolidColorBrush) {
        let st = self.round_stroke.as_ref();
        let dir = if self.open_left { -1.0 } else { 1.0 };
        self.rt.DrawLine(
            D2D_POINT_2F {
                x: cx - 1.6 * s * dir,
                y: cy - 3.2 * s,
            },
            D2D_POINT_2F {
                x: cx + 1.8 * s * dir,
                y: cy,
            },
            brush,
            1.35 * s,
            st,
        );
        self.rt.DrawLine(
            D2D_POINT_2F {
                x: cx + 1.8 * s * dir,
                y: cy,
            },
            D2D_POINT_2F {
                x: cx - 1.6 * s * dir,
                y: cy + 3.2 * s,
            },
            brush,
            1.35 * s,
            st,
        );
    }

    pub(super) unsafe fn draw_wifi_signal_icon(
        &self,
        cx: f32,
        cy: f32,
        s: f32,
        quality: u8,
        active: D2D1_COLOR_F,
        dim: D2D1_COLOR_F,
    ) {
        let base_y = cy + 3.6 * s;
        let lit_bars = if quality >= 70 {
            3
        } else if quality >= 40 {
            2
        } else if quality > 0 {
            1
        } else {
            0
        };
        if let Ok(b0) = self.rt.CreateSolidColorBrush(&active, None) {
            self.rt.FillEllipse(
                &D2D1_ELLIPSE {
                    point: D2D_POINT_2F { x: cx, y: base_y },
                    radiusX: 1.2 * s,
                    radiusY: 1.2 * s,
                },
                &b0,
            );
        }
        let st = self.round_stroke.as_ref();
        for (idx, radius) in [(1, 3.2 * s), (2, 5.4 * s), (3, 7.6 * s)] {
            let col = if idx <= lit_bars { active } else { dim };
            if let Ok(b) = self.rt.CreateSolidColorBrush(&col, None) {
                if let Ok(geo) = self.d2d_factory.CreatePathGeometry() {
                    if let Ok(sink) = geo.Open() {
                        let dx = radius * 0.72;
                        let dy = radius * 0.68;
                        sink.BeginFigure(
                            D2D_POINT_2F {
                                x: cx - dx,
                                y: base_y - dy,
                            },
                            D2D1_FIGURE_BEGIN_HOLLOW,
                        );
                        sink.AddArc(&D2D1_ARC_SEGMENT {
                            point: D2D_POINT_2F {
                                x: cx + dx,
                                y: base_y - dy,
                            },
                            size: D2D_SIZE_F {
                                width: radius,
                                height: radius,
                            },
                            rotationAngle: 0.0,
                            sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                            arcSize: D2D1_ARC_SIZE_SMALL,
                        });
                        sink.EndFigure(D2D1_FIGURE_END_OPEN);
                        let _ = sink.Close();
                        self.rt.DrawGeometry(&geo, &b, 1.25 * s, st);
                    }
                }
            }
        }
    }

    pub(super) unsafe fn draw_bluetooth_icon(
        &self,
        cx: f32,
        cy: f32,
        s: f32,
        brush: &ID2D1SolidColorBrush,
    ) {
        let st = self.round_stroke.as_ref();
        let pts = [
            D2D_POINT_2F {
                x: cx - 2.6 * s,
                y: cy - 2.3 * s,
            },
            D2D_POINT_2F {
                x: cx + 2.8 * s,
                y: cy + 2.4 * s,
            },
            D2D_POINT_2F {
                x: cx,
                y: cy + 4.8 * s,
            },
            D2D_POINT_2F {
                x: cx,
                y: cy - 4.8 * s,
            },
            D2D_POINT_2F {
                x: cx + 2.8 * s,
                y: cy - 2.4 * s,
            },
            D2D_POINT_2F {
                x: cx - 2.6 * s,
                y: cy + 2.3 * s,
            },
        ];
        for seg in pts.windows(2) {
            self.rt.DrawLine(seg[0], seg[1], brush, 1.25 * s, st);
        }
    }

    pub(super) unsafe fn draw_primary_icon(
        &self,
        item: PrimaryItem,
        cx: f32,
        cy: f32,
        s: f32,
        brush: &ID2D1SolidColorBrush,
    ) {
        let st = self.round_stroke.as_ref();
        match item {
            PrimaryItem::WiFi => {
                self.rt.FillEllipse(
                    &D2D1_ELLIPSE {
                        point: D2D_POINT_2F {
                            x: cx,
                            y: cy + 3.4 * s,
                        },
                        radiusX: 1.2 * s,
                        radiusY: 1.2 * s,
                    },
                    brush,
                );
                for radius in [3.4 * s, 6.2 * s] {
                    if let Ok(geo) = self.d2d_factory.CreatePathGeometry() {
                        if let Ok(sink) = geo.Open() {
                            let dx = radius * 0.72;
                            let dy = radius * 0.68;
                            sink.BeginFigure(
                                D2D_POINT_2F {
                                    x: cx - dx,
                                    y: cy + 3.4 * s - dy,
                                },
                                D2D1_FIGURE_BEGIN_HOLLOW,
                            );
                            sink.AddArc(&D2D1_ARC_SEGMENT {
                                point: D2D_POINT_2F {
                                    x: cx + dx,
                                    y: cy + 3.4 * s - dy,
                                },
                                size: D2D_SIZE_F {
                                    width: radius,
                                    height: radius,
                                },
                                rotationAngle: 0.0,
                                sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                                arcSize: D2D1_ARC_SIZE_SMALL,
                            });
                            sink.EndFigure(D2D1_FIGURE_END_OPEN);
                            let _ = sink.Close();
                            self.rt.DrawGeometry(&geo, brush, 1.3 * s, st);
                        }
                    }
                }
            }
            PrimaryItem::Bluetooth => {
                self.draw_bluetooth_icon(cx, cy, s, brush);
            }
            PrimaryItem::Brightness => {
                self.rt.DrawEllipse(
                    &D2D1_ELLIPSE {
                        point: D2D_POINT_2F { x: cx, y: cy },
                        radiusX: 2.5 * s,
                        radiusY: 2.5 * s,
                    },
                    brush,
                    1.25 * s,
                    st,
                );
                let inner_r = 4.2 * s;
                let outer_r = 5.4 * s;
                for step in 0..8 {
                    let angle = (step as f32) * std::f32::consts::FRAC_PI_4;
                    let (sin_a, cos_a) = angle.sin_cos();
                    self.rt.DrawLine(
                        D2D_POINT_2F {
                            x: cx + inner_r * cos_a,
                            y: cy + inner_r * sin_a,
                        },
                        D2D_POINT_2F {
                            x: cx + outer_r * cos_a,
                            y: cy + outer_r * sin_a,
                        },
                        brush,
                        1.25 * s,
                        st,
                    );
                }
            }
            PrimaryItem::LeftClick => {
                let p0 = D2D_POINT_2F {
                    x: cx - 3.2 * s,
                    y: cy - 4.4 * s,
                };
                let p1 = D2D_POINT_2F {
                    x: cx - 3.2 * s,
                    y: cy + 4.4 * s,
                };
                let p2 = D2D_POINT_2F {
                    x: cx - 0.6 * s,
                    y: cy + 1.9 * s,
                };
                let p3 = D2D_POINT_2F {
                    x: cx + 3.2 * s,
                    y: cy + 1.5 * s,
                };
                self.rt.DrawLine(p0, p1, brush, 1.3 * s, st);
                self.rt.DrawLine(p1, p2, brush, 1.3 * s, st);
                self.rt.DrawLine(p2, p3, brush, 1.3 * s, st);
                self.rt.DrawLine(p3, p0, brush, 1.3 * s, st);
            }
            PrimaryItem::Display => {
                for dx in [-3.1 * s, 1.1 * s] {
                    for dy in [-3.1 * s, 1.1 * s] {
                        let rr = D2D1_ROUNDED_RECT {
                            rect: D2D_RECT_F {
                                left: cx + dx,
                                top: cy + dy,
                                right: cx + dx + 3.2 * s,
                                bottom: cy + dy + 3.2 * s,
                            },
                            radiusX: 0.9 * s,
                            radiusY: 0.9 * s,
                        };
                        self.rt.DrawRoundedRectangle(&rr, brush, 1.15 * s, st);
                    }
                }
            }
            PrimaryItem::Settings => {
                draw_settings_icon(&self.rt, cx, cy, s, brush);
            }
        }
    }
}
