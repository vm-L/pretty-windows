use super::dsp::NUM_BARS;
use super::layout::{BarLayout, HitZone, RectF};
use super::marquee::MarqueeState;
use super::smtc::MediaCover;
use super::volume::format_volume_pct;
use crate::color::Rgb;
use crate::ui::glass::{
    create_dc_render_target, create_round_cap_stroke, d2d_color_rgb, d2d_rect, identity_matrix,
    LayeredSurface,
};
use std::mem::ManuallyDrop;
use std::sync::Arc;
use windows::core::w;
use windows::Win32::Foundation::{D2DERR_RECREATE_TARGET, HWND, RECT};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_FIGURE_BEGIN_FILLED,
    D2D1_FIGURE_BEGIN_HOLLOW, D2D1_FIGURE_END_CLOSED, D2D1_FIGURE_END_OPEN, D2D1_GRADIENT_STOP,
    D2D1_PIXEL_FORMAT, D2D_POINT_2F, D2D_RECT_F, D2D_SIZE_F, D2D_SIZE_U,
};
use windows::Win32::Graphics::Direct2D::{
    ID2D1Bitmap, ID2D1Brush, ID2D1DCRenderTarget, ID2D1Factory1, ID2D1Geometry, ID2D1Layer,
    ID2D1SolidColorBrush, ID2D1StrokeStyle, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_ARC_SEGMENT,
    D2D1_ARC_SIZE_SMALL, D2D1_BITMAP_INTERPOLATION_MODE_LINEAR, D2D1_BITMAP_PROPERTIES,
    D2D1_DRAW_TEXT_OPTIONS_CLIP, D2D1_DRAW_TEXT_OPTIONS_NONE, D2D1_ELLIPSE, D2D1_EXTEND_MODE_CLAMP,
    D2D1_GAMMA_2_2, D2D1_LAYER_OPTIONS_NONE, D2D1_LAYER_PARAMETERS,
    D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES, D2D1_ROUNDED_RECT, D2D1_SWEEP_DIRECTION_CLOCKWISE,
};
use windows::Win32::Graphics::DirectWrite::{
    IDWriteFactory, IDWriteTextFormat, IDWriteTextLayout, DWRITE_FONT_STRETCH_NORMAL,
    DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_WEIGHT_SEMI_BOLD,
    DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_LEADING,
    DWRITE_TEXT_ALIGNMENT_TRAILING, DWRITE_TEXT_METRICS, DWRITE_WORD_WRAPPING_NO_WRAP,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::UI::WindowsAndMessaging::HCURSOR;

/// Draws a dynamic Win11-style vector speaker icon showing mute `×`, 1 wave, or 2 waves based on volume.
#[allow(clippy::too_many_arguments)]
pub unsafe fn draw_speaker_glyph(
    factory: &ID2D1Factory1,
    rt: &ID2D1DCRenderTarget,
    cx: f32,
    cy: f32,
    u: f32,
    volume: f32,
    muted: bool,
    fg_brush: &ID2D1SolidColorBrush,
    muted_brush: &ID2D1SolidColorBrush,
    slash_brush: &ID2D1SolidColorBrush,
    round_stroke: Option<&ID2D1StrokeStyle>,
) {
    let cone_brush = if muted { muted_brush } else { fg_brush };

    if let Ok(cone) = factory.CreatePathGeometry() {
        if let Ok(sink) = cone.Open() {
            sink.BeginFigure(
                D2D_POINT_2F {
                    x: cx - 5.2 * u,
                    y: cy - 2.1 * u,
                },
                D2D1_FIGURE_BEGIN_FILLED,
            );
            sink.AddLine(D2D_POINT_2F {
                x: cx - 2.5 * u,
                y: cy - 2.1 * u,
            });
            sink.AddLine(D2D_POINT_2F {
                x: cx + 0.6 * u,
                y: cy - 4.9 * u,
            });
            sink.AddLine(D2D_POINT_2F {
                x: cx + 0.6 * u,
                y: cy + 4.9 * u,
            });
            sink.AddLine(D2D_POINT_2F {
                x: cx - 2.5 * u,
                y: cy + 2.1 * u,
            });
            sink.AddLine(D2D_POINT_2F {
                x: cx - 5.2 * u,
                y: cy + 2.1 * u,
            });
            sink.EndFigure(D2D1_FIGURE_END_CLOSED);
            let _ = sink.Close();
            rt.FillGeometry(&cone, cone_brush, None);
        }
    }

    if muted || volume < 0.005 {
        let sb = if muted { slash_brush } else { muted_brush };
        rt.DrawLine(
            D2D_POINT_2F {
                x: cx + 2.2 * u,
                y: cy - 1.9 * u,
            },
            D2D_POINT_2F {
                x: cx + 5.8 * u,
                y: cy + 1.9 * u,
            },
            sb,
            1.4 * u,
            round_stroke,
        );
        rt.DrawLine(
            D2D_POINT_2F {
                x: cx + 2.2 * u,
                y: cy + 1.9 * u,
            },
            D2D_POINT_2F {
                x: cx + 5.8 * u,
                y: cy - 1.9 * u,
            },
            sb,
            1.4 * u,
            round_stroke,
        );
        return;
    }

    // Inner sound wave arc (visible whenever volume >= 0.5%)
    if let Ok(arc1) = factory.CreatePathGeometry() {
        if let Ok(sink) = arc1.Open() {
            sink.BeginFigure(
                D2D_POINT_2F {
                    x: cx + 2.4 * u,
                    y: cy - 2.3 * u,
                },
                D2D1_FIGURE_BEGIN_HOLLOW,
            );
            sink.AddArc(&D2D1_ARC_SEGMENT {
                point: D2D_POINT_2F {
                    x: cx + 2.4 * u,
                    y: cy + 2.3 * u,
                },
                size: D2D_SIZE_F {
                    width: 2.9 * u,
                    height: 2.9 * u,
                },
                rotationAngle: 0.0,
                sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                arcSize: D2D1_ARC_SIZE_SMALL,
            });
            sink.EndFigure(D2D1_FIGURE_END_OPEN);
            let _ = sink.Close();
            rt.DrawGeometry(&arc1, fg_brush, 1.35 * u, round_stroke);
        }
    }

    // Outer sound wave arc (visible at >= 50% volume)
    if volume >= 0.50 {
        if let Ok(arc2) = factory.CreatePathGeometry() {
            if let Ok(sink) = arc2.Open() {
                sink.BeginFigure(
                    D2D_POINT_2F {
                        x: cx + 4.1 * u,
                        y: cy - 4.0 * u,
                    },
                    D2D1_FIGURE_BEGIN_HOLLOW,
                );
                sink.AddArc(&D2D1_ARC_SEGMENT {
                    point: D2D_POINT_2F {
                        x: cx + 4.1 * u,
                        y: cy + 4.0 * u,
                    },
                    size: D2D_SIZE_F {
                        width: 5.0 * u,
                        height: 5.0 * u,
                    },
                    rotationAngle: 0.0,
                    sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                    arcSize: D2D1_ARC_SIZE_SMALL,
                });
                sink.EndFigure(D2D1_FIGURE_END_OPEN);
                let _ = sink.Close();
                rt.DrawGeometry(&arc2, fg_brush, 1.35 * u, round_stroke);
            }
        }
    }
}

/// Draws a sleek rounded Liquid Glass volume track, accent fill, and responsive circular thumb knob.
#[allow(clippy::too_many_arguments)]
pub unsafe fn draw_volume_slider(
    rt: &ID2D1DCRenderTarget,
    track: RectF,
    level: f32,
    muted: bool,
    hovered_or_dragging: bool,
    light_taskbar: bool,
    accent: Rgb,
    scale: f32,
) {
    crate::ui::glass::draw_glass_slider(
        rt,
        track,
        level,
        muted,
        hovered_or_dragging,
        light_taskbar,
        accent,
        scale,
    );
}

struct CachedText {
    text: String,
    scale_key: u32,
    layout: IDWriteTextLayout,
    width: f32,
}

struct CachedBitmap {
    hash: u64,
    bitmap: ID2D1Bitmap,
}

/// Input frame data passed to `BarRenderer::render`.
pub struct RenderFrame<'a> {
    pub hwnd: HWND,
    pub win_x: i32,
    pub win_y: i32,
    pub width_px: i32,
    pub height_px: i32,
    pub dpi: u32,
    pub light_taskbar: bool,
    pub is_idle: bool,
    pub is_playing: bool,
    pub title: &'a str,
    pub artist: &'a str,
    pub cover: Option<&'a Arc<MediaCover>>,
    pub accent_color: Rgb,
    pub hover_t: f32,
    pub hover_zone: HitZone,
    pub pressed_zone: HitZone,
    pub locked: bool,
    pub vol_hud_active: bool,
    pub vol_hud_dragging: bool,
    pub master_volume: f32,
    pub master_muted: bool,
    pub popup_open: bool,
    pub progress: Option<f32>,
    pub bar_levels: &'a [f32; NUM_BARS],
    pub title_marquee: &'a mut MarqueeState,
    pub artist_marquee: &'a mut MarqueeState,
}

/// Hardware/Direct2D renderer for the taskbar MediaBar layered window.
pub struct BarRenderer {
    pub d2d_factory: ID2D1Factory1,
    pub dwrite_factory: IDWriteFactory,
    // Declare RT-owned resources and `rt` before `surface` so `rt` releases before `DeleteDC`.
    cached_cover: Option<CachedBitmap>,
    layer: ID2D1Layer,
    rt: ID2D1DCRenderTarget,
    surface: LayeredSurface,
    round_stroke: Option<ID2D1StrokeStyle>,
    font_scale_key: u32,
    title_fmt: Option<IDWriteTextFormat>,
    artist_fmt: Option<IDWriteTextFormat>,
    idle_fmt: Option<IDWriteTextFormat>,
    pct_fmt: Option<IDWriteTextFormat>,
    cached_title: Option<CachedText>,
    cached_artist: Option<CachedText>,
}

impl BarRenderer {
    pub fn new_with_factories(
        d2d_factory: ID2D1Factory1,
        dwrite_factory: IDWriteFactory,
    ) -> windows::core::Result<Self> {
        unsafe {
            let rt = create_dc_render_target(&d2d_factory)?;
            let layer = rt.CreateLayer(None)?;
            let surface = LayeredSurface::new()?;
            let round_stroke = create_round_cap_stroke(&d2d_factory);

            Ok(Self {
                d2d_factory,
                dwrite_factory,
                cached_cover: None,
                layer,
                rt,
                surface,
                round_stroke,
                font_scale_key: 0,
                title_fmt: None,
                artist_fmt: None,
                idle_fmt: None,
                pct_fmt: None,
                cached_title: None,
                cached_artist: None,
            })
        }
    }

    fn recreate_render_target(&mut self) -> windows::core::Result<()> {
        self.cached_cover = None;
        let new_rt = create_dc_render_target(&self.d2d_factory)?;
        let new_layer = unsafe { new_rt.CreateLayer(None)? };
        self.layer = new_layer;
        self.rt = new_rt;
        Ok(())
    }

    fn ensure_formats(&mut self, dpi: u32) -> windows::core::Result<()> {
        if self.font_scale_key == dpi && self.title_fmt.is_some() {
            return Ok(());
        }
        let scale = (dpi.max(96) as f32) / 96.0;
        unsafe {
            let title_fmt = self.dwrite_factory.CreateTextFormat(
                w!("Segoe UI Variable Text"),
                None,
                DWRITE_FONT_WEIGHT_SEMI_BOLD,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                12.5 * scale,
                w!("zh-CN"),
            )?;
            let _ = title_fmt.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_LEADING);
            let _ = title_fmt.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER);
            let _ = title_fmt.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP);

            let artist_fmt = self.dwrite_factory.CreateTextFormat(
                w!("Segoe UI Variable Text"),
                None,
                DWRITE_FONT_WEIGHT_NORMAL,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                11.0 * scale,
                w!("zh-CN"),
            )?;
            let _ = artist_fmt.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_LEADING);
            let _ = artist_fmt.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER);
            let _ = artist_fmt.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP);

            let idle_fmt = self.dwrite_factory.CreateTextFormat(
                w!("Segoe UI Variable Text"),
                None,
                DWRITE_FONT_WEIGHT_NORMAL,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                12.0 * scale,
                w!("zh-CN"),
            )?;
            let _ = idle_fmt.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_LEADING);
            let _ = idle_fmt.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER);
            let _ = idle_fmt.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP);

            let pct_fmt = self.dwrite_factory.CreateTextFormat(
                w!("Segoe UI Variable Text"),
                None,
                DWRITE_FONT_WEIGHT_SEMI_BOLD,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                11.5 * scale,
                w!("zh-CN"),
            )?;
            let _ = pct_fmt.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_TRAILING);
            let _ = pct_fmt.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER);
            let _ = pct_fmt.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP);

            self.title_fmt = Some(title_fmt);
            self.artist_fmt = Some(artist_fmt);
            self.idle_fmt = Some(idle_fmt);
            self.pct_fmt = Some(pct_fmt);
            self.font_scale_key = dpi;
            self.cached_title = None;
            self.cached_artist = None;
        }
        Ok(())
    }

    fn measure_layout(
        factory: &IDWriteFactory,
        fmt: &IDWriteTextFormat,
        cache: &mut Option<CachedText>,
        text: &str,
        dpi: u32,
        height_px: f32,
    ) -> windows::core::Result<(IDWriteTextLayout, f32)> {
        if let Some(c) = cache.as_ref() {
            if c.scale_key == dpi && c.text == text {
                return Ok((c.layout.clone(), c.width));
            }
        }

        let wide: Vec<u16> = text.encode_utf16().collect();
        unsafe {
            let layout = factory.CreateTextLayout(&wide, fmt, 4096.0, height_px.max(12.0))?;
            let mut metrics = DWRITE_TEXT_METRICS::default();
            layout.GetMetrics(&mut metrics)?;
            let width = metrics.widthIncludingTrailingWhitespace;
            *cache = Some(CachedText {
                text: text.to_string(),
                scale_key: dpi,
                layout: layout.clone(),
                width,
            });
            Ok((layout, width))
        }
    }

    fn ensure_cover_bitmap(&mut self, cover: &MediaCover) -> Option<ID2D1Bitmap> {
        if let Some(ref c) = self.cached_cover {
            if c.hash == cover.hash {
                return Some(c.bitmap.clone());
            }
        }
        unsafe {
            let props = D2D1_BITMAP_PROPERTIES {
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: 96.0,
                dpiY: 96.0,
            };
            let bmp = self
                .rt
                .CreateBitmap(
                    D2D_SIZE_U {
                        width: cover.thumb_w,
                        height: cover.thumb_h,
                    },
                    Some(cover.thumb_pbgra.as_ptr().cast()),
                    cover.thumb_w * 4,
                    &props,
                )
                .ok()?;
            self.cached_cover = Some(CachedBitmap {
                hash: cover.hash,
                bitmap: bmp.clone(),
            });
            Some(bmp)
        }
    }

    /// Synchronizes marquee text & viewport measurements without forcing a full draw.
    pub fn sync_marquee_metrics(&mut self, frame: &mut RenderFrame<'_>) {
        if frame.is_idle {
            frame.title_marquee.set_text("", 0.0);
            frame.artist_marquee.set_text("", 0.0);
            return;
        }
        if self.ensure_formats(frame.dpi).is_err() {
            return;
        }
        let layout = BarLayout::compute(frame.height_px, frame.dpi, frame.hover_t, frame.bar_levels);
        if let Some(ref fmt) = self.title_fmt {
            if let Ok((_, w)) = Self::measure_layout(
                &self.dwrite_factory,
                fmt,
                &mut self.cached_title,
                frame.title,
                frame.dpi,
                layout.title_rect.height(),
            ) {
                frame.title_marquee.set_text(frame.title, w);
                frame
                    .title_marquee
                    .set_widths(w, layout.title_rect.width());
            }
        }
        if let Some(ref fmt) = self.artist_fmt {
            if let Ok((_, w)) = Self::measure_layout(
                &self.dwrite_factory,
                fmt,
                &mut self.cached_artist,
                frame.artist,
                frame.dpi,
                layout.artist_rect.height(),
            ) {
                frame.artist_marquee.set_text(frame.artist, w);
                frame
                    .artist_marquee
                    .set_widths(w, layout.artist_rect.width());
            }
        }
    }

    pub fn render(&mut self, frame: &mut RenderFrame<'_>) -> windows::core::Result<()> {
        match self.render_once(frame) {
            Err(e) if e.code() == D2DERR_RECREATE_TARGET => {
                self.recreate_render_target()?;
                self.render_once(frame)
            }
            res => res,
        }
    }

    fn render_once(&mut self, frame: &mut RenderFrame<'_>) -> windows::core::Result<()> {
        self.surface.ensure_size(frame.width_px, frame.height_px)?;
        self.ensure_formats(frame.dpi)?;

        let layout = BarLayout::compute(frame.height_px, frame.dpi, frame.hover_t, frame.bar_levels);
        let cover_bmp = frame.cover.and_then(|c| self.ensure_cover_bitmap(c));

        let title_data = if frame.is_idle {
            self.idle_fmt.as_ref().and_then(|fmt| {
                Self::measure_layout(
                    &self.dwrite_factory,
                    fmt,
                    &mut self.cached_title,
                    "未在播放",
                    frame.dpi,
                    layout.cover_rect.height(),
                )
                .ok()
            })
        } else {
            self.title_fmt.as_ref().and_then(|fmt| {
                Self::measure_layout(
                    &self.dwrite_factory,
                    fmt,
                    &mut self.cached_title,
                    frame.title,
                    frame.dpi,
                    layout.title_rect.height(),
                )
                .ok()
            })
        };

        let artist_data = if frame.is_idle || frame.artist.is_empty() {
            None
        } else {
            self.artist_fmt.as_ref().and_then(|fmt| {
                Self::measure_layout(
                    &self.dwrite_factory,
                    fmt,
                    &mut self.cached_artist,
                    frame.artist,
                    frame.dpi,
                    layout.artist_rect.height(),
                )
                .ok()
            })
        };

        if let Some((_, w)) = &title_data {
            frame.title_marquee.set_text(
                if frame.is_idle { "" } else { frame.title },
                if frame.is_idle { 0.0 } else { *w },
            );
            frame
                .title_marquee
                .set_widths(if frame.is_idle { 0.0 } else { *w }, layout.title_rect.width());
        }
        if let Some((_, w)) = &artist_data {
            frame.artist_marquee.set_text(frame.artist, *w);
            frame
                .artist_marquee
                .set_widths(*w, layout.artist_rect.width());
        } else {
            frame.artist_marquee.set_text("", 0.0);
        }

        let bind_rect = RECT {
            left: 0,
            top: 0,
            right: self.surface.width(),
            bottom: self.surface.height(),
        };

        unsafe {
            self.rt.BindDC(self.surface.dc(), &bind_rect)?;
            self.rt.BeginDraw();
            self.rt.Clear(Some(&D2D1_COLOR_F {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.0,
            }));

            // Paint a nearly invisible alpha=2/255 hit-test floor across the entire layered window
            // so transparent gaps never fall through to Shell_TrayWnd or break hover tracking.
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
                        right: frame.width_px as f32,
                        bottom: frame.height_px as f32,
                    },
                    &hit_brush,
                );
            }

            // 1. Hover micro-translucent taskbar pill backdrop (always crisp when volume HUD or popup is active)
            let backdrop_t = if frame.vol_hud_active || frame.popup_open {
                layout.hover_t.max(1.0)
            } else {
                layout.hover_t
            };
            if backdrop_t > 0.005 {
                self.draw_hover_backdrop(
                    &layout,
                    backdrop_t,
                    frame.light_taskbar,
                    frame.hover_zone == HitZone::MediaInfo,
                    !frame.locked,
                    frame.vol_hud_active,
                    frame.popup_open,
                );
            }

            // 2. Left cover art or default vector music note icon
            if let Some(bmp) = cover_bmp {
                self.draw_cover_bitmap(&layout, &bmp, frame.light_taskbar);
            } else {
                self.draw_default_note_tile(&layout, frame.light_taskbar, frame.is_idle);
            }

            if frame.vol_hud_active {
                // Inline Master Volume HUD replaces text + spectrum + playback controls
                self.draw_inline_volume_hud(&layout, frame);
            } else {
                // 3. Center media info text (or idle prompt)
                if frame.is_idle {
                    if let Some((ref tl, _)) = title_data {
                        let idle_color = if frame.light_taskbar {
                            D2D1_COLOR_F {
                                r: 0.10,
                                g: 0.10,
                                b: 0.12,
                                a: 0.56,
                            }
                        } else {
                            D2D1_COLOR_F {
                                r: 1.0,
                                g: 1.0,
                                b: 1.0,
                                a: 0.56,
                            }
                        };
                        if let Ok(brush) = self.rt.CreateSolidColorBrush(&idle_color, None) {
                            let y = (layout.height_px - layout.cover_rect.height()) * 0.5;
                            self.rt.DrawTextLayout(
                                D2D_POINT_2F {
                                    x: layout.title_rect.left,
                                    y,
                                },
                                tl,
                                &brush,
                                D2D1_DRAW_TEXT_OPTIONS_NONE,
                            );
                        }
                    }
                } else {
                    let (title_color, artist_color) = if frame.light_taskbar {
                        (
                            D2D1_COLOR_F {
                                r: 0.08,
                                g: 0.08,
                                b: 0.10,
                                a: 0.94,
                            },
                            D2D1_COLOR_F {
                                r: 0.18,
                                g: 0.18,
                                b: 0.22,
                                a: 0.68,
                            },
                        )
                    } else {
                        (
                            D2D1_COLOR_F {
                                r: 1.0,
                                g: 1.0,
                                b: 1.0,
                                a: 0.95,
                            },
                            D2D1_COLOR_F {
                                r: 0.92,
                                g: 0.94,
                                b: 0.98,
                                a: 0.68,
                            },
                        )
                    };

                    // If no artist metadata, vertically center the title smoothly.
                    let t_rect = if artist_data.is_none() {
                        let h = layout.title_rect.height();
                        let top = (layout.height_px - h) * 0.5;
                        RectF::new(layout.title_rect.left, top, layout.title_rect.right, top + h)
                    } else {
                        layout.title_rect
                    };

                    if let Some((ref tl, _)) = title_data {
                        self.draw_marquee_text(
                            tl,
                            t_rect,
                            title_color,
                            frame.title_marquee,
                            layout.scale,
                        );
                    }
                    if let Some((ref al, _)) = artist_data {
                        self.draw_marquee_text(
                            al,
                            layout.artist_rect,
                            artist_color,
                            frame.artist_marquee,
                            layout.scale,
                        );
                    }
                }

                // 4. Right spectrum bars (fading out cleanly on hover)
                if layout.spectrum_alpha > 0.01 {
                    self.draw_spectrum_bars(
                        &layout,
                        frame.accent_color,
                        frame.is_idle,
                        frame.light_taskbar,
                    );
                }

                // 5. Right playback control buttons (fading in cleanly on hover)
                if layout.controls_alpha > 0.01 {
                    self.draw_control_buttons(
                        &layout,
                        frame.light_taskbar,
                        frame.is_playing,
                        frame.hover_zone,
                        frame.pressed_zone,
                    );
                }

                // 6. Ultra-slim bottom playback progress hairline when a finite timeline is present
                if !frame.is_idle {
                    if let Some(progress) = frame.progress {
                        self.draw_progress_hairline(
                            &layout,
                            progress,
                            frame.accent_color,
                            frame.is_playing,
                            frame.light_taskbar,
                        );
                    }
                }
            }

            self.rt.EndDraw(None, None)?;
        }

        self.surface.commit(frame.hwnd, frame.win_x, frame.win_y);
        Ok(())
    }

    unsafe fn draw_inline_volume_hud(&self, layout: &BarLayout, frame: &RenderFrame<'_>) {
        let mute_hovered = frame.hover_zone == HitZone::VolumeMuteButton;
        let mute_pressed = frame.pressed_zone == HitZone::VolumeMuteButton;
        if mute_hovered || mute_pressed {
            let bg = if frame.light_taskbar {
                D2D1_COLOR_F {
                    r: 0.0,
                    g: 0.0,
                    b: 0.0,
                    a: if mute_pressed { 0.14 } else { 0.08 },
                }
            } else {
                D2D1_COLOR_F {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: if mute_pressed { 0.18 } else { 0.11 },
                }
            };
            if let Ok(bg_brush) = self.rt.CreateSolidColorBrush(&bg, None) {
                let rr = D2D1_ROUNDED_RECT {
                    rect: d2d_rect(layout.vol_mute_btn_rect),
                    radiusX: 5.5 * layout.scale,
                    radiusY: 5.5 * layout.scale,
                };
                self.rt.FillRoundedRectangle(&rr, &bg_brush);
            }
        }

        let fg_col = if frame.light_taskbar {
            D2D1_COLOR_F {
                r: 0.08,
                g: 0.08,
                b: 0.10,
                a: if mute_pressed { 0.72 } else { 0.92 },
            }
        } else {
            D2D1_COLOR_F {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: if mute_pressed { 0.72 } else { 0.95 },
            }
        };
        let muted_col = D2D1_COLOR_F {
            r: fg_col.r,
            g: fg_col.g,
            b: fg_col.b,
            a: fg_col.a * 0.56,
        };
        let slash_col = if frame.light_taskbar {
            D2D1_COLOR_F {
                r: 0.80,
                g: 0.18,
                b: 0.20,
                a: 0.92,
            }
        } else {
            D2D1_COLOR_F {
                r: 1.0,
                g: 0.46,
                b: 0.46,
                a: 0.95,
            }
        };

        if let (Ok(fg_b), Ok(mu_b), Ok(sl_b)) = (
            self.rt.CreateSolidColorBrush(&fg_col, None),
            self.rt.CreateSolidColorBrush(&muted_col, None),
            self.rt.CreateSolidColorBrush(&slash_col, None),
        ) {
            let icon_scale = if mute_pressed { 0.88 } else { 1.0 };
            let cx = (layout.vol_mute_btn_rect.left + layout.vol_mute_btn_rect.right) * 0.5;
            let cy = (layout.vol_mute_btn_rect.top + layout.vol_mute_btn_rect.bottom) * 0.5;
            let u = (layout.vol_mute_btn_rect.width() / 22.0) * icon_scale;
            draw_speaker_glyph(
                &self.d2d_factory,
                &self.rt,
                cx,
                cy,
                u,
                frame.master_volume,
                frame.master_muted,
                &fg_b,
                &mu_b,
                &sl_b,
                self.round_stroke.as_ref(),
            );
        }

        let slider_active = frame.vol_hud_dragging
            || frame.hover_zone == HitZone::VolumeSlider
            || frame.pressed_zone == HitZone::VolumeSlider;
        draw_volume_slider(
            &self.rt,
            layout.vol_track_rect,
            frame.master_volume,
            frame.master_muted,
            slider_active,
            frame.light_taskbar,
            frame.accent_color,
            layout.scale,
        );

        if let Some(ref fmt) = self.pct_fmt {
            let pct_str = format_volume_pct(frame.master_volume);
            let wide: Vec<u16> = pct_str.encode_utf16().collect();
            if let Ok(tl) = self.dwrite_factory.CreateTextLayout(
                &wide,
                fmt,
                layout.vol_pct_rect.width().max(1.0),
                layout.vol_pct_rect.height().max(12.0),
            ) {
                let pct_col = D2D1_COLOR_F {
                    r: fg_col.r,
                    g: fg_col.g,
                    b: fg_col.b,
                    a: if frame.master_muted { 0.50 } else { 0.92 },
                };
                if let Ok(pb) = self.rt.CreateSolidColorBrush(&pct_col, None) {
                    self.rt.DrawTextLayout(
                        D2D_POINT_2F {
                            x: layout.vol_pct_rect.left,
                            y: layout.vol_pct_rect.top,
                        },
                        &tl,
                        &pb,
                        D2D1_DRAW_TEXT_OPTIONS_CLIP,
                    );
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    unsafe fn draw_hover_backdrop(
        &self,
        layout: &BarLayout,
        t: f32,
        light_taskbar: bool,
        media_hovered: bool,
        unlocked: bool,
        vol_hud_active: bool,
        popup_open: bool,
    ) {
        let bg_color = if light_taskbar {
            D2D1_COLOR_F {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 0.56 * t,
            }
        } else {
            D2D1_COLOR_F {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 0.085 * t,
            }
        };
        let border_color = if light_taskbar {
            D2D1_COLOR_F {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.09 * t,
            }
        } else {
            D2D1_COLOR_F {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: if unlocked { 0.24 * t } else { 0.13 * t },
            }
        };

        let rrect = D2D1_ROUNDED_RECT {
            rect: d2d_rect(layout.bg_rect),
            radiusX: layout.bg_radius,
            radiusY: layout.bg_radius,
        };
        if let Ok(bg_brush) = self.rt.CreateSolidColorBrush(&bg_color, None) {
            self.rt.FillRoundedRectangle(&rrect, &bg_brush);
        }
        if media_hovered && !popup_open {
            let sub_color = if light_taskbar {
                D2D1_COLOR_F {
                    r: 0.0,
                    g: 0.0,
                    b: 0.0,
                    a: 0.045 * t,
                }
            } else {
                D2D1_COLOR_F {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: 0.055 * t,
                }
            };
            if let Ok(sub_brush) = self.rt.CreateSolidColorBrush(&sub_color, None) {
                let target_rect = if vol_hud_active {
                    RectF::new(
                        layout.bg_rect.left,
                        layout.bg_rect.top,
                        layout.cover_rect.right + 4.0 * layout.scale,
                        layout.bg_rect.bottom,
                    )
                } else {
                    layout.media_info_hit_rect
                };
                let info_r = D2D1_ROUNDED_RECT {
                    rect: d2d_rect(target_rect),
                    radiusX: (layout.bg_radius - 1.0).max(2.0),
                    radiusY: (layout.bg_radius - 1.0).max(2.0),
                };
                self.rt.FillRoundedRectangle(&info_r, &sub_brush);
            }
        }
        if let Ok(border_brush) = self.rt.CreateSolidColorBrush(&border_color, None) {
            self.rt
                .DrawRoundedRectangle(&rrect, &border_brush, 1.0 * layout.scale, None);
        }
    }

    unsafe fn draw_cover_bitmap(&self, layout: &BarLayout, bmp: &ID2D1Bitmap, light_taskbar: bool) {
        let rrect = D2D1_ROUNDED_RECT {
            rect: d2d_rect(layout.cover_rect),
            radiusX: layout.cover_radius,
            radiusY: layout.cover_radius,
        };
        if let Ok(geo) = self.d2d_factory.CreateRoundedRectangleGeometry(&rrect) {
            let geom: ID2D1Geometry = geo.into();
            let params = D2D1_LAYER_PARAMETERS {
                contentBounds: d2d_rect(layout.cover_rect),
                geometricMask: ManuallyDrop::new(Some(geom)),
                maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                maskTransform: identity_matrix(),
                opacity: 1.0,
                opacityBrush: ManuallyDrop::new(None),
                layerOptions: D2D1_LAYER_OPTIONS_NONE,
            };
            self.rt.PushLayer(&params, &self.layer);
            self.rt.DrawBitmap(
                bmp,
                Some(&d2d_rect(layout.cover_rect)),
                1.0,
                D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                None,
            );
            self.rt.PopLayer();
            let _ = ManuallyDrop::into_inner(params.geometricMask);
            let _ = ManuallyDrop::into_inner(params.opacityBrush);
        }

        let ring_color = if light_taskbar {
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
        if let Ok(brush) = self.rt.CreateSolidColorBrush(&ring_color, None) {
            self.rt
                .DrawRoundedRectangle(&rrect, &brush, 1.0 * layout.scale, None);
        }
    }

    unsafe fn draw_default_note_tile(&self, layout: &BarLayout, light_taskbar: bool, is_idle: bool) {
        let bg = if light_taskbar {
            D2D1_COLOR_F {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: if is_idle { 0.07 } else { 0.10 },
            }
        } else {
            D2D1_COLOR_F {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: if is_idle { 0.09 } else { 0.13 },
            }
        };
        let note_col = if light_taskbar {
            D2D1_COLOR_F {
                r: 0.15,
                g: 0.15,
                b: 0.18,
                a: if is_idle { 0.55 } else { 0.78 },
            }
        } else {
            D2D1_COLOR_F {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: if is_idle { 0.58 } else { 0.85 },
            }
        };

        let rrect = D2D1_ROUNDED_RECT {
            rect: d2d_rect(layout.cover_rect),
            radiusX: layout.cover_radius,
            radiusY: layout.cover_radius,
        };
        if let Ok(bg_brush) = self.rt.CreateSolidColorBrush(&bg, None) {
            self.rt.FillRoundedRectangle(&rrect, &bg_brush);
        }
        let border = if light_taskbar {
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
                a: 0.12,
            }
        };
        if let Ok(b_brush) = self.rt.CreateSolidColorBrush(&border, None) {
            self.rt
                .DrawRoundedRectangle(&rrect, &b_brush, 1.0 * layout.scale, None);
        }

        // Vector beamed eighth notes centered inside the 32x32 tile
        let Ok(brush) = self.rt.CreateSolidColorBrush(&note_col, None) else {
            return;
        };
        let cx = (layout.cover_rect.left + layout.cover_rect.right) * 0.5;
        let cy = (layout.cover_rect.top + layout.cover_rect.bottom) * 0.5;
        let u = layout.cover_rect.width() / 32.0;

        self.rt.FillEllipse(
            &D2D1_ELLIPSE {
                point: D2D_POINT_2F {
                    x: cx - 4.2 * u,
                    y: cy + 5.2 * u,
                },
                radiusX: 2.7 * u,
                radiusY: 2.1 * u,
            },
            &brush,
        );
        self.rt.FillEllipse(
            &D2D1_ELLIPSE {
                point: D2D_POINT_2F {
                    x: cx + 4.3 * u,
                    y: cy + 3.8 * u,
                },
                radiusX: 2.7 * u,
                radiusY: 2.1 * u,
            },
            &brush,
        );

        let stroke = 1.6 * u;
        self.rt.DrawLine(
            D2D_POINT_2F {
                x: cx - 2.1 * u,
                y: cy + 4.8 * u,
            },
            D2D_POINT_2F {
                x: cx - 2.1 * u,
                y: cy - 6.0 * u,
            },
            &brush,
            stroke,
            None,
        );
        self.rt.DrawLine(
            D2D_POINT_2F {
                x: cx + 6.4 * u,
                y: cy + 3.4 * u,
            },
            D2D_POINT_2F {
                x: cx + 6.4 * u,
                y: cy - 7.5 * u,
            },
            &brush,
            stroke,
            None,
        );

        if let Ok(beam) = self.d2d_factory.CreatePathGeometry() {
            if let Ok(sink) = beam.Open() {
                sink.BeginFigure(
                    D2D_POINT_2F {
                        x: cx - 2.9 * u,
                        y: cy - 6.0 * u,
                    },
                    D2D1_FIGURE_BEGIN_FILLED,
                );
                sink.AddLine(D2D_POINT_2F {
                    x: cx + 7.2 * u,
                    y: cy - 7.6 * u,
                });
                sink.AddLine(D2D_POINT_2F {
                    x: cx + 7.2 * u,
                    y: cy - 5.2 * u,
                });
                sink.AddLine(D2D_POINT_2F {
                    x: cx - 2.9 * u,
                    y: cy - 3.6 * u,
                });
                sink.EndFigure(D2D1_FIGURE_END_CLOSED);
                let _ = sink.Close();
                self.rt.FillGeometry(&beam, &brush, None);
            }
        }
    }

    unsafe fn draw_marquee_text(
        &self,
        text_layout: &IDWriteTextLayout,
        rect: RectF,
        color: D2D1_COLOR_F,
        marquee: &MarqueeState,
        scale: f32,
    ) {
        let Ok(text_brush) = self.rt.CreateSolidColorBrush(&color, None) else {
            return;
        };

        if !marquee.is_overflowing() {
            self.rt
                .PushAxisAlignedClip(&d2d_rect(rect), D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
            self.rt.DrawTextLayout(
                D2D_POINT_2F {
                    x: rect.left,
                    y: rect.top,
                },
                text_layout,
                &text_brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
            );
            self.rt.PopAxisAlignedClip();
            return;
        }

        let (x1, x2_opt) = marquee.render_offsets();
        let w = rect.width().max(1.0);
        let fade_frac = (12.0 * scale / w).clamp(0.05, 0.25);
        let left_alpha = if x1 < -0.5 { 0.0 } else { 1.0 };

        let stops = [
            D2D1_GRADIENT_STOP {
                position: 0.0,
                color: D2D1_COLOR_F {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: left_alpha,
                },
            },
            D2D1_GRADIENT_STOP {
                position: fade_frac,
                color: D2D1_COLOR_F {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: 1.0,
                },
            },
            D2D1_GRADIENT_STOP {
                position: 1.0 - fade_frac,
                color: D2D1_COLOR_F {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: 1.0,
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

        let mut layer_params: Option<D2D1_LAYER_PARAMETERS> = None;
        if let Ok(stop_coll) =
            self.rt
                .CreateGradientStopCollection(&stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP)
        {
            let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                startPoint: D2D_POINT_2F {
                    x: rect.left,
                    y: rect.top,
                },
                endPoint: D2D_POINT_2F {
                    x: rect.right,
                    y: rect.top,
                },
            };
            if let Ok(grad) = self
                .rt
                .CreateLinearGradientBrush(&props, None, &stop_coll)
            {
                let brush_iface: ID2D1Brush = grad.into();
                let lp = D2D1_LAYER_PARAMETERS {
                    contentBounds: d2d_rect(rect),
                    geometricMask: ManuallyDrop::new(None),
                    maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                    maskTransform: identity_matrix(),
                    opacity: 1.0,
                    opacityBrush: ManuallyDrop::new(Some(brush_iface)),
                    layerOptions: D2D1_LAYER_OPTIONS_NONE,
                };
                self.rt.PushLayer(&lp, &self.layer);
                layer_params = Some(lp);
            }
        }

        if layer_params.is_none() {
            self.rt
                .PushAxisAlignedClip(&d2d_rect(rect), D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
        }

        self.rt.DrawTextLayout(
            D2D_POINT_2F {
                x: rect.left + x1,
                y: rect.top,
            },
            text_layout,
            &text_brush,
            D2D1_DRAW_TEXT_OPTIONS_NONE,
        );
        if let Some(x2) = x2_opt {
            self.rt.DrawTextLayout(
                D2D_POINT_2F {
                    x: rect.left + x2,
                    y: rect.top,
                },
                text_layout,
                &text_brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
            );
        }

        if let Some(lp) = layer_params {
            self.rt.PopLayer();
            let _ = ManuallyDrop::into_inner(lp.geometricMask);
            let _ = ManuallyDrop::into_inner(lp.opacityBrush);
        } else {
            self.rt.PopAxisAlignedClip();
        }
    }

    unsafe fn draw_spectrum_bars(
        &self,
        layout: &BarLayout,
        accent: Rgb,
        is_idle: bool,
        light_taskbar: bool,
    ) {
        let base_alpha = if is_idle {
            0.32 * layout.spectrum_alpha
        } else {
            0.94 * layout.spectrum_alpha
        };

        let top_col = if is_idle {
            if light_taskbar {
                D2D1_COLOR_F {
                    r: 0.25,
                    g: 0.25,
                    b: 0.28,
                    a: base_alpha,
                }
            } else {
                D2D1_COLOR_F {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: base_alpha,
                }
            }
        } else {
            let c = d2d_color_rgb(accent, base_alpha);
            D2D1_COLOR_F {
                r: (c.r * 1.12).min(1.0),
                g: (c.g * 1.12).min(1.0),
                b: (c.b * 1.12).min(1.0),
                a: c.a,
            }
        };

        let bot_col = if is_idle {
            top_col
        } else {
            let c = d2d_color_rgb(accent, base_alpha * 0.78);
            D2D1_COLOR_F {
                r: c.r * 0.84,
                g: c.g * 0.84,
                b: c.b * 0.84,
                a: c.a,
            }
        };

        let stops = [
            D2D1_GRADIENT_STOP {
                position: 0.0,
                color: top_col,
            },
            D2D1_GRADIENT_STOP {
                position: 1.0,
                color: bot_col,
            },
        ];

        let Ok(stop_coll) =
            self.rt
                .CreateGradientStopCollection(&stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP)
        else {
            return;
        };

        for rect in &layout.spectrum_rects {
            let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                startPoint: D2D_POINT_2F {
                    x: rect.left,
                    y: rect.top,
                },
                endPoint: D2D_POINT_2F {
                    x: rect.left,
                    y: rect.bottom,
                },
            };
            if let Ok(brush) = self.rt.CreateLinearGradientBrush(&props, None, &stop_coll) {
                let rad = rect.width() * 0.5;
                let rr = D2D1_ROUNDED_RECT {
                    rect: d2d_rect(*rect),
                    radiusX: rad,
                    radiusY: rad,
                };
                self.rt.FillRoundedRectangle(&rr, &brush);
            }
        }
    }

    unsafe fn draw_control_buttons(
        &self,
        layout: &BarLayout,
        light_taskbar: bool,
        is_playing: bool,
        hover_zone: HitZone,
        pressed_zone: HitZone,
    ) {
        let alpha = layout.controls_alpha;
        let buttons = [
            (HitZone::PrevButton, layout.prev_btn_rect),
            (HitZone::PlayPauseButton, layout.play_btn_rect),
            (HitZone::NextButton, layout.next_btn_rect),
        ];

        for (zone, rect) in buttons {
            let hovered = hover_zone == zone;
            let pressed = pressed_zone == zone;
            if hovered || pressed {
                let bg = if light_taskbar {
                    D2D1_COLOR_F {
                        r: 0.0,
                        g: 0.0,
                        b: 0.0,
                        a: (if pressed { 0.14 } else { 0.08 }) * alpha,
                    }
                } else {
                    D2D1_COLOR_F {
                        r: 1.0,
                        g: 1.0,
                        b: 1.0,
                        a: (if pressed { 0.18 } else { 0.11 }) * alpha,
                    }
                };
                if let Ok(bg_brush) = self.rt.CreateSolidColorBrush(&bg, None) {
                    let rr = D2D1_ROUNDED_RECT {
                        rect: d2d_rect(rect),
                        radiusX: 5.5 * layout.scale,
                        radiusY: 5.5 * layout.scale,
                    };
                    self.rt.FillRoundedRectangle(&rr, &bg_brush);
                }
            }

            let fg = if light_taskbar {
                D2D1_COLOR_F {
                    r: 0.08,
                    g: 0.08,
                    b: 0.10,
                    a: (if pressed { 0.72 } else { 0.90 }) * alpha,
                }
            } else {
                D2D1_COLOR_F {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: (if pressed { 0.72 } else { 0.94 }) * alpha,
                }
            };
            let Ok(brush) = self.rt.CreateSolidColorBrush(&fg, None) else {
                continue;
            };

            let icon_scale = if pressed { 0.88 } else { 1.0 };
            let cx = (rect.left + rect.right) * 0.5;
            let cy = (rect.top + rect.bottom) * 0.5;
            let u = (rect.width() / 24.0) * icon_scale;

            match zone {
                HitZone::PrevButton => draw_prev_icon(&self.d2d_factory, &self.rt, cx, cy, u, &brush),
                HitZone::NextButton => draw_next_icon(&self.d2d_factory, &self.rt, cx, cy, u, &brush),
                HitZone::PlayPauseButton => {
                    if is_playing {
                        draw_pause_icon(&self.rt, cx, cy, u, &brush);
                    } else {
                        draw_play_icon(&self.d2d_factory, &self.rt, cx, cy, u, &brush);
                    }
                }
                _ => {}
            }
        }
    }

    unsafe fn draw_progress_hairline(
        &self,
        layout: &BarLayout,
        progress: f32,
        accent: Rgb,
        is_playing: bool,
        light_taskbar: bool,
    ) {
        let track = layout.progress_track_rect;
        let rad = track.height() * 0.5;
        let p = progress.clamp(0.0, 1.0);

        let track_col = if light_taskbar {
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
                a: 0.12,
            }
        };
        if let Ok(tb) = self.rt.CreateSolidColorBrush(&track_col, None) {
            let rr = D2D1_ROUNDED_RECT {
                rect: d2d_rect(track),
                radiusX: rad,
                radiusY: rad,
            };
            self.rt.FillRoundedRectangle(&rr, &tb);
        }

        let fill_w = (track.width() * p).clamp(0.0, track.width());
        if fill_w < 0.8 {
            return;
        }
        let fill_rect = RectF::new(track.left, track.top, track.left + fill_w, track.bottom);
        let alpha = if is_playing { 0.92 } else { 0.56 };

        let start_col = d2d_color_rgb(accent, alpha * 0.80);
        let tip_c = d2d_color_rgb(accent, alpha);
        let end_col = D2D1_COLOR_F {
            r: (tip_c.r * 1.18 + 0.08).min(1.0),
            g: (tip_c.g * 1.18 + 0.08).min(1.0),
            b: (tip_c.b * 1.18 + 0.08).min(1.0),
            a: alpha,
        };
        let stops = [
            D2D1_GRADIENT_STOP {
                position: 0.0,
                color: start_col,
            },
            D2D1_GRADIENT_STOP {
                position: 1.0,
                color: end_col,
            },
        ];

        let fill_rr = D2D1_ROUNDED_RECT {
            rect: d2d_rect(fill_rect),
            radiusX: rad,
            radiusY: rad,
        };
        if let Ok(sc) =
            self.rt
                .CreateGradientStopCollection(&stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP)
        {
            let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                startPoint: D2D_POINT_2F {
                    x: fill_rect.left,
                    y: fill_rect.top,
                },
                endPoint: D2D_POINT_2F {
                    x: fill_rect.right,
                    y: fill_rect.top,
                },
            };
            if let Ok(grad) = self.rt.CreateLinearGradientBrush(&props, None, &sc) {
                self.rt.FillRoundedRectangle(&fill_rr, &grad);
            }
        } else if let Ok(fb) = self.rt.CreateSolidColorBrush(&tip_c, None) {
            self.rt.FillRoundedRectangle(&fill_rr, &fb);
        }

        // Subtle luminous specular dot at the leading edge while playing
        if is_playing && p > 0.01 && p < 0.995 {
            let cy = (track.top + track.bottom) * 0.5;
            let glow_r = (2.6 * layout.scale).max(2.0);
            let glow_col = D2D1_COLOR_F {
                r: end_col.r,
                g: end_col.g,
                b: end_col.b,
                a: if light_taskbar { 0.34 } else { 0.42 },
            };
            if let Ok(gb) = self.rt.CreateSolidColorBrush(&glow_col, None) {
                self.rt.FillEllipse(
                    &D2D1_ELLIPSE {
                        point: D2D_POINT_2F {
                            x: fill_rect.right,
                            y: cy,
                        },
                        radiusX: glow_r,
                        radiusY: glow_r * 0.72,
                    },
                    &gb,
                );
            }
        }
    }

    /// Renders a DPI-aware anti-aliased grabbing hand cursor (`HCURSOR`) with dark outline and white fill.
    /// Returns `(cursor, is_owned_icon)` where `is_owned_icon` indicates `DestroyIcon` cleanup is required.
    pub fn create_grabbing_cursor(&self, dpi: u32) -> windows::core::Result<(HCURSOR, bool)> {
        crate::ui::glass::create_grabbing_cursor(&self.rt, dpi)
    }
}

pub(super) unsafe fn draw_prev_icon(
    factory: &ID2D1Factory1,
    rt: &ID2D1DCRenderTarget,
    cx: f32,
    cy: f32,
    u: f32,
    brush: &ID2D1SolidColorBrush,
) {
    let bar = D2D1_ROUNDED_RECT {
        rect: D2D_RECT_F {
            left: cx - 5.2 * u,
            top: cy - 5.0 * u,
            right: cx - 3.4 * u,
            bottom: cy + 5.0 * u,
        },
        radiusX: 0.9 * u,
        radiusY: 0.9 * u,
    };
    rt.FillRoundedRectangle(&bar, brush);

    if let Ok(tri) = factory.CreatePathGeometry() {
        if let Ok(sink) = tri.Open() {
            sink.BeginFigure(
                D2D_POINT_2F {
                    x: cx - 2.3 * u,
                    y: cy,
                },
                D2D1_FIGURE_BEGIN_FILLED,
            );
            sink.AddLine(D2D_POINT_2F {
                x: cx + 5.0 * u,
                y: cy - 5.0 * u,
            });
            sink.AddLine(D2D_POINT_2F {
                x: cx + 5.0 * u,
                y: cy + 5.0 * u,
            });
            sink.EndFigure(D2D1_FIGURE_END_CLOSED);
            let _ = sink.Close();
            rt.FillGeometry(&tri, brush, None);
        }
    }
}

pub(super) unsafe fn draw_next_icon(
    factory: &ID2D1Factory1,
    rt: &ID2D1DCRenderTarget,
    cx: f32,
    cy: f32,
    u: f32,
    brush: &ID2D1SolidColorBrush,
) {
    let bar = D2D1_ROUNDED_RECT {
        rect: D2D_RECT_F {
            left: cx + 3.4 * u,
            top: cy - 5.0 * u,
            right: cx + 5.2 * u,
            bottom: cy + 5.0 * u,
        },
        radiusX: 0.9 * u,
        radiusY: 0.9 * u,
    };
    rt.FillRoundedRectangle(&bar, brush);

    if let Ok(tri) = factory.CreatePathGeometry() {
        if let Ok(sink) = tri.Open() {
            sink.BeginFigure(
                D2D_POINT_2F {
                    x: cx + 2.3 * u,
                    y: cy,
                },
                D2D1_FIGURE_BEGIN_FILLED,
            );
            sink.AddLine(D2D_POINT_2F {
                x: cx - 5.0 * u,
                y: cy - 5.0 * u,
            });
            sink.AddLine(D2D_POINT_2F {
                x: cx - 5.0 * u,
                y: cy + 5.0 * u,
            });
            sink.EndFigure(D2D1_FIGURE_END_CLOSED);
            let _ = sink.Close();
            rt.FillGeometry(&tri, brush, None);
        }
    }
}

pub(super) unsafe fn draw_play_icon(
    factory: &ID2D1Factory1,
    rt: &ID2D1DCRenderTarget,
    cx: f32,
    cy: f32,
    u: f32,
    brush: &ID2D1SolidColorBrush,
) {
    if let Ok(tri) = factory.CreatePathGeometry() {
        if let Ok(sink) = tri.Open() {
            sink.BeginFigure(
                D2D_POINT_2F {
                    x: cx - 4.0 * u,
                    y: cy - 6.0 * u,
                },
                D2D1_FIGURE_BEGIN_FILLED,
            );
            sink.AddLine(D2D_POINT_2F {
                x: cx + 5.8 * u,
                y: cy,
            });
            sink.AddLine(D2D_POINT_2F {
                x: cx - 4.0 * u,
                y: cy + 6.0 * u,
            });
            sink.EndFigure(D2D1_FIGURE_END_CLOSED);
            let _ = sink.Close();
            rt.FillGeometry(&tri, brush, None);
        }
    }
}

pub(super) unsafe fn draw_pause_icon(
    rt: &ID2D1DCRenderTarget,
    cx: f32,
    cy: f32,
    u: f32,
    brush: &ID2D1SolidColorBrush,
) {
    let left = D2D1_ROUNDED_RECT {
        rect: D2D_RECT_F {
            left: cx - 4.5 * u,
            top: cy - 5.5 * u,
            right: cx - 1.5 * u,
            bottom: cy + 5.5 * u,
        },
        radiusX: 1.2 * u,
        radiusY: 1.2 * u,
    };
    let right = D2D1_ROUNDED_RECT {
        rect: D2D_RECT_F {
            left: cx + 1.5 * u,
            top: cy - 5.5 * u,
            right: cx + 4.5 * u,
            bottom: cy + 5.5 * u,
        },
        radiusX: 1.2 * u,
        radiusY: 1.2 * u,
    };
    rt.FillRoundedRectangle(&left, brush);
    rt.FillRoundedRectangle(&right, brush);
}
