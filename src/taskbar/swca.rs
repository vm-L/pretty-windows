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

/// Soft ambient shadow outer padding (DIP at 96 DPI) reserved around standalone menu/JumpList glass cards.
#[cfg_attr(not(windows), allow(dead_code))]
pub const MENU_FLYOUT_SHADOW_PAD_DIP: f32 = 10.0;

/// Maximum number of simultaneous menu/JumpList surfaces tracked concurrently (main + 2 nested submenus).
#[cfg_attr(not(windows), allow(dead_code))]
pub const MAX_MENU_FLYOUT_SLOTS: usize = 3;

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

/// Converts a finite DIP coordinate/dimension into bounded physical pixels without `i32` overflow.
#[cfg_attr(not(windows), allow(dead_code))]
fn scale_dip_to_px_checked(dip: f32, scale: f32) -> Option<i32> {
    if !dip.is_finite() || !scale.is_finite() || scale <= 0.0 {
        return None;
    }
    let scaled = (dip * scale).round();
    if !scaled.is_finite() || scaled.abs() > 1_000_000.0 {
        return None;
    }
    Some(scaled as i32)
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
    if card_dip.iter().any(|v| !v.is_finite())
        || host_rect[0] >= host_rect[2]
        || host_rect[1] >= host_rect[3]
    {
        return None;
    }
    let scale = (dpi.max(96) as f32) / 96.0;
    let l_off = scale_dip_to_px_checked(card_dip[0], scale)?;
    let t_off = scale_dip_to_px_checked(card_dip[1], scale)?;
    let r_off = scale_dip_to_px_checked(card_dip[2], scale)?;
    let b_off = scale_dip_to_px_checked(card_dip[3], scale)?;
    let min_w = scale_dip_to_px_checked(min_w_dip, scale)?;
    let min_h = scale_dip_to_px_checked(min_h_dip, scale)?;

    let left = host_rect[0]
        .saturating_add(l_off)
        .clamp(host_rect[0], host_rect[2]);
    let top = host_rect[1]
        .saturating_add(t_off)
        .clamp(host_rect[1], host_rect[3]);
    let right = host_rect[0]
        .saturating_add(r_off)
        .clamp(host_rect[0], host_rect[2]);
    let bottom = host_rect[1]
        .saturating_add(b_off)
        .clamp(host_rect[1], host_rect[3]);
    if right.saturating_sub(left) < min_w || bottom.saturating_sub(top) < min_h {
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

#[cfg_attr(not(windows), allow(dead_code))]
#[allow(clippy::too_many_arguments)]
fn map_popup_card_rect_with_bounds_tiered(
    host_rect: [i32; 4],
    dpi: u32,
    card_dip: [f32; 4],
    island_origins: &[[i32; 2]],
    min_w_dip: f32,
    max_w_dip: f32,
    min_h_dip: f32,
    max_h_dip: f32,
    max_pad_dip: f32,
) -> Option<(u8, [i32; 4])> {
    if card_dip.iter().any(|v| !v.is_finite())
        || host_rect[0] >= host_rect[2]
        || host_rect[1] >= host_rect[3]
    {
        return None;
    }
    let rw = card_dip[2] - card_dip[0];
    let rh = card_dip[3] - card_dip[1];
    if !rw.is_finite()
        || !rh.is_finite()
        || rw < min_w_dip
        || rh < min_h_dip
        || rw > max_w_dip
        || rh > max_h_dip
    {
        return None;
    }
    let scale = (dpi.max(96) as f32) / 96.0;
    let host_w = host_rect[2].saturating_sub(host_rect[0]);
    let host_h = host_rect[3].saturating_sub(host_rect[1]);
    let min_w = scale_dip_to_px_checked(min_w_dip, scale)?;
    let min_h = scale_dip_to_px_checked(min_h_dip, scale)?;
    if host_w < min_w || host_h < min_h {
        return None;
    }
    let w_px = scale_dip_to_px_checked(rw, scale)?;
    let h_px = scale_dip_to_px_checked(rh, scale)?;
    let l0_px = scale_dip_to_px_checked(card_dip[0], scale)?;
    let t0_px = scale_dip_to_px_checked(card_dip[1], scale)?;
    let r0_px = scale_dip_to_px_checked(card_dip[2], scale)?;
    let b0_px = scale_dip_to_px_checked(card_dip[3], scale)?;
    let tol = (4.0 * scale).round().max(4.0) as i32;
    let max_pad = scale_dip_to_px_checked(max_pad_dip, scale)?;

    if w_px > host_w.saturating_add(tol * 2)
        || h_px > host_h.saturating_add(tol * 2)
        || host_w > w_px.saturating_add(max_pad)
        || host_h > h_px.saturating_add(max_pad)
    {
        return None;
    }

    // Tier 0: Island-relative TransformToVisual(null) coordinates (exact pixel placement).
    for &[ox, oy] in island_origins {
        if ox == host_rect[0] && oy == host_rect[1] {
            continue;
        }
        let l = ox.saturating_add(l0_px);
        let t = oy.saturating_add(t0_px);
        let r = ox.saturating_add(r0_px);
        let b = oy.saturating_add(b0_px);
        if l >= host_rect[0].saturating_sub(tol)
            && t >= host_rect[1].saturating_sub(tol)
            && r <= host_rect[2].saturating_add(tol)
            && b <= host_rect[3].saturating_add(tol)
        {
            let cl = l.clamp(host_rect[0], host_rect[2]);
            let ct = t.clamp(host_rect[1], host_rect[3]);
            let cr = r.clamp(host_rect[0], host_rect[2]);
            let cb = b.clamp(host_rect[1], host_rect[3]);
            if cr.saturating_sub(cl) >= min_w && cb.saturating_sub(ct) >= min_h {
                return Some((0, [cl, ct, cr, cb]));
            }
        }
    }

    // Tier 1: Popup-local coordinates inside `Xaml_WindowedPopupClass`.
    if card_dip[0] >= -2.0
        && card_dip[1] >= -2.0
        && r0_px <= host_w.saturating_add(tol)
        && b0_px <= host_h.saturating_add(tol)
    {
        return map_xaml_card_rect_to_screen_with_min(
            host_rect, dpi, card_dip, min_w_dip, min_h_dip,
        )
        .map(|rect| (1, rect));
    }

    // Tier 2: Centered fallback when owning Island origin is unavailable (account for downward ThemeShadow pad).
    let cw = w_px.min(host_w);
    let ch = h_px.min(host_h);
    let dx = (host_w - cw) / 2;
    let dy = ((((host_h - ch) as f32) * 0.35).round() as i32).clamp(0, host_h - ch);
    Some((
        2,
        [
            host_rect[0] + dx,
            host_rect[1] + dy,
            host_rect[0] + dx + cw,
            host_rect[1] + dy + ch,
        ],
    ))
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
    map_popup_card_rect_with_bounds_tiered(
        host_rect,
        dpi,
        card_dip,
        island_origins,
        20.0,
        680.0,
        16.0,
        420.0,
        48.0,
    )
    .map(|(_, rect)| rect)
}

#[cfg_attr(not(windows), allow(dead_code))]
fn map_menu_flyout_card_rect_to_screen_tiered(
    host_rect: [i32; 4],
    dpi: u32,
    card_dip: [f32; 4],
    island_origins: &[[i32; 2]],
) -> Option<(u8, [i32; 4])> {
    map_popup_card_rect_with_bounds_tiered(
        host_rect,
        dpi,
        card_dip,
        island_origins,
        64.0,
        760.0,
        28.0,
        1080.0,
        64.0,
    )
}

/// Maps `MenuFlyoutPresenter` / `JumpList` surface DIP bounds inside `Xaml_WindowedPopupClass`
/// to clamped screen pixels.
#[allow(dead_code)]
pub fn map_menu_flyout_card_rect_to_screen(
    host_rect: [i32; 4],
    dpi: u32,
    card_dip: [f32; 4],
    island_origins: &[[i32; 2]],
) -> Option<[i32; 4]> {
    map_menu_flyout_card_rect_to_screen_tiered(host_rect, dpi, card_dip, island_origins)
        .map(|(_, rect)| rect)
}

/// Matches up to `MAX_MENU_FLYOUT_SLOTS` visible `Xaml_WindowedPopupClass` host windows
/// against TAP-reported active menu/JumpList card rectangles in 1-to-1 precision-then-size order.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn match_menu_flyout_hosts_to_rects(
    hosts: &[([i32; 4], u32, u32)],
    tap_rects: &[(u32, [f32; 4])],
    island_origins_per_host: &[Vec<[i32; 2]>],
) -> Vec<(usize, [i32; 4])> {
    let mut candidates: Vec<(u8, i32, usize, usize, [i32; 4])> = Vec::new();
    for (hi, &(host_rect, dpi, host_tid)) in hosts.iter().enumerate() {
        let origins = island_origins_per_host
            .get(hi)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let scale = (dpi.max(96) as f32) / 96.0;
        let host_w = host_rect[2].saturating_sub(host_rect[0]);
        let host_h = host_rect[3].saturating_sub(host_rect[1]);
        for (ti, &(tap_tid, card_dip)) in tap_rects.iter().enumerate() {
            if tap_tid != 0 && tap_tid != host_tid {
                continue;
            }
            let Some((precision_tier, card_rect)) =
                map_menu_flyout_card_rect_to_screen_tiered(host_rect, dpi, card_dip, origins)
            else {
                continue;
            };
            let Some(card_w) = scale_dip_to_px_checked(card_dip[2] - card_dip[0], scale) else {
                continue;
            };
            let Some(card_h) = scale_dip_to_px_checked(card_dip[3] - card_dip[1], scale) else {
                continue;
            };
            let score = host_w
                .saturating_sub(card_w)
                .saturating_abs()
                .saturating_add(host_h.saturating_sub(card_h).saturating_abs());
            candidates.push((precision_tier, score, hi, ti, card_rect));
        }
    }
    candidates.sort_by_key(|&(tier, score, hi, ti, _)| (tier, score, hi, ti));

    let mut used_hosts = vec![false; hosts.len()];
    let mut used_taps = vec![false; tap_rects.len()];
    let mut matched = Vec::with_capacity(MAX_MENU_FLYOUT_SLOTS.min(hosts.len()));
    for (_, _, hi, ti, card_rect) in candidates {
        if used_hosts[hi] || used_taps[ti] {
            continue;
        }
        used_hosts[hi] = true;
        used_taps[ti] = true;
        matched.push((hi, card_rect));
        if matched.len() >= MAX_MENU_FLYOUT_SLOTS {
            break;
        }
    }
    matched
}

/// Maximum number of concurrent Tier-1 Shell flyout/card backdrops managed across
/// `ShellExperienceHost.exe` and `ShellHost.exe` (Quick Settings, Media Card,
/// Notification Center, Calendar, Floating Toasts, JumpList and cascading submenus).
#[cfg_attr(not(windows), allow(dead_code))]
pub const MAX_SHELL_CARD_SLOTS: usize = 8;

/// Computes intersection-over-union in `[0.0, 1.0]` for two screen pixel rectangles.
#[cfg_attr(not(windows), allow(dead_code))]
fn rect_iou_px(a: [i32; 4], b: [i32; 4]) -> f32 {
    let ix0 = a[0].max(b[0]);
    let iy0 = a[1].max(b[1]);
    let ix1 = a[2].min(b[2]);
    let iy1 = a[3].min(b[3]);
    let iw = (ix1 - ix0).max(0) as f64;
    let ih = (iy1 - iy0).max(0) as f64;
    let inter = iw * ih;
    if inter <= 0.0 {
        return 0.0;
    }
    let area_a = ((a[2] - a[0]).max(0) as f64) * ((a[3] - a[1]).max(0) as f64);
    let area_b = ((b[2] - b[0]).max(0) as f64) * ((b[3] - b[1]).max(0) as f64);
    let union = area_a + area_b - inter;
    if union <= 0.0 {
        0.0
    } else {
        (inter / union) as f32
    }
}

/// Matches visible Shell host windows (`CoreWindow`, `ControlCenterWindow`, and secondary
/// `Xaml_WindowedPopupClass` popups) against TAP-reported DIP card bounds, allowing multi-card
/// hosts (Notification Center + Calendar, Quick Settings + Media Card) while binding each popup
/// host 1-to-1 and deduplicating nearly identical rectangles before returning deterministic
/// `(host_hwnd_raw, screen_card_rect)` pairs.
#[cfg_attr(not(windows), allow(dead_code))]
type ShellCardCandidate = (u8, u8, i32, usize, usize, isize, [i32; 4]);

#[cfg_attr(not(windows), allow(dead_code))]
pub fn match_shell_host_cards_to_screen(
    hosts: &[([i32; 4], u32, u32, isize, bool)],
    tap_rects: &[(u32, [f32; 4])],
) -> Vec<(isize, [i32; 4])> {
    if hosts.is_empty() || tap_rects.is_empty() {
        return Vec::new();
    }

    let mut candidates: Vec<ShellCardCandidate> = Vec::new();
    for (hi, &(host_rect, dpi, host_tid, hwnd_raw, is_popup)) in hosts.iter().enumerate() {
        let scale = (dpi.max(96) as f32) / 96.0;
        let host_w = host_rect[2].saturating_sub(host_rect[0]);
        let host_h = host_rect[3].saturating_sub(host_rect[1]);
        if host_w <= 0 || host_h <= 0 {
            continue;
        }
        let tol = (4.0 * scale).round().max(4.0) as i32;
        for (ti, &(tap_tid, card_dip)) in tap_rects.iter().enumerate() {
            if tap_tid != 0 && tap_tid != host_tid {
                continue;
            }
            if card_dip.iter().any(|v| !v.is_finite()) {
                continue;
            }
            let rw = card_dip[2] - card_dip[0];
            let rh = card_dip[3] - card_dip[1];
            if !rw.is_finite() || !rh.is_finite() || rw < 64.0 || rh < 28.0 {
                continue;
            }
            let Some(w_px) = scale_dip_to_px_checked(rw, scale) else {
                continue;
            };
            let Some(h_px) = scale_dip_to_px_checked(rh, scale) else {
                continue;
            };
            if w_px > host_w.saturating_add(tol * 2) || h_px > host_h.saturating_add(tol * 2) {
                continue;
            }
            let mapped = if is_popup {
                map_menu_flyout_card_rect_to_screen_tiered(host_rect, dpi, card_dip, &[])
            } else {
                map_xaml_card_rect_to_screen_with_min(host_rect, dpi, card_dip, 64.0, 28.0)
                    .map(|rect| (0u8, rect))
            };
            let Some((precision_tier, card_rect)) = mapped else {
                continue;
            };
            let host_tier = u8::from(!is_popup);
            let score = host_w
                .saturating_sub(w_px)
                .saturating_abs()
                .saturating_add(host_h.saturating_sub(h_px).saturating_abs());
            candidates.push((host_tier, precision_tier, score, hi, ti, hwnd_raw, card_rect));
        }
    }
    candidates.sort_by_key(|&(host_tier, prec_tier, score, hi, ti, _, _)| {
        (host_tier, prec_tier, score, hi, ti)
    });

    let mut used_popup_hosts = vec![false; hosts.len()];
    let mut used_taps = vec![false; tap_rects.len()];
    let mut matched: Vec<(isize, [i32; 4])> = Vec::with_capacity(MAX_SHELL_CARD_SLOTS);
    for (host_tier, _, _, hi, ti, hwnd_raw, card_rect) in candidates {
        if used_taps[ti] || (host_tier == 0 && used_popup_hosts[hi]) {
            continue;
        }
        let overlaps_existing = matched.iter().any(|&(_, prev)| {
            let near_all_edges = (prev[0] - card_rect[0]).abs() <= 6
                && (prev[1] - card_rect[1]).abs() <= 6
                && (prev[2] - card_rect[2]).abs() <= 6
                && (prev[3] - card_rect[3]).abs() <= 6;
            near_all_edges || rect_iou_px(prev, card_rect) > 0.85
        });
        if overlaps_existing {
            continue;
        }
        used_taps[ti] = true;
        if host_tier == 0 {
            used_popup_hosts[hi] = true;
        }
        matched.push((hwnd_raw, card_rect));
        if matched.len() >= MAX_SHELL_CARD_SLOTS {
            break;
        }
    }
    matched.sort_by_key(|&(hwnd_raw, r)| (hwnd_raw, r[1], r[0], r[3], r[2]));
    matched
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
    use crate::ui::lift_argb_alpha;
    match (style, light_glass) {
        (TaskbarStyle::Default, _) => None,
        (TaskbarStyle::Transparent, false) => Some(AltTabItemCardColors {
            bg_argb: lift_argb_alpha(0x41_181B22),
            border_argb: 0x30_FFFFFF,
        }),
        (TaskbarStyle::Blur, false) => Some(AltTabItemCardColors {
            bg_argb: lift_argb_alpha(0x50_1A1C22),
            border_argb: 0x34_FFFFFF,
        }),
        (TaskbarStyle::Acrylic, false) => Some(AltTabItemCardColors {
            bg_argb: lift_argb_alpha(0x58_181A20),
            border_argb: 0x38_FFFFFF,
        }),
        (TaskbarStyle::LiquidGlass, false) => Some(AltTabItemCardColors {
            bg_argb: lift_argb_alpha(0x49_1A1D24),
            border_argb: 0x42_FFFFFF,
        }),
        (TaskbarStyle::Transparent, true) => Some(AltTabItemCardColors {
            bg_argb: lift_argb_alpha(0x56_FAFBFD),
            border_argb: 0x22_1A1D24,
        }),
        (TaskbarStyle::Blur, true) => Some(AltTabItemCardColors {
            bg_argb: lift_argb_alpha(0x67_F8FAFC),
            border_argb: 0x22_1A1D24,
        }),
        (TaskbarStyle::Acrylic, true) => Some(AltTabItemCardColors {
            bg_argb: lift_argb_alpha(0x70_F6F8FB),
            border_argb: 0x26_1A1D24,
        }),
        (TaskbarStyle::LiquidGlass, true) => Some(AltTabItemCardColors {
            bg_argb: lift_argb_alpha(0x5F_FAFCFF),
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
    use crate::ui::lift_argb_alpha;
    match (style, light_glass) {
        (TaskbarStyle::Default, _) => None,
        (TaskbarStyle::Transparent, false) => Some(OsdFallbackCardColors {
            bg_argb: lift_argb_alpha(0x47_12151D),
            border_argb: 0x38_FFFFFF,
        }),
        (TaskbarStyle::Blur, false) => Some(OsdFallbackCardColors {
            bg_argb: lift_argb_alpha(0x5C_161922),
            border_argb: 0x42_FFFFFF,
        }),
        (TaskbarStyle::Acrylic, false) => Some(OsdFallbackCardColors {
            bg_argb: lift_argb_alpha(0x69_181B24),
            border_argb: 0x48_FFFFFF,
        }),
        (TaskbarStyle::LiquidGlass, false) => Some(OsdFallbackCardColors {
            bg_argb: lift_argb_alpha(0x52_141822),
            border_argb: 0x52_FFFFFF,
        }),
        (TaskbarStyle::Transparent, true) => Some(OsdFallbackCardColors {
            bg_argb: lift_argb_alpha(0x5B_FAFBFD),
            border_argb: 0x24_1A1D24,
        }),
        (TaskbarStyle::Blur, true) => Some(OsdFallbackCardColors {
            bg_argb: lift_argb_alpha(0x6D_F8FAFC),
            border_argb: 0x28_1A1D24,
        }),
        (TaskbarStyle::Acrylic, true) => Some(OsdFallbackCardColors {
            bg_argb: lift_argb_alpha(0x7D_F6F8FB),
            border_argb: 0x2C_1A1D24,
        }),
        (TaskbarStyle::LiquidGlass, true) => Some(OsdFallbackCardColors {
            bg_argb: lift_argb_alpha(0x66_FAFCFF),
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
    use crate::ui::lift_argb_alpha;
    match (style, light_glass) {
        (TaskbarStyle::Default, _) => None,
        (TaskbarStyle::Transparent, false) => Some(TooltipFallbackCardColors {
            bg_argb: lift_argb_alpha(0x52_12151D),
            border_argb: 0x3C_FFFFFF,
        }),
        (TaskbarStyle::Blur, false) => Some(TooltipFallbackCardColors {
            bg_argb: lift_argb_alpha(0x62_161922),
            border_argb: 0x44_FFFFFF,
        }),
        (TaskbarStyle::Acrylic, false) => Some(TooltipFallbackCardColors {
            bg_argb: lift_argb_alpha(0x6F_181B24),
            border_argb: 0x4A_FFFFFF,
        }),
        (TaskbarStyle::LiquidGlass, false) => Some(TooltipFallbackCardColors {
            bg_argb: lift_argb_alpha(0x5C_141822),
            border_argb: 0x54_FFFFFF,
        }),
        (TaskbarStyle::Transparent, true) => Some(TooltipFallbackCardColors {
            bg_argb: lift_argb_alpha(0x60_FAFBFD),
            border_argb: 0x26_1A1D24,
        }),
        (TaskbarStyle::Blur, true) => Some(TooltipFallbackCardColors {
            bg_argb: lift_argb_alpha(0x71_F8FAFC),
            border_argb: 0x2A_1A1D24,
        }),
        (TaskbarStyle::Acrylic, true) => Some(TooltipFallbackCardColors {
            bg_argb: lift_argb_alpha(0x81_F6F8FB),
            border_argb: 0x2E_1A1D24,
        }),
        (TaskbarStyle::LiquidGlass, true) => Some(TooltipFallbackCardColors {
            bg_argb: lift_argb_alpha(0x6A_FAFCFF),
            border_argb: 0x2C_1E2430,
        }),
    }
}

/// 任务栏与托盘现代右键菜单 / JumpList 在独占全屏或上层置顶窗口遮挡场景下切换为原地
/// 半透明内联玻璃时使用的背景与边框色彩（AARRGGBB）。
pub type MenuFlyoutFallbackCardColors = OsdFallbackCardColors;

/// 根据当前任务栏主题风格与深浅色返回右键菜单 / JumpList 内联半透明玻璃配色（`Default` 返回 `None`）。
#[cfg_attr(not(windows), allow(dead_code))]
pub const fn menu_flyout_fallback_card_colors(
    style: TaskbarStyle,
    light_glass: bool,
) -> Option<MenuFlyoutFallbackCardColors> {
    use crate::ui::lift_argb_alpha;
    match (style, light_glass) {
        (TaskbarStyle::Default, _) => None,
        (TaskbarStyle::Transparent, false) => Some(MenuFlyoutFallbackCardColors {
            bg_argb: lift_argb_alpha(0x55_12151D),
            border_argb: 0x3E_FFFFFF,
        }),
        (TaskbarStyle::Blur, false) => Some(MenuFlyoutFallbackCardColors {
            bg_argb: lift_argb_alpha(0x66_161922),
            border_argb: 0x46_FFFFFF,
        }),
        (TaskbarStyle::Acrylic, false) => Some(MenuFlyoutFallbackCardColors {
            bg_argb: lift_argb_alpha(0x73_181B24),
            border_argb: 0x4C_FFFFFF,
        }),
        (TaskbarStyle::LiquidGlass, false) => Some(MenuFlyoutFallbackCardColors {
            bg_argb: lift_argb_alpha(0x60_141822),
            border_argb: 0x56_FFFFFF,
        }),
        (TaskbarStyle::Transparent, true) => Some(MenuFlyoutFallbackCardColors {
            bg_argb: lift_argb_alpha(0x64_FAFBFD),
            border_argb: 0x26_1A1D24,
        }),
        (TaskbarStyle::Blur, true) => Some(MenuFlyoutFallbackCardColors {
            bg_argb: lift_argb_alpha(0x75_F8FAFC),
            border_argb: 0x2A_1A1D24,
        }),
        (TaskbarStyle::Acrylic, true) => Some(MenuFlyoutFallbackCardColors {
            bg_argb: lift_argb_alpha(0x85_F6F8FB),
            border_argb: 0x2E_1A1D24,
        }),
        (TaskbarStyle::LiquidGlass, true) => Some(MenuFlyoutFallbackCardColors {
            bg_argb: lift_argb_alpha(0x6E_FAFCFF),
            border_argb: 0x2C_1E2430,
        }),
    }
}

/// Win11 任务视图（`Win + Tab`）底部虚拟桌面条在 XAML 视觉树内渲染的背景与边框色彩（AARRGGBB）。
pub type VirtualDesktopBarColors = AltTabItemCardColors;

/// 根据当前主题风格与深浅色返回 `Win + Tab` 任务视图底部虚拟桌面条玻璃配色（`Default` 返回 `None` 还原原生）。
#[cfg_attr(not(windows), allow(dead_code))]
pub const fn virtual_desktop_bar_colors(
    style: TaskbarStyle,
    light_glass: bool,
) -> Option<VirtualDesktopBarColors> {
    use crate::ui::lift_argb_alpha;
    match (style, light_glass) {
        (TaskbarStyle::Default, _) => None,
        (TaskbarStyle::Transparent, false) => Some(VirtualDesktopBarColors {
            bg_argb: lift_argb_alpha(0x38_10141C),
            border_argb: 0x36_FFFFFF,
        }),
        (TaskbarStyle::LiquidGlass, false) => Some(VirtualDesktopBarColors {
            bg_argb: lift_argb_alpha(0x4E_141A26),
            border_argb: 0x58_E6F0FF,
        }),
        (TaskbarStyle::Blur, false) => Some(VirtualDesktopBarColors {
            bg_argb: lift_argb_alpha(0x62_181C26),
            border_argb: 0x44_FFFFFF,
        }),
        (TaskbarStyle::Acrylic, false) => Some(VirtualDesktopBarColors {
            bg_argb: lift_argb_alpha(0x7A_1B1F28),
            border_argb: 0x3A_FFFFFF,
        }),
        (TaskbarStyle::Transparent, true) => Some(VirtualDesktopBarColors {
            bg_argb: lift_argb_alpha(0x44_F2F6FC),
            border_argb: 0x58_FFFFFF,
        }),
        (TaskbarStyle::LiquidGlass, true) => Some(VirtualDesktopBarColors {
            bg_argb: lift_argb_alpha(0x5C_F4F8FF),
            border_argb: 0x78_FFFFFF,
        }),
        (TaskbarStyle::Blur, true) => Some(VirtualDesktopBarColors {
            bg_argb: lift_argb_alpha(0x72_F2F5FA),
            border_argb: 0x60_FFFFFF,
        }),
        (TaskbarStyle::Acrylic, true) => Some(VirtualDesktopBarColors {
            bg_argb: lift_argb_alpha(0x8A_F0F3F8),
            border_argb: 0x52_FFFFFF,
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
                (0x4C..=0x70).contains(&dark_bg_alpha),
                "dark alpha {dark_bg_alpha:#x} out of calibrated glass range"
            );
            assert!(
                (0x5E..=0x88).contains(&light_bg_alpha),
                "light alpha {light_bg_alpha:#x} out of calibrated frost range"
            );
            // Dark theme uses white glass edge; light theme uses subtle dark ink edge.
            assert_eq!(dark.border_argb & 0x00FF_FFFF, 0x00FF_FFFF);
            assert_ne!(light.border_argb & 0x00FF_FFFF, 0x00FF_FFFF);
            assert!((dark.bg_argb & 0xFF) < 0x40);
            assert!((light.bg_argb & 0xFF) > 0xE0);
        }

        for light in [false, true] {
            let t = alt_tab_item_card_colors(TaskbarStyle::Transparent, light).unwrap();
            let lg = alt_tab_item_card_colors(TaskbarStyle::LiquidGlass, light).unwrap();
            let b = alt_tab_item_card_colors(TaskbarStyle::Blur, light).unwrap();
            let a = alt_tab_item_card_colors(TaskbarStyle::Acrylic, light).unwrap();
            assert!((t.bg_argb >> 24) < (lg.bg_argb >> 24));
            assert!((lg.bg_argb >> 24) < (b.bg_argb >> 24));
            assert!((b.bg_argb >> 24) < (a.bg_argb >> 24));
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
            assert!((0x50..=0x80).contains(&dark_alpha));
            assert!((0x60..=0x94).contains(&light_alpha));
            assert_eq!(dark.border_argb & 0x00FF_FFFF, 0x00FF_FFFF);
            assert_ne!(light.border_argb & 0x00FF_FFFF, 0x00FF_FFFF);
            assert!((dark.bg_argb & 0xFF) < 0x40);
            assert!((light.bg_argb & 0xFF) > 0xE0);
        }

        for light in [false, true] {
            let t = osd_fallback_card_colors(TaskbarStyle::Transparent, light).unwrap();
            let lg = osd_fallback_card_colors(TaskbarStyle::LiquidGlass, light).unwrap();
            let b = osd_fallback_card_colors(TaskbarStyle::Blur, light).unwrap();
            let a = osd_fallback_card_colors(TaskbarStyle::Acrylic, light).unwrap();
            assert!((t.bg_argb >> 24) < (lg.bg_argb >> 24));
            assert!((lg.bg_argb >> 24) < (b.bg_argb >> 24));
            assert!((b.bg_argb >> 24) < (a.bg_argb >> 24));
        }
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
            assert!((0x58..=0x88).contains(&dark_alpha));
            assert!((0x68..=0x98).contains(&light_alpha));
            assert_eq!(dark.border_argb & 0x00FF_FFFF, 0x00FF_FFFF);
            assert_ne!(light.border_argb & 0x00FF_FFFF, 0x00FF_FFFF);
            assert!((dark.bg_argb & 0xFF) < 0x40);
            assert!((light.bg_argb & 0xFF) > 0xE0);
        }

        for light in [false, true] {
            let t = tooltip_fallback_card_colors(TaskbarStyle::Transparent, light).unwrap();
            let lg = tooltip_fallback_card_colors(TaskbarStyle::LiquidGlass, light).unwrap();
            let b = tooltip_fallback_card_colors(TaskbarStyle::Blur, light).unwrap();
            let a = tooltip_fallback_card_colors(TaskbarStyle::Acrylic, light).unwrap();
            assert!((t.bg_argb >> 24) < (lg.bg_argb >> 24));
            assert!((lg.bg_argb >> 24) < (b.bg_argb >> 24));
            assert!((b.bg_argb >> 24) < (a.bg_argb >> 24));
        }
    }

    #[test]
    fn map_menu_flyout_handles_tall_jumplist_negative_monitor_and_cascade_matching() {
        // Tall JumpList card (w=320, h=620 DIP) at 150% HiDPI on negative secondary monitor
        assert_eq!(
            map_menu_flyout_card_rect_to_screen(
                [-1936, 360, -1424, 1344],
                144,
                [4.0, -648.0, 324.0, -28.0],
                &[[-1920, 1344]]
            ),
            Some([-1914, 372, -1434, 1302])
        );

        // Size mismatch must be rejected
        assert_eq!(
            map_menu_flyout_card_rect_to_screen(
                [100, 100, 150, 130],
                96,
                [0.0, 0.0, 260.0, 400.0],
                &[]
            ),
            None
        );

        // 3 concurrent cascading MenuFlyouts match 1-to-1 by tightest size fit without cross-stealing
        let hosts = [
            ([100, 500, 380, 920], 96, 101u32), // main: 280x420
            ([376, 620, 576, 810], 96, 101u32), // sub1: 200x190
            ([572, 680, 742, 810], 96, 101u32), // sub2: 170x130
        ];
        // Deliberately reversed order from TAP
        let tap_rects = [
            (101u32, [6.0, 6.0, 152.0, 112.0]),  // 146x106 -> sub2
            (101u32, [8.0, 8.0, 264.0, 396.0]),  // 256x388 -> main
            (101u32, [6.0, 6.0, 182.0, 168.0]),  // 176x162 -> sub1
        ];
        let origins = [Vec::new(), Vec::new(), Vec::new()];
        let matched = match_menu_flyout_hosts_to_rects(&hosts, &tap_rects, &origins);
        assert_eq!(matched.len(), 3);
        let mapped_for = |host_idx: usize| {
            matched
                .iter()
                .find(|(hi, _)| *hi == host_idx)
                .map(|&(_, r)| r)
                .expect("expected host matched")
        };
        assert_eq!(mapped_for(0)[2] - mapped_for(0)[0], 256);
        assert_eq!(mapped_for(1)[2] - mapped_for(1)[0], 176);
        assert_eq!(mapped_for(2)[2] - mapped_for(2)[0], 146);
    }

    #[test]
    fn menu_flyout_fallback_card_colors_scale_across_styles_and_themes() {
        assert_eq!(menu_flyout_fallback_card_colors(TaskbarStyle::Default, false), None);
        assert_eq!(menu_flyout_fallback_card_colors(TaskbarStyle::Default, true), None);

        for style in [
            TaskbarStyle::Transparent,
            TaskbarStyle::Blur,
            TaskbarStyle::Acrylic,
            TaskbarStyle::LiquidGlass,
        ] {
            let dark =
                menu_flyout_fallback_card_colors(style, false).expect("non-default dark menu");
            let light =
                menu_flyout_fallback_card_colors(style, true).expect("non-default light menu");
            assert!((0x5C..=0x8C).contains(&(dark.bg_argb >> 24)));
            assert!((0x68..=0x9A).contains(&(light.bg_argb >> 24)));
            assert_eq!(dark.border_argb & 0x00FF_FFFF, 0x00FF_FFFF);
            assert_ne!(light.border_argb & 0x00FF_FFFF, 0x00FF_FFFF);
        }
        for light in [false, true] {
            let t = menu_flyout_fallback_card_colors(TaskbarStyle::Transparent, light).unwrap();
            let lg = menu_flyout_fallback_card_colors(TaskbarStyle::LiquidGlass, light).unwrap();
            let b = menu_flyout_fallback_card_colors(TaskbarStyle::Blur, light).unwrap();
            let a = menu_flyout_fallback_card_colors(TaskbarStyle::Acrylic, light).unwrap();
            assert!((t.bg_argb >> 24) < (lg.bg_argb >> 24));
            assert!((lg.bg_argb >> 24) < (b.bg_argb >> 24));
            assert!((b.bg_argb >> 24) < (a.bg_argb >> 24));
        }
    }

    #[test]
    fn match_shell_host_cards_handles_multi_card_hosts_popup_isolation_and_dedup() {
        // Notification Center + Calendar inside a single CoreWindow (tid=201),
        // plus Quick Settings + Media card + cascading menu popup in ShellHost (tid=301).
        let hosts = [
            ([1560, 0, 1920, 1032], 96, 201u32, 1001isize, false), // Notification/Calendar CoreWindow
            ([1536, 0, 1920, 1032], 96, 301u32, 2001isize, false), // ControlCenterWindow
            ([1600, 720, 1820, 910], 96, 301u32, 2002isize, true), // Popup menu inside Quick Settings
        ];
        let tap_rects = [
            (201u32, [12.0, 20.0, 348.0, 540.0]),  // NotificationCenterGrid
            (201u32, [12.0, 552.0, 348.0, 996.0]), // CalendarCenterGrid
            (201u32, [13.0, 21.0, 347.0, 539.0]),  // Duplicate overlapping rect -> deduped
            (999u32, [0.0, 0.0, 360.0, 100.0]),    // Hidden/cloaked thread not in hosts -> ignored
            (301u32, [8.0, 8.0, 212.0, 182.0]),    // Popup menu inside ControlCenter
            (301u32, [12.0, 480.0, 372.0, 600.0]), // MediaTransportControlsRegion
            (301u32, [12.0, 612.0, 372.0, 1016.0]), // RootGridBorder / L1Grid
        ];

        let matched = match_shell_host_cards_to_screen(&hosts, &tap_rects);
        assert_eq!(matched.len(), 5);
        assert_eq!(
            matched,
            vec![
                (1001isize, [1572, 20, 1908, 540]),
                (1001isize, [1572, 552, 1908, 996]),
                (2001isize, [1548, 480, 1908, 600]),
                (2001isize, [1548, 612, 1908, 1016]),
                (2002isize, [1608, 728, 1812, 902]),
            ]
        );
    }

    #[test]
    fn virtual_desktop_bar_colors_order_by_style_and_theme() {
        assert_eq!(virtual_desktop_bar_colors(TaskbarStyle::Default, false), None);
        assert_eq!(virtual_desktop_bar_colors(TaskbarStyle::Default, true), None);

        for light in [false, true] {
            let t = virtual_desktop_bar_colors(TaskbarStyle::Transparent, light).unwrap();
            let lg = virtual_desktop_bar_colors(TaskbarStyle::LiquidGlass, light).unwrap();
            let b = virtual_desktop_bar_colors(TaskbarStyle::Blur, light).unwrap();
            let a = virtual_desktop_bar_colors(TaskbarStyle::Acrylic, light).unwrap();

            assert!(t.bg_argb >> 24 < lg.bg_argb >> 24);
            assert!(lg.bg_argb >> 24 < b.bg_argb >> 24);
            assert!(b.bg_argb >> 24 < a.bg_argb >> 24);
            assert!(lg.border_argb >> 24 > t.border_argb >> 24);
        }
    }

    #[test]
    fn scale_dip_overflow_and_menu_precision_tier_ordering_work_safely() {
        assert_eq!(
            map_xaml_card_rect_to_screen([0, 0, 1920, 1080], 192, [0.0, 0.0, 1e12, 500.0]),
            None
        );
        assert_eq!(
            map_tooltip_card_rect_to_screen(
                [0, 0, 300, 100],
                144,
                [f32::INFINITY, 0.0, 120.0, 40.0],
                &[]
            ),
            None
        );

        // Host 0 uses exact popup-local placement (tier 1); Host 1 only falls back to tier 2,
        // so identical sizes still award the higher-precision match first.
        let hosts = [
            ([100, 100, 320, 340], 96, 55u32), // 220x240 -> exact local fit -> tier 1
            ([500, 100, 720, 340], 96, 55u32), // 220x240 -> negative offsets -> tier 2 fallback
        ];
        let tap_rects = [
            (55u32, [-10.0, -50.0, 190.0, 170.0]), // 200x220 -> only matches tier 2
            (55u32, [10.0, 10.0, 210.0, 230.0]),   // 200x220 -> matches host 0 & 1 with tier 1
        ];
        let origins = [
            Vec::new(),
            Vec::new(),
        ];
        let matched = match_menu_flyout_hosts_to_rects(&hosts, &tap_rects, &origins);
        assert_eq!(matched.len(), 2);
        let for_h0 = matched.iter().find(|(hi, _)| *hi == 0).unwrap().1;
        assert_eq!(for_h0, [110, 110, 310, 330]);
    }
}
