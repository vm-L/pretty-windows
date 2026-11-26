//! UI-thread brush backup store, panic-safe TLS facades, property-change guards, and re-exports.

mod callback;
mod restore;
mod tracking;

pub(super) use callback::*;
pub(super) use restore::*;
pub(super) use tracking::*;

use super::super::tree_match::TargetKind;
use super::raw_com::*;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};

pub(super) enum SavedBrush {
    /// Property had `DependencyProperty.UnsetValue` locally; retains the pre-mutation effective
    /// brush (if any) so restore can call `ClearValue` first and fall back to restoring the exact
    /// effective brush when `ClearValue` loses an active `VisualState.Setter` (e.g., accent tint).
    Unset(Option<ComRaw>),
    Local(ComRaw),
}

pub(super) enum SavedCornerRadius {
    Unset,
    Local(RawCornerRadius),
}

pub(super) struct TooltipTrackedEntry {
    pub(super) presenter_weak: ComRaw,
    pub(super) tooltip_key: Option<u64>,
    pub(super) tooltip_weak: Option<ComRaw>,
}

#[derive(Default)]
pub(super) struct SavedTargetBrushes {
    pub(super) background: Option<SavedBrush>,
    pub(super) border_brush: Option<SavedBrush>,
    pub(super) focus_secondary: Option<SavedBrush>,
    pub(super) shadow: Option<SavedBrush>,
}

#[derive(Clone, Copy)]
pub(super) enum BrushSlot {
    Background,
    BorderBrush,
    FocusSecondary,
    Shadow,
}

pub(super) struct PropWatchEntry {
    pub(super) weak_obj: ComRaw,
    pub(super) dp: ComRaw,
    pub(super) token: i64,
    pub(super) kind: TargetKind,
}

#[derive(Default)]
pub(super) struct UiThreadBrushStore {
    pub(super) trans_brush: Option<ComRaw>,
    pub(super) tinted_brushes: HashMap<u32, ComRaw>,
    pub(super) tinted_brush_order: VecDeque<u32>,
    pub(super) unset_value: Option<ComRaw>,
    pub(super) border_bg_dp: Option<ComRaw>,
    pub(super) border_brush_dp: Option<ComRaw>,
    pub(super) panel_bg_dp: Option<ComRaw>,
    pub(super) grid_border_brush_dp: Option<ComRaw>,
    pub(super) content_presenter_bg_dp: Option<ComRaw>,
    pub(super) content_presenter_border_brush_dp: Option<ComRaw>,
    pub(super) control_bg_dp: Option<ComRaw>,
    pub(super) control_border_brush_dp: Option<ComRaw>,
    pub(super) control_corner_radius_dp: Option<ComRaw>,
    pub(super) focus_secondary_brush_dp: Option<ComRaw>,
    pub(super) focus_secondary_thickness_dp: Option<ComRaw>,
    pub(super) element_shadow_dp: Option<ComRaw>,
    pub(super) visual_tree_helper: Option<ComRaw>,
    pub(super) element_comp_preview: Option<ComRaw>,
    pub(super) grid_statics: Option<ComRaw>,
    pub(super) prewarmed: bool,
    pub(super) handle_to_identity: HashMap<u64, u64>,
    pub(super) saved: HashMap<u64, SavedTargetBrushes>,
    pub(super) watches: HashMap<(u64, u8), PropWatchEntry>,
    pub(super) alttab_subtargets: HashMap<u64, (TargetKind, ComRaw)>,
    pub(super) alttab_item_orig_comp_visible: HashMap<u64, u8>,
    pub(super) alttab_item_orig_focus_thickness: HashMap<u64, RawThickness>,
    pub(super) hover_flyout_shapes: HashMap<u64, (ComRaw, f64, i32)>,
    pub(super) tooltip_orig_corner_radius: HashMap<u64, SavedCornerRadius>,
    pub(super) tooltip_modified_targets: HashMap<u64, TooltipTrackedEntry>,
    pub(super) menu_flyout_orig_corner_radius: HashMap<u64, SavedCornerRadius>,
    pub(super) menu_flyout_modified_targets: HashMap<u64, TooltipTrackedEntry>,
    pub(super) shell_card_orig_corner_radius: HashMap<u64, SavedCornerRadius>,
    pub(super) shell_card_orig_comp_visible: HashMap<u64, u8>,
    pub(super) start_modal_applied_style: HashMap<u64, (u8, bool, u32, u32)>,
    pub(super) shell_card_modified_targets: HashMap<u64, TooltipTrackedEntry>,
    pub(super) general_modified_targets: HashMap<u64, (TargetKind, ComRaw)>,
    pub(super) modified_shapes: HashMap<u64, (ComRaw, f64, i32)>,
}

thread_local! {
    static UI_BRUSH_STORE_TLS: RefCell<UiThreadBrushStore> =
        RefCell::new(UiThreadBrushStore::default());
    static IN_PROP_SYNC_TLS: Cell<bool> = const { Cell::new(false) };
}

/// Panic-safe facade over `UI_BRUSH_STORE_TLS` that turns post-TLS-teardown callbacks into safe
/// no-ops instead of panicking/aborting injected Windows Shell host processes.
pub(super) struct SafeUiBrushStore;

impl SafeUiBrushStore {
    #[inline]
    pub(super) fn with<R: Default>(&self, f: impl FnOnce(&RefCell<UiThreadBrushStore>) -> R) -> R {
        UI_BRUSH_STORE_TLS.try_with(f).unwrap_or_default()
    }
}

pub(super) struct SafeInPropSync;

impl SafeInPropSync {
    #[inline]
    pub(super) fn with<R: Default>(&self, f: impl FnOnce(&Cell<bool>) -> R) -> R {
        IN_PROP_SYNC_TLS.try_with(f).unwrap_or_default()
    }
}

pub(super) static UI_BRUSH_STORE: SafeUiBrushStore = SafeUiBrushStore;
pub(super) static IN_PROP_SYNC: SafeInPropSync = SafeInPropSync;

pub(super) struct PropSyncGuard {
    pub(super) prev: bool,
}

impl PropSyncGuard {
    pub(super) fn enter() -> Self {
        let prev = IN_PROP_SYNC.with(|c| {
            let old = c.get();
            c.set(true);
            old
        });
        Self { prev }
    }
}

impl Drop for PropSyncGuard {
    fn drop(&mut self) {
        let prev = self.prev;
        IN_PROP_SYNC.with(|c| c.set(prev));
    }
}

