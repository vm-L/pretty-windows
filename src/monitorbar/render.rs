//! Direct2D / DirectWrite renderer for the MonitorBar taskbar widget.

use super::layout::{MetricSlot, MonitorLayout};
use super::metrics::{format_percent, format_speed, MetricsSnapshot, HIGH_LOAD_THRESHOLD};
use crate::color::Rgb;
use crate::ui::glass::{
    create_text_format, d2d_rect, draw_text_in_rect, identity_matrix, primary_text_color,
    LayeredSurface,
};
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_FIGURE_BEGIN_HOLLOW, D2D1_FIGURE_END_OPEN,
    D2D1_PIXEL_FORMAT, D2D_POINT_2F, D2D_RECT_F, D2D_SIZE_F,
};
use windows::Win32::Graphics::Direct2D::{
    ID2D1DCRenderTarget, ID2D1Factory1, D2D1_ARC_SEGMENT, D2D1_ARC_SIZE_SMALL,
    D2D1_DRAW_TEXT_OPTIONS_CLIP, D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE_DEFAULT,
    D2D1_RENDER_TARGET_USAGE_NONE, D2D1_ROUNDED_RECT, D2D1_SWEEP_DIRECTION_CLOCKWISE,
    D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE,
};
use windows::Win32::Graphics::DirectWrite::{IDWriteFactory, IDWriteTextFormat};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::UI::WindowsAndMessaging::HCURSOR;

/// Inputs required to render one frame of the MonitorBar layered window.
pub struct MonitorRenderInput<'a> {
    pub layout: &'a MonitorLayout,
    pub snapshot: MetricsSnapshot,
    pub dpi: u32,
    pub light_taskbar: bool,
    pub accent: Rgb,
    pub hover_t: f32,
    pub dragging: bool,
    pub popup_open_span_x: Option<(f32, f32)>,
}

/// Hardware-backed Direct2D renderer committing frames into a 32-bit premultiplied layered surface.
pub struct MonitorRenderer {
    pub d2d_factory: ID2D1Factory1,
    pub dwrite_factory: IDWriteFactory,
    rt: ID2D1DCRenderTarget,
    surface: LayeredSurface,
    font_scale_key: u32,
    label_fmt: Option<IDWriteTextFormat>,
    value_fmt: Option<IDWriteTextFormat>,
}

impl MonitorRenderer {
    pub fn new_with_factories(
        d2d_factory: ID2D1Factory1,
        dwrite_factory: IDWriteFactory,
    ) -> windows::core::Result<Self> {
        unsafe {
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
            let rt = d2d_factory.CreateDCRenderTarget(&rt_props)?;
            rt.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            let surface = LayeredSurface::new()?;

            Ok(Self {
                d2d_factory,
                dwrite_factory,
                rt,
                surface,
                font_scale_key: 0,
                label_fmt: None,
                value_fmt: None,
            })
        }
    }

    fn ensure_formats(&mut self, dpi: u32) -> windows::core::Result<()> {
        let key = dpi.max(96);
        if self.font_scale_key == key && self.label_fmt.is_some() && self.value_fmt.is_some() {
            return Ok(());
        }
        let scale = (key as f32) / 96.0;
        let font_size = (11.5 * scale).max(10.0);
        self.label_fmt = Some(create_text_format(
            &self.dwrite_factory,
            font_size,
            false,
            false,
            false,
        )?);
        self.value_fmt = Some(create_text_format(
            &self.dwrite_factory,
            font_size,
            false,
            true,
            false,
        )?);
        self.font_scale_key = key;
        let _ = D2D1_DRAW_TEXT_OPTIONS_CLIP;
        Ok(())
    }

    pub fn render(&mut self, input: &MonitorRenderInput<'_>) -> windows::core::Result<()> {
        let w = (input.layout.width_px.round() as i32).max(1);
        let h = (input.layout.height_px.round() as i32).max(1);
        self.surface.ensure_size(w, h)?;
        self.ensure_formats(input.dpi)?;

        let label_fmt = self
            .label_fmt
            .as_ref()
            .expect("label_fmt initialized by ensure_formats");
        let value_fmt = self
            .value_fmt
            .as_ref()
            .expect("value_fmt initialized by ensure_formats");

        let bind_rect = RECT {
            left: 0,
            top: 0,
            right: w,
            bottom: h,
        };

        let s = input.layout.scale;
        let hover_t = input.hover_t.clamp(0.0, 1.0);
        let _accent = input.accent;

        unsafe {
            self.rt.BindDC(self.surface.dc(), &bind_rect)?;
            self.rt.BeginDraw();
            self.rt.SetTransform(&identity_matrix());
            self.rt.Clear(Some(&D2D1_COLOR_F {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.0,
            }));

            // Paint a nearly invisible alpha=2/255 hit-test floor across the entire layered window
            // so transparent gaps never fall through to Shell_TrayWnd or break hover/click tracking.
            if let Ok(hit_brush) = self.rt.CreateSolidColorBrush(
                &D2D1_COLOR_F {
                    r: 0.0,
                    g: 0.0,
                    b: 0.0,
                    a: 2.0 / 255.0,
                },
                None,
            ) {
                self.rt.FillRectangle(
                    &D2D_RECT_F {
                        left: 0.0,
                        top: 0.0,
                        right: w as f32,
                        bottom: h as f32,
                    },
                    &hit_brush,
                );
            }

            // Hover / drag / open-popup subtle Liquid Glass capsule pill; fully transparent at rest.
            let backdrop_t = if input.popup_open_span_x.is_some() {
                hover_t.max(1.0)
            } else {
                hover_t
            };
            if backdrop_t > 0.005 {
                let rr = D2D1_ROUNDED_RECT {
                    rect: d2d_rect(input.layout.bg_rect),
                    radiusX: input.layout.bg_radius,
                    radiusY: input.layout.bg_radius,
                };
                let drag_boost = if input.dragging { 1.25 } else { 1.0 };
                let fill_col = if input.light_taskbar {
                    D2D1_COLOR_F {
                        r: 1.0,
                        g: 1.0,
                        b: 1.0,
                        a: (0.56 * backdrop_t * drag_boost).min(0.82),
                    }
                } else {
                    D2D1_COLOR_F {
                        r: 1.0,
                        g: 1.0,
                        b: 1.0,
                        a: (0.095 * backdrop_t * drag_boost).min(0.18),
                    }
                };
                if let Ok(fb) = self.rt.CreateSolidColorBrush(&fill_col, None) {
                    self.rt.FillRoundedRectangle(&rr, &fb);
                }

                let border_col = if input.light_taskbar {
                    D2D1_COLOR_F {
                        r: 0.0,
                        g: 0.0,
                        b: 0.0,
                        a: 0.14 * backdrop_t,
                    }
                } else {
                    D2D1_COLOR_F {
                        r: 1.0,
                        g: 1.0,
                        b: 1.0,
                        a: 0.22 * backdrop_t,
                    }
                };
                if let Ok(bb) = self.rt.CreateSolidColorBrush(&border_col, None) {
                    if let Some((raw_l, raw_r)) = input.popup_open_span_x {
                        let r = input.layout.bg_rect;
                        let rad = input.layout.bg_radius;
                        let gap_l = raw_l.clamp(r.left + rad, r.right - rad);
                        let gap_r = raw_r.clamp(gap_l, r.right - rad);
                        if let Ok(geo) = self.d2d_factory.CreatePathGeometry() {
                            if let Ok(sink) = geo.Open() {
                                sink.BeginFigure(
                                    D2D_POINT_2F { x: gap_r, y: r.top },
                                    D2D1_FIGURE_BEGIN_HOLLOW,
                                );
                                sink.AddLine(D2D_POINT_2F {
                                    x: r.right - rad,
                                    y: r.top,
                                });
                                sink.AddArc(&D2D1_ARC_SEGMENT {
                                    point: D2D_POINT_2F {
                                        x: r.right,
                                        y: r.top + rad,
                                    },
                                    size: D2D_SIZE_F {
                                        width: rad,
                                        height: rad,
                                    },
                                    rotationAngle: 0.0,
                                    sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                                    arcSize: D2D1_ARC_SIZE_SMALL,
                                });
                                sink.AddLine(D2D_POINT_2F {
                                    x: r.right,
                                    y: r.bottom - rad,
                                });
                                sink.AddArc(&D2D1_ARC_SEGMENT {
                                    point: D2D_POINT_2F {
                                        x: r.right - rad,
                                        y: r.bottom,
                                    },
                                    size: D2D_SIZE_F {
                                        width: rad,
                                        height: rad,
                                    },
                                    rotationAngle: 0.0,
                                    sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                                    arcSize: D2D1_ARC_SIZE_SMALL,
                                });
                                sink.AddLine(D2D_POINT_2F {
                                    x: r.left + rad,
                                    y: r.bottom,
                                });
                                sink.AddArc(&D2D1_ARC_SEGMENT {
                                    point: D2D_POINT_2F {
                                        x: r.left,
                                        y: r.bottom - rad,
                                    },
                                    size: D2D_SIZE_F {
                                        width: rad,
                                        height: rad,
                                    },
                                    rotationAngle: 0.0,
                                    sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                                    arcSize: D2D1_ARC_SIZE_SMALL,
                                });
                                sink.AddLine(D2D_POINT_2F {
                                    x: r.left,
                                    y: r.top + rad,
                                });
                                sink.AddArc(&D2D1_ARC_SEGMENT {
                                    point: D2D_POINT_2F {
                                        x: r.left + rad,
                                        y: r.top,
                                    },
                                    size: D2D_SIZE_F {
                                        width: rad,
                                        height: rad,
                                    },
                                    rotationAngle: 0.0,
                                    sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                                    arcSize: D2D1_ARC_SIZE_SMALL,
                                });
                                sink.AddLine(D2D_POINT_2F { x: gap_l, y: r.top });
                                sink.EndFigure(D2D1_FIGURE_END_OPEN);
                                let _ = sink.Close();
                                self.rt.DrawGeometry(&geo, &bb, 1.0 * s, None);
                            }
                        }
                    } else {
                        self.rt.DrawRoundedRectangle(&rr, &bb, 1.0 * s, None);
                    }
                }
            }

            let base_primary = primary_text_color(input.light_taskbar);
            let mut label_col = base_primary;
            label_col.a *= 0.90;

            let warn_col = if input.light_taskbar {
                D2D1_COLOR_F {
                    r: 0.80,
                    g: 0.22,
                    b: 0.14,
                    a: 0.96,
                }
            } else {
                D2D1_COLOR_F {
                    r: 1.0,
                    g: 0.52,
                    b: 0.38,
                    a: 0.96,
                }
            };

            for cell in &input.layout.cells {
                draw_text_in_rect(
                    &self.dwrite_factory,
                    &self.rt,
                    cell.slot.label(),
                    cell.label_rect,
                    label_fmt,
                    label_col,
                );

                let (val_text, is_high) = match cell.slot {
                    MetricSlot::NetUp => (format_speed(input.snapshot.up_bps), false),
                    MetricSlot::NetDown => (format_speed(input.snapshot.down_bps), false),
                    MetricSlot::Cpu => (
                        format_percent(input.snapshot.cpu_pct),
                        input.snapshot.cpu_pct >= HIGH_LOAD_THRESHOLD,
                    ),
                    MetricSlot::Memory => (
                        format_percent(input.snapshot.mem_pct),
                        input.snapshot.mem_pct >= HIGH_LOAD_THRESHOLD,
                    ),
                    MetricSlot::Gpu => (
                        format_percent(input.snapshot.gpu_pct),
                        input.snapshot.gpu_pct >= HIGH_LOAD_THRESHOLD,
                    ),
                    MetricSlot::Disk => (
                        format_percent(input.snapshot.disk_pct),
                        input.snapshot.disk_pct >= HIGH_LOAD_THRESHOLD,
                    ),
                };

                let val_col = if is_high { warn_col } else { base_primary };
                draw_text_in_rect(
                    &self.dwrite_factory,
                    &self.rt,
                    &val_text,
                    cell.value_rect,
                    value_fmt,
                    val_col,
                );
            }

            self.rt.EndDraw(None, None)
        }
    }

    pub fn commit_to_window(&self, hwnd: HWND, x: i32, y: i32) {
        self.surface.commit(hwnd, x, y);
    }

    /// Creates a DPI-aware grabbing hand cursor for unlocked MonitorBar dragging.
    pub fn create_grabbing_cursor(&self, dpi: u32) -> windows::core::Result<(HCURSOR, bool)> {
        crate::ui::glass::create_grabbing_cursor(&self.rt, dpi)
    }
}
