//! Raw COM/WinRT vtable definitions for XAML controls, brushes, dependency objects, and weak references.

use super::{RawCornerRadius, RawThickness};
use std::ffi::c_void;
use windows::core::{GUID, HRESULT};

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) struct RawUiColor {
    pub(in super::super) a: u8,
    pub(in super::super) r: u8,
    pub(in super::super) g: u8,
    pub(in super::super) b: u8,
}

impl RawUiColor {
    pub(in super::super) const TRANSPARENT: Self = Self {
        a: 0,
        r: 0,
        g: 0,
        b: 0,
    };

    pub(in super::super) const fn from_argb(argb: u32) -> Self {
        Self {
            a: ((argb >> 24) & 0xFF) as u8,
            r: ((argb >> 16) & 0xFF) as u8,
            g: ((argb >> 8) & 0xFF) as u8,
            b: (argb & 0xFF) as u8,
        }
    }

    pub(in super::super) const fn to_argb(self) -> u32 {
        ((self.a as u32) << 24)
            | ((self.r as u32) << 16)
            | ((self.g as u32) << 8)
            | (self.b as u32)
    }
}

const _: () = {
    assert!(size_of::<RawUiColor>() == 4);
    assert!(core::mem::align_of::<RawUiColor>() == 1);
};

#[repr(C)]
#[allow(dead_code)]
pub(in super::super) struct RawIBorderVtbl {
    pub(in super::super) query_interface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    pub(in super::super) add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(in super::super) release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(in super::super) get_iids: unsafe extern "system" fn(*mut c_void),
    pub(in super::super) get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
    pub(in super::super) get_trust_level: unsafe extern "system" fn(*mut c_void),
    pub(in super::super) get_border_brush: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    pub(in super::super) put_border_brush: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
    pub(in super::super) get_border_thickness: unsafe extern "system" fn(*mut c_void, *mut RawThickness) -> HRESULT,
    pub(in super::super) put_border_thickness: unsafe extern "system" fn(*mut c_void, RawThickness) -> HRESULT,
    pub(in super::super) get_background: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    pub(in super::super) put_background: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
    pub(in super::super) get_corner_radius: unsafe extern "system" fn(*mut c_void, *mut RawCornerRadius) -> HRESULT,
    pub(in super::super) put_corner_radius: unsafe extern "system" fn(*mut c_void, RawCornerRadius) -> HRESULT,
}

#[repr(C)]
pub(in super::super) struct RawIBorder {
    pub(in super::super) lp_vtbl: *const RawIBorderVtbl,
}

#[repr(C)]
#[allow(dead_code)]
pub(in super::super) struct RawIPanelVtbl {
    pub(in super::super) query_interface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    pub(in super::super) add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(in super::super) release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(in super::super) get_iids: unsafe extern "system" fn(*mut c_void),
    pub(in super::super) get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
    pub(in super::super) get_trust_level: unsafe extern "system" fn(*mut c_void),
    pub(in super::super) get_children: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    pub(in super::super) get_background: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    pub(in super::super) put_background: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
}

#[repr(C)]
pub(in super::super) struct RawIPanel {
    pub(in super::super) lp_vtbl: *const RawIPanelVtbl,
}

#[repr(C)]
#[allow(dead_code)]
pub(in super::super) struct RawIUIElementVectorVtbl {
    pub(in super::super) query_interface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    pub(in super::super) add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(in super::super) release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(in super::super) get_iids: unsafe extern "system" fn(*mut c_void),
    pub(in super::super) get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
    pub(in super::super) get_trust_level: unsafe extern "system" fn(*mut c_void),
    pub(in super::super) get_at: unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> HRESULT,
    pub(in super::super) get_size: unsafe extern "system" fn(*mut c_void, *mut u32) -> HRESULT,
    pub(in super::super) _pad8_10: [*const c_void; 3],
    pub(in super::super) insert_at: unsafe extern "system" fn(*mut c_void, u32, *mut c_void) -> HRESULT,
    pub(in super::super) remove_at: unsafe extern "system" fn(*mut c_void, u32) -> HRESULT,
}

#[repr(C)]
pub(in super::super) struct RawIUIElementVector {
    pub(in super::super) lp_vtbl: *const RawIUIElementVectorVtbl,
}

#[repr(C)]
#[allow(dead_code)]
pub(in super::super) struct RawIElementCompositionPreviewStaticsVtbl {
    pub(in super::super) query_interface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    pub(in super::super) add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(in super::super) release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(in super::super) get_iids: unsafe extern "system" fn(*mut c_void),
    pub(in super::super) get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
    pub(in super::super) get_trust_level: unsafe extern "system" fn(*mut c_void),
    pub(in super::super) get_element_visual:
        unsafe extern "system" fn(*mut c_void, *mut c_void, *mut *mut c_void) -> HRESULT,
    pub(in super::super) get_element_child_visual:
        unsafe extern "system" fn(*mut c_void, *mut c_void, *mut *mut c_void) -> HRESULT,
    pub(in super::super) set_element_child_visual:
        unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> HRESULT,
}

#[repr(C)]
pub(in super::super) struct RawIElementCompositionPreviewStatics {
    pub(in super::super) lp_vtbl: *const RawIElementCompositionPreviewStaticsVtbl,
}

#[repr(C)]
#[allow(dead_code)]
pub(in super::super) struct RawIVisualVtbl {
    pub(in super::super) query_interface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    pub(in super::super) add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(in super::super) release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(in super::super) get_iids: unsafe extern "system" fn(*mut c_void),
    pub(in super::super) get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
    pub(in super::super) get_trust_level: unsafe extern "system" fn(*mut c_void),
    pub(in super::super) _pad6_17: [*const c_void; 12],
    pub(in super::super) get_is_visible: unsafe extern "system" fn(*mut c_void, *mut u8) -> HRESULT,
    pub(in super::super) put_is_visible: unsafe extern "system" fn(*mut c_void, u8) -> HRESULT,
}

#[repr(C)]
pub(in super::super) struct RawIVisual {
    pub(in super::super) lp_vtbl: *const RawIVisualVtbl,
}

#[repr(C)]
#[allow(dead_code)]
pub(in super::super) struct RawISolidColorBrushVtbl {
    pub(in super::super) query_interface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    pub(in super::super) add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(in super::super) release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(in super::super) get_iids: unsafe extern "system" fn(*mut c_void),
    pub(in super::super) get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
    pub(in super::super) get_trust_level: unsafe extern "system" fn(*mut c_void),
    pub(in super::super) get_color: unsafe extern "system" fn(*mut c_void, *mut RawUiColor) -> HRESULT,
    pub(in super::super) put_color: unsafe extern "system" fn(*mut c_void, RawUiColor) -> HRESULT,
}

#[repr(C)]
pub(in super::super) struct RawISolidColorBrush {
    pub(in super::super) lp_vtbl: *const RawISolidColorBrushVtbl,
}

#[repr(C)]
#[allow(dead_code)]
pub(in super::super) struct RawISolidColorBrushFactoryVtbl {
    pub(in super::super) query_interface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    pub(in super::super) add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(in super::super) release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(in super::super) get_iids: unsafe extern "system" fn(*mut c_void),
    pub(in super::super) get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
    pub(in super::super) get_trust_level: unsafe extern "system" fn(*mut c_void),
    pub(in super::super) create_instance_with_color:
        unsafe extern "system" fn(*mut c_void, RawUiColor, *mut *mut c_void) -> HRESULT,
}

#[repr(C)]
pub(in super::super) struct RawISolidColorBrushFactory {
    pub(in super::super) lp_vtbl: *const RawISolidColorBrushFactoryVtbl,
}

#[repr(C)]
#[allow(dead_code)]
pub(in super::super) struct RawIGridStaticsVtbl {
    pub(in super::super) query_interface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    pub(in super::super) add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(in super::super) release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(in super::super) get_iids: unsafe extern "system" fn(*mut c_void),
    pub(in super::super) get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
    pub(in super::super) get_trust_level: unsafe extern "system" fn(*mut c_void),
    pub(in super::super) get_row_property: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    pub(in super::super) get_row: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut i32) -> HRESULT,
    pub(in super::super) set_row: unsafe extern "system" fn(*mut c_void, *mut c_void, i32) -> HRESULT,
    pub(in super::super) get_column_property:
        unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    pub(in super::super) get_column: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut i32) -> HRESULT,
    pub(in super::super) set_column: unsafe extern "system" fn(*mut c_void, *mut c_void, i32) -> HRESULT,
    pub(in super::super) get_row_span_property:
        unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    pub(in super::super) get_row_span:
        unsafe extern "system" fn(*mut c_void, *mut c_void, *mut i32) -> HRESULT,
    pub(in super::super) set_row_span: unsafe extern "system" fn(*mut c_void, *mut c_void, i32) -> HRESULT,
    pub(in super::super) get_column_span_property:
        unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    pub(in super::super) get_column_span:
        unsafe extern "system" fn(*mut c_void, *mut c_void, *mut i32) -> HRESULT,
    pub(in super::super) set_column_span: unsafe extern "system" fn(*mut c_void, *mut c_void, i32) -> HRESULT,
}

#[repr(C)]
pub(in super::super) struct RawIGridStatics {
    pub(in super::super) lp_vtbl: *const RawIGridStaticsVtbl,
}

#[repr(C)]
#[allow(dead_code)]
pub(in super::super) struct RawIDependencyObjectVtbl {
    pub(in super::super) query_interface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    pub(in super::super) add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(in super::super) release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(in super::super) get_iids: unsafe extern "system" fn(*mut c_void),
    pub(in super::super) get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
    pub(in super::super) get_trust_level: unsafe extern "system" fn(*mut c_void),
    pub(in super::super) get_value: unsafe extern "system" fn(*mut c_void),
    pub(in super::super) set_value: unsafe extern "system" fn(*mut c_void),
    pub(in super::super) clear_value: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
    pub(in super::super) read_local_value:
        unsafe extern "system" fn(*mut c_void, *mut c_void, *mut *mut c_void) -> HRESULT,
    pub(in super::super) get_animation_base_value: unsafe extern "system" fn(*mut c_void),
    pub(in super::super) get_dispatcher: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
}

#[repr(C)]
pub(in super::super) struct RawIDependencyObject {
    pub(in super::super) lp_vtbl: *const RawIDependencyObjectVtbl,
}

#[repr(C)]
pub(in super::super) struct RawIDependencyObject2Vtbl {
    pub(in super::super) query_interface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    pub(in super::super) add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(in super::super) release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(in super::super) get_iids: unsafe extern "system" fn(*mut c_void),
    pub(in super::super) get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
    pub(in super::super) get_trust_level: unsafe extern "system" fn(*mut c_void),
    pub(in super::super) register_property_changed_callback: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        *mut c_void,
        *mut i64,
    ) -> HRESULT,
    pub(in super::super) unregister_property_changed_callback:
        unsafe extern "system" fn(*mut c_void, *mut c_void, i64) -> HRESULT,
}

#[repr(C)]
pub(in super::super) struct RawIDependencyObject2 {
    pub(in super::super) lp_vtbl: *const RawIDependencyObject2Vtbl,
}

#[repr(C)]
#[allow(dead_code)]
pub(in super::super) struct RawIBorderStaticsVtbl {
    pub(in super::super) query_interface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    pub(in super::super) add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(in super::super) release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(in super::super) get_iids: unsafe extern "system" fn(*mut c_void),
    pub(in super::super) get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
    pub(in super::super) get_trust_level: unsafe extern "system" fn(*mut c_void),
    pub(in super::super) get_border_brush_property:
        unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    pub(in super::super) get_border_thickness_property:
        unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    pub(in super::super) get_background_property:
        unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
}

#[repr(C)]
pub(in super::super) struct RawIBorderStatics {
    pub(in super::super) lp_vtbl: *const RawIBorderStaticsVtbl,
}

#[repr(C)]
pub(in super::super) struct RawIPanelStaticsVtbl {
    pub(in super::super) query_interface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    pub(in super::super) add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(in super::super) release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(in super::super) get_iids: unsafe extern "system" fn(*mut c_void),
    pub(in super::super) get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
    pub(in super::super) get_trust_level: unsafe extern "system" fn(*mut c_void),
    pub(in super::super) get_background_property:
        unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
}

#[repr(C)]
pub(in super::super) struct RawIPanelStatics {
    pub(in super::super) lp_vtbl: *const RawIPanelStaticsVtbl,
}

#[repr(C)]
#[allow(dead_code)]
pub(in super::super) struct RawIGrid2Vtbl {
    pub(in super::super) query_interface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    pub(in super::super) add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(in super::super) release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(in super::super) get_iids: unsafe extern "system" fn(*mut c_void),
    pub(in super::super) get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
    pub(in super::super) get_trust_level: unsafe extern "system" fn(*mut c_void),
    pub(in super::super) get_border_brush: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    pub(in super::super) put_border_brush: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
}

#[repr(C)]
pub(in super::super) struct RawIGrid2 {
    pub(in super::super) lp_vtbl: *const RawIGrid2Vtbl,
}

#[repr(C)]
#[allow(dead_code)]
pub(in super::super) struct RawIGridStatics2Vtbl {
    pub(in super::super) query_interface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    pub(in super::super) add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(in super::super) release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(in super::super) get_iids: unsafe extern "system" fn(*mut c_void),
    pub(in super::super) get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
    pub(in super::super) get_trust_level: unsafe extern "system" fn(*mut c_void),
    pub(in super::super) get_border_brush_property:
        unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
}

#[repr(C)]
pub(in super::super) struct RawIGridStatics2 {
    pub(in super::super) lp_vtbl: *const RawIGridStatics2Vtbl,
}

#[repr(C)]
#[allow(dead_code)]
pub(in super::super) struct RawIContentPresenter4Vtbl {
    pub(in super::super) query_interface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    pub(in super::super) add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(in super::super) release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(in super::super) get_iids: unsafe extern "system" fn(*mut c_void),
    pub(in super::super) get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
    pub(in super::super) get_trust_level: unsafe extern "system" fn(*mut c_void),
    pub(in super::super) _pad6_13: [*const c_void; 8],
    pub(in super::super) get_border_brush: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    pub(in super::super) put_border_brush: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
    pub(in super::super) get_border_thickness: unsafe extern "system" fn(*mut c_void, *mut RawThickness) -> HRESULT,
    pub(in super::super) put_border_thickness: unsafe extern "system" fn(*mut c_void, RawThickness) -> HRESULT,
    pub(in super::super) get_corner_radius: unsafe extern "system" fn(*mut c_void, *mut RawCornerRadius) -> HRESULT,
    pub(in super::super) put_corner_radius: unsafe extern "system" fn(*mut c_void, RawCornerRadius) -> HRESULT,
    pub(in super::super) get_padding: unsafe extern "system" fn(*mut c_void, *mut RawThickness) -> HRESULT,
    pub(in super::super) put_padding: unsafe extern "system" fn(*mut c_void, RawThickness) -> HRESULT,
    pub(in super::super) get_background: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    pub(in super::super) put_background: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
}

#[repr(C)]
pub(in super::super) struct RawIContentPresenter4 {
    pub(in super::super) lp_vtbl: *const RawIContentPresenter4Vtbl,
}

#[repr(C)]
#[allow(dead_code)]
pub(in super::super) struct RawIContentPresenterStatics4Vtbl {
    pub(in super::super) query_interface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    pub(in super::super) add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(in super::super) release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(in super::super) get_iids: unsafe extern "system" fn(*mut c_void),
    pub(in super::super) get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
    pub(in super::super) get_trust_level: unsafe extern "system" fn(*mut c_void),
    pub(in super::super) _pad6_9: [*const c_void; 4],
    pub(in super::super) get_border_brush_property:
        unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    pub(in super::super) _pad11_13: [*const c_void; 3],
    pub(in super::super) get_background_property:
        unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
}

#[repr(C)]
pub(in super::super) struct RawIContentPresenterStatics4 {
    pub(in super::super) lp_vtbl: *const RawIContentPresenterStatics4Vtbl,
}

#[repr(C)]
#[allow(dead_code)]
pub(in super::super) struct RawIToolTipVtbl {
    pub(in super::super) query_interface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    pub(in super::super) add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(in super::super) release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(in super::super) get_iids: unsafe extern "system" fn(*mut c_void),
    pub(in super::super) get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
    pub(in super::super) get_trust_level: unsafe extern "system" fn(*mut c_void),
    pub(in super::super) get_horizontal_offset: unsafe extern "system" fn(*mut c_void, *mut f64) -> HRESULT,
    pub(in super::super) put_horizontal_offset: unsafe extern "system" fn(*mut c_void, f64) -> HRESULT,
    pub(in super::super) get_is_open: unsafe extern "system" fn(*mut c_void, *mut u8) -> HRESULT,
}

#[repr(C)]
pub(in super::super) struct RawIToolTip {
    pub(in super::super) lp_vtbl: *const RawIToolTipVtbl,
}

#[repr(C)]
#[allow(dead_code)]
pub(in super::super) struct RawIControlVtbl {
    pub(in super::super) query_interface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    pub(in super::super) add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(in super::super) release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(in super::super) get_iids: unsafe extern "system" fn(*mut c_void),
    pub(in super::super) get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
    pub(in super::super) get_trust_level: unsafe extern "system" fn(*mut c_void),
    pub(in super::super) _pad6_35: [*const c_void; 30],
    pub(in super::super) get_background: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    pub(in super::super) put_background: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
    pub(in super::super) _pad38_39: [*const c_void; 2],
    pub(in super::super) get_border_brush: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    pub(in super::super) put_border_brush: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
}

#[repr(C)]
pub(in super::super) struct RawIControl {
    pub(in super::super) lp_vtbl: *const RawIControlVtbl,
}

#[repr(C)]
#[allow(dead_code)]
pub(in super::super) struct RawIControl7Vtbl {
    pub(in super::super) query_interface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    pub(in super::super) add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(in super::super) release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(in super::super) get_iids: unsafe extern "system" fn(*mut c_void),
    pub(in super::super) get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
    pub(in super::super) get_trust_level: unsafe extern "system" fn(*mut c_void),
    pub(in super::super) _pad6_7: [*const c_void; 2],
    pub(in super::super) get_corner_radius: unsafe extern "system" fn(*mut c_void, *mut RawCornerRadius) -> HRESULT,
    pub(in super::super) put_corner_radius: unsafe extern "system" fn(*mut c_void, RawCornerRadius) -> HRESULT,
}

#[repr(C)]
pub(in super::super) struct RawIControl7 {
    pub(in super::super) lp_vtbl: *const RawIControl7Vtbl,
}

#[repr(C)]
#[allow(dead_code)]
pub(in super::super) struct RawIControlStaticsVtbl {
    pub(in super::super) query_interface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    pub(in super::super) add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(in super::super) release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(in super::super) get_iids: unsafe extern "system" fn(*mut c_void),
    pub(in super::super) get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
    pub(in super::super) get_trust_level: unsafe extern "system" fn(*mut c_void),
    pub(in super::super) _pad6_20: [*const c_void; 15],
    pub(in super::super) get_background_property:
        unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    pub(in super::super) _pad22: *const c_void,
    pub(in super::super) get_border_brush_property:
        unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
}

#[repr(C)]
pub(in super::super) struct RawIControlStatics {
    pub(in super::super) lp_vtbl: *const RawIControlStaticsVtbl,
}

#[repr(C)]
#[allow(dead_code)]
pub(in super::super) struct RawIControlStatics7Vtbl {
    pub(in super::super) query_interface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    pub(in super::super) add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(in super::super) release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(in super::super) get_iids: unsafe extern "system" fn(*mut c_void),
    pub(in super::super) get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
    pub(in super::super) get_trust_level: unsafe extern "system" fn(*mut c_void),
    pub(in super::super) _pad6: *const c_void,
    pub(in super::super) get_corner_radius_property:
        unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
}

#[repr(C)]
pub(in super::super) struct RawIControlStatics7 {
    pub(in super::super) lp_vtbl: *const RawIControlStatics7Vtbl,
}

#[repr(C)]
pub(in super::super) struct RawIDependencyPropertyStaticsVtbl {
    pub(in super::super) query_interface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    pub(in super::super) add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(in super::super) release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(in super::super) get_iids: unsafe extern "system" fn(*mut c_void),
    pub(in super::super) get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
    pub(in super::super) get_trust_level: unsafe extern "system" fn(*mut c_void),
    pub(in super::super) get_unset_value: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
}

#[repr(C)]
pub(in super::super) struct RawIDependencyPropertyStatics {
    pub(in super::super) lp_vtbl: *const RawIDependencyPropertyStaticsVtbl,
}

#[repr(C)]
pub(in super::super) struct RawICoreDispatcherVtbl {
    pub(in super::super) query_interface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    pub(in super::super) add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(in super::super) release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(in super::super) get_iids: unsafe extern "system" fn(*mut c_void),
    pub(in super::super) get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
    pub(in super::super) get_trust_level: unsafe extern "system" fn(*mut c_void),
    pub(in super::super) get_has_thread_access: unsafe extern "system" fn(*mut c_void),
    pub(in super::super) process_events: unsafe extern "system" fn(*mut c_void),
    pub(in super::super) run_async: unsafe extern "system" fn(
        *mut c_void,
        i32,
        *mut c_void,
        *mut *mut c_void,
    ) -> HRESULT,
}

#[repr(C)]
pub(in super::super) struct RawICoreDispatcher {
    pub(in super::super) lp_vtbl: *const RawICoreDispatcherVtbl,
}

#[repr(C)]
pub(in super::super) struct RawIWeakReferenceSourceVtbl {
    pub(in super::super) query_interface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    pub(in super::super) add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(in super::super) release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(in super::super) get_weak_reference: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
}

#[repr(C)]
pub(in super::super) struct RawIWeakReferenceSource {
    pub(in super::super) lp_vtbl: *const RawIWeakReferenceSourceVtbl,
}

#[repr(C)]
pub(in super::super) struct RawIWeakReferenceVtbl {
    pub(in super::super) query_interface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    pub(in super::super) add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(in super::super) release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(in super::super) resolve:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
}

#[repr(C)]
pub(in super::super) struct RawIWeakReference {
    pub(in super::super) lp_vtbl: *const RawIWeakReferenceVtbl,
}
