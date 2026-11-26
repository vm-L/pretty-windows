//! Flyout/card bounds queries and per-target fallback color/suppression configuration on `TapRuntime`.

use super::super::tree_match::{TargetKind, MAX_MENU_FLYOUT_RECTS, MAX_SHELL_CARD_RECTS};
use super::raw_com::RawUiColor;
use super::TapRuntime;
use std::sync::atomic::Ordering;
use std::sync::Arc;

impl TapRuntime {
    pub(super) fn query_alt_tab_bounds_dip(self: &Arc<Self>, wait_ms: u64) -> Option<(u32, [f32; 4])> {
        let dispatched = self.dispatch_apply_filtered(wait_ms, Some(TargetKind::AltTabBorder));
        if dispatched == 0 {
            if let Ok(mut b) = self.alt_tab_bounds_dip.lock() {
                *b = None;
            }
            return None;
        }
        self.alt_tab_bounds_dip.lock().ok().and_then(|b| *b)
    }

    pub(super) fn query_hover_flyout_bounds_dip(
        self: &Arc<Self>,
        wait_ms: u64,
    ) -> Option<(u32, [f32; 4])> {
        let dispatched =
            self.dispatch_apply_filtered(wait_ms, Some(TargetKind::HoverFlyoutBorder));
        if dispatched == 0 {
            if let Ok(mut b) = self.hover_flyout_bounds_dip.lock() {
                *b = None;
            }
            return None;
        }
        self.hover_flyout_bounds_dip.lock().ok().and_then(|b| *b)
    }

    pub(super) fn query_osd_bounds_dip(self: &Arc<Self>, wait_ms: u64) -> Option<(u32, [f32; 4])> {
        if self.is_osd_suppressed() {
            if let Ok(mut b) = self.osd_bounds_dip.lock() {
                *b = None;
            }
            return None;
        }
        let dispatched = self.dispatch_apply_filtered(wait_ms, Some(TargetKind::OsdPanel));
        if dispatched == 0 {
            if let Ok(mut b) = self.osd_bounds_dip.lock() {
                *b = None;
            }
            return None;
        }
        self.osd_bounds_dip.lock().ok().and_then(|b| *b)
    }

    #[inline]
    pub(super) fn osd_target_colors(&self) -> (RawUiColor, RawUiColor) {
        if self.is_osd_suppressed() {
            (
                RawUiColor::from_argb(self.osd_fallback_bg_argb.load(Ordering::Acquire)),
                RawUiColor::from_argb(self.osd_fallback_border_argb.load(Ordering::Acquire)),
            )
        } else {
            (RawUiColor::TRANSPARENT, RawUiColor::TRANSPARENT)
        }
    }

    pub(super) fn set_osd_suppressed(self: &Arc<Self>, suppressed: bool) {
        let prev = self.osd_suppressed.swap(suppressed, Ordering::AcqRel);
        if suppressed {
            if let Ok(mut b) = self.osd_bounds_dip.lock() {
                *b = None;
            }
        }
        if prev != suppressed && self.desired_hide() {
            self.dispatch_apply_filtered(0, Some(TargetKind::OsdPanel));
        }
    }

    pub(super) fn set_osd_fallback_colors(self: &Arc<Self>, bg_argb: u32, border_argb: u32) {
        let prev_bg = self.osd_fallback_bg_argb.swap(bg_argb, Ordering::AcqRel);
        let prev_bd = self
            .osd_fallback_border_argb
            .swap(border_argb, Ordering::AcqRel);
        if (prev_bg != bg_argb || prev_bd != border_argb)
            && self.desired_hide()
            && self.is_osd_suppressed()
        {
            self.dispatch_apply_filtered(0, Some(TargetKind::OsdPanel));
        }
    }

    pub(super) fn query_input_switcher_bounds_dip(
        self: &Arc<Self>,
        wait_ms: u64,
    ) -> Option<(u32, [f32; 4])> {
        if self.is_input_switcher_suppressed() {
            if let Ok(mut b) = self.input_switcher_bounds_dip.lock() {
                *b = None;
            }
            return None;
        }
        let dispatched =
            self.dispatch_apply_filtered(wait_ms, Some(TargetKind::InputSwitcherPanel));
        if dispatched == 0 {
            if let Ok(mut b) = self.input_switcher_bounds_dip.lock() {
                *b = None;
            }
            return None;
        }
        self.input_switcher_bounds_dip.lock().ok().and_then(|b| *b)
    }

    #[inline]
    pub(super) fn input_switcher_target_colors(&self) -> (RawUiColor, RawUiColor) {
        if self.is_input_switcher_suppressed() {
            (
                RawUiColor::from_argb(
                    self.input_switcher_fallback_bg_argb.load(Ordering::Acquire),
                ),
                RawUiColor::from_argb(
                    self.input_switcher_fallback_border_argb
                        .load(Ordering::Acquire),
                ),
            )
        } else {
            (RawUiColor::TRANSPARENT, RawUiColor::TRANSPARENT)
        }
    }

    pub(super) fn set_input_switcher_suppressed(self: &Arc<Self>, suppressed: bool) {
        let prev = self
            .input_switcher_suppressed
            .swap(suppressed, Ordering::AcqRel);
        if suppressed {
            if let Ok(mut b) = self.input_switcher_bounds_dip.lock() {
                *b = None;
            }
        }
        if prev != suppressed && self.desired_hide() {
            self.dispatch_apply_filtered(0, Some(TargetKind::InputSwitcherPanel));
        }
    }

    pub(super) fn set_input_switcher_fallback_colors(self: &Arc<Self>, bg_argb: u32, border_argb: u32) {
        let prev_bg = self
            .input_switcher_fallback_bg_argb
            .swap(bg_argb, Ordering::AcqRel);
        let prev_bd = self
            .input_switcher_fallback_border_argb
            .swap(border_argb, Ordering::AcqRel);
        if (prev_bg != bg_argb || prev_bd != border_argb)
            && self.desired_hide()
            && self.is_input_switcher_suppressed()
        {
            self.dispatch_apply_filtered(0, Some(TargetKind::InputSwitcherPanel));
        }
    }

    pub(super) fn query_tooltip_bounds_dip(self: &Arc<Self>, wait_ms: u64) -> Option<(u32, [f32; 4])> {
        if self.is_tooltip_suppressed() {
            if let Ok(mut b) = self.tooltip_bounds_dip.lock() {
                *b = None;
            }
            return None;
        }
        let dispatched = self.dispatch_apply_filtered(wait_ms, Some(TargetKind::ToolTipPresenter));
        if dispatched == 0 {
            if let Ok(mut b) = self.tooltip_bounds_dip.lock() {
                *b = None;
            }
            return None;
        }
        self.tooltip_bounds_dip.lock().ok().and_then(|b| *b)
    }

    #[inline]
    pub(super) fn tooltip_target_colors(&self) -> (RawUiColor, RawUiColor) {
        if self.is_tooltip_suppressed() {
            (
                RawUiColor::from_argb(self.tooltip_fallback_bg_argb.load(Ordering::Acquire)),
                RawUiColor::from_argb(self.tooltip_fallback_border_argb.load(Ordering::Acquire)),
            )
        } else {
            (RawUiColor::TRANSPARENT, RawUiColor::TRANSPARENT)
        }
    }

    pub(super) fn set_tooltip_suppressed(self: &Arc<Self>, suppressed: bool) {
        let prev = self.tooltip_suppressed.swap(suppressed, Ordering::AcqRel);
        if suppressed {
            if let Ok(mut b) = self.tooltip_bounds_dip.lock() {
                *b = None;
            }
        }
        if prev != suppressed && self.desired_hide() {
            self.dispatch_apply_filtered(0, Some(TargetKind::ToolTipPresenter));
        }
    }

    pub(super) fn set_tooltip_fallback_colors(self: &Arc<Self>, bg_argb: u32, border_argb: u32) {
        let prev_bg = self.tooltip_fallback_bg_argb.swap(bg_argb, Ordering::AcqRel);
        let prev_bd = self
            .tooltip_fallback_border_argb
            .swap(border_argb, Ordering::AcqRel);
        if (prev_bg != bg_argb || prev_bd != border_argb)
            && self.desired_hide()
            && self.is_tooltip_suppressed()
        {
            self.dispatch_apply_filtered(0, Some(TargetKind::ToolTipPresenter));
        }
    }

    pub(super) fn query_menu_flyout_bounds_dip(
        self: &Arc<Self>,
        wait_ms: u64,
    ) -> Vec<(u32, [f32; 4])> {
        if self.is_menu_flyout_suppressed() {
            if let Ok(mut b) = self.menu_flyout_bounds_by_tid.lock() {
                b.clear();
            }
            return Vec::new();
        }
        let dispatched =
            self.dispatch_apply_filtered(wait_ms, Some(TargetKind::MenuFlyoutSurface));
        if dispatched == 0 {
            if let Ok(mut b) = self.menu_flyout_bounds_by_tid.lock() {
                b.clear();
            }
            return Vec::new();
        }
        let (active_tids, has_core_menu) = if let Ok(g) = self.state.lock() {
            let mut tids = Vec::new();
            let mut has_core = false;
            for t in g.targets.values() {
                if t.kind == TargetKind::MenuFlyoutSurface {
                    if let Some(tid) = t.thread_id {
                        if !tids.contains(&tid) {
                            tids.push(tid);
                        }
                    } else {
                        has_core = true;
                    }
                }
            }
            (tids, has_core)
        } else {
            (Vec::new(), true)
        };
        let mut out = Vec::new();
        if let Ok(mut b) = self.menu_flyout_bounds_by_tid.lock() {
            if !has_core_menu {
                b.retain(|tid, _| active_tids.contains(tid));
            }
            for (&tid, rects) in b.iter() {
                for &r in rects {
                    out.push((tid, r));
                }
            }
        }
        out.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then_with(|| a.1[1].total_cmp(&b.1[1]))
                .then_with(|| a.1[0].total_cmp(&b.1[0]))
        });
        out.truncate(MAX_MENU_FLYOUT_RECTS);
        out
    }

    pub(super) fn query_shell_card_bounds_dip(
        self: &Arc<Self>,
        wait_ms: u64,
    ) -> Vec<(u32, [f32; 4])> {
        if self.is_menu_flyout_suppressed() {
            if let Ok(mut b) = self.menu_flyout_bounds_by_tid.lock() {
                b.clear();
            }
            if let Ok(mut b) = self.shell_card_bounds_by_handle.lock() {
                b.clear();
            }
            return Vec::new();
        }

        let (menu_tids, shell_handles, has_core_menu) =
            if let Ok(g) = self.state.lock() {
                let mut m_tids = Vec::new();
                let mut s_handles = Vec::new();
                let mut core_menu = false;
                for (&h, t) in &g.targets {
                    match t.kind {
                        TargetKind::MenuFlyoutSurface => {
                            if let Some(tid) = t.thread_id {
                                if !m_tids.contains(&tid) {
                                    m_tids.push(tid);
                                }
                            } else {
                                core_menu = true;
                            }
                        }
                        TargetKind::ShellCardSurface => {
                            s_handles.push(h);
                        }
                        _ => {}
                    }
                }
                (m_tids, s_handles, core_menu)
            } else {
                (Vec::new(), Vec::new(), false)
            };

        let need_menu_dispatch = has_core_menu || !menu_tids.is_empty();
        let need_shell_dispatch = !shell_handles.is_empty();
        if !need_menu_dispatch && !need_shell_dispatch {
            if let Ok(mut b) = self.menu_flyout_bounds_by_tid.lock() {
                b.clear();
            }
            if let Ok(mut b) = self.shell_card_bounds_by_handle.lock() {
                b.clear();
            }
            return Vec::new();
        }

        if need_menu_dispatch {
            let _ = self.dispatch_apply_filtered(wait_ms, Some(TargetKind::MenuFlyoutSurface));
        }
        if need_shell_dispatch {
            let _ = self.dispatch_apply_filtered(wait_ms, Some(TargetKind::ShellCardSurface));
        }

        let mut out: Vec<(u32, [f32; 4])> = Vec::new();
        if let Ok(mut b) = self.menu_flyout_bounds_by_tid.lock() {
            if !need_menu_dispatch {
                b.clear();
            } else {
                if !has_core_menu {
                    b.retain(|tid, _| menu_tids.contains(tid));
                }
                for (&tid, rects) in b.iter() {
                    for &r in rects {
                        out.push((tid, r));
                    }
                }
            }
        }
        if let Ok(mut b) = self.shell_card_bounds_by_handle.lock() {
            if !need_shell_dispatch {
                b.clear();
            } else {
                b.retain(|h, _| shell_handles.contains(h));
                for &(tid, r) in b.values() {
                    let is_dup = out.iter().any(|&(pt, prev)| {
                        pt == tid
                            && (prev[0] - r[0]).abs() <= 4.0
                            && (prev[1] - r[1]).abs() <= 4.0
                            && (prev[2] - r[2]).abs() <= 4.0
                            && (prev[3] - r[3]).abs() <= 4.0
                    });
                    if !is_dup {
                        out.push((tid, r));
                    }
                }
            }
        }
        out.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then_with(|| a.1[1].total_cmp(&b.1[1]))
                .then_with(|| a.1[0].total_cmp(&b.1[0]))
                .then_with(|| a.1[3].total_cmp(&b.1[3]))
                .then_with(|| a.1[2].total_cmp(&b.1[2]))
        });
        out.truncate(MAX_SHELL_CARD_RECTS);
        out
    }

    #[inline]
    pub(super) fn start_modal_style_spec(&self) -> (u8, bool, u32, u32) {
        self.start_modal_style
            .lock()
            .map(|g| *g)
            .unwrap_or((4, false, 0xC2C2C7, 0xD1D1D6))
    }

    pub(super) fn set_start_modal_style(
        self: &Arc<Self>,
        style_id: u8,
        light_glass: bool,
        halo0_rgb: u32,
        halo1_rgb: u32,
    ) {
        let next = (style_id, light_glass, halo0_rgb, halo1_rgb);
        let changed = self
            .start_modal_style
            .lock()
            .map(|mut g| {
                let prev = *g;
                *g = next;
                prev != next
            })
            .unwrap_or(false);
        if changed && self.desired_hide() {
            self.dispatch_apply_filtered(0, Some(TargetKind::StartMenuModalSurface));
        }
    }

    #[inline]
    pub(super) fn menu_flyout_target_colors(&self) -> (RawUiColor, RawUiColor) {
        if self.is_menu_flyout_suppressed() {
            (
                RawUiColor::from_argb(
                    self.menu_flyout_fallback_bg_argb.load(Ordering::Acquire),
                ),
                RawUiColor::from_argb(
                    self.menu_flyout_fallback_border_argb
                        .load(Ordering::Acquire),
                ),
            )
        } else {
            (RawUiColor::TRANSPARENT, RawUiColor::TRANSPARENT)
        }
    }

    pub(super) fn set_menu_flyout_suppressed(self: &Arc<Self>, suppressed: bool) {
        let prev = self.menu_flyout_suppressed.swap(suppressed, Ordering::AcqRel);
        if suppressed {
            if let Ok(mut b) = self.menu_flyout_bounds_by_tid.lock() {
                b.clear();
            }
            if let Ok(mut b) = self.shell_card_bounds_by_handle.lock() {
                b.clear();
            }
        }
        if prev != suppressed && self.desired_hide() {
            self.dispatch_apply_filtered(0, Some(TargetKind::MenuFlyoutSurface));
            self.dispatch_apply_filtered(0, Some(TargetKind::ShellCardSurface));
        }
    }

    pub(super) fn set_menu_flyout_fallback_colors(self: &Arc<Self>, bg_argb: u32, border_argb: u32) {
        let prev_bg = self
            .menu_flyout_fallback_bg_argb
            .swap(bg_argb, Ordering::AcqRel);
        let prev_bd = self
            .menu_flyout_fallback_border_argb
            .swap(border_argb, Ordering::AcqRel);
        if (prev_bg != bg_argb || prev_bd != border_argb)
            && self.desired_hide()
            && self.is_menu_flyout_suppressed()
        {
            self.dispatch_apply_filtered(0, Some(TargetKind::MenuFlyoutSurface));
            self.dispatch_apply_filtered(0, Some(TargetKind::ShellCardSurface));
        }
    }

    pub(super) fn set_alt_tab_item_colors(self: &Arc<Self>, bg_argb: u32, border_argb: u32) {
        let prev_bg = self.alt_tab_item_bg_argb.swap(bg_argb, Ordering::AcqRel);
        let prev_bd = self
            .alt_tab_item_border_argb
            .swap(border_argb, Ordering::AcqRel);
        if (prev_bg != bg_argb || prev_bd != border_argb) && self.desired_hide() {
            self.dispatch_apply_filtered(0, Some(TargetKind::AltTabItemBorder));
            self.dispatch_apply_filtered(0, Some(TargetKind::AltTabBorder));
        }
    }

    pub(super) fn virtual_desktop_bar_target_colors(&self) -> (RawUiColor, RawUiColor) {
        (
            RawUiColor::from_argb(self.virtual_desktop_bar_bg_argb.load(Ordering::Acquire)),
            RawUiColor::from_argb(
                self.virtual_desktop_bar_border_argb.load(Ordering::Acquire),
            ),
        )
    }

    pub(super) fn set_virtual_desktop_bar_colors(self: &Arc<Self>, bg_argb: u32, border_argb: u32) {
        let prev_bg = self
            .virtual_desktop_bar_bg_argb
            .swap(bg_argb, Ordering::AcqRel);
        let prev_bd = self
            .virtual_desktop_bar_border_argb
            .swap(border_argb, Ordering::AcqRel);
        if (prev_bg != bg_argb || prev_bd != border_argb) && self.desired_hide() {
            self.dispatch_apply_filtered(0, Some(TargetKind::VirtualDesktopBarSurface));
        }
    }
}
