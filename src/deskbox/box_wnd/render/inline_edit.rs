//! Direct2D inline rename and inline search pill rendering synced with the hidden EDIT proxy.

use super::super::{BoxInner, HoverTarget, InlineEditMode};
use super::primitives::draw_icon_btn_hover;
use crate::color::Rgb;
use crate::deskbox::layout::DeskBoxLayout;
use crate::ui::glass::{
    d2d_color_rgb, d2d_rect, draw_text_in_rect, muted_text_color, primary_text_color,
};
use crate::ui::RectF;
use windows::Win32::Foundation::{BOOL, LPARAM, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::{D2D1_COLOR_F, D2D_POINT_2F, D2D_RECT_F};
use windows::Win32::Graphics::Direct2D::{
    D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_DRAW_TEXT_OPTIONS_CLIP, D2D1_ELLIPSE, D2D1_ROUNDED_RECT,
};
use windows::Win32::Graphics::DirectWrite::{IDWriteTextFormat, DWRITE_HIT_TEST_METRICS};
use windows::Win32::UI::Controls::EM_GETSEL;
use windows::Win32::UI::WindowsAndMessaging::{GetWindowTextLengthW, GetWindowTextW, SendMessageW};

impl BoxInner {
    pub(super) unsafe fn draw_inline_rename_pill(
        &mut self,
        layout: &DeskBoxLayout,
        edit_fmt: &IDWriteTextFormat,
        accent: Rgb,
        s: f32,
    ) {
        let eh = self.inline_edit_hwnd;
        let len = GetWindowTextLengthW(eh).max(0) as usize;
        let mut wide = vec![0u16; len + 1];
        let copied = if len > 0 {
            GetWindowTextW(eh, &mut wide).max(0) as usize
        } else {
            0
        };
        let text_utf16 = &wide[..copied];

        let mut sel_start: u32 = 0;
        let mut sel_end: u32 = 0;
        let _ = SendMessageW(
            eh,
            EM_GETSEL,
            WPARAM(&mut sel_start as *mut u32 as usize),
            LPARAM(&mut sel_end as *mut u32 as isize),
        );
        let text_len = text_utf16.len() as u32;
        let sel_a = sel_start.min(text_len);
        let sel_b = sel_end.min(text_len);
        let (sel_lo, sel_hi) = if sel_a <= sel_b {
            (sel_a, sel_b)
        } else {
            (sel_b, sel_a)
        };

        let pill_rect = Self::inline_edit_pill_rect(layout, InlineEditMode::Rename);
        let radius = 6.5 * s;
        let pad_x = 7.0 * s;
        let text_rect = RectF::new(
            pill_rect.left + pad_x,
            pill_rect.top,
            (pill_rect.right - pad_x).max(pill_rect.left + pad_x + 8.0 * s),
            pill_rect.bottom,
        );

        let halo_col = d2d_color_rgb(accent, if self.light_glass { 0.18 } else { 0.22 });
        let halo_rr = D2D1_ROUNDED_RECT {
            rect: D2D_RECT_F {
                left: pill_rect.left - 1.2 * s,
                top: pill_rect.top - 1.2 * s,
                right: pill_rect.right + 1.2 * s,
                bottom: pill_rect.bottom + 1.2 * s,
            },
            radiusX: radius + 1.0 * s,
            radiusY: radius + 1.0 * s,
        };
        if let Ok(hb) = self.rt.CreateSolidColorBrush(&halo_col, None) {
            self.rt.DrawRoundedRectangle(&halo_rr, &hb, 2.0 * s, None);
        }

        let pill_rr = D2D1_ROUNDED_RECT {
            rect: d2d_rect(pill_rect),
            radiusX: radius,
            radiusY: radius,
        };
        if self.light_glass {
            let fill = D2D1_COLOR_F {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 0.56,
            };
            if let Ok(b) = self.rt.CreateSolidColorBrush(&fill, None) {
                self.rt.FillRoundedRectangle(&pill_rr, &b);
            }
        } else {
            let scrim = D2D1_COLOR_F {
                r: 0.06,
                g: 0.08,
                b: 0.12,
                a: 0.30,
            };
            if let Ok(b) = self.rt.CreateSolidColorBrush(&scrim, None) {
                self.rt.FillRoundedRectangle(&pill_rr, &b);
            }
            let frosted = D2D1_COLOR_F {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 0.08,
            };
            if let Ok(fb) = self.rt.CreateSolidColorBrush(&frosted, None) {
                self.rt.FillRoundedRectangle(&pill_rr, &fb);
            }
        }

        let rim_col = d2d_color_rgb(accent, if self.light_glass { 0.78 } else { 0.86 });
        if let Ok(rb) = self.rt.CreateSolidColorBrush(&rim_col, None) {
            self.rt.DrawRoundedRectangle(&pill_rr, &rb, 1.25 * s, None);
        }

        let avail_w = text_rect.width().max(4.0);
        let text_h = text_rect.height().max(12.0);
        let Ok(text_layout) = self
            .dwrite_factory
            .CreateTextLayout(text_utf16, edit_fmt, 100_000.0, text_h)
        else {
            return;
        };

        let hit_x = |pos: u32| -> f32 {
            let mut px = 0.0f32;
            let mut py = 0.0f32;
            let mut metrics = DWRITE_HIT_TEST_METRICS::default();
            let _ = text_layout.HitTestTextPosition(
                pos.min(text_len),
                BOOL(0),
                &mut px,
                &mut py,
                &mut metrics,
            );
            px
        };

        let end_x = hit_x(text_len);
        let x_lo = hit_x(sel_lo);
        let x_hi = hit_x(sel_hi);
        let caret_x_in_text = x_hi;

        let max_scroll = (end_x - avail_w + 4.0 * s).max(0.0);
        if sel_lo == 0 && sel_hi == text_len {
            self.inline_edit_scroll_x = 0.0;
        } else {
            if caret_x_in_text - self.inline_edit_scroll_x > avail_w - 3.0 * s {
                self.inline_edit_scroll_x = caret_x_in_text - avail_w + 3.0 * s;
            }
            if caret_x_in_text - self.inline_edit_scroll_x < 0.0 {
                self.inline_edit_scroll_x = caret_x_in_text;
            }
        }
        self.inline_edit_scroll_x = self.inline_edit_scroll_x.clamp(0.0, max_scroll);
        let scroll_x = self.inline_edit_scroll_x;

        self.rt
            .PushAxisAlignedClip(&d2d_rect(text_rect), D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);

        if sel_hi > sel_lo {
            let sel_col = d2d_color_rgb(accent, if self.light_glass { 0.28 } else { 0.36 });
            if let Ok(sel_brush) = self.rt.CreateSolidColorBrush(&sel_col, None) {
                let sel_rr = D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F {
                        left: text_rect.left + x_lo - scroll_x - 1.0 * s,
                        top: text_rect.top + 3.0 * s,
                        right: text_rect.left + x_hi - scroll_x + 1.0 * s,
                        bottom: text_rect.bottom - 3.0 * s,
                    },
                    radiusX: 3.5 * s,
                    radiusY: 3.5 * s,
                };
                self.rt.FillRoundedRectangle(&sel_rr, &sel_brush);
            }
        }

        let fg = primary_text_color(self.light_glass);
        if let Ok(fg_brush) = self.rt.CreateSolidColorBrush(&fg, None) {
            self.rt.DrawTextLayout(
                D2D_POINT_2F {
                    x: text_rect.left - scroll_x,
                    y: text_rect.top,
                },
                &text_layout,
                &fg_brush,
                D2D1_DRAW_TEXT_OPTIONS_CLIP,
            );
        }

        if sel_hi == sel_lo && self.inline_edit_caret_visible {
            let caret_col = d2d_color_rgb(accent, 0.96);
            if let Ok(caret_brush) = self.rt.CreateSolidColorBrush(&caret_col, None) {
                let cx = text_rect.left + caret_x_in_text - scroll_x;
                let caret_rr = D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F {
                        left: cx - 0.75 * s,
                        top: text_rect.top + 4.5 * s,
                        right: cx + 0.75 * s,
                        bottom: text_rect.bottom - 4.5 * s,
                    },
                    radiusX: 0.75 * s,
                    radiusY: 0.75 * s,
                };
                self.rt.FillRoundedRectangle(&caret_rr, &caret_brush);
            }
        }

        self.rt.PopAxisAlignedClip();
    }

    pub(super) unsafe fn draw_inline_search_pill(
        &mut self,
        layout: &DeskBoxLayout,
        edit_fmt: &IDWriteTextFormat,
        accent: Rgb,
        s: f32,
    ) {
        let is_editing = !self.inline_edit_hwnd.is_invalid()
            && self.inline_edit_mode == Some(InlineEditMode::Search);
        let (text_utf16, sel_lo, sel_hi): (Vec<u16>, u32, u32) = if is_editing {
            let eh = self.inline_edit_hwnd;
            let len = GetWindowTextLengthW(eh).max(0) as usize;
            let mut wide = vec![0u16; len + 1];
            let copied = if len > 0 {
                GetWindowTextW(eh, &mut wide).max(0) as usize
            } else {
                0
            };
            wide.truncate(copied);
            let mut sel_start: u32 = 0;
            let mut sel_end: u32 = 0;
            let _ = SendMessageW(
                eh,
                EM_GETSEL,
                WPARAM(&mut sel_start as *mut u32 as usize),
                LPARAM(&mut sel_end as *mut u32 as isize),
            );
            let text_len = wide.len() as u32;
            let a = sel_start.min(text_len);
            let b = sel_end.min(text_len);
            if a <= b {
                (wide, a, b)
            } else {
                (wide, b, a)
            }
        } else {
            (self.search_query.encode_utf16().collect(), 0, 0)
        };
        let text_len = text_utf16.len() as u32;

        let pill_rect = Self::inline_edit_pill_rect(layout, InlineEditMode::Search);
        let radius = 6.5 * s;

        if is_editing {
            let halo_col = d2d_color_rgb(accent, if self.light_glass { 0.16 } else { 0.20 });
            let halo_rr = D2D1_ROUNDED_RECT {
                rect: D2D_RECT_F {
                    left: pill_rect.left - 1.0 * s,
                    top: pill_rect.top - 1.0 * s,
                    right: pill_rect.right + 1.0 * s,
                    bottom: pill_rect.bottom + 1.0 * s,
                },
                radiusX: radius + 1.0 * s,
                radiusY: radius + 1.0 * s,
            };
            if let Ok(hb) = self.rt.CreateSolidColorBrush(&halo_col, None) {
                self.rt.DrawRoundedRectangle(&halo_rr, &hb, 1.8 * s, None);
            }
        }

        let pill_rr = D2D1_ROUNDED_RECT {
            rect: d2d_rect(pill_rect),
            radiusX: radius,
            radiusY: radius,
        };
        if self.light_glass {
            let fill = D2D1_COLOR_F {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: if is_editing { 0.56 } else { 0.44 },
            };
            if let Ok(b) = self.rt.CreateSolidColorBrush(&fill, None) {
                self.rt.FillRoundedRectangle(&pill_rr, &b);
            }
        } else {
            let scrim = D2D1_COLOR_F {
                r: 0.06,
                g: 0.08,
                b: 0.12,
                a: if is_editing { 0.30 } else { 0.22 },
            };
            if let Ok(b) = self.rt.CreateSolidColorBrush(&scrim, None) {
                self.rt.FillRoundedRectangle(&pill_rr, &b);
            }
            let frosted = D2D1_COLOR_F {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 0.08,
            };
            if let Ok(fb) = self.rt.CreateSolidColorBrush(&frosted, None) {
                self.rt.FillRoundedRectangle(&pill_rr, &fb);
            }
        }

        let rim_alpha = if is_editing {
            if self.light_glass { 0.76 } else { 0.84 }
        } else if self.light_glass {
            0.38
        } else {
            0.44
        };
        let rim_col = d2d_color_rgb(accent, rim_alpha);
        if let Ok(rb) = self.rt.CreateSolidColorBrush(&rim_col, None) {
            self.rt.DrawRoundedRectangle(&pill_rr, &rb, 1.15 * s, None);
        }

        // Magnifying glass icon on the left side of the pill
        let stroke = self.round_stroke.as_ref();
        let mut icon_col = primary_text_color(self.light_glass);
        icon_col.a *= 0.72;
        if let Ok(ib) = self.rt.CreateSolidColorBrush(&icon_col, None) {
            let icx = pill_rect.left + 10.5 * s;
            let icy = (pill_rect.top + pill_rect.bottom) * 0.5 - 0.6 * s;
            let r = 2.9 * s;
            self.rt.DrawEllipse(
                &D2D1_ELLIPSE {
                    point: D2D_POINT_2F { x: icx, y: icy },
                    radiusX: r,
                    radiusY: r,
                },
                &ib,
                1.2 * s,
                stroke,
            );
            self.rt.DrawLine(
                D2D_POINT_2F {
                    x: icx + 2.1 * s,
                    y: icy + 2.1 * s,
                },
                D2D_POINT_2F {
                    x: icx + 4.4 * s,
                    y: icy + 4.4 * s,
                },
                &ib,
                1.25 * s,
                stroke,
            );
        }

        // Clickable clear "×" button on the right
        let clr_rect = layout.search_clear_btn_rect;
        if clr_rect.width() > 1.0 {
            let clr_hovered = self.hover == Some(HoverTarget::SearchClearButton);
            if clr_hovered {
                draw_icon_btn_hover(&self.rt, clr_rect, s, self.light_glass);
            }
            let mut clr_col = primary_text_color(self.light_glass);
            clr_col.a *= if clr_hovered { 0.96 } else { 0.66 };
            if let Ok(cb) = self.rt.CreateSolidColorBrush(&clr_col, None) {
                let cx = (clr_rect.left + clr_rect.right) * 0.5;
                let cy = (clr_rect.top + clr_rect.bottom) * 0.5;
                let arm = 3.2 * s;
                self.rt.DrawLine(
                    D2D_POINT_2F { x: cx - arm, y: cy - arm },
                    D2D_POINT_2F { x: cx + arm, y: cy + arm },
                    &cb,
                    1.25 * s,
                    stroke,
                );
                self.rt.DrawLine(
                    D2D_POINT_2F { x: cx + arm, y: cy - arm },
                    D2D_POINT_2F { x: cx - arm, y: cy + arm },
                    &cb,
                    1.25 * s,
                    stroke,
                );
            }
        }

        let text_left = pill_rect.left + 21.0 * s;
        let text_right = if clr_rect.width() > 1.0 {
            (clr_rect.left - 3.0 * s).max(text_left + 12.0 * s)
        } else {
            (pill_rect.right - 6.0 * s).max(text_left + 12.0 * s)
        };
        let text_rect = RectF::new(text_left, pill_rect.top, text_right, pill_rect.bottom);

        self.rt
            .PushAxisAlignedClip(&d2d_rect(text_rect), D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);

        if text_utf16.is_empty() {
            let mut ph_col = muted_text_color(self.light_glass);
            ph_col.a *= 0.80;
            draw_text_in_rect(
                &self.dwrite_factory,
                &self.rt,
                "搜索...",
                text_rect,
                edit_fmt,
                ph_col,
            );
            if is_editing && self.inline_edit_caret_visible {
                let caret_col = d2d_color_rgb(accent, 0.96);
                if let Ok(caret_brush) = self.rt.CreateSolidColorBrush(&caret_col, None) {
                    let cx = text_rect.left + 0.8 * s;
                    let caret_rr = D2D1_ROUNDED_RECT {
                        rect: D2D_RECT_F {
                            left: cx - 0.75 * s,
                            top: text_rect.top + 4.5 * s,
                            right: cx + 0.75 * s,
                            bottom: text_rect.bottom - 4.5 * s,
                        },
                        radiusX: 0.75 * s,
                        radiusY: 0.75 * s,
                    };
                    self.rt.FillRoundedRectangle(&caret_rr, &caret_brush);
                }
            }
            self.rt.PopAxisAlignedClip();
            return;
        }

        let avail_w = text_rect.width().max(4.0);
        let text_h = text_rect.height().max(12.0);
        if let Ok(text_layout) = self
            .dwrite_factory
            .CreateTextLayout(&text_utf16, edit_fmt, 100_000.0, text_h)
        {
            let hit_x = |pos: u32| -> f32 {
                let mut px = 0.0f32;
                let mut py = 0.0f32;
                let mut metrics = DWRITE_HIT_TEST_METRICS::default();
                let _ = text_layout.HitTestTextPosition(
                    pos.min(text_len),
                    BOOL(0),
                    &mut px,
                    &mut py,
                    &mut metrics,
                );
                px
            };

            let end_x = hit_x(text_len);
            let x_lo = hit_x(sel_lo);
            let x_hi = hit_x(sel_hi);
            let max_scroll = (end_x - avail_w + 4.0 * s).max(0.0);
            if is_editing {
                if sel_lo == 0 && sel_hi == text_len {
                    self.inline_edit_scroll_x = 0.0;
                } else {
                    if x_hi - self.inline_edit_scroll_x > avail_w - 3.0 * s {
                        self.inline_edit_scroll_x = x_hi - avail_w + 3.0 * s;
                    }
                    if x_hi - self.inline_edit_scroll_x < 0.0 {
                        self.inline_edit_scroll_x = x_hi;
                    }
                }
                self.inline_edit_scroll_x = self.inline_edit_scroll_x.clamp(0.0, max_scroll);
            } else {
                self.inline_edit_scroll_x = 0.0;
            }
            let scroll_x = self.inline_edit_scroll_x;

            if is_editing && sel_hi > sel_lo {
                let sel_col = d2d_color_rgb(accent, if self.light_glass { 0.28 } else { 0.36 });
                if let Ok(sel_brush) = self.rt.CreateSolidColorBrush(&sel_col, None) {
                    let sel_rr = D2D1_ROUNDED_RECT {
                        rect: D2D_RECT_F {
                            left: text_rect.left + x_lo - scroll_x - 1.0 * s,
                            top: text_rect.top + 3.0 * s,
                            right: text_rect.left + x_hi - scroll_x + 1.0 * s,
                            bottom: text_rect.bottom - 3.0 * s,
                        },
                        radiusX: 3.5 * s,
                        radiusY: 3.5 * s,
                    };
                    self.rt.FillRoundedRectangle(&sel_rr, &sel_brush);
                }
            }

            let fg = primary_text_color(self.light_glass);
            if let Ok(fg_brush) = self.rt.CreateSolidColorBrush(&fg, None) {
                self.rt.DrawTextLayout(
                    D2D_POINT_2F {
                        x: text_rect.left - scroll_x,
                        y: text_rect.top,
                    },
                    &text_layout,
                    &fg_brush,
                    D2D1_DRAW_TEXT_OPTIONS_CLIP,
                );
            }

            if is_editing && sel_hi == sel_lo && self.inline_edit_caret_visible {
                let caret_col = d2d_color_rgb(accent, 0.96);
                if let Ok(caret_brush) = self.rt.CreateSolidColorBrush(&caret_col, None) {
                    let cx = text_rect.left + x_hi - scroll_x;
                    let caret_rr = D2D1_ROUNDED_RECT {
                        rect: D2D_RECT_F {
                            left: cx - 0.75 * s,
                            top: text_rect.top + 4.5 * s,
                            right: cx + 0.75 * s,
                            bottom: text_rect.bottom - 4.5 * s,
                        },
                        radiusX: 0.75 * s,
                        radiusY: 0.75 * s,
                    };
                    self.rt.FillRoundedRectangle(&caret_rr, &caret_brush);
                }
            }
        }

        self.rt.PopAxisAlignedClip();
    }
}
