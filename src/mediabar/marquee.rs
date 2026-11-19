/// Seamless looping marquee state machine with initial pause and hover/play awareness.
#[derive(Clone, Debug)]
pub struct MarqueeState {
    text: String,
    text_width: f32,
    viewport_width: f32,
    gap: f32,
    pause_secs: f32,
    speed_dip_per_sec: f32,
    elapsed_pause: f32,
    offset: f32,
}

impl Default for MarqueeState {
    fn default() -> Self {
        Self {
            text: String::new(),
            text_width: 0.0,
            viewport_width: 133.0,
            gap: 34.0,
            pause_secs: 2.5,
            speed_dip_per_sec: 28.0,
            elapsed_pause: 0.0,
            offset: 0.0,
        }
    }
}

impl MarqueeState {
    #[cfg(test)]
    pub fn new(viewport_width: f32) -> Self {
        Self {
            viewport_width: viewport_width.max(1.0),
            ..Self::default()
        }
    }

    /// Updates text and resets scroll position only if the string actually changed.
    pub fn set_text(&mut self, text: &str, text_width: f32) {
        self.text_width = text_width.max(0.0);
        if self.text != text {
            self.text.clear();
            self.text.push_str(text);
            self.offset = 0.0;
            self.elapsed_pause = 0.0;
        } else if !self.is_overflowing() {
            self.offset = 0.0;
            self.elapsed_pause = 0.0;
        }
    }

    /// Updates viewport and measured widths (e.g., when hover smoothly narrows available text space).
    pub fn set_widths(&mut self, text_width: f32, viewport_width: f32) {
        self.text_width = text_width.max(0.0);
        self.viewport_width = viewport_width.max(1.0);
        if !self.is_overflowing() {
            self.offset = 0.0;
            self.elapsed_pause = 0.0;
        }
    }

    #[inline]
    pub fn is_overflowing(&self) -> bool {
        self.text_width > self.viewport_width + 1.0
    }

    /// Whether a high-frequency animation timer should stay alive for this marquee.
    #[inline]
    pub fn is_active(&self, is_playing: bool) -> bool {
        is_playing && self.is_overflowing()
    }

    /// Advances the marquee state by `dt_secs`. Returns `true` if the visible offset changed.
    pub fn tick(&mut self, dt_secs: f32, is_playing: bool) -> bool {
        if !is_playing || !self.is_overflowing() || dt_secs <= 0.0 {
            return false;
        }

        let prev_offset = self.offset;
        if self.offset == 0.0 && self.elapsed_pause < self.pause_secs {
            self.elapsed_pause += dt_secs;
            if self.elapsed_pause < self.pause_secs {
                return false;
            }
            let overshoot = self.elapsed_pause - self.pause_secs;
            self.elapsed_pause = self.pause_secs;
            self.offset += overshoot * self.speed_dip_per_sec;
        } else {
            self.offset += dt_secs * self.speed_dip_per_sec;
        }

        let cycle = self.text_width + self.gap;
        if self.offset >= cycle {
            self.offset = 0.0;
            self.elapsed_pause = 0.0;
        }
        (self.offset - prev_offset).abs() > 1e-4
    }

    /// Returns `(primary_x_offset, optional_wrap_copy_x_offset)` relative to the text box left edge.
    pub fn render_offsets(&self) -> (f32, Option<f32>) {
        if !self.is_overflowing() || self.offset <= 0.001 {
            return (0.0, None);
        }
        let second = self.text_width + self.gap - self.offset;
        let wrap = if second < self.viewport_width {
            Some(second)
        } else {
            None
        };
        (-self.offset, wrap)
    }

    #[allow(dead_code)]
    pub fn offset(&self) -> f32 {
        self.offset
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_text_never_scrolls() {
        let mut m = MarqueeState::new(120.0);
        m.set_text("Short Title", 80.0);
        assert!(!m.is_overflowing());
        assert!(!m.tick(5.0, true));
        assert_eq!(m.render_offsets(), (0.0, None));
    }

    #[test]
    fn pauses_then_scrolls_and_wraps_cleanly() {
        let mut m = MarqueeState::new(100.0);
        m.set_text("A Very Long Track Name For Testing", 180.0);
        assert!(m.is_overflowing());

        // Stays still during 2.5s initial pause
        assert!(!m.tick(1.5, true));
        assert_eq!(m.offset(), 0.0);

        // Advances after 2.5s
        assert!(m.tick(1.5, true)); // 0.5s past pause -> 14 DIP
        assert!((m.offset() - 14.0).abs() < 0.1);

        // Freezes when paused
        let frozen = m.offset();
        assert!(!m.tick(1.0, false));
        assert_eq!(m.offset(), frozen);

        // Scrolls until wrap copy enters viewport
        m.tick(3.5, true); // +98 DIP -> 112 DIP; second copy at 180 + 34 - 112 = 102 > 100
        m.tick(0.5, true); // +14 DIP -> 126 DIP; second copy at 214 - 126 = 88 < 100
        let (x1, x2) = m.render_offsets();
        assert!(x1 < 0.0);
        assert!(x2.is_some());

        // Completes cycle and resets to pause at 0
        m.tick(5.0, true);
        assert_eq!(m.offset(), 0.0);
    }
}
