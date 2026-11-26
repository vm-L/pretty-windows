//! Lightweight click-through Liquid Glass tooltip pill for DeskBox action buttons.

use super::super::desktop::monitor_info_at_point;
use crate::color::NEUTRAL_BACKDROP_FALLBACK;
use crate::system::{
    active_taskbar_surface_style, resolve_unified_accent, visible_transient_overlay_rects,
};
use crate::ui::glass::{
    capture_blurred_desktop_backdrop_excluding, create_dc_render_target, create_text_format,
    d2d_rect, draw_glass_interior, draw_rounded_card_rim, draw_rounded_card_shadows,
    draw_text_in_rect, identity_matrix, primary_text_color, update_silk_bitmap, LayeredSurface,
    PopupNoiseCache,
};
use crate::ui::{
    compute_halo_pair, popup_glass_recipe, rgb_luma, GlassSurfaceStyle, RectF, SILK_H, SILK_W,
};
use std::mem::{size_of, ManuallyDrop};
use std::time::Instant;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{D2DERR_RECREATE_TARGET, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;
use windows::Win32::Graphics::Direct2D::{
    ID2D1Bitmap, ID2D1DCRenderTarget, ID2D1Factory1, ID2D1Geometry, ID2D1Layer,
    D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_LAYER_OPTIONS_NONE, D2D1_LAYER_PARAMETERS,
    D2D1_ROUNDED_RECT,
};
use windows::Win32::Graphics::DirectWrite::{
    IDWriteFactory, IDWriteTextFormat, DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_TEXT_METRICS,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, RegisterClassExW, SetWindowPos, ShowWindow,
    HTTRANSPARENT, HWND_TOPMOST, SWP_NOACTIVATE, SWP_SHOWWINDOW, SW_HIDE, WM_NCHITTEST,
    WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_EX_TRANSPARENT, WS_POPUP,
};

const TOOLTIP_CLASS_NAME: PCWSTR = w!("PrettyWindowsDeskBoxTooltipWnd");
const TOOLTIP_PAD_DIP: f32 = 12.0;
const TOOLTIP_RADIUS_DIP: f32 = 8.0;
const TOOLTIP_GAP_DIP: f32 = 6.0;

static TOOLTIP_CLASS_ONCE: std::sync::Once = std::sync::Once::new();

pub struct BoxTooltip {
    hwnd: HWND,
    owner_hwnd: HWND,
    d2d_factory: ID2D1Factory1,
    dwrite_factory: IDWriteFactory,
    // Declare D2D RT-owned resources BEFORE `surface: LayeredSurface` so RT releases before DeleteDC.
    noise_cache: PopupNoiseCache,
    silk_bmp: Option<ID2D1Bitmap>,
    layer: ID2D1Layer,
    rt: ID2D1DCRenderTarget,
    surface: LayeredSurface,
    silk_buf: Vec<u8>,
    last_silk_tick: Option<Instant>,
    cached_fmt: Option<(u32, IDWriteTextFormat)>,
    visible: bool,
    last_text: String,
    last_anchor: RECT,
    last_light_glass: bool,
    last_style: GlassSurfaceStyle,
}

impl BoxTooltip {
    pub fn new(
        d2d_factory: ID2D1Factory1,
        dwrite_factory: IDWriteFactory,
    ) -> windows::core::Result<Self> {
        unsafe {
            let hinst = GetModuleHandleW(None)?;
            TOOLTIP_CLASS_ONCE.call_once(|| {
                let wc = WNDCLASSEXW {
                    cbSize: size_of::<WNDCLASSEXW>() as u32,
                    lpfnWndProc: Some(tooltip_wnd_proc),
                    hInstance: hinst.into(),
                    lpszClassName: TOOLTIP_CLASS_NAME,
                    ..Default::default()
                };
                RegisterClassExW(&wc);
            });

            let hwnd = CreateWindowExW(
                WS_EX_LAYERED
                    | WS_EX_TRANSPARENT
                    | WS_EX_TOPMOST
                    | WS_EX_NOACTIVATE
                    | WS_EX_TOOLWINDOW,
                TOOLTIP_CLASS_NAME,
                w!("DeskBox Tooltip"),
                WS_POPUP,
                0,
                0,
                1,
                1,
                None,
                None,
                hinst,
                None,
            )?;

            let init_res = (|| -> windows::core::Result<(
                ID2D1DCRenderTarget,
                ID2D1Layer,
                LayeredSurface,
            )> {
                let rt = create_dc_render_target(&d2d_factory)?;
                let layer = rt.CreateLayer(None)?;
                let surface = LayeredSurface::new()?;
                Ok((rt, layer, surface))
            })();
            let (rt, layer, surface) = match init_res {
                Ok(v) => v,
                Err(e) => {
                    let _ = DestroyWindow(hwnd);
                    return Err(e);
                }
            };
            Ok(Self {
                hwnd,
                owner_hwnd: HWND::default(),
                d2d_factory,
                dwrite_factory,
                rt,
                layer,
                surface,
                noise_cache: PopupNoiseCache::default(),
                silk_bmp: None,
                silk_buf: vec![0u8; (SILK_W * SILK_H * 4) as usize],
                last_silk_tick: None,
                cached_fmt: None,
                visible: false,
                last_text: String::new(),
                last_anchor: RECT::default(),
                last_light_glass: false,
                last_style: GlassSurfaceStyle::LiquidGlass,
            })
        }
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    pub fn hide(&mut self) {
        if self.visible && !self.hwnd.is_invalid() {
            unsafe {
                let _ = ShowWindow(self.hwnd, SW_HIDE);
            }
            self.visible = false;
        }
        self.owner_hwnd = HWND::default();
        self.last_text.clear();
    }

    pub fn hide_if_owner(&mut self, owner_hwnd: HWND) {
        if self.owner_hwnd == owner_hwnd {
            self.hide();
        }
    }

    pub fn show(
        &mut self,
        owner_hwnd: HWND,
        anchor_rect_screen: RECT,
        text: &str,
        light_glass: bool,
    ) {
        if text.is_empty() || self.hwnd.is_invalid() {
            self.hide();
            return;
        }
        let style = active_taskbar_surface_style();
        if self.visible
            && self.owner_hwnd == owner_hwnd
            && self.last_text == text
            && self.last_anchor == anchor_rect_screen
            && self.last_light_glass == light_glass
            && self.last_style == style
        {
            return;
        }

        // Hide briefly before capturing the desktop backdrop so a previous tooltip never bleeds into the blur.
        if self.visible {
            unsafe {
                let _ = ShowWindow(self.hwnd, SW_HIDE);
            }
            self.visible = false;
        }

        let center_pt = POINT {
            x: (anchor_rect_screen.left + anchor_rect_screen.right) / 2,
            y: (anchor_rect_screen.top + anchor_rect_screen.bottom) / 2,
        };
        let mon = monitor_info_at_point(center_pt);
        let dpi = mon.dpi.max(96);
        let s = (dpi as f32) / 96.0;
        let Ok(fmt) = self.ensure_format(dpi, s) else {
            return;
        };

        let (text_w, text_h) = self.measure_text(text, &fmt, s);
        let pad = (TOOLTIP_PAD_DIP * s).round();
        let card_w = (text_w + 20.0 * s).ceil().max(44.0 * s);
        let card_h = (text_h + 10.0 * s).ceil().max(26.0 * s);
        let win_w = (card_w + pad * 2.0).round() as i32;
        let win_h = (card_h + pad * 2.0).round() as i32;
        let card_w_i = card_w.round() as i32;
        let card_h_i = card_h.round() as i32;
        let gap = (TOOLTIP_GAP_DIP * s).round() as i32;
        let margin = (4.0 * s).round() as i32;

        let desired_left = center_pt.x - card_w_i / 2;
        let min_left = mon.work.left + margin;
        let max_left = (mon.work.right - margin - card_w_i).max(min_left);
        let card_left = desired_left.clamp(min_left, max_left);

        let above_top = anchor_rect_screen.top - gap - card_h_i;
        let card_top = if above_top >= mon.work.top + margin {
            above_top
        } else {
            let below_top = anchor_rect_screen.bottom + gap;
            let max_top = (mon.work.bottom - margin - card_h_i).max(mon.work.top + margin);
            below_top.min(max_top)
        };

        let win_x = card_left - pad as i32;
        let win_y = card_top - pad as i32;

        if self.surface.ensure_size(win_w, win_h).is_err() {
            return;
        }

        let card_rect = RectF::new(pad, pad, pad + card_w, pad + card_h);
        if let Err(e) = self.draw_pill(
            win_x,
            win_y,
            win_w,
            win_h,
            card_rect,
            text,
            &fmt,
            style,
            light_glass,
            s,
        ) {
            if e.code() == D2DERR_RECREATE_TARGET {
                self.silk_bmp = None;
                self.noise_cache.clear();
                self.last_silk_tick = None;
                if let Ok(new_rt) = create_dc_render_target(&self.d2d_factory) {
                    if let Ok(new_layer) = unsafe { new_rt.CreateLayer(None) } {
                        self.rt = new_rt;
                        self.layer = new_layer;
                        let _ = self.draw_pill(
                            win_x,
                            win_y,
                            win_w,
                            win_h,
                            card_rect,
                            text,
                            &fmt,
                            style,
                            light_glass,
                            s,
                        );
                    }
                }
            }
        }

        self.surface.commit(self.hwnd, win_x, win_y);
        unsafe {
            let _ = SetWindowPos(
                self.hwnd,
                HWND_TOPMOST,
                win_x,
                win_y,
                win_w,
                win_h,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
        }

        self.owner_hwnd = owner_hwnd;
        self.visible = true;
        self.last_text.clear();
        self.last_text.push_str(text);
        self.last_anchor = anchor_rect_screen;
        self.last_light_glass = light_glass;
        self.last_style = style;
    }

    fn ensure_format(&mut self, dpi: u32, s: f32) -> windows::core::Result<IDWriteTextFormat> {
        if let Some((cached_dpi, ref fmt)) = self.cached_fmt {
            if cached_dpi == dpi {
                return Ok(fmt.clone());
            }
        }
        let fmt = create_text_format(&self.dwrite_factory, 12.0 * s, false, false, false)?;
        unsafe {
            let _ = fmt.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER);
        }
        self.cached_fmt = Some((dpi, fmt.clone()));
        Ok(fmt)
    }

    fn measure_text(&self, text: &str, fmt: &IDWriteTextFormat, s: f32) -> (f32, f32) {
        let wide: Vec<u16> = text.encode_utf16().collect();
        unsafe {
            if let Ok(layout) = self
                .dwrite_factory
                .CreateTextLayout(&wide, fmt, 400.0 * s, 40.0 * s)
            {
                let mut metrics = DWRITE_TEXT_METRICS::default();
                if layout.GetMetrics(&mut metrics).is_ok() {
                    let w = metrics
                        .width
                        .max(metrics.widthIncludingTrailingWhitespace)
                        .ceil();
                    return (w, metrics.height.ceil());
                }
            }
        }
        ((text.chars().count() as f32) * 12.0 * s, 16.0 * s)
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_pill(
        &mut self,
        win_x: i32,
        win_y: i32,
        win_w: i32,
        win_h: i32,
        card_rect: RectF,
        text: &str,
        fmt: &IDWriteTextFormat,
        style: GlassSurfaceStyle,
        light_glass: bool,
        s: f32,
    ) -> windows::core::Result<()> {
        let initial_recipe = popup_glass_recipe(style, light_glass, 0.5);
        let capture_rect = RECT {
            left: win_x,
            top: win_y,
            right: win_x + win_w,
            bottom: win_y + win_h,
        };
        let occluders = visible_transient_overlay_rects(capture_rect);
        let (backdrop_bmp, sampled_bg) = capture_blurred_desktop_backdrop_excluding(
            &self.rt,
            win_x,
            win_y,
            win_w,
            win_h,
            initial_recipe.blur_radius_4x,
            &occluders,
        );
        let backdrop_rgb = sampled_bg.unwrap_or(NEUTRAL_BACKDROP_FALLBACK);
        let recipe = popup_glass_recipe(style, light_glass, rgb_luma(backdrop_rgb));

        if recipe.silk_alpha > 0.002 {
            let (_, base_dark) = resolve_unified_accent(backdrop_rgb, light_glass);
            let pair = compute_halo_pair(base_dark, light_glass);
            let halo_colors = [pair[0].to_normalized(), pair[1].to_normalized()];
            update_silk_bitmap(
                &self.rt,
                &mut self.silk_bmp,
                &mut self.silk_buf,
                &mut self.last_silk_tick,
                true,
                win_w,
                win_h,
                card_rect.left,
                card_rect.bottom,
                s,
                0.0,
                &halo_colors,
                light_glass,
            );
        }

        let radius = TOOLTIP_RADIUS_DIP * s;
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

            draw_rounded_card_shadows(
                &self.rt,
                card_rect,
                radius,
                win_w as f32,
                win_h as f32,
                s,
                light_glass,
            );

            let card_rr = D2D1_ROUNDED_RECT {
                rect: d2d_rect(card_rect),
                radiusX: radius,
                radiusY: radius,
            };
            if let Ok(geo) = self.d2d_factory.CreateRoundedRectangleGeometry(&card_rr) {
                let geom: ID2D1Geometry = geo.into();
                let params = D2D1_LAYER_PARAMETERS {
                    contentBounds: d2d_rect(card_rect),
                    geometricMask: ManuallyDrop::new(Some(geom)),
                    maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                    maskTransform: identity_matrix(),
                    opacity: 1.0,
                    opacityBrush: ManuallyDrop::new(None),
                    layerOptions: D2D1_LAYER_OPTIONS_NONE,
                };
                self.rt.PushLayer(&params, &self.layer);

                draw_glass_interior(
                    &self.rt,
                    card_rect,
                    win_w,
                    win_h,
                    backdrop_bmp.as_ref(),
                    self.silk_bmp.as_ref(),
                    light_glass,
                    &recipe,
                    &mut self.noise_cache,
                );

                let text_inset = 4.0 * s;
                let text_rect = RectF::new(
                    card_rect.left + text_inset,
                    card_rect.top,
                    (card_rect.right - text_inset).max(card_rect.left + text_inset + 1.0),
                    card_rect.bottom,
                );
                let fg = primary_text_color(light_glass);
                draw_text_in_rect(&self.dwrite_factory, &self.rt, text, text_rect, fmt, fg);

                self.rt.PopLayer();
                let _ = ManuallyDrop::into_inner(params.geometricMask);
                let _ = ManuallyDrop::into_inner(params.opacityBrush);
            }

            draw_rounded_card_rim(&self.rt, card_rect, radius, s, &recipe);

            self.rt.EndDraw(None, None)
        }
    }
}

impl Drop for BoxTooltip {
    fn drop(&mut self) {
        self.hide();
        unsafe {
            if !self.hwnd.is_invalid() {
                let _ = DestroyWindow(self.hwnd);
            }
        }
    }
}

unsafe extern "system" fn tooltip_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_NCHITTEST {
        return LRESULT(HTTRANSPARENT as isize);
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}
