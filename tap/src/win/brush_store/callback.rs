//! WinRT DependencyPropertyChangedCallback COM object for re-asserting or recording native brush updates.

use super::super::appearance::{
    sync_element_shadow_prop, sync_focus_secondary_thickness_prop, sync_overflow_brush_prop,
    sync_overflow_brush_prop_with_mode,
};
use super::super::super::tree_match::TargetKind;
use super::super::raw_com::*;
use super::super::TapRuntime;
use super::*;
use std::ffi::c_void;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use windows::core::{GUID, HRESULT};
use windows::Win32::Foundation::{E_NOINTERFACE, E_POINTER, S_OK};

#[repr(C)]
pub(in super::super) struct RawPropChangedCallbackVtbl {
    pub(in super::super) query_interface: unsafe extern "system" fn(
        *mut PropChangedCallback,
        *const GUID,
        *mut *mut c_void,
    ) -> HRESULT,
    pub(in super::super) add_ref: unsafe extern "system" fn(*mut PropChangedCallback) -> u32,
    pub(in super::super) release: unsafe extern "system" fn(*mut PropChangedCallback) -> u32,
    pub(in super::super) invoke: unsafe extern "system" fn(
        *mut PropChangedCallback,
        *mut c_void,
        *mut c_void,
    ) -> HRESULT,
}

pub(in super::super) static PROP_CHANGED_VTBL: RawPropChangedCallbackVtbl = RawPropChangedCallbackVtbl {
    query_interface: prop_changed_qi,
    add_ref: prop_changed_add_ref,
    release: prop_changed_release,
    invoke: prop_changed_invoke,
};

#[repr(C)]
pub(in super::super) struct PropChangedCallback {
    pub(in super::super) lp_vtbl: *const RawPropChangedCallbackVtbl,
    pub(in super::super) ref_count: AtomicU32,
    pub(in super::super) runtime: Arc<TapRuntime>,
    pub(in super::super) handle: u64,
    pub(in super::super) kind: TargetKind,
    pub(in super::super) prop: BrushPropKind,
}

impl PropChangedCallback {
    pub(in super::super) fn new_com_ptr(
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

pub(in super::super) unsafe extern "system" fn prop_changed_qi(
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

pub(in super::super) unsafe extern "system" fn prop_changed_add_ref(this: *mut PropChangedCallback) -> u32 {
    (*this).ref_count.fetch_add(1, Ordering::Relaxed) + 1
}

pub(in super::super) unsafe extern "system" fn prop_changed_release(this: *mut PropChangedCallback) -> u32 {
    let prev = (*this).ref_count.fetch_sub(1, Ordering::Release);
    if prev == 1 {
        std::sync::atomic::fence(Ordering::Acquire);
        drop(Box::from_raw(this));
        ACTIVE_OBJECTS.fetch_sub(1, Ordering::SeqCst);
    }
    prev - 1
}

pub(in super::super) unsafe extern "system" fn prop_changed_invoke(
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
