use std::f32::consts::PI;

/// FFT window size in mono samples (1024 @ 48kHz -> ~46.875 Hz/bin).
pub const FFT_SIZE: usize = 1024;
/// Number of visual spectrum bars displayed in MediaBar.
pub const NUM_BARS: usize = 10;
/// Minimum samples pushed before recalculating FFT to avoid redundant work on tiny packets.
const HOP_SAMPLES: usize = 320;

#[derive(Clone, Copy, Debug, Default)]
struct Complex {
    re: f32,
    im: f32,
}

impl Complex {
    #[inline]
    fn add(self, other: Self) -> Self {
        Self {
            re: self.re + other.re,
            im: self.im + other.im,
        }
    }

    #[inline]
    fn sub(self, other: Self) -> Self {
        Self {
            re: self.re - other.re,
            im: self.im - other.im,
        }
    }

    #[inline]
    fn mul(self, other: Self) -> Self {
        Self {
            re: self.re * other.re - self.im * other.im,
            im: self.re * other.im + self.im * other.re,
        }
    }

    #[inline]
    fn mag_sq(self) -> f32 {
        self.re * self.re + self.im * self.im
    }
}

/// Computes an in-place Radix-2 Cooley-Tukey FFT on a power-of-two slice.
fn fft_in_place(buf: &mut [Complex]) {
    let n = buf.len();
    debug_assert!(n.is_power_of_two());

    // Bit-reversal permutation
    let mut j = 0usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            buf.swap(i, j);
        }
    }

    // Butterfly stages
    let mut len = 2usize;
    while len <= n {
        let half = len >> 1;
        let angle = -2.0 * PI / (len as f32);
        let wlen = Complex {
            re: angle.cos(),
            im: angle.sin(),
        };
        for start in (0..n).step_by(len) {
            let mut w = Complex { re: 1.0, im: 0.0 };
            for k in 0..half {
                let u = buf[start + k];
                let v = buf[start + k + half].mul(w);
                buf[start + k] = u.add(v);
                buf[start + k + half] = u.sub(v);
                w = w.mul(wlen);
            }
        }
        len <<= 1;
    }
}

/// Real-time 6-band logarithmic FFT spectrum analyzer with AGC and asymmetric smoothing.
pub struct SpectrumAnalyzer {
    sample_rate: f32,
    ring: [f32; FFT_SIZE],
    write_pos: usize,
    pending_samples: usize,
    hann: [f32; FFT_SIZE],
    fft_buf: [Complex; FFT_SIZE],
    raw_bands: [f32; NUM_BARS],
    smoothed_bars: [f32; NUM_BARS],
    agc_peak: f32,
}

impl SpectrumAnalyzer {
    /// Frequency edges (Hz) dividing audible musical content into 10 balanced logarithmic bands.
    const BAND_EDGES_HZ: [f32; NUM_BARS + 1] = [
        40.0, 90.0, 150.0, 250.0, 420.0, 720.0, 1250.0, 2200.0, 3900.0, 7000.0, 14000.0,
    ];
    /// High-frequency tilt compensation so vocal/hi-hat bands match kick/bass visual energy.
    const BAND_TILT: [f32; NUM_BARS] = [
        0.92, 1.02, 1.14, 1.28, 1.44, 1.62, 1.84, 2.10, 2.40, 2.75,
    ];

    pub fn new(sample_rate: u32) -> Self {
        let mut hann = [0.0f32; FFT_SIZE];
        let denom = (FFT_SIZE - 1) as f32;
        for (i, w) in hann.iter_mut().enumerate() {
            *w = 0.5 * (1.0 - (2.0 * PI * (i as f32) / denom).cos());
        }

        Self {
            sample_rate: (sample_rate.max(8000)) as f32,
            ring: [0.0; FFT_SIZE],
            write_pos: 0,
            pending_samples: 0,
            hann,
            fft_buf: [Complex::default(); FFT_SIZE],
            raw_bands: [0.0; NUM_BARS],
            smoothed_bars: [0.0; NUM_BARS],
            agc_peak: 0.12,
        }
    }

    pub fn set_sample_rate(&mut self, sample_rate: u32) {
        self.sample_rate = (sample_rate.max(8000)) as f32;
    }

    /// Pushes mono audio samples in `[-1.0, 1.0]` and updates the spectrum when enough samples arrive.
    /// Returns `true` if a new FFT frame was computed.
    pub fn push_mono_samples(&mut self, samples: &[f32]) -> bool {
        let mut updated = false;
        for &s in samples {
            self.ring[self.write_pos] = s.clamp(-4.0, 4.0);
            self.write_pos = (self.write_pos + 1) % FFT_SIZE;
            self.pending_samples += 1;
            if self.pending_samples >= HOP_SAMPLES {
                self.pending_samples = 0;
                self.compute_fft_frame();
                updated = true;
            }
        }
        updated
    }

    /// Smoothly decays bars toward zero when audio is paused or silent.
    pub fn decay_to_zero(&mut self, dt_secs: f32) {
        let factor = (-dt_secs.max(0.0) * 14.0).exp();
        for bar in &mut self.smoothed_bars {
            *bar *= factor;
            if *bar < 0.002 {
                *bar = 0.0;
            }
        }
        self.agc_peak = (self.agc_peak * 0.995).max(0.12);
    }

    /// Resets internal audio state when switching audio sources.
    pub fn reset(&mut self) {
        self.ring.fill(0.0);
        self.raw_bands.fill(0.0);
        self.smoothed_bars.fill(0.0);
        self.write_pos = 0;
        self.pending_samples = 0;
        self.agc_peak = 0.12;
    }

    pub fn bars(&self) -> [f32; NUM_BARS] {
        self.smoothed_bars
    }

    fn compute_fft_frame(&mut self) {
        // Copy ring buffer oldest -> newest with Hann window applied.
        for i in 0..FFT_SIZE {
            let idx = (self.write_pos + i) % FFT_SIZE;
            self.fft_buf[i] = Complex {
                re: self.ring[idx] * self.hann[i],
                im: 0.0,
            };
        }

        fft_in_place(&mut self.fft_buf);

        let nyquist_bin = FFT_SIZE / 2;
        let hz_per_bin = self.sample_rate / (FFT_SIZE as f32);
        let norm = 2.0 / (FFT_SIZE as f32);

        let mut frame_max = 0.0f32;
        for band_idx in 0..NUM_BARS {
            let f_low = Self::BAND_EDGES_HZ[band_idx];
            let f_high = Self::BAND_EDGES_HZ[band_idx + 1];
            let bin_start = ((f_low / hz_per_bin).floor() as usize).clamp(1, nyquist_bin - 1);
            let mut bin_end = ((f_high / hz_per_bin).ceil() as usize).clamp(bin_start + 1, nyquist_bin);
            if bin_end <= bin_start {
                bin_end = (bin_start + 1).min(nyquist_bin);
            }

            let mut sum_sq = 0.0f32;
            let mut peak_mag = 0.0f32;
            let count = (bin_end - bin_start) as f32;
            for b in bin_start..bin_end {
                let mag = self.fft_buf[b].mag_sq().sqrt() * norm;
                sum_sq += mag * mag;
                if mag > peak_mag {
                    peak_mag = mag;
                }
            }
            let rms = (sum_sq / count).sqrt();
            // Blend RMS and peak so narrow tonal notes and broadband hits both look expressive.
            let energy = (rms * 0.65 + peak_mag * 0.35) * Self::BAND_TILT[band_idx];
            // Soft log compression to expand quiet detail without pinning loud beats.
            let compressed = (energy * 9.0).ln_1p();
            self.raw_bands[band_idx] = compressed;
            if compressed > frame_max {
                frame_max = compressed;
            }
        }

        // Noise gate: suppress ultra-low dithering floor.
        if frame_max < 0.015 {
            self.agc_peak = (self.agc_peak * 0.996).max(0.12);
            for bar in &mut self.smoothed_bars {
                *bar *= 0.80;
                if *bar < 0.002 {
                    *bar = 0.0;
                }
            }
            return;
        }

        // Fast rise, very gentle AGC release.
        if frame_max > self.agc_peak {
            self.agc_peak = self.agc_peak * 0.72 + frame_max * 0.28;
        } else {
            self.agc_peak = (self.agc_peak * 0.996).max(0.12);
        }

        let inv_peak = 1.0 / self.agc_peak.max(0.12);
        for i in 0..NUM_BARS {
            let target = (self.raw_bands[i] * inv_peak).powf(0.85).clamp(0.0, 1.0);
            let current = self.smoothed_bars[i];
            let coeff = if target > current { 0.58 } else { 0.20 };
            self.smoothed_bars[i] = current + (target - current) * coeff;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn generate_sine(freq_hz: f32, sample_rate: u32, len: usize) -> Vec<f32> {
        (0..len)
            .map(|i| 0.5 * (2.0 * PI * freq_hz * (i as f32) / (sample_rate as f32)).sin())
            .collect()
    }

    #[test]
    fn silence_stays_zero() {
        let mut analyzer = SpectrumAnalyzer::new(48_000);
        let silence = vec![0.0f32; 2048];
        analyzer.push_mono_samples(&silence);
        assert_eq!(analyzer.bars(), [0.0; NUM_BARS]);
    }

    #[test]
    fn sine_waves_excite_expected_bands() {
        // Low bass 55 Hz -> band 0
        let mut bass = SpectrumAnalyzer::new(48_000);
        let bass_sig = generate_sine(55.0, 48_000, 4096);
        bass.push_mono_samples(&bass_sig);
        let b_bars = bass.bars();
        assert!(b_bars[0] > 0.4, "expected band 0 active: {b_bars:?}");
        assert!(b_bars[0] > b_bars[5] && b_bars[0] > b_bars[9]);

        // Mid 1000 Hz -> band 5
        let mut mid = SpectrumAnalyzer::new(48_000);
        let mid_sig = generate_sine(1000.0, 48_000, 4096);
        mid.push_mono_samples(&mid_sig);
        let m_bars = mid.bars();
        assert!(m_bars[5] > 0.4, "expected band 5 active: {m_bars:?}");
        assert!(m_bars[5] > m_bars[0] && m_bars[5] > m_bars[9]);

        // High 8500 Hz -> band 9
        let mut high = SpectrumAnalyzer::new(48_000);
        let high_sig = generate_sine(8500.0, 48_000, 4096);
        high.push_mono_samples(&high_sig);
        let h_bars = high.bars();
        assert!(h_bars[9] > 0.4, "expected band 9 active: {h_bars:?}");
        assert!(h_bars[9] > h_bars[0] && h_bars[9] > h_bars[3]);
    }

    #[test]
    fn decay_reduces_bars_smoothly() {
        let mut analyzer = SpectrumAnalyzer::new(48_000);
        let sig = generate_sine(1000.0, 48_000, 2048);
        analyzer.push_mono_samples(&sig);
        let before = analyzer.bars()[5];
        assert!(before > 0.3);

        analyzer.decay_to_zero(0.1);
        let after = analyzer.bars()[5];
        assert!(after < before && after > 0.0);
    }
}
