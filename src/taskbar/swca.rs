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
}

#[cfg(windows)]
pub use win::{enumerate_taskbar_windows, explorer_pid_from_taskbars};

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
}
