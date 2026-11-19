use super::config::TaskbarStyle;
use super::swca::{apply_window_accent, backdrop_blur_policy, AccentPolicySpec};
use crate::color::{adjust_accent_for_theme, Rgb};
use crate::ui::glass::{d2d_rect, update_silk_bitmap, LayeredSurface};
use crate::ui::{compute_halo_pair, RectF, SILK_H, SILK_W};
use std::collections::HashMap;
use std::mem::size_of;
use std::sync::OnceLock;
use std::time::Instant;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_GRADIENT_STOP, D2D1_PIXEL_FORMAT,
    D2D_POINT_2F, D2D_RECT_F,
};
use windows::Win32::Graphics::Direct2D::{
    ID2D1Bitmap, ID2D1DCRenderTarget, ID2D1Factory1, D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
    D2D1_EXTEND_MODE_CLAMP, D2D1_GAMMA_2_2, D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES,
    D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE_DEFAULT, D2D1_RENDER_TARGET_USAGE_NONE,
};
use windows::Win32::Graphics::Dwm::{
    DwmExtendFrameIntoClientArea, DwmSetWindowAttribute, DWMWCP_DONOTROUND,
    DWMWA_WINDOW_CORNER_PREFERENCE, DWM_WINDOW_CORNER_PREFERENCE,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::{
    BeginPaint, EndPaint, FillRect, GetMonitorInfoW, GetStockObject, InvalidateRect,
    MonitorFromWindow, BLACK_BRUSH, HBRUSH, MONITORINFO, MONITOR_DEFAULTTONEAREST, PAINTSTRUCT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::MARGINS;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Shell::{SHAppBarMessage, ABM_GETSTATE, ABS_AUTOHIDE, APPBARDATA};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetWindow, GetWindowRect, IsWindowVisible,
    RegisterClassExW, SetWindowPos, ShowWindow, GW_HWNDNEXT, HTTRANSPARENT, SWP_NOACTIVATE,
    SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW, SW_HIDE, WM_ERASEBKGND, WM_NCHITTEST, WM_PAINT,
    WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_EX_TRANSPARENT, WS_POPUP,
};

const GLASS_BLUR_CLASS: PCWSTR = w!("PrettyWindowsTaskbarBlurWnd");
const GLASS_OVERLAY_CLASS: PCWSTR = w!("PrettyWindowsTaskbarGlassWnd");

struct MonitorGlassBackdrop {
    blur_hwnd: HWND,
    glass_hwnd: HWND,
    surface: LayeredSurface,
    rt: ID2D1DCRenderTarget,
    silk_bmp: Option<ID2D1Bitmap>,
    silk_buf: Vec<u8>,
    last_silk_tick: Option<Instant>,
    last_rect: RECT,
    blur_visible: bool,
    glass_visible: bool,
    blur_policy_dirty: bool,
}

impl Drop for MonitorGlassBackdrop {
    fn drop(&mut self) {
        unsafe {
            if !self.glass_hwnd.is_invalid() {
                let _ = DestroyWindow(self.glass_hwnd);
            }
            if !self.blur_hwnd.is_invalid() {
                let _ = DestroyWindow(self.blur_hwnd);
            }
        }
    }
}

/// Manages full-width DWM blur / acrylic / Liquid Glass backdrop windows behind every monitor's taskbar.
pub struct LiquidGlassManager {
    d2d_factory: ID2D1Factory1,
    style: TaskbarStyle,
    light_glass: bool,
    accent: Rgb,
    halo_colors: [(f32, f32, f32); 2],
    halo_ready: bool,
    flow_phase: f64,
    last_flow_tick: Instant,
    last_color_tick: Instant,
    backdrops: HashMap<isize, MonitorGlassBackdrop>,
}

impl LiquidGlassManager {
    pub fn new(d2d_factory: ID2D1Factory1, light_glass: bool, accent: Rgb) -> Self {
        Self {
            d2d_factory,
            style: TaskbarStyle::Default,
            light_glass,
            accent,
            halo_colors: [(0.35, 0.58, 0.90); 2],
            halo_ready: false,
            flow_phase: 0.0,
            last_flow_tick: Instant::now(),
            last_color_tick: Instant::now(),
            backdrops: HashMap::new(),
        }
    }

    pub fn set_style(&mut self, style: TaskbarStyle, taskbars: &[HWND]) {
        let style_changed = self.style != style;
        self.style = style;
        let needs_backdrop = matches!(
            style,
            TaskbarStyle::Blur | TaskbarStyle::Acrylic | TaskbarStyle::LiquidGlass
        );
        if !needs_backdrop {
            self.backdrops.clear();
            return;
        }
        if style_changed {
            if style == TaskbarStyle::LiquidGlass {
                self.last_flow_tick = Instant::now();
                self.last_color_tick = self.last_flow_tick;
                self.halo_ready = false;
            }
            for b in self.backdrops.values_mut() {
                b.blur_policy_dirty = true;
                if style != TaskbarStyle::LiquidGlass {
                    b.last_silk_tick = None;
                }
            }
        }
        self.sync(taskbars);
    }

    pub fn set_theme(&mut self, light_glass: bool, accent: Rgb) {
        let light_changed = self.light_glass != light_glass;
        let accent_changed = self.accent != accent;
        self.light_glass = light_glass;
        self.accent = accent;
        if light_changed || accent_changed {
            self.halo_ready = false;
            for b in self.backdrops.values_mut() {
                b.last_silk_tick = None;
                if light_changed {
                    b.blur_policy_dirty = true;
                }
            }
        }
    }

    pub fn reset_windows(&mut self) {
        self.backdrops.clear();
    }

    pub fn sync(&mut self, taskbars: &[HWND]) {
        let Some(policy) = backdrop_blur_policy(self.style, self.light_glass) else {
            self.backdrops.clear();
            return;
        };
        let want_glass_overlay = self.style == TaskbarStyle::LiquidGlass;

        if want_glass_overlay {
            let now = Instant::now();
            let dt_flow = now
                .duration_since(self.last_flow_tick)
                .as_secs_f32()
                .clamp(0.001, 0.1);
            self.last_flow_tick = now;
            self.flow_phase += f64::from(dt_flow);

            let dt_color = now
                .duration_since(self.last_color_tick)
                .as_secs_f32()
                .clamp(0.001, 0.05);
            self.last_color_tick = now;

            let base_dark = adjust_accent_for_theme(self.accent, false);
            let pair = compute_halo_pair(base_dark, self.light_glass);
            let target = [pair[0].to_normalized(), pair[1].to_normalized()];
            if !self.halo_ready {
                self.halo_colors = target;
                self.halo_ready = true;
            } else {
                let k = (dt_color * 4.5).clamp(0.02, 1.0);
                for (cur, tgt) in self.halo_colors.iter_mut().zip(target.iter()) {
                    cur.0 += (tgt.0 - cur.0) * k;
                    cur.1 += (tgt.1 - cur.1) * k;
                    cur.2 += (tgt.2 - cur.2) * k;
                }
            }
        }

        let valid_keys: Vec<isize> = taskbars
            .iter()
            .filter(|h| !h.is_invalid())
            .map(|h| h.0 as isize)
            .collect();
        self.backdrops.retain(|k, _| valid_keys.contains(k));

        for &tb_hwnd in taskbars {
            if tb_hwnd.is_invalid() {
                continue;
            }
            let key = tb_hwnd.0 as isize;
            if !self.backdrops.contains_key(&key) {
                if let Ok(entry) = create_backdrop(&self.d2d_factory, policy) {
                    self.backdrops.insert(key, entry);
                } else {
                    continue;
                }
            }
            let Some(entry) = self.backdrops.get_mut(&key) else {
                continue;
            };

            let mut tb_rect = RECT::default();
            let got_rect = unsafe { GetWindowRect(tb_hwnd, &mut tb_rect).is_ok() };
            let should_hide =
                !got_rect || should_hide_backdrop_for_taskbar(tb_hwnd, tb_rect);

            if should_hide {
                unsafe {
                    if entry.glass_visible {
                        let _ = ShowWindow(entry.glass_hwnd, SW_HIDE);
                        entry.glass_visible = false;
                    }
                    if entry.blur_visible {
                        let _ = ShowWindow(entry.blur_hwnd, SW_HIDE);
                        entry.blur_visible = false;
                    }
                }
                continue;
            }

            let w = (tb_rect.right - tb_rect.left).max(1);
            let h = (tb_rect.bottom - tb_rect.top).max(1);
            let dpi = unsafe { GetDpiForWindow(tb_hwnd).max(96) };
            let size_changed = entry.last_rect.left != tb_rect.left
                || entry.last_rect.top != tb_rect.top
                || entry.last_rect.right != tb_rect.right
                || entry.last_rect.bottom != tb_rect.bottom;

            if entry.blur_policy_dirty {
                let _ = apply_window_accent(entry.blur_hwnd, policy);
                unsafe {
                    let _ = InvalidateRect(entry.blur_hwnd, None, false);
                }
                entry.blur_policy_dirty = false;
            }

            if want_glass_overlay {
                let reveal_glass = !entry.glass_visible;
                let mut g_flags = SWP_NOACTIVATE;
                if !size_changed {
                    g_flags |= SWP_NOMOVE | SWP_NOSIZE;
                }
                if reveal_glass {
                    g_flags |= SWP_SHOWWINDOW;
                }
                if reveal_glass
                    || size_changed
                    || unsafe { GetWindow(tb_hwnd, GW_HWNDNEXT) } != Ok(entry.glass_hwnd)
                {
                    unsafe {
                        let _ = SetWindowPos(
                            entry.glass_hwnd,
                            tb_hwnd,
                            tb_rect.left,
                            tb_rect.top,
                            w,
                            h,
                            g_flags,
                        );
                    }
                }
                entry.glass_visible = true;

                let reveal_blur = !entry.blur_visible;
                let mut b_flags = SWP_NOACTIVATE;
                if !size_changed {
                    b_flags |= SWP_NOMOVE | SWP_NOSIZE;
                }
                if reveal_blur {
                    b_flags |= SWP_SHOWWINDOW;
                }
                if reveal_blur
                    || size_changed
                    || unsafe { GetWindow(entry.glass_hwnd, GW_HWNDNEXT) } != Ok(entry.blur_hwnd)
                {
                    unsafe {
                        let _ = SetWindowPos(
                            entry.blur_hwnd,
                            entry.glass_hwnd,
                            tb_rect.left,
                            tb_rect.top,
                            w,
                            h,
                            b_flags,
                        );
                    }
                }
                entry.blur_visible = true;
                entry.last_rect = tb_rect;

                let _ = render_glass_bar(
                    entry,
                    tb_rect,
                    w,
                    h,
                    dpi,
                    self.flow_phase,
                    &self.halo_colors,
                    self.light_glass,
                );
            } else {
                if entry.glass_visible {
                    unsafe {
                        let _ = ShowWindow(entry.glass_hwnd, SW_HIDE);
                    }
                    entry.glass_visible = false;
                }

                let reveal_blur = !entry.blur_visible;
                let mut b_flags = SWP_NOACTIVATE;
                if !size_changed {
                    b_flags |= SWP_NOMOVE | SWP_NOSIZE;
                }
                if reveal_blur {
                    b_flags |= SWP_SHOWWINDOW;
                }
                if reveal_blur
                    || size_changed
                    || unsafe { GetWindow(tb_hwnd, GW_HWNDNEXT) } != Ok(entry.blur_hwnd)
                {
                    unsafe {
                        let _ = SetWindowPos(
                            entry.blur_hwnd,
                            tb_hwnd,
                            tb_rect.left,
                            tb_rect.top,
                            w,
                            h,
                            b_flags,
                        );
                    }
                }
                entry.blur_visible = true;
                entry.last_rect = tb_rect;
            }
        }
    }
}

fn ensure_window_classes() -> windows::core::Result<()> {
    static INIT: OnceLock<bool> = OnceLock::new();
    let ok = *INIT.get_or_init(|| unsafe {
        let Ok(hinst) = GetModuleHandleW(None) else {
            return false;
        };
        let blur_wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(blur_wnd_proc),
            hInstance: hinst.into(),
            lpszClassName: GLASS_BLUR_CLASS,
            ..Default::default()
        };
        let glass_wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(glass_wnd_proc),
            hInstance: hinst.into(),
            lpszClassName: GLASS_OVERLAY_CLASS,
            ..Default::default()
        };
        RegisterClassExW(&blur_wc);
        RegisterClassExW(&glass_wc);
        true
    });
    if ok {
        Ok(())
    } else {
        Err(windows::core::Error::from_win32())
    }
}

unsafe extern "system" fn blur_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);
            if !hdc.is_invalid() {
                let brush = HBRUSH(GetStockObject(BLACK_BRUSH).0);
                let _ = FillRect(hdc, &ps.rcPaint, brush);
            }
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe extern "system" fn glass_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_NCHITTEST {
        LRESULT(HTTRANSPARENT as isize)
    } else {
        DefWindowProcW(hwnd, msg, wparam, lparam)
    }
}

fn create_backdrop(
    d2d_factory: &ID2D1Factory1,
    policy: AccentPolicySpec,
) -> windows::core::Result<MonitorGlassBackdrop> {
    ensure_window_classes()?;
    unsafe {
        let hinst = GetModuleHandleW(None)?;
        let blur_hwnd = CreateWindowExW(
            WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TOPMOST,
            GLASS_BLUR_CLASS,
            w!("Pretty Windows Taskbar Blur"),
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

        let corner = DWMWCP_DONOTROUND;
        let _ = DwmSetWindowAttribute(
            blur_hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            (&corner as *const DWM_WINDOW_CORNER_PREFERENCE).cast(),
            size_of::<DWM_WINDOW_CORNER_PREFERENCE>() as u32,
        );
        let margins = MARGINS {
            cxLeftWidth: -1,
            cxRightWidth: -1,
            cyTopHeight: -1,
            cyBottomHeight: -1,
        };
        let _ = DwmExtendFrameIntoClientArea(blur_hwnd, &margins);
        let _ = apply_window_accent(blur_hwnd, policy);

        let glass_hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TOPMOST,
            GLASS_OVERLAY_CLASS,
            w!("Pretty Windows Taskbar Glass"),
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
        let surface = LayeredSurface::new()?;

        Ok(MonitorGlassBackdrop {
            blur_hwnd,
            glass_hwnd,
            surface,
            rt,
            silk_bmp: None,
            silk_buf: vec![0u8; (SILK_W * SILK_H * 4) as usize],
            last_silk_tick: None,
            last_rect: RECT::default(),
            blur_visible: false,
            glass_visible: false,
            blur_policy_dirty: false,
        })
    }
}

fn should_hide_backdrop_for_taskbar(tb_hwnd: HWND, tb_rect: RECT) -> bool {
    unsafe {
        if !IsWindowVisible(tb_hwnd).as_bool() {
            return true;
        }
        if tb_rect.right <= tb_rect.left || tb_rect.bottom <= tb_rect.top {
            return true;
        }

        let mon_tb = MonitorFromWindow(tb_hwnd, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if GetMonitorInfoW(mon_tb, &mut mi).as_bool() {
            // Check auto-hide state or off-screen collapsed taskbar sliver.
            let mut abd = APPBARDATA {
                cbSize: size_of::<APPBARDATA>() as u32,
                hWnd: tb_hwnd,
                ..Default::default()
            };
            let is_autohide = (SHAppBarMessage(ABM_GETSTATE, &mut abd) as u32 & ABS_AUTOHIDE) != 0;
            let threshold = if is_autohide { 8 } else { 2 };
            if tb_rect.top >= mi.rcMonitor.bottom - threshold
                || tb_rect.bottom <= mi.rcMonitor.top + threshold
            {
                return true;
            }
        }
    }
    crate::system::is_fullscreen_occluding_taskbar(tb_hwnd)
}

#[allow(clippy::too_many_arguments)]
fn render_glass_bar(
    entry: &mut MonitorGlassBackdrop,
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

    update_silk_bitmap(
        &entry.rt,
        &mut entry.silk_bmp,
        &mut entry.silk_buf,
        &mut entry.last_silk_tick,
        false,
        w,
        h,
        0.0,
        hf,
        s,
        flow_phase,
        halo_colors,
        light_glass,
    );

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

        // 1. Neutral frosted tint (keeps underlying DWM acrylic refraction clearly visible).
        let base_col = if light_glass {
            D2D1_COLOR_F {
                r: 246.0 / 255.0,
                g: 246.0 / 255.0,
                b: 248.0 / 255.0,
                a: 0.44,
            }
        } else {
            D2D1_COLOR_F {
                r: 24.0 / 255.0,
                g: 24.0 / 255.0,
                b: 26.0 / 255.0,
                a: 0.34,
            }
        };
        if let Ok(base_brush) = entry.rt.CreateSolidColorBrush(&base_col, None) {
            entry.rt.FillRectangle(&full, &base_brush);
        }

        // 2. Restrained domain-warped Liquid Silk wisps (adapted to system accent).
        if let Some(ref silk) = entry.silk_bmp {
            entry.rt.DrawBitmap(
                silk,
                Some(&full),
                1.0,
                D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                None,
            );
        }

        // 3. Standard diagonal glass specular sweep.
        let sheen_stops = if light_glass {
            [
                D2D1_GRADIENT_STOP {
                    position: 0.0,
                    color: D2D1_COLOR_F {
                        r: 1.0,
                        g: 1.0,
                        b: 1.0,
                        a: 0.26,
                    },
                },
                D2D1_GRADIENT_STOP {
                    position: 0.45,
                    color: D2D1_COLOR_F {
                        r: 1.0,
                        g: 1.0,
                        b: 1.0,
                        a: 0.03,
                    },
                },
                D2D1_GRADIENT_STOP {
                    position: 1.0,
                    color: D2D1_COLOR_F {
                        r: 1.0,
                        g: 1.0,
                        b: 1.0,
                        a: 0.10,
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
                        a: 0.14,
                    },
                },
                D2D1_GRADIENT_STOP {
                    position: 0.45,
                    color: D2D1_COLOR_F {
                        r: 1.0,
                        g: 1.0,
                        b: 1.0,
                        a: 0.02,
                    },
                },
                D2D1_GRADIENT_STOP {
                    position: 1.0,
                    color: D2D1_COLOR_F {
                        r: 1.0,
                        g: 1.0,
                        b: 1.0,
                        a: 0.06,
                    },
                },
            ]
        };
        if let Ok(sc) = entry.rt.CreateGradientStopCollection(
            &sheen_stops,
            D2D1_GAMMA_2_2,
            D2D1_EXTEND_MODE_CLAMP,
        ) {
            let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                startPoint: D2D_POINT_2F { x: 0.0, y: 0.0 },
                endPoint: D2D_POINT_2F { x: wf, y: hf },
            };
            if let Ok(sb) = entry.rt.CreateLinearGradientBrush(&props, None, &sc) {
                entry.rt.FillRectangle(&full, &sb);
            }
        }

        // 4. Top liquid refraction bevel + crisp glass rim stroke.
        let bevel_h = (6.0 * s).min(hf);
        let bevel_stops = [
            D2D1_GRADIENT_STOP {
                position: 0.0,
                color: D2D1_COLOR_F {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: if light_glass { 0.18 } else { 0.10 },
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
        if let Ok(sc) = entry.rt.CreateGradientStopCollection(
            &bevel_stops,
            D2D1_GAMMA_2_2,
            D2D1_EXTEND_MODE_CLAMP,
        ) {
            let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                startPoint: D2D_POINT_2F { x: 0.0, y: 0.0 },
                endPoint: D2D_POINT_2F { x: 0.0, y: bevel_h },
            };
            if let Ok(bb) = entry.rt.CreateLinearGradientBrush(&props, None, &sc) {
                let bevel_rect = D2D_RECT_F {
                    left: 0.0,
                    top: 0.0,
                    right: wf,
                    bottom: bevel_h,
                };
                entry.rt.FillRectangle(&bevel_rect, &bb);
            }
        }

        let edge_y = 0.6 * s;
        if light_glass {
            let def_col = D2D1_COLOR_F {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.09,
            };
            if let Ok(db) = entry.rt.CreateSolidColorBrush(&def_col, None) {
                entry.rt.DrawLine(
                    D2D_POINT_2F { x: 0.0, y: edge_y },
                    D2D_POINT_2F { x: wf, y: edge_y },
                    &db,
                    1.0 * s,
                    None,
                );
            }
        }
        let edge_stops = if light_glass {
            [
                D2D1_GRADIENT_STOP {
                    position: 0.0,
                    color: D2D1_COLOR_F {
                        r: 1.0,
                        g: 1.0,
                        b: 1.0,
                        a: 0.50,
                    },
                },
                D2D1_GRADIENT_STOP {
                    position: 0.5,
                    color: D2D1_COLOR_F {
                        r: 1.0,
                        g: 1.0,
                        b: 1.0,
                        a: 0.86,
                    },
                },
                D2D1_GRADIENT_STOP {
                    position: 1.0,
                    color: D2D1_COLOR_F {
                        r: 1.0,
                        g: 1.0,
                        b: 1.0,
                        a: 0.50,
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
                        a: 0.24,
                    },
                },
                D2D1_GRADIENT_STOP {
                    position: 0.5,
                    color: D2D1_COLOR_F {
                        r: 1.0,
                        g: 1.0,
                        b: 1.0,
                        a: 0.54,
                    },
                },
                D2D1_GRADIENT_STOP {
                    position: 1.0,
                    color: D2D1_COLOR_F {
                        r: 1.0,
                        g: 1.0,
                        b: 1.0,
                        a: 0.24,
                    },
                },
            ]
        };
        if let Ok(sc) = entry.rt.CreateGradientStopCollection(
            &edge_stops,
            D2D1_GAMMA_2_2,
            D2D1_EXTEND_MODE_CLAMP,
        ) {
            let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                startPoint: D2D_POINT_2F { x: 0.0, y: edge_y },
                endPoint: D2D_POINT_2F { x: wf, y: edge_y },
            };
            if let Ok(eb) = entry.rt.CreateLinearGradientBrush(&props, None, &sc) {
                let stroke_w = if light_glass { 1.0 * s } else { 1.15 * s };
                entry.rt.DrawLine(
                    D2D_POINT_2F { x: 0.0, y: edge_y },
                    D2D_POINT_2F { x: wf, y: edge_y },
                    &eb,
                    stroke_w,
                    None,
                );
            }
        }

        entry.rt.EndDraw(None, None)?;
    }

    entry
        .surface
        .commit(entry.glass_hwnd, tb_rect.left, tb_rect.top);
    Ok(())
}
