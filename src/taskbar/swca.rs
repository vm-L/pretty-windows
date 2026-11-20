use super::config::TaskbarStyle;
#[cfg_attr(not(windows), allow(unused_imports))]
pub use crate::ui::generate_acrylic_noise_bgra;

/// Decides which behind-taskbar backdrop layers are active for a style and OS transparency state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BackdropLayerPlan {
    pub use_gpu_blur: bool,
    pub use_layered_surface: bool,
}

/// Live GPU Gaussian blur sigma (DIP) and optical saturation recipe per style & theme.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GpuBackdropRecipe {
    pub blur_sigma_dip: f32,
    pub saturation: f32,
}

/// Determines which behind-taskbar layers to activate based on style and Windows `EnableTransparency`.
/// When Transparency Effects is enabled, `blur_hwnd` runs live GPU blur & saturation via
/// `Windows.UI.Composition` while `glass_hwnd` renders the calibrated Direct2D material surface.
#[cfg_attr(not(windows), allow(dead_code))]
pub const fn backdrop_layer_plan(
    style: TaskbarStyle,
    transparency_enabled: bool,
) -> BackdropLayerPlan {
    match style {
        TaskbarStyle::Default | TaskbarStyle::Transparent => BackdropLayerPlan {
            use_gpu_blur: false,
            use_layered_surface: false,
        },
        TaskbarStyle::Blur | TaskbarStyle::Acrylic | TaskbarStyle::LiquidGlass => {
            BackdropLayerPlan {
                use_gpu_blur: transparency_enabled,
                use_layered_surface: true,
            }
        }
    }
}

/// Determines which behind-overflow-flyout layers to activate for each overall style.
/// Unlike the edge-to-edge taskbar, `Transparent` keeps a lightweight clear rounded glass layer
/// and clean contour stroke so system tray icons remain crisp on complex wallpapers.
#[cfg_attr(not(windows), allow(dead_code))]
pub const fn overflow_backdrop_layer_plan(
    style: TaskbarStyle,
    transparency_enabled: bool,
) -> BackdropLayerPlan {
    match style {
        TaskbarStyle::Default => BackdropLayerPlan {
            use_gpu_blur: false,
            use_layered_surface: false,
        },
        TaskbarStyle::Transparent => BackdropLayerPlan {
            use_gpu_blur: false,
            use_layered_surface: true,
        },
        TaskbarStyle::Blur | TaskbarStyle::Acrylic | TaskbarStyle::LiquidGlass => {
            BackdropLayerPlan {
                use_gpu_blur: transparency_enabled,
                use_layered_surface: true,
            }
        }
    }
}

/// Outer transparent Win11 ThemeShadow margin around `TopLevelWindowForOverflowXamlIsland` at 100% DPI.
#[cfg_attr(not(windows), allow(dead_code))]
pub const OVERFLOW_XAML_ISLAND_MARGIN_DIP: f32 = 13.0;

/// Soft ambient shadow outer padding (DIP at 96 DPI) reserved around standalone OSD glass cards.
#[cfg_attr(not(windows), allow(dead_code))]
pub const OSD_SHADOW_PAD_DIP: f32 = 10.0;

/// Soft ambient shadow outer padding (DIP at 96 DPI) reserved around standalone ToolTip glass cards.
#[cfg_attr(not(windows), allow(dead_code))]
pub const TOOLTIP_SHADOW_PAD_DIP: f32 = 6.0;

/// Expands a card screen rect evenly on all four sides by `shadow_pad_dip` (scaled by `dpi`),
/// returning `(expanded_rect, pad_px)`. Non-positive padding leaves `card_rect` unchanged.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn expand_card_rect_for_shadow(
    card_rect: [i32; 4],
    dpi: u32,
    shadow_pad_dip: f32,
) -> ([i32; 4], i32) {
    if !shadow_pad_dip.is_finite() || shadow_pad_dip <= 0.0 {
        return (card_rect, 0);
    }
    let scale = (dpi.max(96) as f32) / 96.0;
    let pad = (shadow_pad_dip * scale).round() as i32;
    if pad <= 0 {
        return (card_rect, 0);
    }
    (
        [
            card_rect[0] - pad,
            card_rect[1] - pad,
            card_rect[2] + pad,
            card_rect[3] + pad,
        ],
        pad,
    )
}

/// Trims the outer ThemeShadow margin from `TopLevelWindowForOverflowXamlIsland` rects so the
/// behind-flyout glass/blur card hugs the visible overflow icon card edge-to-edge.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn inset_overflow_card_rect(
    win_rect: [i32; 4],
    dpi: u32,
    is_xaml_island: bool,
) -> [i32; 4] {
    if !is_xaml_island {
        return win_rect;
    }
    let scale = (dpi.max(96) as f32) / 96.0;
    let inset = (OVERFLOW_XAML_ISLAND_MARGIN_DIP * scale).round() as i32;
    let min_dim = (48.0 * scale).round() as i32;
    let w = win_rect[2] - win_rect[0];
    let h = win_rect[3] - win_rect[1];
    if w - inset * 2 < min_dim || h - inset * 2 < min_dim {
        return win_rect;
    }
    [
        win_rect[0] + inset,
        win_rect[1] + inset,
        win_rect[2] - inset,
        win_rect[3] - inset,
    ]
}

/// Maps a XAML element's DIP bounding box `[left, top, right, bottom]` inside a Win32 island
/// host window into clamped physical screen coordinates, rejecting degenerate or non-finite bounds.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn map_xaml_card_rect_to_screen_with_min(
    host_rect: [i32; 4],
    dpi: u32,
    card_dip: [f32; 4],
    min_w_dip: f32,
    min_h_dip: f32,
) -> Option<[i32; 4]> {
    if card_dip.iter().any(|v| !v.is_finite()) {
        return None;
    }
    let scale = (dpi.max(96) as f32) / 96.0;
    let left =
        (host_rect[0] + (card_dip[0] * scale).round() as i32).clamp(host_rect[0], host_rect[2]);
    let top =
        (host_rect[1] + (card_dip[1] * scale).round() as i32).clamp(host_rect[1], host_rect[3]);
    let right =
        (host_rect[0] + (card_dip[2] * scale).round() as i32).clamp(host_rect[0], host_rect[2]);
    let bottom =
        (host_rect[1] + (card_dip[3] * scale).round() as i32).clamp(host_rect[1], host_rect[3]);
    let min_w = (min_w_dip * scale).round() as i32;
    let min_h = (min_h_dip * scale).round() as i32;
    if right - left < min_w || bottom - top < min_h {
        return None;
    }
    Some([left, top, right, bottom])
}

#[cfg_attr(not(windows), allow(dead_code))]
pub fn map_xaml_card_rect_to_screen(
    host_rect: [i32; 4],
    dpi: u32,
    card_dip: [f32; 4],
) -> Option<[i32; 4]> {
    map_xaml_card_rect_to_screen_with_min(host_rect, dpi, card_dip, 80.0, 60.0)
}

/// Maps `ToolTip > ContentPresenter#LayoutRoot` DIP bounds inside `Xaml_WindowedPopupClass` to
/// clamped screen pixels, preferring exact Island-origin mapping and popup-local coordinates with
/// downward `ThemeShadow`-weighted fallback while rejecting mismatched/oversized popups (`MenuFlyout`).
#[cfg_attr(not(windows), allow(dead_code))]
pub fn map_tooltip_card_rect_to_screen(
    host_rect: [i32; 4],
    dpi: u32,
    card_dip: [f32; 4],
    island_origins: &[[i32; 2]],
) -> Option<[i32; 4]> {
    if card_dip.iter().any(|v| !v.is_finite()) {
        return None;
    }
    let rw = card_dip[2] - card_dip[0];
    let rh = card_dip[3] - card_dip[1];
    if rw < 20.0 || rh < 16.0 || rw > 680.0 || rh > 420.0 {
        return None;
    }
    let scale = (dpi.max(96) as f32) / 96.0;
    let host_w = host_rect[2] - host_rect[0];
    let host_h = host_rect[3] - host_rect[1];
    let min_w = (20.0 * scale).round() as i32;
    let min_h = (16.0 * scale).round() as i32;
    if host_w < min_w || host_h < min_h {
        return None;
    }
    let w_px = (rw * scale).round() as i32;
    let h_px = (rh * scale).round() as i32;
    let tol = (4.0 * scale).round().max(4.0) as i32;
    let max_pad = (48.0 * scale).round() as i32;

    if w_px > host_w + tol * 2
        || h_px > host_h + tol * 2
        || host_w > w_px + max_pad
        || host_h > h_px + max_pad
    {
        return None;
    }

    // Branch 1: Island-relative TransformToVisual(null) coordinates (exact pixel placement).
    for &[ox, oy] in island_origins {
        if ox == host_rect[0] && oy == host_rect[1] {
            continue;
        }
        let l = ox + (card_dip[0] * scale).round() as i32;
        let t = oy + (card_dip[1] * scale).round() as i32;
        let r = ox + (card_dip[2] * scale).round() as i32;
        let b = oy + (card_dip[3] * scale).round() as i32;
        if l >= host_rect[0] - tol
            && t >= host_rect[1] - tol
            && r <= host_rect[2] + tol
            && b <= host_rect[3] + tol
        {
            let cl = l.clamp(host_rect[0], host_rect[2]);
            let ct = t.clamp(host_rect[1], host_rect[3]);
            let cr = r.clamp(host_rect[0], host_rect[2]);
            let cb = b.clamp(host_rect[1], host_rect[3]);
            if cr - cl >= min_w && cb - ct >= min_h {
                return Some([cl, ct, cr, cb]);
            }
        }
    }

    // Branch 2: Popup-local coordinates inside `Xaml_WindowedPopupClass`.
    if card_dip[0] >= -2.0
        && card_dip[1] >= -2.0
        && (card_dip[2] * scale).round() as i32 <= host_w + tol
        && (card_dip[3] * scale).round() as i32 <= host_h + tol
    {
        return map_xaml_card_rect_to_screen_with_min(host_rect, dpi, card_dip, 20.0, 16.0);
    }

    // Branch 3: Fallback when owning Island origin is unavailable (account for downward ThemeShadow pad).
    let cw = w_px.min(host_w);
    let ch = h_px.min(host_h);
    let dx = (host_w - cw) / 2;
    let dy = ((((host_h - ch) as f32) * 0.35).round() as i32).clamp(0, host_h - ch);
    Some([
        host_rect[0] + dx,
        host_rect[1] + dy,
        host_rect[0] + dx + cw,
        host_rect[1] + dy + ch,
    ])
}

/// Calibrated translucent title-card fill and hairline border (`0xAARRGGBB`) for
/// `Alt + Tab` window items (`Border#BackgroundBorder`), matching overall styles & theme.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AltTabItemCardColors {
    /// Packed ARGB (`0xAARRGGBB`) for `Border#BackgroundBorder.Background`.
    pub bg_argb: u32,
    /// Packed ARGB (`0xAARRGGBB`) for `Border#BackgroundBorder.BorderBrush`.
    pub border_argb: u32,
}

#[cfg_attr(not(windows), allow(dead_code))]
pub const fn alt_tab_item_card_colors(
    style: TaskbarStyle,
    light_glass: bool,
) -> Option<AltTabItemCardColors> {
    match (style, light_glass) {
        (TaskbarStyle::Default, _) => None,
        (TaskbarStyle::Transparent, false) => Some(AltTabItemCardColors {
            bg_argb: 0x30_181B22,
            border_argb: 0x30_FFFFFF,
        }),
        (TaskbarStyle::Blur, false) => Some(AltTabItemCardColors {
            bg_argb: 0x3E_1A1C22,
            border_argb: 0x34_FFFFFF,
        }),
        (TaskbarStyle::Acrylic, false) => Some(AltTabItemCardColors {
            bg_argb: 0x46_181A20,
            border_argb: 0x38_FFFFFF,
        }),
        (TaskbarStyle::LiquidGlass, false) => Some(AltTabItemCardColors {
            bg_argb: 0x38_1A1D24,
            border_argb: 0x42_FFFFFF,
        }),
        (TaskbarStyle::Transparent, true) => Some(AltTabItemCardColors {
            bg_argb: 0x48_FAFBFD,
            border_argb: 0x22_1A1D24,
        }),
        (TaskbarStyle::Blur, true) => Some(AltTabItemCardColors {
            bg_argb: 0x5A_F8FAFC,
            border_argb: 0x22_1A1D24,
        }),
        (TaskbarStyle::Acrylic, true) => Some(AltTabItemCardColors {
            bg_argb: 0x64_F6F8FB,
            border_argb: 0x26_1A1D24,
        }),
        (TaskbarStyle::LiquidGlass, true) => Some(AltTabItemCardColors {
            bg_argb: 0x52_FAFCFF,
            border_argb: 0x20_1E2430,
        }),
    }
}

/// 硬件 OSD 胶囊（音量/亮度等 `band=18` `Grid#ConfirmatorMainGrid`）在全屏独占或外部置顶窗口
/// 遮挡场景下无法依赖 `band=1` 背板时，直接在 `band=18` 内应用的半透明质感背景与边框色彩（AARRGGBB）。
#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OsdFallbackCardColors {
    pub bg_argb: u32,
    pub border_argb: u32,
}

/// 根据当前任务栏主题风格与深浅色返回硬件 OSD 内联半透明玻璃配色（`Default` 返回 `None`）。
#[cfg_attr(not(windows), allow(dead_code))]
pub const fn osd_fallback_card_colors(
    style: TaskbarStyle,
    light_glass: bool,
) -> Option<OsdFallbackCardColors> {
    match (style, light_glass) {
        (TaskbarStyle::Default, _) => None,
        (TaskbarStyle::Transparent, false) => Some(OsdFallbackCardColors {
            bg_argb: 0x26_12151D,
            border_argb: 0x38_FFFFFF,
        }),
        (TaskbarStyle::Blur, false) => Some(OsdFallbackCardColors {
            bg_argb: 0x3A_161922,
            border_argb: 0x42_FFFFFF,
        }),
        (TaskbarStyle::Acrylic, false) => Some(OsdFallbackCardColors {
            bg_argb: 0x4C_181B24,
            border_argb: 0x48_FFFFFF,
        }),
        (TaskbarStyle::LiquidGlass, false) => Some(OsdFallbackCardColors {
            bg_argb: 0x3E_141822,
            border_argb: 0x52_FFFFFF,
        }),
        (TaskbarStyle::Transparent, true) => Some(OsdFallbackCardColors {
            bg_argb: 0x42_FAFBFD,
            border_argb: 0x24_1A1D24,
        }),
        (TaskbarStyle::Blur, true) => Some(OsdFallbackCardColors {
            bg_argb: 0x56_F8FAFC,
            border_argb: 0x28_1A1D24,
        }),
        (TaskbarStyle::Acrylic, true) => Some(OsdFallbackCardColors {
            bg_argb: 0x68_F6F8FB,
            border_argb: 0x2C_1A1D24,
        }),
        (TaskbarStyle::LiquidGlass, true) => Some(OsdFallbackCardColors {
            bg_argb: 0x52_FAFCFF,
            border_argb: 0x2A_1E2430,
        }),
    }
}

/// 任务栏与托盘区悬浮提示胶囊（`ToolTip > ContentPresenter#LayoutRoot`）在独占全屏或上层置顶窗口
/// 遮挡场景下切换为原地半透明内联玻璃时使用的背景与边框色彩（AARRGGBB）。
pub type TooltipFallbackCardColors = OsdFallbackCardColors;

/// 根据当前任务栏主题风格与深浅色返回悬浮提示内联半透明玻璃配色（`Default` 返回 `None`）。
#[cfg_attr(not(windows), allow(dead_code))]
pub const fn tooltip_fallback_card_colors(
    style: TaskbarStyle,
    light_glass: bool,
) -> Option<TooltipFallbackCardColors> {
    match (style, light_glass) {
        (TaskbarStyle::Default, _) => None,
        (TaskbarStyle::Transparent, false) => Some(TooltipFallbackCardColors {
            bg_argb: 0x36_12151D,
            border_argb: 0x3C_FFFFFF,
        }),
        (TaskbarStyle::Blur, false) => Some(TooltipFallbackCardColors {
            bg_argb: 0x42_161922,
            border_argb: 0x44_FFFFFF,
        }),
        (TaskbarStyle::Acrylic, false) => Some(TooltipFallbackCardColors {
            bg_argb: 0x52_181B24,
            border_argb: 0x4A_FFFFFF,
        }),
        (TaskbarStyle::LiquidGlass, false) => Some(TooltipFallbackCardColors {
            bg_argb: 0x44_141822,
            border_argb: 0x54_FFFFFF,
        }),
        (TaskbarStyle::Transparent, true) => Some(TooltipFallbackCardColors {
            bg_argb: 0x48_FAFBFD,
            border_argb: 0x26_1A1D24,
        }),
        (TaskbarStyle::Blur, true) => Some(TooltipFallbackCardColors {
            bg_argb: 0x5A_F8FAFC,
            border_argb: 0x2A_1A1D24,
        }),
        (TaskbarStyle::Acrylic, true) => Some(TooltipFallbackCardColors {
            bg_argb: 0x6C_F6F8FB,
            border_argb: 0x2E_1A1D24,
        }),
        (TaskbarStyle::LiquidGlass, true) => Some(TooltipFallbackCardColors {
            bg_argb: 0x56_FAFCFF,
            border_argb: 0x2C_1E2430,
        }),
    }
}

/// Maps `TaskbarStyle` + system light/dark theme to the live `Windows.UI.Composition` effect recipe.
/// `Default` and `Transparent` need no behind-taskbar GPU backdrop (`None`).
#[cfg_attr(not(windows), allow(dead_code))]
pub const fn gpu_backdrop_recipe(
    style: TaskbarStyle,
    light_taskbar: bool,
) -> Option<GpuBackdropRecipe> {
    match style {
        TaskbarStyle::Default | TaskbarStyle::Transparent => None,
        TaskbarStyle::Blur => Some(GpuBackdropRecipe {
            blur_sigma_dip: 18.0,
            saturation: 1.08,
        }),
        TaskbarStyle::Acrylic => Some(GpuBackdropRecipe {
            blur_sigma_dip: 30.0,
            saturation: 1.16,
        }),
        TaskbarStyle::LiquidGlass => Some(GpuBackdropRecipe {
            blur_sigma_dip: 24.0,
            saturation: if light_taskbar { 1.22 } else { 1.34 },
        }),
    }
}

#[cfg(windows)]
mod win {
    use windows::core::w;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        FindWindowExW, FindWindowW, GetWindowThreadProcessId,
    };

    pub fn enumerate_taskbar_windows() -> Vec<HWND> {
        let mut result = Vec::new();
        unsafe {
            if let Ok(primary) = FindWindowW(w!("Shell_TrayWnd"), None) {
                if !primary.is_invalid() {
                    result.push(primary);
                }
            }
            let mut prev = HWND::default();
            loop {
                let Ok(sec) = FindWindowExW(None, prev, w!("Shell_SecondaryTrayWnd"), None) else {
                    break;
                };
                if sec.is_invalid() {
                    break;
                }
                if !result.contains(&sec) {
                    result.push(sec);
                }
                prev = sec;
            }
        }
        result
    }

    pub fn explorer_pid_from_taskbars(hwnds: &[HWND]) -> Option<u32> {
        for &hwnd in hwnds {
            if hwnd.is_invalid() {
                continue;
            }
            let mut pid = 0u32;
            unsafe {
                let _ = GetWindowThreadProcessId(hwnd, Some(&mut pid));
            }
            if pid != 0 {
                return Some(pid);
            }
        }
        None
    }

    pub fn taskbar_thread_ids(hwnds: &[HWND]) -> Vec<u32> {
        let mut tids = Vec::new();
        for &hwnd in hwnds {
            if hwnd.is_invalid() {
                continue;
            }
            let mut pid = 0u32;
            let tid = unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
            if tid != 0 && !tids.contains(&tid) {
                tids.push(tid);
            }
        }
        tids
    }
}

#[cfg(windows)]
pub use win::{enumerate_taskbar_windows, explorer_pid_from_taskbars, taskbar_thread_ids};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gpu_recipes_separate_blur_acrylic_and_liquid_glass() {
        for light in [false, true] {
            assert_eq!(gpu_backdrop_recipe(TaskbarStyle::Default, light), None);
            assert_eq!(gpu_backdrop_recipe(TaskbarStyle::Transparent, light), None);

            let blur = gpu_backdrop_recipe(TaskbarStyle::Blur, light)
                .expect("Blur must produce GPU recipe");
            let acrylic = gpu_backdrop_recipe(TaskbarStyle::Acrylic, light)
                .expect("Acrylic must produce GPU recipe");
            let glass = gpu_backdrop_recipe(TaskbarStyle::LiquidGlass, light)
                .expect("LiquidGlass must produce GPU recipe");

            assert!(blur.blur_sigma_dip >= 16.0);
            assert!(glass.blur_sigma_dip >= 20.0);
            assert!(blur.blur_sigma_dip < glass.blur_sigma_dip);
            assert!(glass.blur_sigma_dip < acrylic.blur_sigma_dip);
            assert!(blur.saturation >= 1.0);
            assert!(acrylic.saturation > blur.saturation);
            assert!(glass.saturation > acrylic.saturation);
        }
    }

    #[test]
    fn backdrop_layer_plan_switches_cleanly_with_system_transparency() {
        for trans in [false, true] {
            assert_eq!(
                backdrop_layer_plan(TaskbarStyle::Default, trans),
                BackdropLayerPlan {
                    use_gpu_blur: false,
                    use_layered_surface: false,
                }
            );
            assert_eq!(
                backdrop_layer_plan(TaskbarStyle::Transparent, trans),
                BackdropLayerPlan {
                    use_gpu_blur: false,
                    use_layered_surface: false,
                }
            );
        }

        for style in [
            TaskbarStyle::Blur,
            TaskbarStyle::Acrylic,
            TaskbarStyle::LiquidGlass,
        ] {
            assert_eq!(
                backdrop_layer_plan(style, true),
                BackdropLayerPlan {
                    use_gpu_blur: true,
                    use_layered_surface: true,
                }
            );
            assert_eq!(
                backdrop_layer_plan(style, false),
                BackdropLayerPlan {
                    use_gpu_blur: false,
                    use_layered_surface: true,
                }
            );
        }
    }

    #[test]
    fn acrylic_noise_bgra_is_deterministic_premultiplied_and_balanced() {
        let a = generate_acrylic_noise_bgra(128, 0x51A7_C301, 11);
        let b = generate_acrylic_noise_bgra(128, 0x51A7_C301, 11);
        assert_eq!(a, b);
        assert_eq!(a.len(), 128 * 128 * 4);

        let mut bright_count = 0usize;
        let mut dark_count = 0usize;
        for px in a.chunks_exact(4) {
            let (b_ch, g_ch, r_ch, alpha) = (px[0], px[1], px[2], px[3]);
            assert!(alpha <= 11);
            assert!(b_ch <= alpha && g_ch <= alpha && r_ch <= alpha);
            assert_eq!(b_ch, g_ch);
            assert_eq!(g_ch, r_ch);
            if alpha > 0 {
                if r_ch == alpha {
                    bright_count += 1;
                } else {
                    dark_count += 1;
                }
            }
        }
        assert!(bright_count > 1000 && dark_count > 1000);
    }

    #[test]
    fn overflow_layer_plan_enables_clear_glass_for_transparent_and_gpu_blur_for_frosted_tiers() {
        for trans in [false, true] {
            assert_eq!(
                overflow_backdrop_layer_plan(TaskbarStyle::Default, trans),
                BackdropLayerPlan {
                    use_gpu_blur: false,
                    use_layered_surface: false,
                }
            );
            assert_eq!(
                overflow_backdrop_layer_plan(TaskbarStyle::Transparent, trans),
                BackdropLayerPlan {
                    use_gpu_blur: false,
                    use_layered_surface: true,
                }
            );
        }

        for style in [
            TaskbarStyle::Blur,
            TaskbarStyle::Acrylic,
            TaskbarStyle::LiquidGlass,
        ] {
            assert_eq!(
                overflow_backdrop_layer_plan(style, true),
                BackdropLayerPlan {
                    use_gpu_blur: true,
                    use_layered_surface: true,
                }
            );
            assert_eq!(
                overflow_backdrop_layer_plan(style, false),
                BackdropLayerPlan {
                    use_gpu_blur: false,
                    use_layered_surface: true,
                }
            );
        }
    }

    #[test]
    fn inset_overflow_card_rect_trims_xaml_island_shadow_margin_only() {
        // 96 DPI: 234x154 host window -> 208x128 visible card (13px margin removed on each side)
        assert_eq!(
            inset_overflow_card_rect([129, 25, 363, 179], 96, true),
            [142, 38, 350, 166]
        );
        // 150% DPI (144): inset = round(13 * 1.5) = 20px
        assert_eq!(
            inset_overflow_card_rect([100, 100, 460, 340], 144, true),
            [120, 120, 440, 320]
        );
        // Too-small degenerate rect preserves original bounds safely
        assert_eq!(inset_overflow_card_rect([0, 0, 60, 60], 96, true), [0, 0, 60, 60]);
        // Legacy NotifyIconOverflowWindow keeps full rect untouched
        assert_eq!(
            inset_overflow_card_rect([129, 25, 363, 179], 96, false),
            [129, 25, 363, 179]
        );
    }

    #[test]
    fn map_xaml_card_rect_to_screen_scales_clamps_and_rejects_invalid_bounds() {
        assert_eq!(
            map_xaml_card_rect_to_screen([0, 0, 1920, 1032], 96, [410.0, 260.0, 1510.0, 772.0]),
            Some([410, 260, 1510, 772])
        );
        // 150% DPI (144) on secondary monitor starting at x=1920
        assert_eq!(
            map_xaml_card_rect_to_screen(
                [1920, 0, 3840, 1032],
                144,
                [100.0, 80.0, 500.0, 300.0]
            ),
            Some([2070, 120, 2670, 450])
        );
        // Clamps within host window while remaining large enough
        assert_eq!(
            map_xaml_card_rect_to_screen([0, 0, 1000, 600], 96, [-20.0, 100.0, 1040.0, 500.0]),
            Some([0, 100, 1000, 500])
        );
        // Rejects too-small / inverted / non-finite bounds
        assert_eq!(
            map_xaml_card_rect_to_screen([0, 0, 1920, 1032], 96, [100.0, 100.0, 150.0, 140.0]),
            None
        );
        assert_eq!(
            map_xaml_card_rect_to_screen([0, 0, 1920, 1032], 96, [f32::NAN, 100.0, 500.0, 400.0]),
            None
        );
    }

    #[test]
    fn map_xaml_card_rect_with_min_accepts_compact_flyout() {
        // Single-row taskbar overflow / compact preview flyout (~52 DIP tall) passes hover min (48x36)
        assert_eq!(
            map_xaml_card_rect_to_screen_with_min(
                [400, 960, 660, 1032],
                96,
                [8.0, 8.0, 240.0, 60.0],
                48.0,
                36.0
            ),
            Some([408, 968, 640, 1020])
        );
        // Tiny 20x20 artifact is rejected
        assert_eq!(
            map_xaml_card_rect_to_screen_with_min(
                [400, 960, 660, 1032],
                96,
                [8.0, 8.0, 28.0, 28.0],
                48.0,
                36.0
            ),
            None
        );
    }

    #[test]
    fn expand_card_rect_for_shadow_scales_cleanly_and_handles_zero_pad() {
        assert_eq!(
            expand_card_rect_for_shadow([875, 971, 1044, 1019], 96, 10.0),
            ([865, 961, 1054, 1029], 10)
        );
        // 150% DPI -> 15px outer ring
        assert_eq!(
            expand_card_rect_for_shadow([100, 200, 354, 272], 144, 10.0),
            ([85, 185, 369, 287], 15)
        );
        // Zero / non-positive / NaN shadow padding returns card unchanged
        assert_eq!(
            expand_card_rect_for_shadow([10, 20, 140, 68], 96, 0.0),
            ([10, 20, 140, 68], 0)
        );
        assert_eq!(
            expand_card_rect_for_shadow([10, 20, 140, 68], 96, f32::NAN),
            ([10, 20, 140, 68], 0)
        );
        // Compact OSD card (129x48 DIP) maps cleanly with 80x32 minimum
        assert_eq!(
            map_xaml_card_rect_to_screen_with_min(
                [875, 971, 1044, 1032],
                96,
                [10.0, 0.0, 159.0, 48.0],
                80.0,
                32.0
            ),
            Some([885, 971, 1034, 1019])
        );
    }

    #[test]
    fn alt_tab_item_card_colors_scale_across_styles_and_themes() {
        assert_eq!(alt_tab_item_card_colors(TaskbarStyle::Default, false), None);
        assert_eq!(alt_tab_item_card_colors(TaskbarStyle::Default, true), None);

        let dark_styles = [
            TaskbarStyle::Transparent,
            TaskbarStyle::Blur,
            TaskbarStyle::Acrylic,
            TaskbarStyle::LiquidGlass,
        ];
        for style in dark_styles {
            let dark = alt_tab_item_card_colors(style, false).expect("dark palette");
            let light = alt_tab_item_card_colors(style, true).expect("light palette");
            let dark_bg_alpha = (dark.bg_argb >> 24) & 0xFF;
            let light_bg_alpha = (light.bg_argb >> 24) & 0xFF;
            assert!(
                (0x28..=0x50).contains(&dark_bg_alpha),
                "dark alpha {dark_bg_alpha:#x} out of lightweight glass range"
            );
            assert!(
                (0x40..=0x70).contains(&light_bg_alpha),
                "light alpha {light_bg_alpha:#x} out of lightweight frost range"
            );
            // Dark theme uses white glass edge; light theme uses subtle dark ink edge.
            assert_eq!(dark.border_argb & 0x00FF_FFFF, 0x00FF_FFFF);
            assert_ne!(light.border_argb & 0x00FF_FFFF, 0x00FF_FFFF);
            assert!((dark.bg_argb & 0xFF) < 0x40);
            assert!((light.bg_argb & 0xFF) > 0xE0);
        }
    }

    #[test]
    fn osd_fallback_card_colors_scale_across_styles_and_themes() {
        assert_eq!(osd_fallback_card_colors(TaskbarStyle::Default, false), None);
        assert_eq!(osd_fallback_card_colors(TaskbarStyle::Default, true), None);

        for style in [
            TaskbarStyle::Transparent,
            TaskbarStyle::Blur,
            TaskbarStyle::Acrylic,
            TaskbarStyle::LiquidGlass,
        ] {
            let dark =
                osd_fallback_card_colors(style, false).expect("non-default dark osd colors");
            let light =
                osd_fallback_card_colors(style, true).expect("non-default light osd colors");
            let dark_alpha = dark.bg_argb >> 24;
            let light_alpha = light.bg_argb >> 24;
            assert!((0x20..=0x58).contains(&dark_alpha));
            assert!((0x38..=0x78).contains(&light_alpha));
            assert_eq!(dark.border_argb & 0x00FF_FFFF, 0x00FF_FFFF);
            assert_ne!(light.border_argb & 0x00FF_FFFF, 0x00FF_FFFF);
            assert!((dark.bg_argb & 0xFF) < 0x40);
            assert!((light.bg_argb & 0xFF) > 0xE0);
        }

        let t = osd_fallback_card_colors(TaskbarStyle::Transparent, false).unwrap();
        let b = osd_fallback_card_colors(TaskbarStyle::Blur, false).unwrap();
        let a = osd_fallback_card_colors(TaskbarStyle::Acrylic, false).unwrap();
        assert!((t.bg_argb >> 24) < (b.bg_argb >> 24));
        assert!((b.bg_argb >> 24) < (a.bg_argb >> 24));
    }

    #[test]
    fn map_tooltip_card_rect_handles_local_island_relative_and_oversized_popup_rejection() {
        assert_eq!(
            map_tooltip_card_rect_to_screen(
                [1200, 980, 1340, 1020],
                96,
                [0.0, 0.0, 140.0, 40.0],
                &[]
            ),
            Some([1200, 980, 1340, 1020])
        );
        assert_eq!(
            map_tooltip_card_rect_to_screen(
                [1000, 900, 1210, 960],
                144,
                [0.0, 0.0, 140.0, 40.0],
                &[]
            ),
            Some([1000, 900, 1210, 960])
        );
        // Overflow island-relative negative coordinates resolve exact top/bottom placement despite asymmetric ThemeShadow pad (top_pad=8, bot_pad=16)
        assert_eq!(
            map_tooltip_card_rect_to_screen(
                [31, 9, 151, 65],
                96,
                [-12.0, -27.0, 92.0, 5.0],
                &[[51, 44]]
            ),
            Some([39, 17, 143, 49])
        );
        // Taskbar island-relative coordinates match the true taskbar origin (0, 1032) and ignore unrelated overflow origins
        assert_eq!(
            map_tooltip_card_rect_to_screen(
                [1702, 984, 1822, 1040],
                96,
                [1710.0, -40.0, 1814.0, -8.0],
                &[[51, 44], [0, 1032]]
            ),
            Some([1710, 992, 1814, 1024])
        );
        // Negative secondary monitor origin and 150% HiDPI scaling
        assert_eq!(
            map_tooltip_card_rect_to_screen(
                [-212, 968, -32, 1040],
                144,
                [1144.0, -36.0, 1248.0, -4.0],
                &[[-1920, 1032]]
            ),
            Some([-204, 978, -48, 1026])
        );
        // Unknown Island origin fallback accounts for downward ThemeShadow weighting (host_h - ch = 24 -> dy = 8, not 12)
        assert_eq!(
            map_tooltip_card_rect_to_screen(
                [1400, 980, 1528, 1036],
                96,
                [1420.0, -48.0, 1540.0, -16.0],
                &[]
            ),
            Some([1404, 988, 1524, 1020])
        );
        // Oversized unrelated Xaml_WindowedPopupClass (such as MenuFlyout) must be rejected even near an island origin
        assert_eq!(
            map_tooltip_card_rect_to_screen(
                [1000, 600, 1240, 960],
                96,
                [10.0, -40.0, 130.0, -10.0],
                &[[990, 640]]
            ),
            None
        );
        assert_eq!(
            map_tooltip_card_rect_to_screen(
                [0, 0, 200, 60],
                96,
                [f32::NAN, 0.0, 100.0, 30.0],
                &[]
            ),
            None
        );
        assert_eq!(
            map_tooltip_card_rect_to_screen([0, 0, 200, 60], 96, [0.0, 0.0, 10.0, 10.0], &[]),
            None
        );
        assert_eq!(
            map_tooltip_card_rect_to_screen([0, 0, 800, 100], 96, [0.0, 0.0, 700.0, 50.0], &[]),
            None
        );
    }

    #[test]
    fn tooltip_fallback_card_colors_keep_text_legible_across_styles_and_themes() {
        assert_eq!(tooltip_fallback_card_colors(TaskbarStyle::Default, false), None);
        assert_eq!(tooltip_fallback_card_colors(TaskbarStyle::Default, true), None);

        for style in [
            TaskbarStyle::Transparent,
            TaskbarStyle::Blur,
            TaskbarStyle::Acrylic,
            TaskbarStyle::LiquidGlass,
        ] {
            let dark =
                tooltip_fallback_card_colors(style, false).expect("non-default dark tooltip");
            let light =
                tooltip_fallback_card_colors(style, true).expect("non-default light tooltip");
            let dark_alpha = dark.bg_argb >> 24;
            let light_alpha = light.bg_argb >> 24;
            assert!((0x30..=0x60).contains(&dark_alpha));
            assert!((0x40..=0x78).contains(&light_alpha));
            assert_eq!(dark.border_argb & 0x00FF_FFFF, 0x00FF_FFFF);
            assert_ne!(light.border_argb & 0x00FF_FFFF, 0x00FF_FFFF);
            assert!((dark.bg_argb & 0xFF) < 0x40);
            assert!((light.bg_argb & 0xFF) > 0xE0);
        }

        let t = tooltip_fallback_card_colors(TaskbarStyle::Transparent, false).unwrap();
        let b = tooltip_fallback_card_colors(TaskbarStyle::Blur, false).unwrap();
        let a = tooltip_fallback_card_colors(TaskbarStyle::Acrylic, false).unwrap();
        assert!((t.bg_argb >> 24) < (b.bg_argb >> 24));
        assert!((b.bg_argb >> 24) < (a.bg_argb >> 24));
    }
}
