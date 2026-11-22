#[cfg(windows)]
pub mod glass;

use crate::color::{hsl_to_rgb, rgb_to_hsl, Rgb};

/// Outer transparent padding reserved around Liquid Glass popup cards for 3-pass ambient shadows (DIP).
#[cfg_attr(not(windows), allow(dead_code))]
pub const SHADOW_PAD_DIP: f32 = 14.0;
/// Floating vertical breathing gap between a taskbar Bar popup card and the taskbar top edge (DIP).
#[cfg_attr(not(windows), allow(dead_code))]
pub const POPUP_FLOAT_GAP_DIP: f32 = 7.0;
/// Standard card inner padding around menu rows (DIP).
#[cfg_attr(not(windows), allow(dead_code))]
pub const INNER_PAD_DIP: f32 = 8.0;
/// Outer corner radius for Liquid Glass menu panels (DIP).
#[cfg_attr(not(windows), allow(dead_code))]
pub const PANEL_RADIUS_DIP: f32 = 13.0;
/// Standard single-line menu row height (DIP).
#[cfg_attr(not(windows), allow(dead_code))]
pub const ROW_H_DIP: f32 = 34.0;
/// Vertical gap between adjacent rows inside a menu group (DIP).
#[cfg_attr(not(windows), allow(dead_code))]
pub const ROW_GAP_DIP: f32 = 4.0;
/// Total vertical span of a horizontal group divider (DIP).
#[cfg_attr(not(windows), allow(dead_code))]
pub const DIVIDER_H_DIP: f32 = 9.0;

/// Fast animation tick interval while springs/fades are active (ms).
#[cfg_attr(not(windows), allow(dead_code))]
pub const POPUP_ANIM_INTERVAL_MS: u32 = 10;
/// Steady-state idle tick interval driving the liquid silk sheen (ms).
#[cfg_attr(not(windows), allow(dead_code))]
pub const POPUP_FLOW_INTERVAL_MS: u32 = 33;
/// Minimum interval between silk bitmap regenerations (ms).
#[cfg_attr(not(windows), allow(dead_code))]
pub const SILK_UPDATE_INTERVAL_MS: u64 = 32;
/// Duration of the brisk retract-and-fade close animation (seconds).
#[cfg_attr(not(windows), allow(dead_code))]
pub const CLOSE_ANIM_DUR_S: f32 = 0.095;
/// Second-order spring stiffness for Liquid Glass popup opening.
#[cfg_attr(not(windows), allow(dead_code))]
pub const OPEN_SPRING_STIFFNESS: f32 = 780.0;
/// Second-order spring damping for Liquid Glass popup opening.
#[cfg_attr(not(windows), allow(dead_code))]
pub const OPEN_SPRING_DAMPING: f32 = 41.5;
/// Second-order spring stiffness for Liquid Glass cascading submenu width expansion.
#[cfg_attr(not(windows), allow(dead_code))]
pub const SUB_WIDTH_STIFFNESS: f32 = 920.0;
/// Second-order spring damping for Liquid Glass cascading submenu width expansion.
#[cfg_attr(not(windows), allow(dead_code))]
pub const SUB_WIDTH_DAMPING: f32 = 46.0;
/// Debounce window after closing a popup before re-opening on the same trigger (ms).
#[cfg_attr(not(windows), allow(dead_code))]
pub const POPUP_REOPEN_DEBOUNCE_MS: u64 = 240;

/// Mirror of taskbar visual style exposed in `crate::ui` so all popup surfaces harmonize with the taskbar
/// without coupling `mediabar` / `monitorbar` to `taskbar`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GlassSurfaceStyle {
    Default,
    Transparent,
    Blur,
    Acrylic,
    #[default]
    LiquidGlass,
}

impl GlassSurfaceStyle {
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::Default => 0,
            Self::Transparent => 1,
            Self::Blur => 2,
            Self::Acrylic => 3,
            Self::LiquidGlass => 4,
        }
    }

    pub const fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::Default,
            1 => Self::Transparent,
            2 => Self::Blur,
            3 => Self::Acrylic,
            _ => Self::LiquidGlass,
        }
    }
}

/// Calibrated popup glass material parameters harmonized with the active taskbar style and underlying luminance.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(not(windows), allow(dead_code))]
pub struct PopupGlassRecipe {
    pub blur_radius_4x: usize,
    pub backdrop_alpha: f32,
    pub tint_rgb: (f32, f32, f32),
    pub tint_alpha: f32,
    pub frost_veil_top_alpha: f32,
    pub frost_veil_mid_alpha: f32,
    pub frost_veil_bot_alpha: f32,
    pub grain_max_alpha: u8,
    pub silk_alpha: f32,
    pub sheen_scale: f32,
    pub rim_top_alpha: f32,
    pub rim_mid_alpha: f32,
    pub rim_bot_alpha: f32,
    pub rim_dark_sep_alpha: f32,
    pub rim_stroke_dip: f32,
}

/// Returns Rec.709 relative luminance in `[0.0, 1.0]` for an sRGB color.
#[cfg_attr(not(windows), allow(dead_code))]
#[inline]
pub fn rgb_luma(c: Rgb) -> f32 {
    (0.2126 * f32::from(c.r) + 0.7152 * f32::from(c.g) + 0.0722 * f32::from(c.b)) / 255.0
}

/// Uniform opacity lift factor applied across all backdrop surfaces (`a' = a + 0.10 * (1 - a)`).
#[cfg_attr(not(windows), allow(dead_code))]
pub const BACKDROP_OPACITY_LIFT: f32 = 0.10;

/// Moves a normalized `[0.0, 1.0]` alpha 10% closer to fully opaque (`1.0`).
#[cfg_attr(not(windows), allow(dead_code))]
#[inline]
pub const fn lift_backdrop_alpha(alpha: f32) -> f32 {
    let clamped = if alpha < 0.0 {
        0.0
    } else if alpha > 1.0 {
        1.0
    } else {
        alpha
    };
    clamped + BACKDROP_OPACITY_LIFT * (1.0 - clamped)
}

/// Moves an `0xAARRGGBB` color's 8-bit alpha channel 10% closer to `255` (`0xFF`) without altering RGB.
#[cfg_attr(not(windows), allow(dead_code))]
#[inline]
pub const fn lift_argb_alpha(argb: u32) -> u32 {
    let a = (argb >> 24) & 0xFF;
    let lifted = (a * 9 + 255 + 5) / 10;
    let clamped = if lifted > 255 { 255 } else { lifted };
    (clamped << 24) | (argb & 0x00FF_FFFF)
}

/// Computes unified popup glass surface parameters for a given taskbar style, system theme, and sampled backdrop luminance.
/// Keeps dark-theme text white and light-theme text dark while softly boosting backing tint over high-contrast windows
/// (e.g., dark theme popup over a bright white page) and preventing muddy gray cast.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn popup_glass_recipe(
    style: GlassSurfaceStyle,
    light_glass: bool,
    backdrop_luma: f32,
) -> PopupGlassRecipe {
    let luma = backdrop_luma.clamp(0.0, 1.0);
    let dark_bg_boost_t = ((luma - 0.62) / 0.38).clamp(0.0, 1.0);
    let light_bg_boost_t = ((0.38 - luma) / 0.38).clamp(0.0, 1.0);

    let tint = |r: f32, g: f32, b: f32| (r / 255.0, g / 255.0, b / 255.0);
    let rim_stroke_dip = if light_glass { 1.0 } else { 1.05 };

    match (style, light_glass) {
        (GlassSurfaceStyle::Default, false) => PopupGlassRecipe {
            blur_radius_4x: 6,
            backdrop_alpha: 0.86,
            tint_rgb: tint(32.0, 32.0, 33.0),
            tint_alpha: lift_backdrop_alpha(0.62 + 0.04 * dark_bg_boost_t),
            frost_veil_top_alpha: 0.03,
            frost_veil_mid_alpha: 0.01,
            frost_veil_bot_alpha: 0.0,
            grain_max_alpha: 0,
            silk_alpha: 0.0,
            sheen_scale: 0.35,
            rim_top_alpha: 0.24,
            rim_mid_alpha: 0.14,
            rim_bot_alpha: 0.11,
            rim_dark_sep_alpha: 0.08 * dark_bg_boost_t,
            rim_stroke_dip,
        },
        (GlassSurfaceStyle::Default, true) => PopupGlassRecipe {
            blur_radius_4x: 6,
            backdrop_alpha: 0.88,
            tint_rgb: tint(248.0, 248.0, 249.0),
            tint_alpha: lift_backdrop_alpha(0.72 + 0.04 * light_bg_boost_t),
            frost_veil_top_alpha: 0.08,
            frost_veil_mid_alpha: 0.03,
            frost_veil_bot_alpha: 0.0,
            grain_max_alpha: 0,
            silk_alpha: 0.0,
            sheen_scale: 0.35,
            rim_top_alpha: 0.74,
            rim_mid_alpha: 0.38,
            rim_bot_alpha: 0.20,
            rim_dark_sep_alpha: 0.10,
            rim_stroke_dip,
        },
        (GlassSurfaceStyle::Acrylic, false) => PopupGlassRecipe {
            blur_radius_4x: 8,
            backdrop_alpha: 0.89,
            tint_rgb: tint(38.0, 38.0, 40.0),
            tint_alpha: lift_backdrop_alpha(0.52 + 0.08 * dark_bg_boost_t),
            frost_veil_top_alpha: 0.06,
            frost_veil_mid_alpha: 0.02,
            frost_veil_bot_alpha: 0.0,
            grain_max_alpha: 11,
            silk_alpha: 0.0,
            sheen_scale: 0.45,
            rim_top_alpha: 0.30,
            rim_mid_alpha: 0.16,
            rim_bot_alpha: 0.12,
            rim_dark_sep_alpha: 0.09 * dark_bg_boost_t,
            rim_stroke_dip,
        },
        (GlassSurfaceStyle::Acrylic, true) => PopupGlassRecipe {
            blur_radius_4x: 8,
            backdrop_alpha: 0.91,
            tint_rgb: tint(248.0, 248.0, 250.0),
            tint_alpha: lift_backdrop_alpha(0.63 + 0.07 * light_bg_boost_t),
            frost_veil_top_alpha: 0.14,
            frost_veil_mid_alpha: 0.05,
            frost_veil_bot_alpha: 0.0,
            grain_max_alpha: 9,
            silk_alpha: 0.0,
            sheen_scale: 0.45,
            rim_top_alpha: 0.76,
            rim_mid_alpha: 0.40,
            rim_bot_alpha: 0.22,
            rim_dark_sep_alpha: 0.10,
            rim_stroke_dip,
        },
        (GlassSurfaceStyle::Blur, false) => PopupGlassRecipe {
            blur_radius_4x: 6,
            backdrop_alpha: 0.87,
            tint_rgb: tint(46.0, 46.0, 48.0),
            tint_alpha: lift_backdrop_alpha(0.43 + 0.13 * dark_bg_boost_t),
            frost_veil_top_alpha: 0.12,
            frost_veil_mid_alpha: 0.045,
            frost_veil_bot_alpha: 0.015,
            grain_max_alpha: 6,
            silk_alpha: 0.0,
            sheen_scale: 0.55,
            rim_top_alpha: 0.26,
            rim_mid_alpha: 0.14,
            rim_bot_alpha: 0.10,
            rim_dark_sep_alpha: 0.09 * dark_bg_boost_t,
            rim_stroke_dip,
        },
        (GlassSurfaceStyle::Blur, true) => PopupGlassRecipe {
            blur_radius_4x: 6,
            backdrop_alpha: 0.88,
            tint_rgb: tint(250.0, 250.0, 251.0),
            tint_alpha: lift_backdrop_alpha(0.52 + 0.12 * light_bg_boost_t),
            frost_veil_top_alpha: 0.22,
            frost_veil_mid_alpha: 0.08,
            frost_veil_bot_alpha: 0.03,
            grain_max_alpha: 5,
            silk_alpha: 0.0,
            sheen_scale: 0.55,
            rim_top_alpha: 0.70,
            rim_mid_alpha: 0.34,
            rim_bot_alpha: 0.18,
            rim_dark_sep_alpha: 0.09,
            rim_stroke_dip,
        },
        (GlassSurfaceStyle::LiquidGlass, false) => PopupGlassRecipe {
            blur_radius_4x: 7,
            backdrop_alpha: 0.84,
            tint_rgb: tint(42.0, 42.0, 44.0),
            tint_alpha: lift_backdrop_alpha(0.36 + 0.17 * dark_bg_boost_t),
            frost_veil_top_alpha: 0.05,
            frost_veil_mid_alpha: 0.02,
            frost_veil_bot_alpha: 0.0,
            grain_max_alpha: 0,
            silk_alpha: 1.0,
            sheen_scale: 0.95,
            rim_top_alpha: 0.24,
            rim_mid_alpha: 0.13,
            rim_bot_alpha: 0.09,
            rim_dark_sep_alpha: 0.10 * dark_bg_boost_t,
            rim_stroke_dip,
        },
        (GlassSurfaceStyle::LiquidGlass, true) => PopupGlassRecipe {
            blur_radius_4x: 7,
            backdrop_alpha: 0.86,
            tint_rgb: tint(250.0, 250.0, 252.0),
            tint_alpha: lift_backdrop_alpha(0.42 + 0.17 * light_bg_boost_t),
            frost_veil_top_alpha: 0.14,
            frost_veil_mid_alpha: 0.05,
            frost_veil_bot_alpha: 0.0,
            grain_max_alpha: 0,
            silk_alpha: 0.96,
            sheen_scale: 0.95,
            rim_top_alpha: 0.64,
            rim_mid_alpha: 0.30,
            rim_bot_alpha: 0.16,
            rim_dark_sep_alpha: 0.09,
            rim_stroke_dip,
        },
        (GlassSurfaceStyle::Transparent, false) => PopupGlassRecipe {
            blur_radius_4x: 2,
            backdrop_alpha: 0.65,
            tint_rgb: tint(36.0, 36.0, 38.0),
            tint_alpha: lift_backdrop_alpha(0.31 + 0.19 * dark_bg_boost_t),
            frost_veil_top_alpha: 0.02,
            frost_veil_mid_alpha: 0.0,
            frost_veil_bot_alpha: 0.0,
            grain_max_alpha: 0,
            silk_alpha: 0.0,
            sheen_scale: 0.30,
            rim_top_alpha: 0.22,
            rim_mid_alpha: 0.12,
            rim_bot_alpha: 0.09,
            rim_dark_sep_alpha: 0.11 * dark_bg_boost_t,
            rim_stroke_dip,
        },
        (GlassSurfaceStyle::Transparent, true) => PopupGlassRecipe {
            blur_radius_4x: 2,
            backdrop_alpha: 0.69,
            tint_rgb: tint(250.0, 250.0, 251.0),
            tint_alpha: lift_backdrop_alpha(0.35 + 0.20 * light_bg_boost_t),
            frost_veil_top_alpha: 0.06,
            frost_veil_mid_alpha: 0.02,
            frost_veil_bot_alpha: 0.0,
            grain_max_alpha: 0,
            silk_alpha: 0.0,
            sheen_scale: 0.30,
            rim_top_alpha: 0.58,
            rim_mid_alpha: 0.28,
            rim_bot_alpha: 0.14,
            rim_dark_sep_alpha: 0.09,
            rim_stroke_dip,
        },
    }
}

/// Generates a deterministic `size × size` premultiplied BGRA monochrome grain texture for frosted glass/acrylic.
/// Uses a triangular distribution so half the grains softly brighten and half subtly deepen,
/// strictly maintaining premultiplied alpha invariants (`b <= a && g <= a && r <= a && a <= max_alpha`).
#[cfg_attr(not(windows), allow(dead_code))]
pub fn generate_acrylic_noise_bgra(size: usize, seed: u32, max_alpha: u8) -> Vec<u8> {
    let pixel_count = size.saturating_mul(size);
    let mut buf = Vec::with_capacity(pixel_count * 4);
    let mut state = if seed == 0 { 0x9E37_79B9 } else { seed };
    let max_a = f32::from(max_alpha);

    for _ in 0..pixel_count {
        let u1 = next_noise_u32(&mut state);
        let u2 = next_noise_u32(&mut state);
        let n1 = (u1 & 0xFFFF) as f32 / 65535.0;
        let n2 = (u2 & 0xFFFF) as f32 / 65535.0;
        let tri = n1 + n2 - 1.0;
        let bright = tri >= 0.0;
        let mag = (tri.abs() * max_a).round().clamp(0.0, max_a) as u8;
        let rgb = if bright { mag } else { 0 };
        buf.push(rgb);
        buf.push(rgb);
        buf.push(rgb);
        buf.push(mag);
    }

    buf
}

#[inline]
fn next_noise_u32(state: &mut u32) -> u32 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    *state = x;
    x.wrapping_mul(0x85EB_CA6B) ^ (x >> 13)
}

/// Offscreen width of the domain-warped liquid silk buffer.
#[cfg_attr(not(windows), allow(dead_code))]
pub const SILK_W: u32 = 64;
/// Offscreen height of the domain-warped liquid silk buffer.
#[cfg_attr(not(windows), allow(dead_code))]
pub const SILK_H: u32 = 48;

/// Axis-aligned rectangle in physical or logical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RectF {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl RectF {
    pub const fn new(left: f32, top: f32, right: f32, bottom: f32) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }

    #[inline]
    pub fn width(self) -> f32 {
        (self.right - self.left).max(0.0)
    }

    #[inline]
    pub fn height(self) -> f32 {
        (self.bottom - self.top).max(0.0)
    }

    #[inline]
    pub fn contains(self, x: f32, y: f32) -> bool {
        x >= self.left && x <= self.right && y >= self.top && y <= self.bottom
    }
}

/// Outer meniscus fillet radius shared by liquid-fused cascading submenus (DIP).
#[cfg_attr(not(windows), allow(dead_code))]
pub const STEP_MENISCUS_DIP: f32 = 13.5;

/// Lifecycle phase of a Liquid Glass popup card.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(windows), allow(dead_code))]
pub enum PopupAnimPhase {
    Opening,
    Open,
    Closing,
}

/// Computes the primary card's animated envelope anchored at its bottom/sides during open/close.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn animated_primary_rect(
    target: RectF,
    phase: PopupAnimPhase,
    open_pos: f32,
    close_progress: f32,
    s: f32,
) -> RectF {
    let target_h = target.height().max(1.0);
    let min_h = (42.0 * s).min(target_h * 0.65);
    match phase {
        PopupAnimPhase::Opening => {
            let h = (min_h + (target_h - min_h) * open_pos.max(0.0)).max(min_h);
            RectF::new(
                target.left,
                (target.bottom - h).max(4.0 * s),
                target.right,
                target.bottom,
            )
        }
        PopupAnimPhase::Open => target,
        PopupAnimPhase::Closing => {
            let r = ease_retract(close_progress);
            let h = (target_h * (1.0 - 0.42 * r)).max(min_h);
            RectF::new(
                target.left,
                (target.bottom - h).max(4.0 * s),
                target.right,
                target.bottom,
            )
        }
    }
}

/// Computes the initial side-budding submenu droplet anchored at `seam_x` with safe minimum width.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn sub_bud_start_rect(
    target_sub: RectF,
    row: RectF,
    seam_x: f32,
    open_left: bool,
    s: f32,
) -> RectF {
    let min_w = ((STEP_MENISCUS_DIP + PANEL_RADIUS_DIP + 10.0).max(40.0) * s)
        .min(target_sub.width());
    let start_w = (target_sub.width() * 0.26).max(min_w).min(target_sub.width());
    let row_cy = ((row.top + row.bottom) * 0.5).clamp(
        target_sub.top + 20.0 * s,
        (target_sub.bottom - 20.0 * s).max(target_sub.top + 20.0 * s),
    );
    let min_h = (44.0 * s).min(target_sub.height());
    let start_h = (target_sub.height() * 0.42).max(min_h).min(target_sub.height());
    let top = (row_cy - start_h * 0.5)
        .clamp(target_sub.top, (target_sub.bottom - start_h).max(target_sub.top));
    let bottom = top + start_h;
    if open_left {
        RectF::new(seam_x - start_w, top, seam_x, bottom)
    } else {
        RectF::new(seam_x, top, seam_x + start_w, bottom)
    }
}

/// Clamps spring-driven submenu edges into geometry-safe bounds anchored at `seam_x`.
#[cfg_attr(not(windows), allow(dead_code))]
#[allow(clippy::too_many_arguments)]
pub fn clamp_sub_spring_rect(
    spring_top: f32,
    spring_bottom: f32,
    spring_width: f32,
    target_sub: RectF,
    seam_x: f32,
    bottom_limit: f32,
    win_w: f32,
    open_left: bool,
    s: f32,
) -> RectF {
    let min_safe_w = ((STEP_MENISCUS_DIP + PANEL_RADIUS_DIP + 10.0).max(40.0) * s)
        .min(target_sub.width().max(40.0 * s));
    let max_avail_w = if open_left {
        (seam_x - 5.0 * s).max(min_safe_w)
    } else {
        (win_w - 5.0 * s - seam_x).max(min_safe_w)
    };
    let w = spring_width.clamp(min_safe_w, max_avail_w);

    let min_h = (44.0 * s).min(target_sub.height().max(44.0 * s));
    let min_top = 4.0 * s;
    let max_bot = bottom_limit.max(min_top + min_h);
    let mut bot = spring_bottom.clamp(min_top + min_h, max_bot);
    let mut top = spring_top.clamp(min_top, max_bot - min_h);
    if bot - top < min_h {
        top = (bot - min_h).max(min_top);
        bot = (top + min_h).min(max_bot);
    }

    if open_left {
        RectF::new(seam_x - w, top, seam_x, bot)
    } else {
        RectF::new(seam_x, top, seam_x + w, bot)
    }
}

/// Computes the submenu's closing collapse rectangle anchored to `seam_x` and clamped above `bottom_limit`.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn animated_closing_sub_rect(
    sub: RectF,
    anim_primary: RectF,
    seam_x: f32,
    bottom_limit: f32,
    open_left: bool,
    close_progress: f32,
    s: f32,
) -> RectF {
    let r = ease_retract(close_progress);
    let min_w = ((STEP_MENISCUS_DIP + PANEL_RADIUS_DIP + 10.0).max(40.0) * s).min(sub.width());
    let cur_w = (sub.width() * (1.0 - 0.22 * r)).max(min_w);
    let drop = 12.0 * s * r;
    let max_bot = bottom_limit.min(anim_primary.bottom);
    let cur_bot = (sub.bottom + drop).min(max_bot).max(44.0 * s);
    let min_h = (42.0 * s).min(sub.height());
    let cur_h = (sub.height() * (1.0 - 0.20 * r))
        .max(min_h)
        .min((cur_bot - 4.0 * s).max(min_h));
    let cur_top = (cur_bot - cur_h).max(4.0 * s);
    if open_left {
        RectF::new(seam_x - cur_w, cur_top, seam_x, cur_bot)
    } else {
        RectF::new(seam_x, cur_top, seam_x + cur_w, cur_bot)
    }
}

/// Computes vertical `(top, bottom)` for a cascading submenu aligning to `row_top` downward when space permits.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn compute_submenu_vertical_span(
    row_top: f32,
    inner_pad: f32,
    natural_sh: f32,
    top_limit: f32,
    bottom_limit: f32,
) -> (f32, f32) {
    let max_sh = (bottom_limit - top_limit).max(1.0);
    let sh = natural_sh.min(max_sh);

    let down_top = row_top - inner_pad;
    if down_top >= top_limit && down_top + sh <= bottom_limit + 0.5 {
        return (down_top, (down_top + sh).min(bottom_limit));
    }

    let up_top = (bottom_limit - sh).max(top_limit);
    (up_top, (up_top + sh).min(bottom_limit))
}

/// Critically/lightly damped 1D second-order spring preserving velocity across target changes.
#[derive(Clone, Copy, Debug)]
#[cfg_attr(not(windows), allow(dead_code))]
pub struct Spring1D {
    pub pos: f32,
    pub vel: f32,
}

#[cfg_attr(not(windows), allow(dead_code))]
impl Spring1D {
    pub fn still(pos: f32) -> Self {
        Self { pos, vel: 0.0 }
    }

    pub fn new(pos: f32, vel: f32) -> Self {
        Self { pos, vel }
    }

    pub fn step(&mut self, target: f32, stiffness: f32, damping: f32, dt: f32) {
        const SUB_DT: f32 = 0.003;
        let mut rem = dt.clamp(0.0, 0.1);
        while rem > 0.0 {
            let h = rem.min(SUB_DT);
            let acc = (target - self.pos) * stiffness - self.vel * damping;
            self.vel += acc * h;
            self.pos += self.vel * h;
            rem -= h;
        }
    }

    pub fn is_settled(&self, target: f32, pos_eps: f32, vel_eps: f32) -> bool {
        (target - self.pos).abs() <= pos_eps && self.vel.abs() <= vel_eps
    }
}

/// Cubic ease-out curve in `[0, 1]`.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn ease_out_cubic(t: f32) -> f32 {
    let u = 1.0 - t.clamp(0.0, 1.0);
    1.0 - u * u * u
}

/// Smooth responsive retract curve (`t * (0.45 + 0.55*t)`) in `[0, 1]` with immediate response and brisk collapse.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn ease_retract(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * (0.45 + 0.55 * t)
}

/// Overall layered-window opacity envelope across open, steady, and close phases.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn window_master_alpha(phase: PopupAnimPhase, open_elapsed: f32, close_progress: f32) -> f32 {
    match phase {
        PopupAnimPhase::Opening => {
            (0.24 + 0.76 * ease_out_cubic((open_elapsed / 0.050).clamp(0.0, 1.0))).clamp(0.0, 1.0)
        }
        PopupAnimPhase::Open => 1.0,
        PopupAnimPhase::Closing => (1.0 - close_progress.clamp(0.0, 1.0)).powf(1.25),
    }
}

/// Staggered bottom-to-top vertical micro-glide offset (bottom row settles first, top row last).
#[cfg_attr(not(windows), allow(dead_code))]
pub fn row_stagger_dy(
    idx: usize,
    row_count: usize,
    open_elapsed: f32,
    open_pos: f32,
    s: f32,
) -> f32 {
    let last_idx = row_count.saturating_sub(1);
    let order = last_idx.saturating_sub(idx) as f32;
    let delay = order * 0.007;
    let row_t = ((open_elapsed - delay) / 0.105).clamp(0.0, 1.0);
    let settle_glide = (1.0 - ease_out_cubic(row_t)) * (5.5 + order * 1.1) * s;
    let crest_breath = (1.0 - open_pos) * 0.28 * 14.0 * s;
    settle_glide + crest_breath
}

/// Computes two restrained liquid-silk ribbon colors derived from a purified dark-theme base accent.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn compute_halo_pair(base_accent_dark: Rgb, light_theme: bool) -> [Rgb; 2] {
    let (h, s, l) = rgb_to_hsl(base_accent_dark);
    if s < 0.12 {
        let s0 = s.min(0.10);
        let (l0, l1) = if light_theme { (0.66, 0.60) } else { (0.76, 0.82) };
        let c0 = hsl_to_rgb(h, s0, l0);
        let c1 = hsl_to_rgb(h, s0, l1);
        return [c0, c1];
    }

    let s0 = (s * 0.92).clamp(0.12, 0.72);
    let l0 = if light_theme {
        l.clamp(0.60, 0.72)
    } else {
        l.clamp(0.55, 0.68)
    };
    let c0 = hsl_to_rgb(h, s0, l0);

    let shift = 24.0 * ((s - 0.12) / 0.35).clamp(0.25, 1.0);
    let h1 = (h + shift).rem_euclid(360.0);
    let s1 = (s0 * 0.90).clamp(0.10, 0.66);
    let l1 = (l0 + 0.04).clamp(0.57, 0.75);
    let c1 = hsl_to_rgb(h1, s1, l1);
    [c0, c1]
}

/// Evaluates three tonal domain-warped liquid silk light ribbons (primary, neighbor, and soft lifted highlight)
/// with balanced clear-glass negative space.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn sample_liquid_silk_field(
    x: f32,
    y: f32,
    phase: f32,
    colors: &[(f32, f32, f32); 2],
    light_glass: bool,
) -> (f32, f32, f32, f32) {
    let wx = x
        + 0.28 * (2.1 * y + 0.29 * phase).sin()
        + 0.14 * (3.3 * y - 0.19 * phase + 1.3).cos();
    let wy = y
        + 0.26 * (1.9 * x - 0.23 * phase + 0.7).cos()
        + 0.13 * (3.1 * x + 0.31 * phase + 2.4).sin();

    // Ribbon 1: Primary theme/cover accent sweep
    let ang1 = 0.68 + 0.26 * (0.14 * phase).sin();
    let (s1, c1_ang) = ang1.sin_cos();
    let u1 = wx * c1_ang + wy * s1 - 0.19 * phase;
    let v1 = -wx * s1 + wy * c1_ang;
    let crest1 = (0.5 + 0.5 * (u1 * 5.0).sin()).powi(4);
    let along1 = 0.48 + 0.52 * (0.5 + 0.5 * (v1 * 2.4 + 0.34 * phase).sin());
    let breathe1 = 0.84 + 0.16 * (0.37 * phase + 0.5).sin();
    let w1 = crest1 * along1 * breathe1;

    // Ribbon 2: Analogous hue counter-sweep
    let ang2 = -0.82 + 0.22 * (0.11 * phase + 1.8).cos();
    let (s2, c2_ang) = ang2.sin_cos();
    let u2 = wx * c2_ang + wy * s2 + 0.15 * phase + 1.9;
    let v2 = -wx * s2 + wy * c2_ang;
    let crest2 = (0.5 + 0.5 * (u2 * 4.4).sin()).powi(5);
    let along2 = 0.42 + 0.58 * (0.5 + 0.5 * (v2 * 2.1 - 0.27 * phase + 2.2).cos());
    let breathe2 = 0.82 + 0.18 * (0.29 * phase + 2.1).cos();
    let w2 = crest2 * along2 * breathe2 * 0.78;

    // Ribbon 3: Tonal softly lifted pearl-highlight wisp at shallow diagonal angle
    let ang3 = 0.18 + 0.18 * (0.09 * phase + 3.4).sin();
    let (s3, c3_ang) = ang3.sin_cos();
    let u3 = wx * c3_ang + wy * s3 - 0.12 * phase + 3.7;
    let v3 = -wx * s3 + wy * c3_ang;
    let crest3 = (0.5 + 0.5 * (u3 * 3.8).sin()).powi(6);
    let along3 = 0.38 + 0.62 * (0.5 + 0.5 * (v3 * 1.9 + 0.22 * phase + 4.1).sin());
    let breathe3 = 0.78 + 0.22 * (0.23 * phase + 3.8).cos();
    let w3 = crest3 * along3 * breathe3 * 0.66;

    let w_sum = w1 + w2 + w3;
    if w_sum <= 1e-4 {
        return (0.0, 0.0, 0.0, 0.0);
    }
    let (c0, c1) = (colors[0], colors[1]);
    let lift = if light_glass { 0.52 } else { 0.42 };
    let mid_r = (c0.0 + c1.0) * 0.5;
    let mid_g = (c0.1 + c1.1) * 0.5;
    let mid_b = (c0.2 + c1.2) * 0.5;
    let c2 = (
        mid_r + (1.0 - mid_r) * lift,
        mid_g + (1.0 - mid_g) * lift,
        mid_b + (1.0 - mid_b) * lift,
    );

    let r = ((c0.0 * w1 + c1.0 * w2 + c2.0 * w3) / w_sum).clamp(0.0, 1.0);
    let g = ((c0.1 * w1 + c1.1 * w2 + c2.1 * w3) / w_sum).clamp(0.0, 1.0);
    let b = ((c0.2 * w1 + c1.2 * w2 + c2.2 * w3) / w_sum).clamp(0.0, 1.0);

    let peak_a = if light_glass { 0.21 } else { 0.27 };
    let raw_t = w_sum.min(1.65);
    let alpha = (peak_a * (raw_t / (0.62 + 0.32 * raw_t))).clamp(0.0, peak_a);
    (r, g, b, alpha)
}

/// Accumulates raw `WM_MOUSEWHEEL` deltas across precision touchpad ticks and returns signed list scroll steps.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn accumulate_wheel_scroll(accum: &mut i32, delta: i32) -> i32 {
    if delta == 0 {
        return 0;
    }
    if *accum != 0 && (*accum > 0) != (delta > 0) {
        *accum = 0;
    }
    *accum = accum.saturating_add(delta);
    let notches = *accum / 120;
    *accum -= notches * 120;
    -notches
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spring_open_reaches_crest_is_framerate_independent_and_settles() {
        assert!((ease_out_cubic(0.0) - 0.0).abs() < 1e-5);
        assert!((ease_out_cubic(1.0) - 1.0).abs() < 1e-5);
        assert!((ease_retract(0.0) - 0.0).abs() < 1e-5);
        assert!((ease_retract(1.0) - 1.0).abs() < 1e-5);
        assert!((window_master_alpha(PopupAnimPhase::Opening, 0.0, 0.0) - 0.24).abs() < 1e-5);
        assert!((window_master_alpha(PopupAnimPhase::Opening, 0.050, 0.0) - 1.0).abs() < 1e-5);
        assert!((window_master_alpha(PopupAnimPhase::Open, 1.0, 0.0) - 1.0).abs() < 1e-5);
        assert!((window_master_alpha(PopupAnimPhase::Closing, 1.0, 1.0) - 0.0).abs() < 1e-5);

        let mut s_10ms = Spring1D::new(0.14, 2.8);
        let mut s_25ms = Spring1D::new(0.14, 2.8);
        let mut peak_pos = s_10ms.pos;
        let mut max_dt_diff = 0.0f32;
        for _step in 1..=5 {
            for _ in 0..5 {
                s_10ms.step(1.0, OPEN_SPRING_STIFFNESS, OPEN_SPRING_DAMPING, 0.010);
                peak_pos = peak_pos.max(s_10ms.pos);
            }
            for _ in 0..2 {
                s_25ms.step(1.0, OPEN_SPRING_STIFFNESS, OPEN_SPRING_DAMPING, 0.025);
            }
            max_dt_diff = max_dt_diff.max((s_10ms.pos - s_25ms.pos).abs());
        }
        assert!((1.02..=1.06).contains(&peak_pos), "unexpected crest: {peak_pos}");
        assert!(max_dt_diff < 0.01, "dt drift too large: {max_dt_diff}");
        assert!(s_10ms.is_settled(1.0, 0.008, 0.20));
        for _ in 0..15 {
            s_10ms.step(1.0, OPEN_SPRING_STIFFNESS, OPEN_SPRING_DAMPING, 0.010);
        }
        assert!(s_10ms.is_settled(1.0, 0.0015, 0.015));

        let dy_top = row_stagger_dy(0, 3, 0.025, 0.65, 1.0);
        let dy_bot = row_stagger_dy(2, 3, 0.025, 0.65, 1.0);
        assert!(dy_top > dy_bot);
        assert!(row_stagger_dy(0, 3, 0.16, 1.0, 1.0).abs() < 1e-4);
    }

    #[test]
    fn halo_pair_preserves_monochrome_and_silk_stays_sparse() {
        let gray = Rgb::new(128, 128, 128);
        let vivid = Rgb::new(72, 195, 220);
        for light in [false, true] {
            for c in compute_halo_pair(gray, light) {
                let (_, s, l) = rgb_to_hsl(c);
                assert!(s <= 0.12);
                assert!((0.55..=0.85).contains(&l));
            }
            for c in compute_halo_pair(vivid, light) {
                let (_, s, l) = rgb_to_hsl(c);
                assert!((0.40..=0.74).contains(&s));
                assert!((0.52..=0.78).contains(&l));
            }
        }

        let colors = [(0.25, 0.65, 0.80), (0.55, 0.45, 0.85)];
        for light_glass in [false, true] {
            let cap = if light_glass { 0.211 } else { 0.271 };
            let mut max_alpha = 0.0f32;
            let mut quiet = 0usize;
            let mut total = 0usize;
            for iy in 0..48 {
                let y = (iy as f32) / 48.0 * 1.4;
                for ix in 0..64 {
                    let x = (ix as f32) / 64.0 * 2.0;
                    let (_, _, _, a) = sample_liquid_silk_field(x, y, 1.25, &colors, light_glass);
                    assert!(a >= 0.0 && a <= cap);
                    max_alpha = max_alpha.max(a);
                    if a < 0.055 {
                        quiet += 1;
                    }
                    total += 1;
                }
            }
            assert!((quiet as f32) / (total as f32) >= 0.33);
            assert!(max_alpha >= 0.14);
        }
    }

    #[test]
    fn glass_surface_style_round_trips_and_recipes_scale_harmoniously() {
        for style in [
            GlassSurfaceStyle::Default,
            GlassSurfaceStyle::Transparent,
            GlassSurfaceStyle::Blur,
            GlassSurfaceStyle::Acrylic,
            GlassSurfaceStyle::LiquidGlass,
        ] {
            assert_eq!(GlassSurfaceStyle::from_u8(style.to_u8()), style);
        }
        assert_eq!(GlassSurfaceStyle::from_u8(255), GlassSurfaceStyle::LiquidGlass);
        assert!((rgb_luma(Rgb::new(0, 0, 0)) - 0.0).abs() < 1e-5);
        assert!((rgb_luma(Rgb::new(255, 255, 255)) - 1.0).abs() < 1e-5);

        for light in [false, true] {
            let def = popup_glass_recipe(GlassSurfaceStyle::Default, light, 0.5);
            let acr = popup_glass_recipe(GlassSurfaceStyle::Acrylic, light, 0.5);
            let blur = popup_glass_recipe(GlassSurfaceStyle::Blur, light, 0.5);
            let lg = popup_glass_recipe(GlassSurfaceStyle::LiquidGlass, light, 0.5);
            let trans = popup_glass_recipe(GlassSurfaceStyle::Transparent, light, 0.5);

            // Monotonic opacity progression: Default > Acrylic > Blur > LiquidGlass > Transparent
            assert!(def.tint_alpha > acr.tint_alpha);
            assert!(acr.tint_alpha > blur.tint_alpha);
            assert!(blur.tint_alpha > lg.tint_alpha);
            assert!(lg.tint_alpha > trans.tint_alpha);

            // Material accents stay isolated to their respective styles
            assert!(lg.silk_alpha > 0.5);
            assert_eq!(def.silk_alpha, 0.0);
            assert_eq!(acr.silk_alpha, 0.0);
            assert_eq!(blur.silk_alpha, 0.0);
            assert_eq!(trans.silk_alpha, 0.0);

            assert!(acr.grain_max_alpha > blur.grain_max_alpha);
            assert!(blur.grain_max_alpha > 0);
            assert_eq!(lg.grain_max_alpha, 0);
            assert_eq!(trans.grain_max_alpha, 0);

            // Base tint stays strictly neutral (no cold blue cast) across every style and theme
            for r in [def, acr, blur, lg, trans] {
                assert!(
                    (r.tint_rgb.2 - r.tint_rgb.0).abs() <= 3.0 / 255.0,
                    "tint_rgb {:?} should stay neutral without blue cast",
                    r.tint_rgb
                );
            }
        }

        // High-luma backdrop softly reinforces dark-theme tint and edge definition to preserve white text legibility
        let lg_dark_on_dark = popup_glass_recipe(GlassSurfaceStyle::LiquidGlass, false, 0.15);
        let lg_dark_on_white = popup_glass_recipe(GlassSurfaceStyle::LiquidGlass, false, 0.96);
        assert!(lg_dark_on_white.tint_alpha > lg_dark_on_dark.tint_alpha);
        assert!(lg_dark_on_white.rim_dark_sep_alpha > lg_dark_on_dark.rim_dark_sep_alpha);

        // Low-luma backdrop softly reinforces light-theme tint to preserve dark text legibility
        let lg_light_on_white = popup_glass_recipe(GlassSurfaceStyle::LiquidGlass, true, 0.90);
        let lg_light_on_black = popup_glass_recipe(GlassSurfaceStyle::LiquidGlass, true, 0.05);
        assert!(lg_light_on_black.tint_alpha > lg_light_on_white.tint_alpha);
    }

    #[test]
    fn backdrop_opacity_lift_moves_ten_percent_toward_opaque_monotonically() {
        assert!((lift_backdrop_alpha(-0.5) - 0.10).abs() < 1e-6);
        assert!((lift_backdrop_alpha(0.0) - 0.10).abs() < 1e-6);
        assert!((lift_backdrop_alpha(0.26) - 0.334).abs() < 1e-5);
        assert!((lift_backdrop_alpha(0.53) - 0.577).abs() < 1e-5);
        assert!((lift_backdrop_alpha(1.0) - 1.0).abs() < 1e-6);
        assert!((lift_backdrop_alpha(1.5) - 1.0).abs() < 1e-6);
        assert!(lift_backdrop_alpha(0.45) > lift_backdrop_alpha(0.30));

        assert_eq!(lift_argb_alpha(0x00_123456), 0x1A_123456);
        assert_eq!(lift_argb_alpha(0xFF_ABCDEF), 0xFF_ABCDEF);
        assert_eq!(lift_argb_alpha(0x52_141822), 0x63_141822);
        assert_eq!(lift_argb_alpha(0x5C_141822), 0x6C_141822);
        assert_eq!(lift_argb_alpha(0x60_141822), 0x70_141822);
        assert_eq!(lift_argb_alpha(0x49_1A1D24), 0x5B_1A1D24);
        assert_eq!(lift_argb_alpha(0x4E_141A26), 0x60_141A26);
        assert!((lift_argb_alpha(0x60_000000) >> 24) > (lift_argb_alpha(0x40_000000) >> 24));
    }

    #[test]
    fn shared_popup_geometry_preserves_anchors_and_safe_bounds() {
        let target_primary = RectF::new(14.0, 20.0, 168.0, 240.0);
        let opening = animated_primary_rect(target_primary, PopupAnimPhase::Opening, 0.35, 0.0, 1.0);
        assert!((opening.bottom - target_primary.bottom).abs() < 1e-4);
        assert!(opening.top >= 4.0 && opening.height() >= 42.0);
        let closing = animated_primary_rect(target_primary, PopupAnimPhase::Closing, 1.0, 0.6, 1.0);
        assert!((closing.bottom - target_primary.bottom).abs() < 1e-4);

        let (top_fit, bot_fit) = compute_submenu_vertical_span(100.0, 8.0, 110.0, 16.0, 240.0);
        assert_eq!(top_fit, 92.0);
        assert_eq!(bot_fit, 202.0);
        let (top_up, bot_up) = compute_submenu_vertical_span(190.0, 8.0, 120.0, 16.0, 240.0);
        assert_eq!(bot_up, 240.0);
        assert_eq!(top_up, 120.0);

        let sub = RectF::new(168.0, 92.0, 420.0, 240.0);
        let row = RectF::new(22.0, 100.0, 160.0, 134.0);
        let bud = sub_bud_start_rect(sub, row, 168.0, false, 1.0);
        assert!((bud.left - 168.0).abs() < 1e-4);
        assert!(bud.width() >= 40.0 && bud.height() >= 44.0);

        let clamped = clamp_sub_spring_rect(-20.0, 320.0, 300.0, sub, 168.0, 240.0, 460.0, false, 1.0);
        assert!(clamped.top >= 4.0 && clamped.bottom <= 240.0 && clamped.width() >= 40.0);

        let close_sub = animated_closing_sub_rect(
            sub,
            RectF::new(14.0, 80.0, 168.0, 240.0),
            168.0,
            240.0,
            false,
            0.5,
            1.0,
        );
        assert!(close_sub.height() >= 42.0 && close_sub.bottom <= 240.0);
    }

    #[test]
    fn wheel_scroll_accumulator_handles_standard_precision_and_reversals() {
        let mut accum = 0i32;
        assert_eq!(accumulate_wheel_scroll(&mut accum, 0), 0);
        assert_eq!(accumulate_wheel_scroll(&mut accum, 120), -1);
        assert_eq!(accum, 0);
        assert_eq!(accumulate_wheel_scroll(&mut accum, -120), 1);
        assert_eq!(accum, 0);

        // Precision touchpad small deltas accumulate until 120.
        assert_eq!(accumulate_wheel_scroll(&mut accum, 40), 0);
        assert_eq!(accumulate_wheel_scroll(&mut accum, 40), 0);
        assert_eq!(accumulate_wheel_scroll(&mut accum, 40), -1);
        assert_eq!(accum, 0);

        // Remainder is preserved across ticks and cleared cleanly on direction reversal.
        assert_eq!(accumulate_wheel_scroll(&mut accum, 50), 0);
        assert_eq!(accumulate_wheel_scroll(&mut accum, 50), 0);
        assert_eq!(accumulate_wheel_scroll(&mut accum, 50), -1);
        assert_eq!(accum, 30);
        assert_eq!(accumulate_wheel_scroll(&mut accum, -40), 0);
        assert_eq!(accum, -40);
        assert_eq!(accumulate_wheel_scroll(&mut accum, -80), 1);
        assert_eq!(accum, 0);
    }
}
