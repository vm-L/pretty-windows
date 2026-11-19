#[cfg(windows)]
pub mod glass;

use crate::color::{hsl_to_rgb, rgb_to_hsl, Rgb};

/// Outer transparent padding reserved around Liquid Glass popup cards for 3-pass ambient shadows (DIP).
#[cfg_attr(not(windows), allow(dead_code))]
pub const SHADOW_PAD_DIP: f32 = 14.0;
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

/// Lifecycle phase of a Liquid Glass popup card.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(windows), allow(dead_code))]
pub enum PopupAnimPhase {
    Opening,
    Open,
    Closing,
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

/// Evaluates two restrained, domain-warped liquid silk light ribbons with wide clear-glass negative space.
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

    let ang1 = 0.68 + 0.26 * (0.14 * phase).sin();
    let (s1, c1_ang) = ang1.sin_cos();
    let u1 = wx * c1_ang + wy * s1 - 0.19 * phase;
    let v1 = -wx * s1 + wy * c1_ang;
    let crest1 = (0.5 + 0.5 * (u1 * 5.0).sin()).powi(5);
    let along1 = 0.45 + 0.55 * (0.5 + 0.5 * (v1 * 2.4 + 0.34 * phase).sin());
    let breathe1 = 0.82 + 0.18 * (0.37 * phase + 0.5).sin();
    let w1 = crest1 * along1 * breathe1;

    let ang2 = -0.82 + 0.22 * (0.11 * phase + 1.8).cos();
    let (s2, c2_ang) = ang2.sin_cos();
    let u2 = wx * c2_ang + wy * s2 + 0.15 * phase + 1.9;
    let v2 = -wx * s2 + wy * c2_ang;
    let crest2 = (0.5 + 0.5 * (u2 * 4.4).sin()).powi(6);
    let along2 = 0.40 + 0.60 * (0.5 + 0.5 * (v2 * 2.1 - 0.27 * phase + 2.2).cos());
    let breathe2 = 0.80 + 0.20 * (0.29 * phase + 2.1).cos();
    let w2 = crest2 * along2 * breathe2 * 0.68;

    let w_sum = w1 + w2;
    if w_sum <= 1e-4 {
        return (0.0, 0.0, 0.0, 0.0);
    }
    let (c0, c1) = (colors[0], colors[1]);
    let r = ((c0.0 * w1 + c1.0 * w2) / w_sum).clamp(0.0, 1.0);
    let g = ((c0.1 * w1 + c1.1 * w2) / w_sum).clamp(0.0, 1.0);
    let b = ((c0.2 * w1 + c1.2 * w2) / w_sum).clamp(0.0, 1.0);

    let peak_a = if light_glass { 0.095 } else { 0.125 };
    let raw_t = w_sum.min(1.2);
    let alpha = (peak_a * (raw_t / (1.0 + 0.30 * raw_t))).clamp(0.0, peak_a);
    (r, g, b, alpha)
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
            let cap = if light_glass { 0.096 } else { 0.126 };
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
                    if a < 0.025 {
                        quiet += 1;
                    }
                    total += 1;
                }
            }
            assert!((quiet as f32) / (total as f32) >= 0.45);
            assert!(max_alpha >= 0.04);
        }
    }
}
