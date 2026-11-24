use super::super::tree_match::{TargetKind};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap};
use std::ffi::c_void;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc};
use windows::core::{GUID, HRESULT};
use windows::Win32::Foundation::{E_NOINTERFACE, E_POINTER, S_OK};
use super::appearance::{
    sync_element_shadow_prop, sync_focus_secondary_thickness_prop, sync_overflow_brush_prop,
    sync_overflow_brush_prop_with_mode,
};
use super::raw_com::*;
use super::TapRuntime;

pub(super) enum SavedBrush {
    Unset,
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
}

#[derive(Default)]
pub(super) struct UiThreadBrushStore {
    pub(super) trans_brush: Option<ComRaw>,
    pub(super) tinted_brushes: HashMap<u32, ComRaw>,
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
}

thread_local! {
    pub(super) static UI_BRUSH_STORE: RefCell<UiThreadBrushStore> =
        RefCell::new(UiThreadBrushStore::default());
    pub(super) static IN_PROP_SYNC: Cell<bool> = const { Cell::new(false) };
}

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

#[repr(C)]
pub(super) struct RawPropChangedCallbackVtbl {
    pub(super) query_interface: unsafe extern "system" fn(
        *mut PropChangedCallback,
        *const GUID,
        *mut *mut c_void,
    ) -> HRESULT,
    pub(super) add_ref: unsafe extern "system" fn(*mut PropChangedCallback) -> u32,
    pub(super) release: unsafe extern "system" fn(*mut PropChangedCallback) -> u32,
    pub(super) invoke: unsafe extern "system" fn(
        *mut PropChangedCallback,
        *mut c_void,
        *mut c_void,
    ) -> HRESULT,
}

pub(super) static PROP_CHANGED_VTBL: RawPropChangedCallbackVtbl = RawPropChangedCallbackVtbl {
    query_interface: prop_changed_qi,
    add_ref: prop_changed_add_ref,
    release: prop_changed_release,
    invoke: prop_changed_invoke,
};

#[repr(C)]
pub(super) struct PropChangedCallback {
    pub(super) lp_vtbl: *const RawPropChangedCallbackVtbl,
    pub(super) ref_count: AtomicU32,
    pub(super) runtime: Arc<TapRuntime>,
    pub(super) handle: u64,
    pub(super) kind: TargetKind,
    pub(super) prop: BrushPropKind,
}

impl PropChangedCallback {
    pub(super) fn new_com_ptr(
        runtime: Arc<TapRuntime>,
        handle: u64,
        kind: TargetKind,
        prop: BrushPropKind,
    ) -> *mut Self {
        ACTIVE_OBJECTS.fetch_add(1, Ordering::SeqCst);
        Box::into_raw(Box::new(Self {
            lp_vtbl: &PROP_CHANGED_VTBL,
            ref_count: AtomicU32::new(1),
            runtime,
            handle,
            kind,
            prop,
        }))
    }
}

pub(super) unsafe extern "system" fn prop_changed_qi(
    this: *mut PropChangedCallback,
    riid: *const GUID,
    ppv: *mut *mut c_void,
) -> HRESULT {
    if ppv.is_null() || riid.is_null() {
        return E_POINTER;
    }
    *ppv = null_mut();
    let iid = *riid;
    if iid == IID_IUNKNOWN
        || iid == IID_IDEPENDENCYPROPERTYCHANGEDCALLBACK
        || iid == IID_IAGILEOBJECT
    {
        *ppv = this.cast();
        prop_changed_add_ref(this);
        S_OK
    } else {
        E_NOINTERFACE
    }
}

pub(super) unsafe extern "system" fn prop_changed_add_ref(this: *mut PropChangedCallback) -> u32 {
    (*this).ref_count.fetch_add(1, Ordering::Relaxed) + 1
}

pub(super) unsafe extern "system" fn prop_changed_release(this: *mut PropChangedCallback) -> u32 {
    let prev = (*this).ref_count.fetch_sub(1, Ordering::Release);
    if prev == 1 {
        std::sync::atomic::fence(Ordering::Acquire);
        drop(Box::from_raw(this));
        ACTIVE_OBJECTS.fetch_sub(1, Ordering::SeqCst);
    }
    prev - 1
}

pub(super) unsafe extern "system" fn prop_changed_invoke(
    this: *mut PropChangedCallback,
    sender: *mut c_void,
    _dp: *mut c_void,
) -> HRESULT {
    if sender.is_null() {
        return S_OK;
    }
    let cb = &*this;
    if cb.runtime.is_shutdown() || !cb.runtime.desired_hide() {
        return S_OK;
    }
    if IN_PROP_SYNC.with(|c| c.get()) {
        return S_OK;
    }
    let _sync_guard = PropSyncGuard::enter();
    match cb.kind {
        TargetKind::OverflowBorder => {
            let border_com = qi_interface(sender, &IID_IBORDER);
            if !border_com.is_null() {
                sync_overflow_brush_prop(
                    &cb.runtime,
                    cb.handle,
                    sender,
                    border_com.as_ptr(),
                    cb.prop,
                    RawUiColor::TRANSPARENT,
                    true,
                    None,
                    DIAG_OVERFLOW_APPLIED,
                );
            }
        }
        TargetKind::AltTabBorder => {
            let border_com = qi_interface(sender, &IID_IBORDER);
            if !border_com.is_null() {
                sync_overflow_brush_prop(
                    &cb.runtime,
                    cb.handle,
                    sender,
                    border_com.as_ptr(),
                    cb.prop,
                    RawUiColor::TRANSPARENT,
                    true,
                    None,
                    DIAG_ALTTAB_APPLIED,
                );
            }
        }
        TargetKind::AltTabItemBorder => {
            let border_com = qi_interface(sender, &IID_IBORDER);
            if !border_com.is_null() {
                let target_color = match cb.prop {
                    BrushPropKind::BorderBrush => RawUiColor::from_argb(
                        cb.runtime.alt_tab_item_border_argb.load(Ordering::Acquire),
                    ),
                    _ => RawUiColor::from_argb(
                        cb.runtime.alt_tab_item_bg_argb.load(Ordering::Acquire),
                    ),
                };
                sync_overflow_brush_prop(
                    &cb.runtime,
                    cb.handle,
                    sender,
                    border_com.as_ptr(),
                    cb.prop,
                    target_color,
                    true,
                    None,
                    DIAG_ALTTAB_ITEM_APPLIED,
                );
            }
        }
        TargetKind::OverflowPanel => {
            let panel_com = qi_interface(sender, &IID_IPANEL);
            if !panel_com.is_null() {
                sync_overflow_brush_prop(
                    &cb.runtime,
                    cb.handle,
                    sender,
                    panel_com.as_ptr(),
                    cb.prop,
                    RawUiColor::TRANSPARENT,
                    true,
                    None,
                    DIAG_OVERFLOW_APPLIED,
                );
            }
        }
        TargetKind::AltTabItemFocus => {
            if matches!(cb.prop, BrushPropKind::FocusVisualSecondaryThickness) {
                sync_focus_secondary_thickness_prop(
                    &cb.runtime,
                    cb.handle,
                    sender,
                    true,
                    true,
                );
            } else {
                let fe4_com = qi_interface(sender, &IID_IFRAMEWORKELEMENT4);
                if !fe4_com.is_null() {
                    sync_overflow_brush_prop(
                        &cb.runtime,
                        cb.handle,
                        sender,
                        fe4_com.as_ptr(),
                        BrushPropKind::FocusVisualSecondaryBrush,
                        RawUiColor::TRANSPARENT,
                        true,
                        None,
                        DIAG_ALTTAB_FOCUS_APPLIED,
                    );
                }
            }
        }
        TargetKind::HoverFlyoutBorder => {
            if matches!(cb.prop, BrushPropKind::ElementShadow) {
                sync_element_shadow_prop(
                    &cb.runtime,
                    cb.handle,
                    sender,
                    true,
                    None,
                    DIAG_HOVER_FLYOUT_SHADOW_APPLIED,
                    false,
                );
            } else {
                let border_com = qi_interface(sender, &IID_IBORDER);
                if !border_com.is_null() {
                    sync_overflow_brush_prop(
                        &cb.runtime,
                        cb.handle,
                        sender,
                        border_com.as_ptr(),
                        cb.prop,
                        RawUiColor::TRANSPARENT,
                        true,
                        None,
                        DIAG_HOVER_FLYOUT_APPLIED,
                    );
                }
            }
        }
        TargetKind::OsdPanel => {
            let (target_bg, target_border) = cb.runtime.osd_target_colors();
            match cb.prop {
                BrushPropKind::PanelBackground => {
                    let panel_com = qi_interface(sender, &IID_IPANEL);
                    if !panel_com.is_null() {
                        sync_overflow_brush_prop(
                            &cb.runtime,
                            cb.handle,
                            sender,
                            panel_com.as_ptr(),
                            BrushPropKind::PanelBackground,
                            target_bg,
                            true,
                            None,
                            DIAG_OSD_APPLIED,
                        );
                    }
                }
                BrushPropKind::GridBorderBrush => {
                    let grid2_com = qi_interface(sender, &IID_IGRID2);
                    if !grid2_com.is_null() {
                        sync_overflow_brush_prop(
                            &cb.runtime,
                            cb.handle,
                            sender,
                            grid2_com.as_ptr(),
                            BrushPropKind::GridBorderBrush,
                            target_border,
                            true,
                            None,
                            DIAG_OSD_APPLIED,
                        );
                    }
                }
                BrushPropKind::ElementShadow => {
                    sync_element_shadow_prop(
                        &cb.runtime,
                        cb.handle,
                        sender,
                        true,
                        None,
                        DIAG_OSD_SHADOW_APPLIED,
                        false,
                    );
                }
                _ => {}
            }
        }
        TargetKind::ToolTipPresenter => {
            let (target_bg, target_border) = cb.runtime.tooltip_target_colors();
            match cb.prop {
                BrushPropKind::ControlBackground => {
                    let ctrl_com = qi_interface(sender, &IID_ICONTROL);
                    if !ctrl_com.is_null() {
                        sync_overflow_brush_prop(
                            &cb.runtime,
                            cb.handle,
                            sender,
                            ctrl_com.as_ptr(),
                            BrushPropKind::ControlBackground,
                            target_bg,
                            true,
                            None,
                            DIAG_TOOLTIP_APPLIED,
                        );
                    }
                }
                BrushPropKind::ControlBorderBrush => {
                    let ctrl_com = qi_interface(sender, &IID_ICONTROL);
                    if !ctrl_com.is_null() {
                        sync_overflow_brush_prop(
                            &cb.runtime,
                            cb.handle,
                            sender,
                            ctrl_com.as_ptr(),
                            BrushPropKind::ControlBorderBrush,
                            target_border,
                            true,
                            None,
                            DIAG_TOOLTIP_APPLIED,
                        );
                    }
                }
                BrushPropKind::ContentPresenterBackground => {
                    let cp_com = qi_interface(sender, &IID_ICONTENTPRESENTER4);
                    if !cp_com.is_null() {
                        let _ = sync_overflow_brush_prop_with_mode(
                            &cb.runtime,
                            cb.handle,
                            sender,
                            cp_com.as_ptr(),
                            BrushPropKind::ContentPresenterBackground,
                            target_bg,
                            true,
                            None,
                            DIAG_TOOLTIP_APPLIED,
                            true,
                        );
                    }
                }
                BrushPropKind::ContentPresenterBorderBrush => {
                    let cp_com = qi_interface(sender, &IID_ICONTENTPRESENTER4);
                    if !cp_com.is_null() {
                        let _ = sync_overflow_brush_prop_with_mode(
                            &cb.runtime,
                            cb.handle,
                            sender,
                            cp_com.as_ptr(),
                            BrushPropKind::ContentPresenterBorderBrush,
                            target_border,
                            true,
                            None,
                            DIAG_TOOLTIP_APPLIED,
                            true,
                        );
                    }
                }
                BrushPropKind::ElementShadow => {
                    let is_cp = !qi_interface(sender, &IID_ICONTENTPRESENTER4).is_null();
                    sync_element_shadow_prop(
                        &cb.runtime,
                        cb.handle,
                        sender,
                        true,
                        None,
                        DIAG_TOOLTIP_SHADOW_APPLIED,
                        is_cp,
                    );
                }
                _ => {}
            }
        }
        TargetKind::MenuFlyoutSurface => {
            let (target_bg, target_border) = cb.runtime.menu_flyout_target_colors();
            match cb.prop {
                BrushPropKind::ControlBackground | BrushPropKind::ControlBorderBrush => {
                    let ctrl_com = qi_interface(sender, &IID_ICONTROL);
                    if !ctrl_com.is_null() {
                        let col = if matches!(cb.prop, BrushPropKind::ControlBackground) {
                            target_bg
                        } else {
                            target_border
                        };
                        sync_overflow_brush_prop(
                            &cb.runtime,
                            cb.handle,
                            sender,
                            ctrl_com.as_ptr(),
                            cb.prop,
                            col,
                            true,
                            None,
                            DIAG_MENU_FLYOUT_APPLIED,
                        );
                    }
                }
                BrushPropKind::BorderBackground | BrushPropKind::BorderBrush => {
                    let border_com = qi_interface(sender, &IID_IBORDER);
                    if !border_com.is_null() {
                        let col = if matches!(cb.prop, BrushPropKind::BorderBackground) {
                            target_bg
                        } else {
                            target_border
                        };
                        let _ = sync_overflow_brush_prop_with_mode(
                            &cb.runtime,
                            cb.handle,
                            sender,
                            border_com.as_ptr(),
                            cb.prop,
                            col,
                            true,
                            None,
                            DIAG_MENU_FLYOUT_APPLIED,
                            true,
                        );
                    }
                }
                BrushPropKind::PanelBackground => {
                    let panel_com = qi_interface(sender, &IID_IPANEL);
                    if !panel_com.is_null() {
                        sync_overflow_brush_prop(
                            &cb.runtime,
                            cb.handle,
                            sender,
                            panel_com.as_ptr(),
                            BrushPropKind::PanelBackground,
                            target_bg,
                            true,
                            None,
                            DIAG_MENU_FLYOUT_APPLIED,
                        );
                    }
                }
                BrushPropKind::GridBorderBrush => {
                    let grid2_com = qi_interface(sender, &IID_IGRID2);
                    if !grid2_com.is_null() {
                        sync_overflow_brush_prop(
                            &cb.runtime,
                            cb.handle,
                            sender,
                            grid2_com.as_ptr(),
                            BrushPropKind::GridBorderBrush,
                            target_border,
                            true,
                            None,
                            DIAG_MENU_FLYOUT_APPLIED,
                        );
                    }
                }
                BrushPropKind::ElementShadow => {
                    sync_element_shadow_prop(
                        &cb.runtime,
                        cb.handle,
                        sender,
                        true,
                        None,
                        DIAG_MENU_FLYOUT_SHADOW_APPLIED,
                        false,
                    );
                }
                _ => {}
            }
        }
        TargetKind::ShellCardSurface | TargetKind::ShellCardSubSurface => {
            let (target_bg, target_border) = if cb.kind == TargetKind::ShellCardSubSurface {
                (RawUiColor::TRANSPARENT, RawUiColor::TRANSPARENT)
            } else {
                cb.runtime.menu_flyout_target_colors()
            };
            match cb.prop {
                BrushPropKind::BorderBackground => {
                    let border_com = qi_interface(sender, &IID_IBORDER);
                    if !border_com.is_null() {
                        let _ = sync_overflow_brush_prop_with_mode(
                            &cb.runtime,
                            cb.handle,
                            sender,
                            border_com.as_ptr(),
                            BrushPropKind::BorderBackground,
                            target_bg,
                            true,
                            None,
                            DIAG_SHELL_CARD_APPLIED,
                            true,
                        );
                    }
                }
                BrushPropKind::BorderBrush => {
                    let border_com = qi_interface(sender, &IID_IBORDER);
                    if !border_com.is_null() {
                        let _ = sync_overflow_brush_prop_with_mode(
                            &cb.runtime,
                            cb.handle,
                            sender,
                            border_com.as_ptr(),
                            BrushPropKind::BorderBrush,
                            target_border,
                            true,
                            None,
                            DIAG_SHELL_CARD_APPLIED,
                            true,
                        );
                    }
                }
                BrushPropKind::PanelBackground => {
                    let panel_com = qi_interface(sender, &IID_IPANEL);
                    if !panel_com.is_null() {
                        sync_overflow_brush_prop(
                            &cb.runtime,
                            cb.handle,
                            sender,
                            panel_com.as_ptr(),
                            BrushPropKind::PanelBackground,
                            target_bg,
                            true,
                            None,
                            DIAG_SHELL_CARD_APPLIED,
                        );
                    }
                }
                BrushPropKind::GridBorderBrush => {
                    let grid2_com = qi_interface(sender, &IID_IGRID2);
                    if !grid2_com.is_null() {
                        sync_overflow_brush_prop(
                            &cb.runtime,
                            cb.handle,
                            sender,
                            grid2_com.as_ptr(),
                            BrushPropKind::GridBorderBrush,
                            target_border,
                            true,
                            None,
                            DIAG_SHELL_CARD_APPLIED,
                        );
                    }
                }
                BrushPropKind::ElementShadow => {
                    sync_element_shadow_prop(
                        &cb.runtime,
                        cb.handle,
                        sender,
                        true,
                        None,
                        DIAG_SHELL_CARD_SHADOW_APPLIED,
                        false,
                    );
                }
                _ => {}
            }
        }
        TargetKind::StartMenuModalSurface => {}
        TargetKind::VirtualDesktopBarSurface => {
            let (target_bg, target_border) = cb.runtime.virtual_desktop_bar_target_colors();
            match cb.prop {
                BrushPropKind::BorderBackground => {
                    let border_com = qi_interface(sender, &IID_IBORDER);
                    if !border_com.is_null() {
                        let _ = sync_overflow_brush_prop_with_mode(
                            &cb.runtime,
                            cb.handle,
                            sender,
                            border_com.as_ptr(),
                            BrushPropKind::BorderBackground,
                            target_bg,
                            true,
                            None,
                            DIAG_VIRTUAL_DESKTOP_BAR_APPLIED,
                            true,
                        );
                    }
                }
                BrushPropKind::BorderBrush => {
                    let border_com = qi_interface(sender, &IID_IBORDER);
                    if !border_com.is_null() {
                        let _ = sync_overflow_brush_prop_with_mode(
                            &cb.runtime,
                            cb.handle,
                            sender,
                            border_com.as_ptr(),
                            BrushPropKind::BorderBrush,
                            target_border,
                            true,
                            None,
                            DIAG_VIRTUAL_DESKTOP_BAR_APPLIED,
                            true,
                        );
                    }
                }
                BrushPropKind::PanelBackground => {
                    let panel_com = qi_interface(sender, &IID_IPANEL);
                    if !panel_com.is_null() {
                        sync_overflow_brush_prop(
                            &cb.runtime,
                            cb.handle,
                            sender,
                            panel_com.as_ptr(),
                            BrushPropKind::PanelBackground,
                            target_bg,
                            true,
                            None,
                            DIAG_VIRTUAL_DESKTOP_BAR_APPLIED,
                        );
                    }
                }
                BrushPropKind::GridBorderBrush => {
                    let grid2_com = qi_interface(sender, &IID_IGRID2);
                    if !grid2_com.is_null() {
                        sync_overflow_brush_prop(
                            &cb.runtime,
                            cb.handle,
                            sender,
                            grid2_com.as_ptr(),
                            BrushPropKind::GridBorderBrush,
                            target_border,
                            true,
                            None,
                            DIAG_VIRTUAL_DESKTOP_BAR_APPLIED,
                        );
                    }
                }
                _ => {}
            }
        }
        TargetKind::ShapeElement | TargetKind::HoverFlyoutShape => {}
        TargetKind::InputSwitcherPanel => {
            let (target_bg, target_border) = cb.runtime.input_switcher_target_colors();
            match cb.prop {
                BrushPropKind::BorderBackground | BrushPropKind::BorderBrush => {
                    let border_com = qi_interface(sender, &IID_IBORDER);
                    if !border_com.is_null() {
                        let col = if matches!(cb.prop, BrushPropKind::BorderBackground) {
                            target_bg
                        } else {
                            target_border
                        };
                        sync_overflow_brush_prop(
                            &cb.runtime,
                            cb.handle,
                            sender,
                            border_com.as_ptr(),
                            cb.prop,
                            col,
                            true,
                            None,
                            DIAG_INPUT_SWITCHER_APPLIED,
                        );
                    }
                }
                BrushPropKind::PanelBackground => {
                    let panel_com = qi_interface(sender, &IID_IPANEL);
                    if !panel_com.is_null() {
                        sync_overflow_brush_prop(
                            &cb.runtime,
                            cb.handle,
                            sender,
                            panel_com.as_ptr(),
                            BrushPropKind::PanelBackground,
                            target_bg,
                            true,
                            None,
                            DIAG_INPUT_SWITCHER_APPLIED,
                        );
                    }
                }
                BrushPropKind::GridBorderBrush => {
                    let grid2_com = qi_interface(sender, &IID_IGRID2);
                    if !grid2_com.is_null() {
                        sync_overflow_brush_prop(
                            &cb.runtime,
                            cb.handle,
                            sender,
                            grid2_com.as_ptr(),
                            BrushPropKind::GridBorderBrush,
                            target_border,
                            true,
                            None,
                            DIAG_INPUT_SWITCHER_APPLIED,
                        );
                    }
                }
                BrushPropKind::ElementShadow => {
                    sync_element_shadow_prop(
                        &cb.runtime,
                        cb.handle,
                        sender,
                        true,
                        None,
                        DIAG_INPUT_SWITCHER_SHADOW_APPLIED,
                        false,
                    );
                }
                _ => {}
            }
        }
        TargetKind::InputSwitcherSubPanel => {
            match cb.prop {
                BrushPropKind::BorderBackground => {
                    let border_com = qi_interface(sender, &IID_IBORDER);
                    if !border_com.is_null() {
                        sync_overflow_brush_prop(
                            &cb.runtime,
                            cb.handle,
                            sender,
                            border_com.as_ptr(),
                            BrushPropKind::BorderBackground,
                            RawUiColor::TRANSPARENT,
                            true,
                            None,
                            DIAG_INPUT_SWITCHER_APPLIED,
                        );
                    }
                }
                BrushPropKind::PanelBackground => {
                    let panel_com = qi_interface(sender, &IID_IPANEL);
                    if !panel_com.is_null() {
                        sync_overflow_brush_prop(
                            &cb.runtime,
                            cb.handle,
                            sender,
                            panel_com.as_ptr(),
                            BrushPropKind::PanelBackground,
                            RawUiColor::TRANSPARENT,
                            true,
                            None,
                            DIAG_INPUT_SWITCHER_APPLIED,
                        );
                    }
                }
                _ => {}
            }
        }
    }
    S_OK
}

pub(super) unsafe fn prewarm_overflow_brush_store_for_current_thread() {
    let already = UI_BRUSH_STORE.with(|cell| {
        let Ok(mut s) = cell.try_borrow_mut() else {
            return true;
        };
        if s.prewarmed {
            true
        } else {
            s.prewarmed = true;
            false
        }
    });
    if already {
        return;
    }
    let _ = ensure_cached_com(
        |s| s.trans_brush.as_ref(),
        |s, v| s.trans_brush = Some(v),
        fetch_transparent_brush,
    );
    let _ = ensure_cached_com(
        |s| s.unset_value.as_ref(),
        |s, v| s.unset_value = Some(v),
        fetch_unset_value,
    );
    let _ = ensure_cached_com(
        |s| s.border_bg_dp.as_ref(),
        |s, v| s.border_bg_dp = Some(v),
        fetch_border_bg_dp,
    );
    let _ = ensure_cached_com(
        |s| s.border_brush_dp.as_ref(),
        |s, v| s.border_brush_dp = Some(v),
        fetch_border_brush_dp,
    );
    let _ = ensure_cached_com(
        |s| s.panel_bg_dp.as_ref(),
        |s, v| s.panel_bg_dp = Some(v),
        fetch_panel_bg_dp,
    );
    let _ = ensure_cached_com(
        |s| s.grid_border_brush_dp.as_ref(),
        |s, v| s.grid_border_brush_dp = Some(v),
        fetch_grid_border_brush_dp,
    );
    let _ = ensure_cached_com(
        |s| s.content_presenter_bg_dp.as_ref(),
        |s, v| s.content_presenter_bg_dp = Some(v),
        fetch_content_presenter_bg_dp,
    );
    let _ = ensure_cached_com(
        |s| s.content_presenter_border_brush_dp.as_ref(),
        |s, v| s.content_presenter_border_brush_dp = Some(v),
        fetch_content_presenter_border_brush_dp,
    );
    let _ = ensure_cached_com(
        |s| s.control_bg_dp.as_ref(),
        |s, v| s.control_bg_dp = Some(v),
        fetch_control_bg_dp,
    );
    let _ = ensure_cached_com(
        |s| s.control_border_brush_dp.as_ref(),
        |s, v| s.control_border_brush_dp = Some(v),
        fetch_control_border_brush_dp,
    );
    let _ = ensure_cached_com(
        |s| s.control_corner_radius_dp.as_ref(),
        |s, v| s.control_corner_radius_dp = Some(v),
        fetch_control_corner_radius_dp,
    );
    let _ = ensure_cached_com(
        |s| s.focus_secondary_brush_dp.as_ref(),
        |s, v| s.focus_secondary_brush_dp = Some(v),
        fetch_focus_secondary_brush_dp,
    );
    let _ = ensure_cached_com(
        |s| s.focus_secondary_thickness_dp.as_ref(),
        |s, v| s.focus_secondary_thickness_dp = Some(v),
        fetch_focus_secondary_thickness_dp,
    );
    let _ = ensure_cached_com(
        |s| s.element_shadow_dp.as_ref(),
        |s, v| s.element_shadow_dp = Some(v),
        fetch_element_shadow_dp,
    );
    let _ = cached_visual_tree_helper();
    let _ = cached_element_composition_preview();
}

pub(super) unsafe fn unregister_watch_entry(entry: PropWatchEntry) {
    if entry.token != 0 && !entry.weak_obj.is_null() && !entry.dp.is_null() {
        let wr = entry.weak_obj.as_ptr().cast::<RawIWeakReference>();
        let mut insp = ComRaw::null();
        if ((*(*wr).lp_vtbl).resolve)(
            entry.weak_obj.as_ptr(),
            &IID_IINSPECTABLE,
            &mut insp.0,
        )
        .is_ok()
            && !insp.is_null()
        {
            let dep2 = qi_interface(insp.as_ptr(), &IID_IDEPENDENCYOBJECT2);
            if !dep2.is_null() {
                let d2 = dep2.as_ptr().cast::<RawIDependencyObject2>();
                let _ = ((*(*d2).lp_vtbl).unregister_property_changed_callback)(
                    dep2.as_ptr(),
                    entry.dp.as_ptr(),
                    entry.token,
                );
            }
        }
    }
    drop(entry);
}

pub(super) unsafe fn ensure_prop_watch_registered(
    rt: &Arc<TapRuntime>,
    handle: u64,
    kind: TargetKind,
    prop: BrushPropKind,
    insp_ptr: *mut c_void,
) {
    if handle == 0 || insp_ptr.is_null() {
        return;
    }
    let key = (handle, prop.key());
    let exists = UI_BRUSH_STORE.with(|cell| {
        cell.try_borrow()
            .ok()
            .is_some_and(|s| s.watches.contains_key(&key))
    });
    if exists {
        return;
    }

    let Some(dp) = prop.dependency_property() else {
        return;
    };
    let dep2 = qi_interface(insp_ptr, &IID_IDEPENDENCYOBJECT2);
    if dep2.is_null() {
        return;
    }
    let weak_src = qi_interface(insp_ptr, &IID_IWEAKREFERENCESOURCE);
    if weak_src.is_null() {
        return;
    }
    let src_iface = weak_src.as_ptr().cast::<RawIWeakReferenceSource>();
    let mut weak_obj = ComRaw::null();
    if ((*(*src_iface).lp_vtbl).get_weak_reference)(weak_src.as_ptr(), &mut weak_obj.0)
        .is_err()
        || weak_obj.is_null()
    {
        return;
    }

    let cb = PropChangedCallback::new_com_ptr(Arc::clone(rt), handle, kind, prop);
    let d2 = dep2.as_ptr().cast::<RawIDependencyObject2>();
    let mut token = 0i64;
    let hr = ((*(*d2).lp_vtbl).register_property_changed_callback)(
        dep2.as_ptr(),
        dp.as_ptr(),
        cb.cast(),
        &mut token,
    );
    prop_changed_release(cb);
    if hr.is_err() || token == 0 {
        return;
    }

    let new_entry = PropWatchEntry {
        weak_obj,
        dp,
        token,
    };
    let dup = UI_BRUSH_STORE.with(|cell| {
        if let Ok(mut s) = cell.try_borrow_mut() {
            if let std::collections::hash_map::Entry::Vacant(e) = s.watches.entry(key) {
                e.insert(new_entry);
                return None;
            }
        }
        Some(new_entry)
    });
    if let Some(entry) = dup {
        unregister_watch_entry(entry);
    }
}

pub(super) fn clear_all_watches_for_current_thread() {
    let removed_watches: Vec<PropWatchEntry> = UI_BRUSH_STORE.with(|cell| {
        let Ok(mut s) = cell.try_borrow_mut() else {
            return Vec::new();
        };
        s.watches.drain().map(|(_, e)| e).collect()
    });
    for entry in removed_watches {
        unsafe {
            unregister_watch_entry(entry);
        }
    }
}

pub(super) unsafe fn ensure_cached_com(
    pick: fn(&UiThreadBrushStore) -> Option<&ComRaw>,
    put: fn(&mut UiThreadBrushStore, ComRaw),
    fetch: unsafe fn() -> Option<ComRaw>,
) -> Option<ComRaw> {
    if let Some(hit) = UI_BRUSH_STORE.with(|cell| {
        cell.try_borrow()
            .ok()
            .and_then(|s| pick(&s).map(ComRaw::clone_ref))
    }) {
        return Some(hit);
    }
    let created = fetch()?;
    let to_store = created.clone_ref();
    let rejected = UI_BRUSH_STORE.with(|cell| {
        if let Ok(mut s) = cell.try_borrow_mut() {
            if pick(&s).is_none() {
                put(&mut s, to_store);
                return None;
            }
        }
        Some(to_store)
    });
    drop(rejected);
    Some(created)
}

pub(super) unsafe fn cached_solid_brush(color: RawUiColor) -> Option<ComRaw> {
    if color.a == 0 {
        return ensure_cached_com(
            |s| s.trans_brush.as_ref(),
            |s, v| s.trans_brush = Some(v),
            fetch_transparent_brush,
        );
    }
    let key = color.to_argb();
    if let Some(hit) = UI_BRUSH_STORE.with(|cell| {
        cell.try_borrow()
            .ok()
            .and_then(|s| s.tinted_brushes.get(&key).map(ComRaw::clone_ref))
    }) {
        return Some(hit);
    }
    let created = fetch_solid_brush(color)?;
    let to_store = created.clone_ref();
    let (rejected, evicted) = UI_BRUSH_STORE.with(|cell| {
        if let Ok(mut s) = cell.try_borrow_mut() {
            if s.tinted_brushes.contains_key(&key) {
                return (Some(to_store), None);
            }
            let drained = if s.tinted_brushes.len() >= 32 {
                Some(s.tinted_brushes.drain().map(|(_, v)| v).collect::<Vec<_>>())
            } else {
                None
            };
            s.tinted_brushes.insert(key, to_store);
            return (None, drained);
        }
        (Some(to_store), None)
    });
    drop(rejected);
    drop(evicted);
    Some(created)
}

pub(super) fn has_saved_brush_slot(handle: u64, slot: BrushSlot) -> bool {
    UI_BRUSH_STORE.with(|cell| {
        cell.try_borrow().ok().is_some_and(|s| {
            s.saved.get(&handle).is_some_and(|entry| match slot {
                BrushSlot::Background => entry.background.is_some(),
                BrushSlot::BorderBrush => entry.border_brush.is_some(),
                BrushSlot::FocusSecondary => entry.focus_secondary.is_some(),
                BrushSlot::Shadow => entry.shadow.is_some(),
            })
        })
    })
}

pub(super) fn save_brush_slot_if_absent(handle: u64, slot: BrushSlot, val: SavedBrush) {
    let displaced = UI_BRUSH_STORE.with(|cell| {
        if let Ok(mut s) = cell.try_borrow_mut() {
            let entry = s.saved.entry(handle).or_default();
            let target = match slot {
                BrushSlot::Background => &mut entry.background,
                BrushSlot::BorderBrush => &mut entry.border_brush,
                BrushSlot::FocusSecondary => &mut entry.focus_secondary,
                BrushSlot::Shadow => &mut entry.shadow,
            };
            if target.is_none() {
                *target = Some(val);
                return None;
            }
        }
        Some(val)
    });
    drop(displaced);
}

pub(super) fn is_alttab_subtarget_tracked(key: u64) -> bool {
    UI_BRUSH_STORE.with(|cell| {
        cell.try_borrow()
            .ok()
            .is_some_and(|s| s.alttab_subtargets.contains_key(&key))
    })
}

pub(super) unsafe fn track_alttab_subtarget_weak(key: u64, kind: TargetKind, insp_ptr: *mut c_void) {
    if key == 0 || insp_ptr.is_null() || is_alttab_subtarget_tracked(key) {
        return;
    }
    let weak_src = qi_interface(insp_ptr, &IID_IWEAKREFERENCESOURCE);
    if weak_src.is_null() {
        return;
    }
    let src_iface = weak_src.as_ptr().cast::<RawIWeakReferenceSource>();
    let mut weak_ref = ComRaw::null();
    if ((*(*src_iface).lp_vtbl).get_weak_reference)(weak_src.as_ptr(), &mut weak_ref.0).is_err()
        || weak_ref.is_null()
    {
        return;
    }

    let need_prune = UI_BRUSH_STORE.with(|cell| {
        cell.try_borrow()
            .ok()
            .is_some_and(|s| s.alttab_subtargets.len() >= 256)
    });
    if need_prune {
        let candidates: Vec<(u64, ComRaw)> = UI_BRUSH_STORE.with(|cell| {
            cell.try_borrow()
                .ok()
                .map(|s| {
                    s.alttab_subtargets
                        .iter()
                        .map(|(&k, (_, w))| (k, w.clone_ref()))
                        .collect()
                })
                .unwrap_or_default()
        });
        let mut dead = Vec::new();
        for (ck, wr_com) in candidates {
            let wr = wr_com.as_ptr().cast::<RawIWeakReference>();
            let mut resolved = ComRaw::null();
            if ((*(*wr).lp_vtbl).resolve)(wr_com.as_ptr(), &IID_IINSPECTABLE, &mut resolved.0)
                .is_err()
                || resolved.is_null()
            {
                dead.push(ck);
            }
        }
        if !dead.is_empty() {
            let removed = UI_BRUSH_STORE.with(|cell| {
                let Ok(mut s) = cell.try_borrow_mut() else {
                    return Vec::new();
                };
                dead.iter()
                    .filter_map(|k| s.alttab_subtargets.remove(k))
                    .collect::<Vec<_>>()
            });
            drop(removed);
            purge_saved_brushes_for_current_thread(&dead);
        }
    }

    let displaced = UI_BRUSH_STORE.with(|cell| {
        if let Ok(mut s) = cell.try_borrow_mut() {
            if let std::collections::hash_map::Entry::Vacant(e) =
                s.alttab_subtargets.entry(key)
            {
                e.insert((kind, weak_ref));
                return None;
            }
        }
        Some(weak_ref)
    });
    drop(displaced);
}

pub(super) fn unregister_watches_for_keys(keys: &[u64]) {
    if keys.is_empty() {
        return;
    }
    let removed_watches: Vec<PropWatchEntry> = UI_BRUSH_STORE.with(|cell| {
        let Ok(mut s) = cell.try_borrow_mut() else {
            return Vec::new();
        };
        let mut watches = Vec::new();
        if !s.watches.is_empty() {
            for &k in keys {
                for prop_idx in 0u8..BrushPropKind::COUNT {
                    if let Some(entry) = s.watches.remove(&(k, prop_idx)) {
                        watches.push(entry);
                    }
                }
            }
        }
        watches
    });
    for entry in removed_watches {
        unsafe {
            unregister_watch_entry(entry);
        }
    }
}

pub(super) unsafe fn make_weak_ref_com(insp_ptr: *mut c_void) -> Option<ComRaw> {
    if insp_ptr.is_null() {
        return None;
    }
    let weak_src = qi_interface(insp_ptr, &IID_IWEAKREFERENCESOURCE);
    if weak_src.is_null() {
        return None;
    }
    let src_iface = weak_src.as_ptr().cast::<RawIWeakReferenceSource>();
    let mut weak_ref = ComRaw::null();
    if ((*(*src_iface).lp_vtbl).get_weak_reference)(weak_src.as_ptr(), &mut weak_ref.0).is_ok()
        && !weak_ref.is_null()
    {
        Some(weak_ref)
    } else {
        None
    }
}

pub(super) unsafe fn resolve_weak_inspectable(weak_com: &ComRaw) -> Option<ComRaw> {
    if weak_com.is_null() {
        return None;
    }
    let wr = weak_com.as_ptr().cast::<RawIWeakReference>();
    let mut insp = ComRaw::null();
    if ((*(*wr).lp_vtbl).resolve)(weak_com.as_ptr(), &IID_IINSPECTABLE, &mut insp.0).is_ok()
        && !insp.is_null()
    {
        Some(insp)
    } else {
        None
    }
}

pub(super) unsafe fn resolve_weak_raw_inspectable(weak_raw: usize) -> Option<ComRaw> {
    if weak_raw == 0 {
        return None;
    }
    add_ref_raw_com_addr(weak_raw);
    let weak_com = ComRaw(weak_raw as *mut c_void);
    resolve_weak_inspectable(&weak_com)
}

pub(super) unsafe fn track_paired_modified_target(
    primary_key: u64,
    primary_insp: *mut c_void,
    parent_pair: Option<(u64, *mut c_void)>,
    get_map: fn(&mut UiThreadBrushStore) -> &mut HashMap<u64, TooltipTrackedEntry>,
    read_map: fn(&UiThreadBrushStore) -> &HashMap<u64, TooltipTrackedEntry>,
) {
    if primary_key == 0 || primary_insp.is_null() {
        return;
    }
    let (already_exists, need_parent_fill, need_prune) = UI_BRUSH_STORE.with(|cell| {
        cell.try_borrow()
            .ok()
            .map(|s| {
                let map = read_map(&s);
                let exists = map.get(&primary_key);
                let fill_parent = exists.is_some_and(|e| e.tooltip_weak.is_none())
                    && parent_pair.is_some();
                (
                    exists.is_some(),
                    fill_parent,
                    exists.is_none() && map.len() >= 128,
                )
            })
            .unwrap_or((false, false, false))
    });

    if already_exists {
        if need_parent_fill {
            if let Some((pk, p_ptr)) = parent_pair {
                if let Some(p_weak) = make_weak_ref_com(p_ptr) {
                    let rejected = UI_BRUSH_STORE.with(|cell| {
                        if let Ok(mut s) = cell.try_borrow_mut() {
                            if let Some(entry) = get_map(&mut s).get_mut(&primary_key) {
                                if entry.tooltip_weak.is_none() {
                                    entry.tooltip_key = Some(pk);
                                    entry.tooltip_weak = Some(p_weak);
                                    return None;
                                }
                            }
                        }
                        Some(p_weak)
                    });
                    drop(rejected);
                }
            }
        }
        return;
    }

    if need_prune {
        let candidates: Vec<(u64, ComRaw, Option<u64>, Option<ComRaw>)> = UI_BRUSH_STORE
            .with(|cell| {
                cell.try_borrow()
                    .ok()
                    .map(|s| {
                        read_map(&s)
                            .iter()
                            .map(|(&k, e)| {
                                (
                                    k,
                                    e.presenter_weak.clone_ref(),
                                    e.tooltip_key,
                                    e.tooltip_weak.as_ref().map(ComRaw::clone_ref),
                                )
                            })
                            .collect()
                    })
                    .unwrap_or_default()
            });
        let mut dead_entry_keys = Vec::new();
        let mut dead_purge_keys = Vec::new();
        for (pk, p_weak, tk_opt, t_weak_opt) in candidates {
            let p_alive = resolve_weak_inspectable(&p_weak).is_some();
            let t_alive = t_weak_opt
                .as_ref()
                .and_then(|tw| resolve_weak_inspectable(tw))
                .is_some();
            if !p_alive && !t_alive {
                dead_entry_keys.push(pk);
                dead_purge_keys.push(pk);
                if let Some(tk) = tk_opt {
                    dead_purge_keys.push(tk);
                }
            }
        }
        if !dead_entry_keys.is_empty() {
            let removed: Vec<TooltipTrackedEntry> = UI_BRUSH_STORE.with(|cell| {
                let Ok(mut s) = cell.try_borrow_mut() else {
                    return Vec::new();
                };
                let map = get_map(&mut s);
                dead_entry_keys.iter().filter_map(|k| map.remove(k)).collect()
            });
            drop(removed);
            purge_saved_brushes_for_current_thread(&dead_purge_keys);
        }
    }

    let Some(presenter_weak) = make_weak_ref_com(primary_insp) else {
        return;
    };
    let (tooltip_key, tooltip_weak) = match parent_pair {
        Some((tk, t_ptr)) => match make_weak_ref_com(t_ptr) {
            Some(tw) => (Some(tk), Some(tw)),
            None => (Some(tk), None),
        },
        None => (None, None),
    };
    let displaced = UI_BRUSH_STORE.with(|cell| {
        if let Ok(mut s) = cell.try_borrow_mut() {
            if let std::collections::hash_map::Entry::Vacant(e) = get_map(&mut s).entry(primary_key) {
                e.insert(TooltipTrackedEntry {
                    presenter_weak,
                    tooltip_key,
                    tooltip_weak,
                });
                return None;
            }
        }
        Some((presenter_weak, tooltip_weak))
    });
    drop(displaced);
}

pub(super) unsafe fn track_tooltip_modified_target(
    presenter_key: u64,
    presenter_insp: *mut c_void,
    parent_tooltip: Option<(u64, *mut c_void)>,
) {
    track_paired_modified_target(
        presenter_key,
        presenter_insp,
        parent_tooltip,
        |s| &mut s.tooltip_modified_targets,
        |s| &s.tooltip_modified_targets,
    );
}

pub(super) unsafe fn track_menu_flyout_modified_target(
    surface_key: u64,
    surface_insp: *mut c_void,
    parent_ctrl: Option<(u64, *mut c_void)>,
) {
    track_paired_modified_target(
        surface_key,
        surface_insp,
        parent_ctrl,
        |s| &mut s.menu_flyout_modified_targets,
        |s| &s.menu_flyout_modified_targets,
    );
}

pub(super) unsafe fn track_shell_card_modified_target(surface_key: u64, surface_insp: *mut c_void) {
    track_paired_modified_target(
        surface_key,
        surface_insp,
        None,
        |s| &mut s.shell_card_modified_targets,
        |s| &s.shell_card_modified_targets,
    );
}

pub(super) fn take_saved_target_brushes(handle: u64) -> SavedTargetBrushes {
    UI_BRUSH_STORE
        .with(|cell| {
            cell.try_borrow_mut()
                .ok()
                .and_then(|mut s| s.saved.remove(&handle))
        })
        .unwrap_or_default()
}

pub(super) fn purge_saved_brushes_for_current_thread(handles: &[u64]) {
    if handles.is_empty() {
        return;
    }
    let (removed_brushes, removed_watches): (Vec<SavedTargetBrushes>, Vec<PropWatchEntry>) =
        UI_BRUSH_STORE.with(|cell| {
            let Ok(mut s) = cell.try_borrow_mut() else {
                return (Vec::new(), Vec::new());
            };
            if !s.tooltip_orig_corner_radius.is_empty() {
                for &h in handles {
                    s.tooltip_orig_corner_radius.remove(&h);
                }
            }
            if !s.menu_flyout_orig_corner_radius.is_empty() {
                for &h in handles {
                    s.menu_flyout_orig_corner_radius.remove(&h);
                }
            }
            if !s.shell_card_orig_corner_radius.is_empty() {
                for &h in handles {
                    s.shell_card_orig_corner_radius.remove(&h);
                }
            }
            if !s.shell_card_orig_comp_visible.is_empty() {
                for &h in handles {
                    s.shell_card_orig_comp_visible.remove(&h);
                }
            }
            if !s.start_modal_applied_style.is_empty() {
                for &h in handles {
                    s.start_modal_applied_style.remove(&h);
                }
            }
            let brushes: Vec<SavedTargetBrushes> = if s.saved.is_empty() {
                Vec::new()
            } else {
                handles.iter().filter_map(|h| s.saved.remove(h)).collect()
            };
            if !s.alttab_item_orig_comp_visible.is_empty() {
                for &h in handles {
                    s.alttab_item_orig_comp_visible.remove(&h);
                }
            }
            if !s.alttab_item_orig_focus_thickness.is_empty() {
                for &h in handles {
                    s.alttab_item_orig_focus_thickness.remove(&h);
                }
            }
            let mut watches = Vec::new();
            if !s.watches.is_empty() {
                for &h in handles {
                    for prop_idx in 0u8..BrushPropKind::COUNT {
                        if let Some(entry) = s.watches.remove(&(h, prop_idx)) {
                            watches.push(entry);
                        }
                    }
                }
            }
            (brushes, watches)
        });
    drop(removed_brushes);
    for entry in removed_watches {
        unsafe {
            unregister_watch_entry(entry);
        }
    }
}

pub(super) unsafe fn read_orig_brush(
    insp_ptr: *mut c_void,
    dp: Option<&ComRaw>,
    unset_val: Option<&ComRaw>,
    current_brush: &ComRaw,
) -> Option<SavedBrush> {
    if let (Some(dp_com), Some(unset_com)) = (dp, unset_val) {
        let dep_com = qi_interface(insp_ptr, &IID_IDEPENDENCYOBJECT);
        if !dep_com.is_null() {
            let dep = dep_com.as_ptr().cast::<RawIDependencyObject>();
            let mut local_val = ComRaw::null();
            if ((*(*dep).lp_vtbl).read_local_value)(
                dep_com.as_ptr(),
                dp_com.as_ptr(),
                &mut local_val.0,
            )
            .is_ok()
                && !local_val.is_null()
            {
                if com_same_identity(local_val.as_ptr(), unset_com.as_ptr()) {
                    return Some(SavedBrush::Unset);
                }
                let local_brush = qi_interface(local_val.as_ptr(), &IID_IBRUSH);
                if !local_brush.is_null() && !is_transparent_solid_brush(local_brush.as_ptr())
                {
                    return Some(SavedBrush::Local(local_brush));
                }
            }
        }
    }
    if !current_brush.is_null() && !is_transparent_solid_brush(current_brush.as_ptr()) {
        Some(SavedBrush::Local(current_brush.clone_ref()))
    } else if current_brush.is_null() {
        Some(SavedBrush::Unset)
    } else {
        None
    }
}

pub(super) unsafe fn clear_dp_value(insp_ptr: *mut c_void, dp: Option<&ComRaw>) -> bool {
    let Some(dp_com) = dp else {
        return false;
    };
    let dep_com = qi_interface(insp_ptr, &IID_IDEPENDENCYOBJECT);
    if dep_com.is_null() {
        return false;
    }
    let dep = dep_com.as_ptr().cast::<RawIDependencyObject>();
    ((*(*dep).lp_vtbl).clear_value)(dep_com.as_ptr(), dp_com.as_ptr()).is_ok()
}

#[derive(Clone, Copy)]
pub(super) enum BrushPropKind {
    BorderBackground,
    BorderBrush,
    PanelBackground,
    FocusVisualSecondaryBrush,
    ElementShadow,
    FocusVisualSecondaryThickness,
    GridBorderBrush,
    ContentPresenterBackground,
    ContentPresenterBorderBrush,
    ControlBackground,
    ControlBorderBrush,
}

impl BrushPropKind {
    const COUNT: u8 = 11;

    #[inline]
    pub(super) fn key(self) -> u8 {
        match self {
            Self::BorderBackground => 0,
            Self::BorderBrush => 1,
            Self::PanelBackground => 2,
            Self::FocusVisualSecondaryBrush => 3,
            Self::ElementShadow => 4,
            Self::FocusVisualSecondaryThickness => 5,
            Self::GridBorderBrush => 6,
            Self::ContentPresenterBackground => 7,
            Self::ContentPresenterBorderBrush => 8,
            Self::ControlBackground => 9,
            Self::ControlBorderBrush => 10,
        }
    }

    #[inline]
    pub(super) fn brush_slot(self) -> BrushSlot {
        match self {
            Self::BorderBrush
            | Self::GridBorderBrush
            | Self::ContentPresenterBorderBrush
            | Self::ControlBorderBrush => BrushSlot::BorderBrush,
            Self::FocusVisualSecondaryBrush | Self::FocusVisualSecondaryThickness => {
                BrushSlot::FocusSecondary
            }
            Self::ElementShadow => BrushSlot::Shadow,
            Self::BorderBackground
            | Self::PanelBackground
            | Self::ContentPresenterBackground
            | Self::ControlBackground => BrushSlot::Background,
        }
    }

    #[inline]
    pub(super) fn allow_null_on_hide(self) -> bool {
        matches!(
            self,
            Self::BorderBackground | Self::FocusVisualSecondaryBrush
        )
    }

    pub(super) unsafe fn get(self, iface: *mut c_void) -> (HRESULT, ComRaw) {
        let mut cur = ComRaw::null();
        let hr = match self {
            Self::BorderBackground => {
                let b = iface.cast::<RawIBorder>();
                ((*(*b).lp_vtbl).get_background)(iface, &mut cur.0)
            }
            Self::BorderBrush => {
                let b = iface.cast::<RawIBorder>();
                ((*(*b).lp_vtbl).get_border_brush)(iface, &mut cur.0)
            }
            Self::PanelBackground => {
                let p = iface.cast::<RawIPanel>();
                ((*(*p).lp_vtbl).get_background)(iface, &mut cur.0)
            }
            Self::GridBorderBrush => {
                let g = iface.cast::<RawIGrid2>();
                ((*(*g).lp_vtbl).get_border_brush)(iface, &mut cur.0)
            }
            Self::ContentPresenterBackground => {
                let cp = iface.cast::<RawIContentPresenter4>();
                ((*(*cp).lp_vtbl).get_background)(iface, &mut cur.0)
            }
            Self::ContentPresenterBorderBrush => {
                let cp = iface.cast::<RawIContentPresenter4>();
                ((*(*cp).lp_vtbl).get_border_brush)(iface, &mut cur.0)
            }
            Self::ControlBackground => {
                let c = iface.cast::<RawIControl>();
                ((*(*c).lp_vtbl).get_background)(iface, &mut cur.0)
            }
            Self::ControlBorderBrush => {
                let c = iface.cast::<RawIControl>();
                ((*(*c).lp_vtbl).get_border_brush)(iface, &mut cur.0)
            }
            Self::FocusVisualSecondaryBrush => {
                let fe4 = iface.cast::<RawIFrameworkElement4>();
                ((*(*fe4).lp_vtbl).get_focus_visual_secondary_brush)(iface, &mut cur.0)
            }
            Self::ElementShadow => {
                let ui10 = iface.cast::<RawIUIElement10>();
                ((*(*ui10).lp_vtbl).get_shadow)(iface, &mut cur.0)
            }
            Self::FocusVisualSecondaryThickness => E_FAIL_HR,
        };
        (hr, cur)
    }

    pub(super) unsafe fn put(self, iface: *mut c_void, brush: *mut c_void) -> HRESULT {
        match self {
            Self::BorderBackground => {
                let b = iface.cast::<RawIBorder>();
                ((*(*b).lp_vtbl).put_background)(iface, brush)
            }
            Self::BorderBrush => {
                let b = iface.cast::<RawIBorder>();
                ((*(*b).lp_vtbl).put_border_brush)(iface, brush)
            }
            Self::PanelBackground => {
                let p = iface.cast::<RawIPanel>();
                ((*(*p).lp_vtbl).put_background)(iface, brush)
            }
            Self::GridBorderBrush => {
                let g = iface.cast::<RawIGrid2>();
                ((*(*g).lp_vtbl).put_border_brush)(iface, brush)
            }
            Self::ContentPresenterBackground => {
                let cp = iface.cast::<RawIContentPresenter4>();
                ((*(*cp).lp_vtbl).put_background)(iface, brush)
            }
            Self::ContentPresenterBorderBrush => {
                let cp = iface.cast::<RawIContentPresenter4>();
                ((*(*cp).lp_vtbl).put_border_brush)(iface, brush)
            }
            Self::ControlBackground => {
                let c = iface.cast::<RawIControl>();
                ((*(*c).lp_vtbl).put_background)(iface, brush)
            }
            Self::ControlBorderBrush => {
                let c = iface.cast::<RawIControl>();
                ((*(*c).lp_vtbl).put_border_brush)(iface, brush)
            }
            Self::FocusVisualSecondaryBrush => {
                let fe4 = iface.cast::<RawIFrameworkElement4>();
                ((*(*fe4).lp_vtbl).put_focus_visual_secondary_brush)(iface, brush)
            }
            Self::ElementShadow => {
                let ui10 = iface.cast::<RawIUIElement10>();
                ((*(*ui10).lp_vtbl).put_shadow)(iface, brush)
            }
            Self::FocusVisualSecondaryThickness => E_FAIL_HR,
        }
    }

    pub(super) unsafe fn dependency_property(self) -> Option<ComRaw> {
        match self {
            Self::BorderBackground => ensure_cached_com(
                |s| s.border_bg_dp.as_ref(),
                |s, v| s.border_bg_dp = Some(v),
                fetch_border_bg_dp,
            ),
            Self::BorderBrush => ensure_cached_com(
                |s| s.border_brush_dp.as_ref(),
                |s, v| s.border_brush_dp = Some(v),
                fetch_border_brush_dp,
            ),
            Self::PanelBackground => ensure_cached_com(
                |s| s.panel_bg_dp.as_ref(),
                |s, v| s.panel_bg_dp = Some(v),
                fetch_panel_bg_dp,
            ),
            Self::GridBorderBrush => ensure_cached_com(
                |s| s.grid_border_brush_dp.as_ref(),
                |s, v| s.grid_border_brush_dp = Some(v),
                fetch_grid_border_brush_dp,
            ),
            Self::ContentPresenterBackground => ensure_cached_com(
                |s| s.content_presenter_bg_dp.as_ref(),
                |s, v| s.content_presenter_bg_dp = Some(v),
                fetch_content_presenter_bg_dp,
            ),
            Self::ContentPresenterBorderBrush => ensure_cached_com(
                |s| s.content_presenter_border_brush_dp.as_ref(),
                |s, v| s.content_presenter_border_brush_dp = Some(v),
                fetch_content_presenter_border_brush_dp,
            ),
            Self::ControlBackground => ensure_cached_com(
                |s| s.control_bg_dp.as_ref(),
                |s, v| s.control_bg_dp = Some(v),
                fetch_control_bg_dp,
            ),
            Self::ControlBorderBrush => ensure_cached_com(
                |s| s.control_border_brush_dp.as_ref(),
                |s, v| s.control_border_brush_dp = Some(v),
                fetch_control_border_brush_dp,
            ),
            Self::FocusVisualSecondaryBrush => ensure_cached_com(
                |s| s.focus_secondary_brush_dp.as_ref(),
                |s, v| s.focus_secondary_brush_dp = Some(v),
                fetch_focus_secondary_brush_dp,
            ),
            Self::ElementShadow => ensure_cached_com(
                |s| s.element_shadow_dp.as_ref(),
                |s, v| s.element_shadow_dp = Some(v),
                fetch_element_shadow_dp,
            ),
            Self::FocusVisualSecondaryThickness => ensure_cached_com(
                |s| s.focus_secondary_thickness_dp.as_ref(),
                |s, v| s.focus_secondary_thickness_dp = Some(v),
                fetch_focus_secondary_thickness_dp,
            ),
        }
    }
}

