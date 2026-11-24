/// Neutral fallback base color when screen sampling is unavailable (never forces Win11 blue).
pub const NEUTRAL_BACKDROP_FALLBACK: Rgb = Rgb::new(128, 128, 130);

/// RGB color representation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    pub fn to_normalized(self) -> (f32, f32, f32) {
        (
            f32::from(self.r) / 255.0,
            f32::from(self.g) / 255.0,
            f32::from(self.b) / 255.0,
        )
    }
}

/// Converts sRGB (0..255) to HSL (`h` in `[0, 360)`, `s` and `l` in `[0, 1]`).
pub fn rgb_to_hsl(rgb: Rgb) -> (f32, f32, f32) {
    let (r, g, b) = rgb.to_normalized();
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = 0.5 * (max + min);
    let delta = max - min;

    if delta <= 1e-5 {
        return (0.0, 0.0, l);
    }

    let s = if l < 0.5 {
        delta / (max + min)
    } else {
        delta / (2.0 - max - min)
    };

    let mut h = if (max - r).abs() < 1e-6 {
        (g - b) / delta + if g < b { 6.0 } else { 0.0 }
    } else if (max - g).abs() < 1e-6 {
        (b - r) / delta + 2.0
    } else {
        (r - g) / delta + 4.0
    };
    h *= 60.0;
    (h.rem_euclid(360.0), s.clamp(0.0, 1.0), l.clamp(0.0, 1.0))
}

/// Converts HSL (`h` in `[0, 360)`, `s` and `l` in `[0, 1]`) back to sRGB.
pub fn hsl_to_rgb(h: f32, s: f32, l: f32) -> Rgb {
    let s = s.clamp(0.0, 1.0);
    let l = l.clamp(0.0, 1.0);
    if s <= 1e-5 {
        let v = (l * 255.0).round() as u8;
        return Rgb::new(v, v, v);
    }

    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    let hk = h.rem_euclid(360.0) / 360.0;

    let hue_to_rgb = |mut t: f32| -> f32 {
        if t < 0.0 {
            t += 1.0;
        }
        if t > 1.0 {
            t -= 1.0;
        }
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    };

    Rgb::new(
        (hue_to_rgb(hk + 1.0 / 3.0) * 255.0).round().clamp(0.0, 255.0) as u8,
        (hue_to_rgb(hk) * 255.0).round().clamp(0.0, 255.0) as u8,
        (hue_to_rgb(hk - 1.0 / 3.0) * 255.0).round().clamp(0.0, 255.0) as u8,
    )
}

/// Normalizes an accent color so spectrum bars stay vibrant and readable on dark or light taskbars.
pub fn adjust_accent_for_theme(color: Rgb, light_taskbar: bool) -> Rgb {
    let (h, s, l) = rgb_to_hsl(color);
    if s < 0.08 {
        let target_l = if light_taskbar { 0.37 } else { 0.76 };
        return hsl_to_rgb(h, s, target_l);
    }

    let min_s = s.min(0.20);
    let (target_s, target_l) = if light_taskbar {
        ((s * 1.08).clamp(min_s, 0.82), l.clamp(0.32, 0.48))
    } else {
        ((s * 1.10).clamp(min_s, 0.84), l.clamp(0.58, 0.74))
    };
    hsl_to_rgb(h, target_s, target_l)
}

/// Extracts a perceptually faithful accent color from raw BGRA8 album artwork pixels.
/// Preserves monochrome/grayscale and low-saturation covers instead of forcing a blue fallback.
/// Falls back to `fallback` only when the image is empty or completely transparent.
pub fn extract_accent_from_bgra(
    bgra: &[u8],
    width: u32,
    height: u32,
    light_taskbar: bool,
    fallback: Rgb,
) -> Rgb {
    const BUCKETS: usize = 24;
    let Some(total_pixels) = (width as usize).checked_mul(height as usize) else {
        return adjust_accent_for_theme(fallback, light_taskbar);
    };
    let Some(required_len) = total_pixels.checked_mul(4) else {
        return adjust_accent_for_theme(fallback, light_taskbar);
    };
    if total_pixels == 0 || bgra.len() < required_len {
        return adjust_accent_for_theme(fallback, light_taskbar);
    }

    let w_usize = width as usize;
    let stride = (total_pixels / 4096).max(1);
    let mut bucket_weight = [0.0f32; BUCKETS];
    let mut bucket_r = [0.0f32; BUCKETS];
    let mut bucket_g = [0.0f32; BUCKETS];
    let mut bucket_b = [0.0f32; BUCKETS];

    let mut opaque_count = 0usize;
    let mut chromatic_count = 0usize;
    let mut sum_nr = 0.0f32;
    let mut sum_ng = 0.0f32;
    let mut sum_nb = 0.0f32;
    let mut sum_nw = 0.0f32;

    let mut idx = 0usize;
    while idx < total_pixels {
        let off = idx * 4;
        let b = bgra[off];
        let g = bgra[off + 1];
        let r = bgra[off + 2];
        let a = bgra[off + 3];

        if a >= 128 {
            opaque_count += 1;
            let px = idx % w_usize;
            let py = idx / w_usize;
            let u = ((px as f32) + 0.5) / (width as f32);
            let v = ((py as f32) + 0.5) / (height as f32);
            let spatial_w = if (0.15..=0.85).contains(&u) && (0.15..=0.85).contains(&v) {
                1.0f32
            } else {
                0.65f32
            };

            let nr = f32::from(r) / 255.0;
            let ng = f32::from(g) / 255.0;
            let nb = f32::from(b) / 255.0;
            let max_c = nr.max(ng).max(nb);
            let min_c = nr.min(ng).min(nb);
            let chroma = max_c - min_c;
            let l = 0.5 * (max_c + min_c);

            let tone_w = (0.15 + 0.85 * (1.0 - (2.0 * l - 1.0).powi(2)).max(0.0)) * spatial_w;
            sum_nr += f32::from(r) * tone_w;
            sum_ng += f32::from(g) * tone_w;
            sum_nb += f32::from(b) * tone_w;
            sum_nw += tone_w;

            if chroma >= 0.11 && (0.08..=0.92).contains(&l) {
                let (h, _, _) = rgb_to_hsl(Rgb::new(r, g, b));
                let bucket = ((h / 360.0) * (BUCKETS as f32)).floor() as usize % BUCKETS;
                let mid_l_bonus = (1.0 - (l - 0.5).abs() * 1.1).max(0.15);
                let weight = chroma * chroma * mid_l_bonus * spatial_w;
                bucket_weight[bucket] += weight;
                bucket_r[bucket] += f32::from(r) * weight;
                bucket_g[bucket] += f32::from(g) * weight;
                bucket_b[bucket] += f32::from(b) * weight;
                chromatic_count += 1;
            }
        }
        idx += stride;
    }

    if opaque_count == 0 {
        return adjust_accent_for_theme(fallback, light_taskbar);
    }

    let chromatic_ratio = (chromatic_count as f32) / (opaque_count as f32);
    let mut best_bucket = None;
    let mut best_score = 0.001f32;
    for i in 0..BUCKETS {
        let prev = (i + BUCKETS - 1) % BUCKETS;
        let next = (i + 1) % BUCKETS;
        let score = 0.25 * bucket_weight[prev] + bucket_weight[i] + 0.25 * bucket_weight[next];
        if bucket_weight[i] > 0.0 && score > best_score {
            best_score = score;
            best_bucket = Some(i);
        }
    }

    if chromatic_ratio < 0.04 || best_bucket.is_none() {
        let neutral_rgb = Rgb::new(
            (sum_nr / sum_nw.max(1e-4)).round().clamp(0.0, 255.0) as u8,
            (sum_ng / sum_nw.max(1e-4)).round().clamp(0.0, 255.0) as u8,
            (sum_nb / sum_nw.max(1e-4)).round().clamp(0.0, 255.0) as u8,
        );
        let (nh, ns, _) = rgb_to_hsl(neutral_rgb);
        let target_l = if light_taskbar { 0.37 } else { 0.76 };
        return hsl_to_rgb(nh, ns.min(0.12), target_l);
    }

    let best = best_bucket.unwrap();
    let prev = (best + BUCKETS - 1) % BUCKETS;
    let next = (best + 1) % BUCKETS;
    let mut cluster_w = 0.0f32;
    let mut cluster_r = 0.0f32;
    let mut cluster_g = 0.0f32;
    let mut cluster_b = 0.0f32;
    for b_idx in [prev, best, next] {
        cluster_w += bucket_weight[b_idx];
        cluster_r += bucket_r[b_idx];
        cluster_g += bucket_g[b_idx];
        cluster_b += bucket_b[b_idx];
    }

    let mut dominant = Rgb::new(
        (cluster_r / cluster_w).round().clamp(0.0, 255.0) as u8,
        (cluster_g / cluster_w).round().clamp(0.0, 255.0) as u8,
        (cluster_b / cluster_w).round().clamp(0.0, 255.0) as u8,
    );
    if chromatic_ratio < 0.16 {
        let presence = (chromatic_ratio / 0.16).clamp(0.45, 1.0);
        let (dh, ds, dl) = rgb_to_hsl(dominant);
        dominant = hsl_to_rgb(dh, ds * presence, dl);
    }

    adjust_accent_for_theme(dominant, light_taskbar)
}

/// Computes the mean opaque RGB color of a BGRA8 image (used to sample popup/taskbar backdrops).
pub fn extract_backdrop_base_rgb(bgra: &[u8], width: usize, height: usize) -> Option<Rgb> {
    let total_pixels = width.checked_mul(height)?;
    let byte_len = total_pixels.checked_mul(4)?;
    if total_pixels == 0 || bgra.len() < byte_len {
        return None;
    }
    let mut sum_r = 0u64;
    let mut sum_g = 0u64;
    let mut sum_b = 0u64;
    let mut count = 0u64;
    for px in bgra[..byte_len].chunks_exact(4) {
        if px[3] >= 128 {
            sum_b += u64::from(px[0]);
            sum_g += u64::from(px[1]);
            sum_r += u64::from(px[2]);
            count += 1;
        }
    }
    if count == 0 {
        return None;
    }
    Some(Rgb::new(
        (sum_r / count).min(255) as u8,
        (sum_g / count).min(255) as u8,
        (sum_b / count).min(255) as u8,
    ))
}

/// Derives a unified theme accent directly from a backdrop color by preserving its hue & natural
/// saturation while lifting lightness for crisp legibility (never falling back to blue).
pub fn derive_background_accent(bg: Rgb, light_glass: bool) -> Rgb {
    let (h, s, l) = rgb_to_hsl(bg);
    let max_c = bg.r.max(bg.g).max(bg.b);
    let min_c = bg.r.min(bg.g).min(bg.b);
    let chroma = f32::from(max_c - min_c) / 255.0;
    let chroma_factor = ((chroma - 0.025) / 0.055).clamp(0.0, 1.0);
    let s_out = if s < 0.04 || chroma_factor <= 0.0 {
        0.0
    } else {
        (s * chroma_factor).min(0.72)
    };
    let target_l = if light_glass {
        (l + 0.20).clamp(0.34, 0.48)
    } else {
        (l + 0.34).clamp(0.70, 0.86)
    };
    hsl_to_rgb(h, s_out, target_l)
}

#[inline]
fn rgb_luma_01(c: Rgb) -> f32 {
    (0.2126 * f32::from(c.r) + 0.7152 * f32::from(c.g) + 0.0722 * f32::from(c.b)) / 255.0
}

#[inline]
fn rgb_chroma_01(c: Rgb) -> f32 {
    let max_c = c.r.max(c.g).max(c.b);
    let min_c = c.r.min(c.g).min(c.b);
    f32::from(max_c - min_c) / 255.0
}

#[cfg_attr(not(test), allow(dead_code))]
fn is_in_ui_accent_deadzone(c: Rgb, light_theme: bool) -> bool {
    let y = rgb_luma_01(c);
    let chroma = rgb_chroma_01(c);
    if light_theme {
        if y > 0.60 {
            return true;
        }
        let clamped_y = y.clamp(0.54, 0.96);
        let dy = (y - clamped_y) / 0.24;
        let dc = chroma / 0.42;
        (dy * dy + dc * dc) < 1.0
    } else {
        if y < 0.36 {
            return true;
        }
        let clamped_y = y.clamp(0.16, 0.62);
        let dy = (y - clamped_y) / 0.22;
        let dc = chroma / 0.42;
        (dy * dy + dc * dc) < 1.0
    }
}

/// Pushes an accent color outside a static theme-aware glass similarity deadzone while
/// preserving its hue (or preserving neutral low saturation as crisp pearl-white / charcoal).
#[cfg_attr(not(windows), allow(dead_code))]
pub fn enforce_ui_accent_deadzone(color: Rgb, light_theme: bool) -> Rgb {
    if !is_in_ui_accent_deadzone(color, light_theme) {
        return color;
    }

    let (h, s, l) = rgb_to_hsl(color);
    let chroma = rgb_chroma_01(color);

    if s < 0.10 || chroma < 0.06 {
        let s_out = s.min(0.08);
        return if light_theme {
            hsl_to_rgb(h, s_out, l.min(0.22))
        } else {
            hsl_to_rgb(h, s_out, l.max(0.89))
        };
    }

    let move_toward = |cur: f32, target: f32, step: f32| -> f32 {
        if cur < target {
            (cur + step).min(target)
        } else {
            (cur - step).max(target)
        }
    };

    let mut cur_s = s;
    let mut cur_l = l;
    let mut cand = color;
    for _ in 0..48 {
        let y = rgb_luma_01(cand);
        if light_theme {
            if y > 0.60 {
                cur_l = (cur_l - 0.025).max(0.24);
                cur_s = (cur_s + 0.015).min(0.84);
            } else {
                cur_s = (cur_s + 0.035).min(0.84);
                cur_l = move_toward(cur_l, 0.34, 0.014);
            }
        } else if y < 0.36 {
            cur_l = (cur_l + 0.025).min(0.76);
            cur_s = (cur_s + 0.015).min(0.84);
        } else {
            cur_s = (cur_s + 0.035).min(0.84);
            cur_l = move_toward(cur_l, 0.68, 0.014);
        }
        cand = hsl_to_rgb(h, cur_s, cur_l);
        if !is_in_ui_accent_deadzone(cand, light_theme) {
            return cand;
        }
    }
    cand
}

/// Applies a fast 3-pass separable box blur (approximating Gaussian blur) on BGRA8 pixels.
pub fn gaussian_blur_bgra(bgra: &[u8], width: usize, height: usize, radius: usize) -> Vec<u8> {
    let Some(byte_len) = width.checked_mul(height).and_then(|px| px.checked_mul(4)) else {
        return bgra.to_vec();
    };
    if byte_len == 0 || radius == 0 || bgra.len() < byte_len {
        return bgra.to_vec();
    }

    let mut src = bgra[..byte_len].to_vec();
    let mut tmp = vec![0u8; src.len()];

    for _ in 0..3 {
        box_blur_h(&src, &mut tmp, width, height, radius);
        box_blur_v(&tmp, &mut src, width, height, radius);
    }
    src
}

/// Builds a heavily blurred, vibrancy-boosted, opaque BGRA8 cover atmosphere buffer
/// suitable for soft low-opacity glass background washes (preserving neutral grayscale covers).
pub fn build_ambient_cover_bgra(bgra: &[u8], width: usize, height: usize) -> Vec<u8> {
    let mut out = gaussian_blur_bgra(bgra, width, height, 11);
    for px in out.chunks_exact_mut(4) {
        let (h, s, l) = rgb_to_hsl(Rgb::new(px[2], px[1], px[0]));
        let s_out = if s < 0.08 {
            s
        } else {
            (s * 1.28).clamp(0.0, 0.90)
        };
        let l_out = l.clamp(0.18, 0.78);
        let rgb = hsl_to_rgb(h, s_out, l_out);
        px[0] = rgb.b;
        px[1] = rgb.g;
        px[2] = rgb.r;
        px[3] = 255;
    }
    out
}

fn box_blur_h(src: &[u8], dst: &mut [u8], width: usize, height: usize, radius: usize) {
    let win = (radius * 2 + 1) as f32;
    let inv_win = 1.0 / win;
    let w_last = width - 1;

    for y in 0..height {
        let row = y * width * 4;
        let first = row;
        let last = row + w_last * 4;

        let mut acc_b = f32::from(src[first]) * ((radius + 1) as f32);
        let mut acc_g = f32::from(src[first + 1]) * ((radius + 1) as f32);
        let mut acc_r = f32::from(src[first + 2]) * ((radius + 1) as f32);
        let mut acc_a = f32::from(src[first + 3]) * ((radius + 1) as f32);

        for x in 1..=radius {
            let cx = x.min(w_last);
            let p = row + cx * 4;
            acc_b += f32::from(src[p]);
            acc_g += f32::from(src[p + 1]);
            acc_r += f32::from(src[p + 2]);
            acc_a += f32::from(src[p + 3]);
        }

        for x in 0..width {
            let out = row + x * 4;
            dst[out] = (acc_b * inv_win).round().clamp(0.0, 255.0) as u8;
            dst[out + 1] = (acc_g * inv_win).round().clamp(0.0, 255.0) as u8;
            dst[out + 2] = (acc_r * inv_win).round().clamp(0.0, 255.0) as u8;
            dst[out + 3] = (acc_a * inv_win).round().clamp(0.0, 255.0) as u8;

            let add_x = (x + radius + 1).min(w_last);
            let sub_x = x.saturating_sub(radius);
            let add_p = row + add_x * 4;
            let sub_p = if x < radius { first } else { row + sub_x * 4 };
            let _ = last;

            acc_b += f32::from(src[add_p]) - f32::from(src[sub_p]);
            acc_g += f32::from(src[add_p + 1]) - f32::from(src[sub_p + 1]);
            acc_r += f32::from(src[add_p + 2]) - f32::from(src[sub_p + 2]);
            acc_a += f32::from(src[add_p + 3]) - f32::from(src[sub_p + 3]);
        }
    }
}

fn box_blur_v(src: &[u8], dst: &mut [u8], width: usize, height: usize, radius: usize) {
    let win = (radius * 2 + 1) as f32;
    let inv_win = 1.0 / win;
    let h_last = height - 1;
    let stride = width * 4;

    for x in 0..width {
        let col = x * 4;
        let first = col;

        let mut acc_b = f32::from(src[first]) * ((radius + 1) as f32);
        let mut acc_g = f32::from(src[first + 1]) * ((radius + 1) as f32);
        let mut acc_r = f32::from(src[first + 2]) * ((radius + 1) as f32);
        let mut acc_a = f32::from(src[first + 3]) * ((radius + 1) as f32);

        for y in 1..=radius {
            let cy = y.min(h_last);
            let p = cy * stride + col;
            acc_b += f32::from(src[p]);
            acc_g += f32::from(src[p + 1]);
            acc_r += f32::from(src[p + 2]);
            acc_a += f32::from(src[p + 3]);
        }

        for y in 0..height {
            let out = y * stride + col;
            dst[out] = (acc_b * inv_win).round().clamp(0.0, 255.0) as u8;
            dst[out + 1] = (acc_g * inv_win).round().clamp(0.0, 255.0) as u8;
            dst[out + 2] = (acc_r * inv_win).round().clamp(0.0, 255.0) as u8;
            dst[out + 3] = (acc_a * inv_win).round().clamp(0.0, 255.0) as u8;

            let add_y = (y + radius + 1).min(h_last);
            let sub_y = y.saturating_sub(radius);
            let add_p = add_y * stride + col;
            let sub_p = if y < radius { first } else { sub_y * stride + col };

            acc_b += f32::from(src[add_p]) - f32::from(src[sub_p]);
            acc_g += f32::from(src[add_p + 1]) - f32::from(src[sub_p + 1]);
            acc_r += f32::from(src[add_p + 2]) - f32::from(src[sub_p + 2]);
            acc_a += f32::from(src[add_p + 3]) - f32::from(src[sub_p + 3]);
        }
    }
}

/// Replaces pixels inside transient occluding rectangles (e.g. dismissing overflow flyouts/tooltips)
/// by smoothly interpolating from nearest unoccluded orthogonal neighbors along 4 cardinal rays,
/// falling back to the unoccluded mean RGB when an entire row/column is covered.
/// Returns `false` if the entire buffer is occluded (caller should use a neutral glass fallback).
#[cfg_attr(not(windows), allow(dead_code))]
pub fn inpaint_occluded_rects_bgra(
    bgra: &mut [u8],
    width: usize,
    height: usize,
    occluded_rects: &[(usize, usize, usize, usize)],
) -> bool {
    let Some(total) = width.checked_mul(height) else {
        return false;
    };
    let Some(byte_len) = total.checked_mul(4) else {
        return false;
    };
    if width == 0 || height == 0 || bgra.len() < byte_len {
        return false;
    }
    if occluded_rects.is_empty() {
        return true;
    }

    let mut mask = vec![false; total];
    let mut occluded_count = 0usize;
    for &(rx0, ry0, rx1, ry1) in occluded_rects {
        let x0 = rx0.min(width);
        let x1 = rx1.min(width);
        let y0 = ry0.min(height);
        let y1 = ry1.min(height);
        if x0 >= x1 || y0 >= y1 {
            continue;
        }
        for y in y0..y1 {
            let row_off = y * width;
            for x in x0..x1 {
                let idx = row_off + x;
                if !mask[idx] {
                    mask[idx] = true;
                    occluded_count += 1;
                }
            }
        }
    }

    if occluded_count == 0 {
        return true;
    }
    if occluded_count >= total {
        return false;
    }

    let mut sum_b = 0u64;
    let mut sum_g = 0u64;
    let mut sum_r = 0u64;
    let mut clean_count = 0u64;
    for (i, px) in bgra[..byte_len].chunks_exact(4).enumerate() {
        if !mask[i] {
            sum_b += u64::from(px[0]);
            sum_g += u64::from(px[1]);
            sum_r += u64::from(px[2]);
            clean_count += 1;
        }
    }
    let clean_denom = clean_count.max(1) as f32;
    let mean_b = (sum_b as f32) / clean_denom;
    let mean_g = (sum_g as f32) / clean_denom;
    let mean_r = (sum_r as f32) / clean_denom;

    let clean = bgra[..byte_len].to_vec();
    for y in 0..height {
        for x in 0..width {
            if !mask[y * width + x] {
                continue;
            }
            let mut acc_b = 0.0f32;
            let mut acc_g = 0.0f32;
            let mut acc_r = 0.0f32;
            let mut w_sum = 0.0f32;

            let mut accumulate_clean = |nx: usize, ny: usize, dist: usize| {
                let p = (ny * width + nx) * 4;
                let w = 1.0 / (dist.max(1) as f32);
                acc_b += f32::from(clean[p]) * w;
                acc_g += f32::from(clean[p + 1]) * w;
                acc_r += f32::from(clean[p + 2]) * w;
                w_sum += w;
            };

            for lx in (0..x).rev() {
                if !mask[y * width + lx] {
                    accumulate_clean(lx, y, x - lx);
                    break;
                }
            }
            for rx in (x + 1)..width {
                if !mask[y * width + rx] {
                    accumulate_clean(rx, y, rx - x);
                    break;
                }
            }
            for uy in (0..y).rev() {
                if !mask[uy * width + x] {
                    accumulate_clean(x, uy, y - uy);
                    break;
                }
            }
            for dy in (y + 1)..height {
                if !mask[dy * width + x] {
                    accumulate_clean(x, dy, dy - y);
                    break;
                }
            }

            let out = (y * width + x) * 4;
            if w_sum > 0.0 {
                bgra[out] = (acc_b / w_sum).round().clamp(0.0, 255.0) as u8;
                bgra[out + 1] = (acc_g / w_sum).round().clamp(0.0, 255.0) as u8;
                bgra[out + 2] = (acc_r / w_sum).round().clamp(0.0, 255.0) as u8;
            } else {
                bgra[out] = mean_b.round().clamp(0.0, 255.0) as u8;
                bgra[out + 1] = mean_g.round().clamp(0.0, 255.0) as u8;
                bgra[out + 2] = mean_r.round().clamp(0.0, 255.0) as u8;
            }
            bgra[out + 3] = 255;
        }
    }

    true
}

/// Adjusts saturation and brightness on straight BGRA pixels for vibrant liquid-glass backdrops.
#[allow(dead_code)]
pub fn boost_vibrance_bgra(bgra: &mut [u8], sat_mul: f32, luma_mul: f32) {
    for px in bgra.chunks_exact_mut(4) {
        let b = f32::from(px[0]);
        let g = f32::from(px[1]);
        let r = f32::from(px[2]);
        let luma = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        px[0] = ((luma + (b - luma) * sat_mul) * luma_mul).round().clamp(0.0, 255.0) as u8;
        px[1] = ((luma + (g - luma) * sat_mul) * luma_mul).round().clamp(0.0, 255.0) as u8;
        px[2] = ((luma + (r - luma) * sat_mul) * luma_mul).round().clamp(0.0, 255.0) as u8;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hsl_roundtrip() {
        let samples = [
            Rgb::new(235, 64, 52),
            Rgb::new(45, 196, 120),
            Rgb::new(66, 135, 245),
            Rgb::new(180, 90, 220),
        ];
        for c in samples {
            let (h, s, l) = rgb_to_hsl(c);
            let back = hsl_to_rgb(h, s, l);
            assert!((i16::from(c.r) - i16::from(back.r)).abs() <= 2);
            assert!((i16::from(c.g) - i16::from(back.g)).abs() <= 2);
            assert!((i16::from(c.b) - i16::from(back.b)).abs() <= 2);
        }
    }

    #[test]
    fn extract_accent_picks_dominant_hue_and_respects_theme() {
        // 8x8 vivid crimson-orange image (BGRA)
        let mut img = Vec::with_capacity(8 * 8 * 4);
        for _ in 0..(8 * 8) {
            img.extend_from_slice(&[35, 85, 230, 255]);
        }
        let fallback = Rgb::new(0, 120, 212);

        let dark_theme_color = extract_accent_from_bgra(&img, 8, 8, false, fallback);
        let (h_d, _, l_d) = rgb_to_hsl(dark_theme_color);
        assert!(
            !(35.0..=340.0).contains(&h_d),
            "expected warm hue, got {h_d}"
        );
        assert!((0.54..=0.76).contains(&l_d), "expected dark-theme lightness, got {l_d}");

        let light_theme_color = extract_accent_from_bgra(&img, 8, 8, true, fallback);
        let (_, _, l_l) = rgb_to_hsl(light_theme_color);
        assert!((0.30..=0.52).contains(&l_l), "expected light-theme lightness, got {l_l}");
    }

    #[test]
    fn extract_accent_monochrome_and_shadow_noise_stay_neutral_gray() {
        let blue_fallback = Rgb::new(0, 120, 212);

        // 1. Pure monochrome gray square
        let mut gray = Vec::with_capacity(8 * 8 * 4);
        for _ in 0..64 {
            gray.extend_from_slice(&[120, 120, 120, 255]);
        }
        for light in [false, true] {
            let out = extract_accent_from_bgra(&gray, 8, 8, light, blue_fallback);
            let (_, s, l) = rgb_to_hsl(out);
            assert!(s <= 0.12, "pure gray must remain neutral, got s={s} ({out:?})");
            if light {
                assert!((0.30..=0.45).contains(&l));
            } else {
                assert!((0.68..=0.84).contains(&l));
            }
        }

        // 2. B&W vinyl album cover simulation (ivory background + bluish dark-shadow JPEG noise + warm gray subject)
        let mut vinyl = Vec::with_capacity(16 * 16 * 4);
        for y in 0..16 {
            for x in 0..16 {
                let (r, g, b) = if (4..12).contains(&x) && (4..12).contains(&y) {
                    (236u8, 235, 229)
                } else if (x + y) % 3 == 0 {
                    (122u8, 119, 112)
                } else {
                    (26u8, 31, 38)
                };
                vinyl.extend_from_slice(&[b, g, r, 255]);
            }
        }
        for light in [false, true] {
            let out = extract_accent_from_bgra(&vinyl, 16, 16, light, blue_fallback);
            let (_, s, _) = rgb_to_hsl(out);
            assert!(
                s <= 0.12,
                "monochrome cover with dark-shadow noise must not turn blue/chromatic: s={s} ({out:?})"
            );
        }

        // 3. Empty cover still uses calibrated system accent fallback
        let empty_out = extract_accent_from_bgra(&[], 0, 0, false, blue_fallback);
        let (eh, _, _) = rgb_to_hsl(empty_out);
        assert!((195.0..=225.0).contains(&eh), "expected blue fallback on empty image, got {eh}");
    }

    #[test]
    fn extract_accent_muted_cover_is_not_over_saturated() {
        // Muted khaki / warm sage cover (~s=0.19)
        let mut muted = Vec::with_capacity(8 * 8 * 4);
        for _ in 0..64 {
            muted.extend_from_slice(&[115, 155, 160, 255]);
        }
        let fallback = Rgb::new(0, 120, 212);
        for light in [false, true] {
            let out = extract_accent_from_bgra(&muted, 8, 8, light, fallback);
            let (h, s, _) = rgb_to_hsl(out);
            assert!((35.0..=75.0).contains(&h), "expected warm sage/khaki hue, got {h}");
            assert!(s < 0.30, "muted cover must stay restrained (s < 0.30), got {s}");
        }
    }

    #[test]
    fn gaussian_blur_preserves_uniform_color_and_diffuses_impulse() {
        let w = 16usize;
        let h = 16usize;
        let uniform = [40u8, 120, 200, 255].repeat(w * h);
        let blurred_uniform = gaussian_blur_bgra(&uniform, w, h, 3);
        assert_eq!(blurred_uniform.len(), uniform.len());
        assert_eq!(&blurred_uniform[0..4], &[40, 120, 200, 255]);

        let mut impulse = vec![0u8; w * h * 4];
        let center = (8 * w + 8) * 4;
        impulse[center] = 255;
        impulse[center + 1] = 255;
        impulse[center + 2] = 255;
        impulse[center + 3] = 255;
        let blurred = gaussian_blur_bgra(&impulse, w, h, 2);
        assert!(blurred[center] < 255 && blurred[center] > 0);
        let neighbor = (8 * w + 9) * 4;
        assert!(blurred[neighbor] > 0);
    }

    #[test]
    fn background_accent_preserves_hue_lifts_lightness_and_never_turns_blue() {
        // Dark emerald backdrop -> preserves green hue (~120-165), higher lightness, no blue drift
        let dark_green = Rgb::new(22, 64, 42);
        let (gh, gs, gl) = rgb_to_hsl(dark_green);
        let derived_dark = derive_background_accent(dark_green, false);
        let (dh, ds, dl) = rgb_to_hsl(derived_dark);
        assert!((gh - dh).abs() <= 4.0, "hue drifted: {gh} -> {dh}");
        assert!(ds <= gs + 0.02, "saturation unexpectedly boosted: {gs} -> {ds}");
        assert!(dl > gl && dl >= 0.70, "expected lifted lightness: {gl} -> {dl}");

        // Warm amber backdrop on light glass -> stays warm in readable light-glass range
        let warm_amber = Rgb::new(110, 62, 24);
        let (wh, ws, _) = rgb_to_hsl(warm_amber);
        let derived_light = derive_background_accent(warm_amber, true);
        let (lh, ls, ll) = rgb_to_hsl(derived_light);
        assert!((wh - lh).abs() <= 4.0, "warm hue drifted: {wh} -> {lh}");
        assert!(ls <= ws + 0.02);
        assert!((0.34..=0.48).contains(&ll));

        // Neutral gray/black/white backdrops stay neutral (zero saturation, never blue)
        for gray in [Rgb::new(18, 18, 20), NEUTRAL_BACKDROP_FALLBACK, Rgb::new(245, 245, 247)] {
            for light_glass in [false, true] {
                let out = derive_background_accent(gray, light_glass);
                let (_, s, _) = rgb_to_hsl(out);
                assert!(s <= 0.06, "neutral bg {gray:?} became saturated {out:?} (s={s})");
            }
        }

        // BGRA backdrop average extraction
        let px = [20u8, 60, 100, 255, 40, 80, 120, 255];
        assert_eq!(extract_backdrop_base_rgb(&px, 2, 1), Some(Rgb::new(110, 70, 30)));
        assert_eq!(extract_backdrop_base_rgb(&[], 0, 0), None);
    }

    #[test]
    fn inpaint_occluded_rects_replaces_transient_corner_cleanly() {
        let w = 8usize;
        let h = 8usize;
        let mut buf = [247u8, 247, 247, 255].repeat(w * h);
        // Pollute the bottom-left 4x4 corner with a dark-blue BGRA swatch (B=180, G=140, R=100)
        for y in 4..8 {
            for x in 0..4 {
                let i = (y * w + x) * 4;
                buf[i] = 180;
                buf[i + 1] = 140;
                buf[i + 2] = 100;
                buf[i + 3] = 255;
            }
        }

        // Empty occlusion list is a no-op returning true
        let untouched_snapshot = buf.clone();
        assert!(inpaint_occluded_rects_bgra(&mut buf, w, h, &[]));
        assert_eq!(buf, untouched_snapshot);

        // Full coverage reports false so caller can use neutral fallback
        let mut full_buf = buf.clone();
        assert!(!inpaint_occluded_rects_bgra(&mut full_buf, w, h, &[(0, 0, w, h)]));

        // Occluding bottom-left corner restores pure white from surrounding clean neighbors
        assert!(inpaint_occluded_rects_bgra(&mut buf, w, h, &[(0, 4, 4, 8)]));
        for px in buf.chunks_exact(4) {
            assert_eq!(px, &[247, 247, 247, 255]);
        }
    }

    fn circular_hue_diff_deg(a: f32, b: f32) -> f32 {
        let d = (a - b).abs().rem_euclid(360.0);
        d.min(360.0 - d)
    }

    #[test]
    fn ui_accent_deadzone_preserves_clear_vibrant_accents() {
        for dark_clear in [
            Rgb::new(228, 186, 78),
            Rgb::new(82, 205, 198),
            Rgb::new(0, 120, 212),
        ] {
            assert_eq!(enforce_ui_accent_deadzone(dark_clear, false), dark_clear);
        }
        for light_clear in [
            Rgb::new(152, 114, 24),
            Rgb::new(28, 138, 132),
            Rgb::new(0, 120, 212),
        ] {
            assert_eq!(enforce_ui_accent_deadzone(light_clear, true), light_clear);
        }
    }

    #[test]
    fn ui_accent_deadzone_pushes_muddy_colors_out_preserving_hue() {
        let samples = [
            Rgb::new(125, 167, 171), // muted gray-cyan
            Rgb::new(162, 130, 136), // dusty mauve
            Rgb::new(170, 165, 125), // muddy khaki
        ];
        for sample in samples {
            let (orig_h, orig_s, _) = rgb_to_hsl(sample);
            for light_theme in [false, true] {
                let out = enforce_ui_accent_deadzone(sample, light_theme);
                assert!(
                    !is_in_ui_accent_deadzone(out, light_theme),
                    "{sample:?} remained in deadzone (light={light_theme}): {out:?}"
                );
                let (out_h, out_s, _) = rgb_to_hsl(out);
                assert!(
                    circular_hue_diff_deg(orig_h, out_h) <= 4.0,
                    "hue drifted for {sample:?} (light={light_theme}): {orig_h} -> {out_h}"
                );
                assert!(
                    out_s > orig_s,
                    "expected pushed saturation for {sample:?} (light={light_theme}): {orig_s} -> {out_s}"
                );
            }
        }
    }

    #[test]
    fn ui_accent_deadzone_fixes_extreme_luma_while_preserving_hue() {
        let dark_blue = Rgb::new(30, 40, 215);
        let (orig_hb, _, _) = rgb_to_hsl(dark_blue);
        let out_dark = enforce_ui_accent_deadzone(dark_blue, false);
        assert!(!is_in_ui_accent_deadzone(out_dark, false));
        assert!(rgb_luma_01(out_dark) >= 0.36);
        assert!(circular_hue_diff_deg(orig_hb, rgb_to_hsl(out_dark).0) <= 4.0);

        let bright_yellow = Rgb::new(225, 210, 25);
        let (orig_hy, _, _) = rgb_to_hsl(bright_yellow);
        let out_light = enforce_ui_accent_deadzone(bright_yellow, true);
        assert!(!is_in_ui_accent_deadzone(out_light, true));
        assert!(rgb_luma_01(out_light) <= 0.60);
        assert!(circular_hue_diff_deg(orig_hy, rgb_to_hsl(out_light).0) <= 4.0);
    }

    #[test]
    fn ui_accent_deadzone_keeps_neutral_gray_achromatic() {
        for gray in [
            Rgb::new(150, 150, 150),
            Rgb::new(194, 194, 194),
            Rgb::new(94, 94, 94),
        ] {
            let dark_out = enforce_ui_accent_deadzone(gray, false);
            let (_, dark_s, dark_l) = rgb_to_hsl(dark_out);
            assert!(!is_in_ui_accent_deadzone(dark_out, false));
            assert!(dark_s <= 0.08, "neutral turned chromatic: {dark_out:?}");
            assert!(dark_l >= 0.88, "expected pearl-white lightness: {dark_l}");

            let light_out = enforce_ui_accent_deadzone(gray, true);
            let (_, light_s, light_l) = rgb_to_hsl(light_out);
            assert!(!is_in_ui_accent_deadzone(light_out, true));
            assert!(light_s <= 0.08, "neutral turned chromatic: {light_out:?}");
            assert!(light_l <= 0.22, "expected charcoal lightness: {light_l}");
        }
    }

    #[test]
    fn ui_accent_deadzone_clears_24_hue_sweep_across_muted_grid() {
        for step in 0..24 {
            let h = (step as f32) * 15.0;
            for s in [0.12f32, 0.25, 0.42] {
                for l in [0.30f32, 0.48, 0.65, 0.78] {
                    let sample = hsl_to_rgb(h, s, l);
                    let (orig_h, _, _) = rgb_to_hsl(sample);
                    let sample_chroma = rgb_chroma_01(sample);
                    for light_theme in [false, true] {
                        let out = enforce_ui_accent_deadzone(sample, light_theme);
                        assert!(
                            !is_in_ui_accent_deadzone(out, light_theme),
                            "24-hue sweep failed at h={h}, s={s}, l={l}, light={light_theme}: {sample:?} -> {out:?}"
                        );
                        if sample_chroma >= 0.06 {
                            let (out_h, _, _) = rgb_to_hsl(out);
                            assert!(
                                circular_hue_diff_deg(orig_h, out_h) <= 4.0,
                                "24-hue sweep drifted at h={h}, s={s}, l={l}, light={light_theme}: {orig_h} -> {out_h}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn dimension_multiplication_overflow_falls_back_safely_without_panic() {
        let tiny = [0u8, 0, 0, 255];
        let fb = Rgb::new(40, 100, 220);
        let accent = extract_accent_from_bgra(&tiny, (usize::MAX / 2) as u32, 4, false, fb);
        assert_eq!(accent, adjust_accent_for_theme(fb, false));
        assert_eq!(extract_backdrop_base_rgb(&tiny, usize::MAX / 2, 4), None);
        assert_eq!(gaussian_blur_bgra(&tiny, usize::MAX / 2, 4, 2), tiny);
    }

    #[test]
    fn ambient_cover_wash_preserves_neutral_gray_and_boosts_opaque_color() {
        let w = 8usize;
        let h = 8usize;
        let mut gray_bgra = vec![0u8; w * h * 4];
        for px in gray_bgra.chunks_exact_mut(4) {
            px[0] = 120;
            px[1] = 120;
            px[2] = 120;
            px[3] = 255;
        }
        let gray_out = build_ambient_cover_bgra(&gray_bgra, w, h);
        assert_eq!(gray_out.len(), gray_bgra.len());
        for px in gray_out.chunks_exact(4) {
            assert_eq!(px[3], 255);
            assert!(px[0].abs_diff(px[1]) <= 1 && px[1].abs_diff(px[2]) <= 1);
        }

        let mut warm_bgra = vec![0u8; w * h * 4];
        for (idx, px) in warm_bgra.chunks_exact_mut(4).enumerate() {
            px[0] = if idx % 2 == 0 { 45 } else { 70 };
            px[1] = 65;
            px[2] = 195;
            px[3] = 255;
        }
        let warm_out = build_ambient_cover_bgra(&warm_bgra, w, h);
        assert_eq!(warm_out.len(), warm_bgra.len());
        for px in warm_out.chunks_exact(4) {
            assert_eq!(px[3], 255);
            assert!(px[2] > px[0]);
        }
    }
}
