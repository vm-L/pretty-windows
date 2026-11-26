//! Click-through Liquid Glass alignment guide overlay displayed during DeskBox magnetic snap.

use super::snap::SnapGuideLine;
use crate::color::Rgb;
use crate::ui::glass::{create_dc_render_target, d2d_color_rgb, LayeredSurface};
use std::mem::size_of;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{D2DERR_RECREATE_TARGET, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::{D2D1_COLOR_F, D2D_POINT_2F};
use windows::Win32::Graphics::Direct2D::{ID2D1DCRenderTarget, ID2D1Factory1};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, RegisterClassExW, SetWindowPos, ShowWindow,
    HTTRANSPARENT, HWND_TOPMOST, SWP_NOACTIVATE, SWP_SHOWWINDOW, SW_HIDE, WM_NCHITTEST,
    WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_EX_TRANSPARENT, WS_POPUP,
};

const SNAP_OVERLAY_CLASS: PCWSTR = w!("PrettyWindowsDeskBoxSnapOverlay");
static SNAP_CLASS_ONCE: std::sync::Once = std::sync::Once::new();

#[derive(Clone, PartialEq, Eq)]
struct OverlayCacheKey {
    mon_rect: [i32; 4],
    dpi: u32,
    guides: Vec<SnapGuideLine>,
    accent: (u8, u8, u8),
    light_glass: bool,
}

pub struct SnapGuideOverlay {
    hwnd: HWND,
    d2d_factory: ID2D1Factory1,
    rt: ID2D1DCRenderTarget,
    surface: LayeredSurface,
    visible: bool,
    last_key: Option<OverlayCacheKey>,
}

impl SnapGuideOverlay {
    pub fn new(d2d_factory: ID2D1Factory1) -> windows::core::Result<Self> {
        unsafe {
            let hinst = GetModuleHandleW(None)?;
            SNAP_CLASS_ONCE.call_once(|| {
                let wc = WNDCLASSEXW {
                    cbSize: size_of::<WNDCLASSEXW>() as u32,
                    lpfnWndProc: Some(snap_overlay_wnd_proc),
                    hInstance: hinst.into(),
                    lpszClassName: SNAP_OVERLAY_CLASS,
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
                SNAP_OVERLAY_CLASS,
                w!("DeskBox Snap Guides"),
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

            let rt = create_dc_render_target(&d2d_factory)?;
            let surface = LayeredSurface::new()?;
            Ok(Self {
                hwnd,
                d2d_factory,
                rt,
                surface,
                visible: false,
                last_key: None,
            })
        }
    }

    pub fn hide(&mut self) {
        if self.visible && !self.hwnd.is_invalid() {
            unsafe {
                let _ = ShowWindow(self.hwnd, SW_HIDE);
            }
            self.visible = false;
        }
        self.last_key = None;
        // Release full-monitor DIB memory (e.g. ~33 MB at 4K) while no snap guides are shown.
        self.surface.release_bitmap();
    }

    pub fn show_guides(
        &mut self,
        mon_rect: RECT,
        dpi: u32,
        guides: &[SnapGuideLine],
        accent: Rgb,
        light_glass: bool,
    ) {
        if guides.is_empty() {
            self.hide();
            return;
        }
        let key = OverlayCacheKey {
            mon_rect: [mon_rect.left, mon_rect.top, mon_rect.right, mon_rect.bottom],
            dpi,
            guides: guides.to_vec(),
            accent: (accent.r, accent.g, accent.b),
            light_glass,
        };
        if self.visible && self.last_key.as_ref() == Some(&key) {
            return;
        }
        let w = (mon_rect.right - mon_rect.left).max(1);
        let h = (mon_rect.bottom - mon_rect.top).max(1);
        if self.surface.ensure_size(w, h).is_err() {
            return;
        }

        if let Err(e) = self.draw_guides(mon_rect, dpi, guides, accent, light_glass) {
            if e.code() == D2DERR_RECREATE_TARGET {
                if let Ok(new_rt) = create_dc_render_target(&self.d2d_factory) {
                    self.rt = new_rt;
                    let _ = self.draw_guides(mon_rect, dpi, guides, accent, light_glass);
                }
            }
        }

        self.surface.commit(self.hwnd, mon_rect.left, mon_rect.top);
        unsafe {
            let mut flags = SWP_NOACTIVATE;
            if !self.visible {
                flags |= SWP_SHOWWINDOW;
            }
            let _ = SetWindowPos(
                self.hwnd,
                HWND_TOPMOST,
                mon_rect.left,
                mon_rect.top,
                w,
                h,
                flags,
            );
        }
        self.visible = true;
        self.last_key = Some(key);
    }

    fn draw_guides(
        &mut self,
        mon_rect: RECT,
        dpi: u32,
        guides: &[SnapGuideLine],
        accent: Rgb,
        light_glass: bool,
    ) -> windows::core::Result<()> {
        let s = (dpi.max(96) as f32) / 96.0;
        let bind_rect = RECT {
            left: 0,
            top: 0,
            right: self.surface.width(),
            bottom: self.surface.height(),
        };
        let glow_col = d2d_color_rgb(accent, if light_glass { 0.22 } else { 0.28 });
        let core_col = d2d_color_rgb(accent, if light_glass { 0.78 } else { 0.88 });

        unsafe {
            self.rt.BindDC(self.surface.dc(), &bind_rect)?;
            self.rt.BeginDraw();
            self.rt.Clear(Some(&D2D1_COLOR_F {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.0,
            }));

            let glow_brush = self.rt.CreateSolidColorBrush(&glow_col, None).ok();
            let core_brush = self.rt.CreateSolidColorBrush(&core_col, None).ok();

            for g in guides {
                let (p0, p1) = if g.is_vertical {
                    let x = (g.pos - mon_rect.left) as f32 + 0.5;
                    (
                        D2D_POINT_2F {
                            x,
                            y: (g.start - mon_rect.top) as f32,
                        },
                        D2D_POINT_2F {
                            x,
                            y: (g.end - mon_rect.top) as f32,
                        },
                    )
                } else {
                    let y = (g.pos - mon_rect.top) as f32 + 0.5;
                    (
                        D2D_POINT_2F {
                            x: (g.start - mon_rect.left) as f32,
                            y,
                        },
                        D2D_POINT_2F {
                            x: (g.end - mon_rect.left) as f32,
                            y,
                        },
                    )
                };

                if let Some(ref gb) = glow_brush {
                    self.rt.DrawLine(p0, p1, gb, 3.5 * s, None);
                }
                if let Some(ref cb) = core_brush {
                    self.rt.DrawLine(p0, p1, cb, 1.25 * s, None);
                }
            }

            self.rt.EndDraw(None, None)
        }
    }
}

impl Drop for SnapGuideOverlay {
    fn drop(&mut self) {
        self.hide();
        unsafe {
            if !self.hwnd.is_invalid() {
                let _ = DestroyWindow(self.hwnd);
            }
        }
    }
}

unsafe extern "system" fn snap_overlay_wnd_proc(
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
