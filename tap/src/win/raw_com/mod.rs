//! Core COM/WinRT IID/CLSID constants, diagnostic structures, and visual-tree/UIElement raw vtables.

mod helpers;
mod vtables;

pub(super) use helpers::*;
pub(super) use vtables::*;

use super::super::tree_match::CLSID_PRETTY_WINDOWS_TAP_U128;
use std::ffi::c_void;
use std::mem::size_of;
use std::sync::atomic::{AtomicBool, AtomicU32};
use windows::core::{GUID, HRESULT, HSTRING};

pub(super) const E_FAIL_HR: HRESULT = HRESULT(0x80004005u32 as i32);

pub(super) static DLL_PINNED: AtomicBool = AtomicBool::new(false);

pub(super) const CLSID_PRETTY_WINDOWS_TAP: GUID = GUID::from_u128(CLSID_PRETTY_WINDOWS_TAP_U128);
pub(super) const CLSID_STD_GLOBAL_INTERFACE_TABLE: GUID =
    GUID::from_u128(0x00000323_0000_0000_C000_000000000046);

pub(super) const IID_IUNKNOWN: GUID = GUID::from_u128(0x00000000_0000_0000_C000_000000000046);
pub(super) const IID_IINSPECTABLE: GUID = GUID::from_u128(0xAF86E2E0_B12D_4C6A_9C5A_D7AA65101E90);
pub(super) const IID_ICLASSFACTORY: GUID = GUID::from_u128(0x00000001_0000_0000_C000_000000000046);
pub(super) const IID_IMARSHAL: GUID = GUID::from_u128(0x00000003_0000_0000_C000_000000000046);
pub(super) const IID_IWEAKREFERENCESOURCE: GUID = GUID::from_u128(0x00000038_0000_0000_C000_000000000046);
pub(super) const IID_IAGILEOBJECT: GUID = GUID::from_u128(0x94EA2B94_E9CC_49E0_C0FF_EE64CA8F5B90);
pub(super) const IID_IOBJECTWITHSITE: GUID = GUID::from_u128(0xFC4801A3_2BA9_11CF_A229_00AA003D7352);
pub(super) const IID_IVISUALTREESERVICECALLBACK: GUID =
    GUID::from_u128(0xAA7A8931_80E4_4FEC_8F3B_553F87B4966E);
pub(super) const IID_IVISUALTREESERVICECALLBACK2: GUID =
    GUID::from_u128(0xBAD9EB88_AE77_4397_B948_5FA2DB0A19EA);
pub(super) const IID_IVISUALTREESERVICE: GUID = GUID::from_u128(0xA593B11A_D17F_48BB_8F66_83910731C8A5);
pub(super) const IID_IXAMLDIAGNOSTICS: GUID = GUID::from_u128(0x18C9E2B6_3F43_4116_9F2B_FF935D7770D2);
pub(super) const IID_IUIELEMENT: GUID = GUID::from_u128(0x676D0BE9_B65C_41C6_BA40_58CF87F201C1);
pub(super) const IID_IUIELEMENT10: GUID = GUID::from_u128(0xD531C629_AD2C_5F6B_ADCF_FB87287D18D7);
pub(super) const IID_IUIELEMENTSTATICS10: GUID = GUID::from_u128(0x60D25362_4B3E_53DA_8B78_38DB94AE8F26);
pub(super) const IID_IFRAMEWORKELEMENT: GUID = GUID::from_u128(0xA391D09B_4A99_4B7C_9D8D_6FA5D01F6FBF);
pub(super) const IID_IFRAMEWORKELEMENT4: GUID = GUID::from_u128(0x6B765BB3_FBA3_4404_BDEE_1A45D1CA5F21);
pub(super) const IID_IFRAMEWORKELEMENTSTATICS4: GUID =
    GUID::from_u128(0x9C41B155_C5D8_4663_BFF2_D8D54FB5DBB3);
pub(super) const IID_IDEPENDENCYOBJECT: GUID = GUID::from_u128(0x5C526665_F60E_4912_AF59_5FE0680F089D);
pub(super) const IID_IDEPENDENCYOBJECT2: GUID = GUID::from_u128(0x29FED85D_3D22_43A1_ADD0_17027C08B212);
pub(super) const IID_IDEPENDENCYPROPERTYCHANGEDCALLBACK: GUID =
    GUID::from_u128(0x45883D16_27BF_4BC1_AC26_94C1601F3A49);
pub(super) const IID_ICOREDISPATCHER: GUID = GUID::from_u128(0x60DB2FA8_B705_4FDE_A7D6_EBBB1891D39E);
pub(super) const IID_IDISPATCHEDHANDLER: GUID = GUID::from_u128(0xD1F276C4_98D8_4636_BF49_EB79507548E9);
pub(super) const IID_IBORDER: GUID = GUID::from_u128(0x797C4539_45BD_4633_A044_BFB02EF5170F);
pub(super) const IID_IBORDERSTATICS: GUID = GUID::from_u128(0xB81D1639_E911_4FD7_A4C4_B9C7F008B7FC);
pub(super) const IID_ICONTENTPRESENTER4: GUID = GUID::from_u128(0x997B1B17_3BF8_432B_8FD9_68C7BAB21F9D);
pub(super) const IID_ICONTENTPRESENTERSTATICS4: GUID =
    GUID::from_u128(0xFF8BE292_AF28_47EB_A5C3_DC44CED40FD2);
pub(super) const IID_ICONTROL: GUID = GUID::from_u128(0xA8912263_2951_4F58_A9C5_5A134EAA7F07);
pub(super) const IID_ICONTROL7: GUID = GUID::from_u128(0x3550DBE1_E561_5934_A3EF_7DB8BF6E3B45);
pub(super) const IID_ICONTROLSTATICS: GUID = GUID::from_u128(0x473B305B_877D_4C35_8FCA_46A05F96607A);
pub(super) const IID_ICONTROLSTATICS7: GUID = GUID::from_u128(0xAEE1CA45_C910_5DAB_A266_FDF50C0BEDD2);
pub(super) const IID_ITOOLTIP: GUID = GUID::from_u128(0x7DE5D75F_4415_429F_B523_699BDB8C30DB);
pub(super) const IID_IPANEL: GUID = GUID::from_u128(0xA50A4BBD_8361_469C_90DA_E9A40C7474DF);
pub(super) const IID_IPANELSTATICS: GUID = GUID::from_u128(0xF23D6B5D_8330_47A6_A046_25F509B25232);
pub(super) const IID_IGRID2: GUID = GUID::from_u128(0xF76EFA41_380E_45DB_BE87_9E1326BA4B57);
pub(super) const IID_IGRIDSTATICS2: GUID = GUID::from_u128(0x2D666658_58B6_4A1B_841B_875BA93B5D6A);
pub(super) const IID_IBRUSH: GUID = GUID::from_u128(0x8806A321_1E06_422C_A1CC_01696559E021);
pub(super) const IID_ISOLIDCOLORBRUSH: GUID = GUID::from_u128(0x9D850850_66F3_48DF_9A8F_824BD5E070AF);
pub(super) const IID_ISOLIDCOLORBRUSHFACTORY: GUID =
    GUID::from_u128(0xD935CE0C_86F5_4DA6_8A27_B1619EF7F92B);
pub(super) const IID_IGRIDSTATICS: GUID = GUID::from_u128(0x64FE2E9F_F951_42B6_A9CE_BB179AF11595);
pub(super) const IID_IDEPENDENCYPROPERTYSTATICS: GUID =
    GUID::from_u128(0x49E5F28F_8259_4D5C_AAE0_83D56DBB68D9);
pub(super) const IID_IVISUALTREEHELPERSTATICS: GUID =
    GUID::from_u128(0xE75758C4_D25D_4B1D_971F_596F17F12BAA);
pub(super) const IID_IELEMENTCOMPOSITIONPREVIEWSTATICS: GUID =
    GUID::from_u128(0x08C92B38_EC99_4C55_BC85_A1C180B27646);

pub(super) static ACTIVE_OBJECTS: AtomicU32 = AtomicU32::new(0);

// Raw C ABI layouts strictly matching Windows SDK `xamlOM.h`, `windows.ui.xaml.h`,
// `windows.ui.core.h`, and `weakreference.h`.
#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct RawSourceInfo {
    pub(super) file_name: *const u16,
    pub(super) line_number: u32,
    pub(super) column_number: u32,
    pub(super) char_position: u32,
    pub(super) hash: *const u16,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct RawParentChildRelation {
    pub(super) parent: u64,
    pub(super) child: u64,
    pub(super) child_index: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct RawVisualElement {
    pub(super) handle: u64,
    pub(super) src_info: RawSourceInfo,
    pub(super) type_name: *const u16,
    pub(super) name: *const u16,
    pub(super) num_children: u32,
}

const _: () = {
    assert!(size_of::<RawSourceInfo>() == 32);
    assert!(size_of::<RawParentChildRelation>() == 24);
    assert!(size_of::<RawVisualElement>() == 64);
};

#[repr(C)]
pub(super) struct RawIUnknownVtbl {
    pub(super) query_interface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    pub(super) add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(super) release: unsafe extern "system" fn(*mut c_void) -> u32,
}

#[repr(C)]
pub(super) struct RawIUnknown {
    pub(super) lp_vtbl: *const RawIUnknownVtbl,
}

#[repr(C)]
pub(super) struct RawIVisualTreeServiceVtbl {
    pub(super) query_interface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    pub(super) add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(super) release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(super) advise_visual_tree_change: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
    pub(super) unadvise_visual_tree_change: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
}

#[repr(C)]
pub(super) struct RawIVisualTreeService {
    pub(super) lp_vtbl: *const RawIVisualTreeServiceVtbl,
}

#[repr(C)]
pub(super) struct RawIXamlDiagnosticsVtbl {
    pub(super) query_interface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    pub(super) add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(super) release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(super) get_dispatcher: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    pub(super) get_ui_layer: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    pub(super) get_application: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    pub(super) get_iinspectable_from_handle:
        unsafe extern "system" fn(*mut c_void, u64, *mut *mut c_void) -> HRESULT,
}

#[repr(C)]
pub(super) struct RawIXamlDiagnostics {
    pub(super) lp_vtbl: *const RawIXamlDiagnosticsVtbl,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct RawSize {
    pub(super) width: f32,
    pub(super) height: f32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct RawPoint {
    pub(super) x: f32,
    pub(super) y: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq)]
pub(super) struct RawThickness {
    pub(super) left: f64,
    pub(super) top: f64,
    pub(super) right: f64,
    pub(super) bottom: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq)]
pub(super) struct RawCornerRadius {
    pub(super) top_left: f64,
    pub(super) top_right: f64,
    pub(super) bottom_right: f64,
    pub(super) bottom_left: f64,
}

#[repr(C)]
#[allow(dead_code)]
pub(super) struct RawIUIElementVtbl {
    pub(super) query_interface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    pub(super) add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(super) release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(super) get_iids: unsafe extern "system" fn(*mut c_void),
    pub(super) get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
    pub(super) get_trust_level: unsafe extern "system" fn(*mut c_void),
    pub(super) get_desired_size: unsafe extern "system" fn(*mut c_void),
    pub(super) get_allow_drop: unsafe extern "system" fn(*mut c_void),
    pub(super) put_allow_drop: unsafe extern "system" fn(*mut c_void),
    pub(super) get_opacity: unsafe extern "system" fn(*mut c_void, *mut f64) -> HRESULT,
    pub(super) put_opacity: unsafe extern "system" fn(*mut c_void, f64) -> HRESULT,
    pub(super) get_clip: unsafe extern "system" fn(*mut c_void),
    pub(super) put_clip: unsafe extern "system" fn(*mut c_void),
    pub(super) get_render_transform: unsafe extern "system" fn(*mut c_void),
    pub(super) put_render_transform: unsafe extern "system" fn(*mut c_void),
    pub(super) get_projection: unsafe extern "system" fn(*mut c_void),
    pub(super) put_projection: unsafe extern "system" fn(*mut c_void),
    pub(super) get_render_transform_origin: unsafe extern "system" fn(*mut c_void),
    pub(super) put_render_transform_origin: unsafe extern "system" fn(*mut c_void),
    pub(super) get_is_hit_test_visible: unsafe extern "system" fn(*mut c_void, *mut u8) -> HRESULT,
    pub(super) put_is_hit_test_visible: unsafe extern "system" fn(*mut c_void, u8) -> HRESULT,
    pub(super) get_visibility: unsafe extern "system" fn(*mut c_void, *mut i32) -> HRESULT,
    pub(super) put_visibility: unsafe extern "system" fn(*mut c_void, i32) -> HRESULT,
    pub(super) get_render_size: unsafe extern "system" fn(*mut c_void, *mut RawSize) -> HRESULT,
    pub(super) _pad24_97: [*const c_void; 74],
    pub(super) transform_to_visual:
        unsafe extern "system" fn(*mut c_void, *mut c_void, *mut *mut c_void) -> HRESULT,
}

#[repr(C)]
#[allow(dead_code)]
pub(super) struct RawIGeneralTransformVtbl {
    pub(super) query_interface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    pub(super) add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(super) release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(super) get_iids: unsafe extern "system" fn(*mut c_void),
    pub(super) get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
    pub(super) get_trust_level: unsafe extern "system" fn(*mut c_void),
    pub(super) get_inverse: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    pub(super) transform_point:
        unsafe extern "system" fn(*mut c_void, RawPoint, *mut RawPoint) -> HRESULT,
}

#[repr(C)]
pub(super) struct RawIGeneralTransform {
    pub(super) lp_vtbl: *const RawIGeneralTransformVtbl,
}

#[repr(C)]
#[allow(dead_code)]
pub(super) struct RawIVisualTreeHelperStaticsVtbl {
    pub(super) query_interface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    pub(super) add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(super) release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(super) get_iids: unsafe extern "system" fn(*mut c_void),
    pub(super) get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
    pub(super) get_trust_level: unsafe extern "system" fn(*mut c_void),
    pub(super) _pad6_9: [*const c_void; 4],
    pub(super) get_child:
        unsafe extern "system" fn(*mut c_void, *mut c_void, i32, *mut *mut c_void) -> HRESULT,
    pub(super) get_children_count: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut i32) -> HRESULT,
    pub(super) get_parent:
        unsafe extern "system" fn(*mut c_void, *mut c_void, *mut *mut c_void) -> HRESULT,
}

#[repr(C)]
pub(super) struct RawIVisualTreeHelperStatics {
    pub(super) lp_vtbl: *const RawIVisualTreeHelperStaticsVtbl,
}

#[repr(C)]
#[allow(dead_code)]
pub(super) struct RawIInspectableVtbl {
    pub(super) query_interface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    pub(super) add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(super) release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(super) get_iids: unsafe extern "system" fn(*mut c_void),
    pub(super) get_runtime_class_name: unsafe extern "system" fn(*mut c_void, *mut HSTRING) -> HRESULT,
    pub(super) get_trust_level: unsafe extern "system" fn(*mut c_void),
}

#[repr(C)]
pub(super) struct RawIInspectable {
    pub(super) lp_vtbl: *const RawIInspectableVtbl,
}

#[repr(C)]
#[allow(dead_code)]
pub(super) struct RawIFrameworkElementVtbl {
    pub(super) query_interface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    pub(super) add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(super) release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(super) get_iids: unsafe extern "system" fn(*mut c_void),
    pub(super) get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
    pub(super) get_trust_level: unsafe extern "system" fn(*mut c_void),
    pub(super) _pad6_30: [*const c_void; 25],
    pub(super) get_margin: unsafe extern "system" fn(*mut c_void, *mut RawThickness) -> HRESULT,
    pub(super) put_margin: unsafe extern "system" fn(*mut c_void, RawThickness) -> HRESULT,
    pub(super) get_name: unsafe extern "system" fn(*mut c_void, *mut HSTRING) -> HRESULT,
    pub(super) put_name: unsafe extern "system" fn(*mut c_void, core::mem::MaybeUninit<HSTRING>) -> HRESULT,
}

#[repr(C)]
pub(super) struct RawIFrameworkElement {
    pub(super) lp_vtbl: *const RawIFrameworkElementVtbl,
}

#[repr(C)]
#[allow(dead_code)]
pub(super) struct RawIFrameworkElement4Vtbl {
    pub(super) query_interface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    pub(super) add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(super) release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(super) get_iids: unsafe extern "system" fn(*mut c_void),
    pub(super) get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
    pub(super) get_trust_level: unsafe extern "system" fn(*mut c_void),
    pub(super) _pad6_9: [*const c_void; 4],
    pub(super) get_focus_visual_secondary_thickness:
        unsafe extern "system" fn(*mut c_void, *mut RawThickness) -> HRESULT,
    pub(super) put_focus_visual_secondary_thickness:
        unsafe extern "system" fn(*mut c_void, RawThickness) -> HRESULT,
    pub(super) _pad12_13: [*const c_void; 2],
    pub(super) get_focus_visual_secondary_brush:
        unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    pub(super) put_focus_visual_secondary_brush:
        unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
}

#[repr(C)]
pub(super) struct RawIFrameworkElement4 {
    pub(super) lp_vtbl: *const RawIFrameworkElement4Vtbl,
}

#[repr(C)]
#[allow(dead_code)]
pub(super) struct RawIFrameworkElementStatics4Vtbl {
    pub(super) query_interface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    pub(super) add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(super) release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(super) get_iids: unsafe extern "system" fn(*mut c_void),
    pub(super) get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
    pub(super) get_trust_level: unsafe extern "system" fn(*mut c_void),
    pub(super) _pad6_7: [*const c_void; 2],
    pub(super) get_focus_visual_secondary_thickness_property:
        unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    pub(super) _pad9: *const c_void,
    pub(super) get_focus_visual_secondary_brush_property:
        unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
}

#[repr(C)]
pub(super) struct RawIFrameworkElementStatics4 {
    pub(super) lp_vtbl: *const RawIFrameworkElementStatics4Vtbl,
}

#[repr(C)]
#[allow(dead_code)]
pub(super) struct RawIUIElement10Vtbl {
    pub(super) query_interface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    pub(super) add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(super) release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(super) get_iids: unsafe extern "system" fn(*mut c_void),
    pub(super) get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
    pub(super) get_trust_level: unsafe extern "system" fn(*mut c_void),
    pub(super) _pad6_10: [*const c_void; 5],
    pub(super) get_shadow: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    pub(super) put_shadow: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
}

#[repr(C)]
pub(super) struct RawIUIElement10 {
    pub(super) lp_vtbl: *const RawIUIElement10Vtbl,
}

#[repr(C)]
#[allow(dead_code)]
pub(super) struct RawIUIElementStatics10Vtbl {
    pub(super) query_interface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    pub(super) add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(super) release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(super) get_iids: unsafe extern "system" fn(*mut c_void),
    pub(super) get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
    pub(super) get_trust_level: unsafe extern "system" fn(*mut c_void),
    pub(super) get_shadow_property: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
}

#[repr(C)]
pub(super) struct RawIUIElementStatics10 {
    pub(super) lp_vtbl: *const RawIUIElementStatics10Vtbl,
}

const _: () = {
    const PTR_SIZE: usize = size_of::<usize>();
    assert!(size_of::<RawSize>() == 8);
    assert!(size_of::<RawPoint>() == 8);
    assert!(size_of::<RawThickness>() == 32);
    assert!(size_of::<RawCornerRadius>() == 32);
    assert!(size_of::<HSTRING>() == size_of::<usize>());
    assert!(
        core::mem::offset_of!(RawIUIElementVtbl, put_is_hit_test_visible)
            == 20 * size_of::<usize>()
    );
    assert!(core::mem::offset_of!(RawIUIElementVtbl, get_render_size) == 23 * size_of::<usize>());
    assert!(
        core::mem::offset_of!(RawIUIElementVtbl, transform_to_visual) == 98 * size_of::<usize>()
    );
    assert!(
        core::mem::offset_of!(RawIUIElement10Vtbl, get_shadow) == 11 * size_of::<usize>()
    );
    assert!(
        core::mem::offset_of!(RawIUIElement10Vtbl, put_shadow) == 12 * size_of::<usize>()
    );
    assert!(
        core::mem::offset_of!(RawIUIElementStatics10Vtbl, get_shadow_property)
            == 6 * size_of::<usize>()
    );
    assert!(
        core::mem::offset_of!(RawIVisualTreeHelperStaticsVtbl, get_child)
            == 10 * size_of::<usize>()
    );
    assert!(
        core::mem::offset_of!(RawIVisualTreeHelperStaticsVtbl, get_children_count)
            == 11 * size_of::<usize>()
    );
    assert!(
        core::mem::offset_of!(RawIVisualTreeHelperStaticsVtbl, get_parent)
            == 12 * size_of::<usize>()
    );
    assert!(
        core::mem::offset_of!(RawIFrameworkElementVtbl, get_margin) == 31 * size_of::<usize>()
    );
    assert!(
        core::mem::offset_of!(RawIFrameworkElementVtbl, put_margin) == 32 * size_of::<usize>()
    );
    assert!(
        core::mem::offset_of!(RawIFrameworkElementVtbl, get_name) == 33 * size_of::<usize>()
    );
    assert!(
        core::mem::offset_of!(RawIFrameworkElementVtbl, put_name) == 34 * size_of::<usize>()
    );
    assert!(
        core::mem::offset_of!(
            RawIFrameworkElement4Vtbl,
            get_focus_visual_secondary_thickness
        ) == 10 * size_of::<usize>()
    );
    assert!(
        core::mem::offset_of!(
            RawIFrameworkElement4Vtbl,
            put_focus_visual_secondary_thickness
        ) == 11 * size_of::<usize>()
    );
    assert!(
        core::mem::offset_of!(
            RawIFrameworkElement4Vtbl,
            get_focus_visual_secondary_brush
        ) == 14 * size_of::<usize>()
    );
    assert!(
        core::mem::offset_of!(
            RawIFrameworkElement4Vtbl,
            put_focus_visual_secondary_brush
        ) == 15 * size_of::<usize>()
    );
    assert!(
        core::mem::offset_of!(
            RawIFrameworkElementStatics4Vtbl,
            get_focus_visual_secondary_thickness_property
        ) == 8 * PTR_SIZE
    );
    assert!(
        core::mem::offset_of!(
            RawIFrameworkElementStatics4Vtbl,
            get_focus_visual_secondary_brush_property
        ) == 10 * size_of::<usize>()
    );
    assert!(
        core::mem::offset_of!(RawIBorderVtbl, get_border_thickness) == 8 * PTR_SIZE
    );
    assert!(
        core::mem::offset_of!(RawIBorderVtbl, put_border_thickness) == 9 * size_of::<usize>()
    );
    assert!(
        core::mem::offset_of!(RawIBorderVtbl, get_corner_radius) == 12 * size_of::<usize>()
    );
    assert!(
        core::mem::offset_of!(RawIBorderVtbl, put_corner_radius) == 13 * size_of::<usize>()
    );
    assert!(core::mem::offset_of!(RawIPanelVtbl, get_children) == 6 * size_of::<usize>());
    assert!(
        core::mem::offset_of!(RawIUIElementVectorVtbl, get_at) == 6 * size_of::<usize>()
    );
    assert!(
        core::mem::offset_of!(RawIUIElementVectorVtbl, get_size) == 7 * size_of::<usize>()
    );
    assert!(
        core::mem::offset_of!(RawIUIElementVectorVtbl, insert_at) == 11 * size_of::<usize>()
    );
    assert!(
        core::mem::offset_of!(RawIUIElementVectorVtbl, remove_at) == 12 * size_of::<usize>()
    );
    assert!(
        core::mem::offset_of!(RawIElementCompositionPreviewStaticsVtbl, get_element_visual)
            == 6 * size_of::<usize>()
    );
    assert!(core::mem::offset_of!(RawIVisualVtbl, get_is_visible) == 18 * size_of::<usize>());
    assert!(core::mem::offset_of!(RawIVisualVtbl, put_is_visible) == 19 * size_of::<usize>());
    assert!(core::mem::offset_of!(RawIGrid2Vtbl, get_border_brush) == 6 * size_of::<usize>());
    assert!(core::mem::offset_of!(RawIGrid2Vtbl, put_border_brush) == 7 * size_of::<usize>());
    assert!(
        core::mem::offset_of!(RawIGridStatics2Vtbl, get_border_brush_property)
            == 6 * size_of::<usize>()
    );
    assert!(
        core::mem::offset_of!(RawIContentPresenter4Vtbl, get_border_brush)
            == 14 * size_of::<usize>()
    );
    assert!(
        core::mem::offset_of!(RawIContentPresenter4Vtbl, put_border_brush)
            == 15 * size_of::<usize>()
    );
    assert!(
        core::mem::offset_of!(RawIContentPresenter4Vtbl, get_corner_radius)
            == 18 * size_of::<usize>()
    );
    assert!(
        core::mem::offset_of!(RawIContentPresenter4Vtbl, put_corner_radius)
            == 19 * size_of::<usize>()
    );
    assert!(
        core::mem::offset_of!(RawIContentPresenter4Vtbl, get_background)
            == 22 * size_of::<usize>()
    );
    assert!(
        core::mem::offset_of!(RawIContentPresenter4Vtbl, put_background)
            == 23 * size_of::<usize>()
    );
    assert!(
        core::mem::offset_of!(RawIContentPresenterStatics4Vtbl, get_border_brush_property)
            == 10 * size_of::<usize>()
    );
    assert!(
        core::mem::offset_of!(RawIContentPresenterStatics4Vtbl, get_background_property)
            == 14 * size_of::<usize>()
    );
    assert!(core::mem::offset_of!(RawIControlVtbl, get_background) == 36 * size_of::<usize>());
    assert!(core::mem::offset_of!(RawIControlVtbl, put_background) == 37 * size_of::<usize>());
    assert!(core::mem::offset_of!(RawIControlVtbl, get_border_brush) == 40 * size_of::<usize>());
    assert!(core::mem::offset_of!(RawIControlVtbl, put_border_brush) == 41 * size_of::<usize>());
    assert!(core::mem::offset_of!(RawIControl7Vtbl, get_corner_radius) == 8 * PTR_SIZE);
    assert!(core::mem::offset_of!(RawIControl7Vtbl, put_corner_radius) == 9 * size_of::<usize>());
    assert!(
        core::mem::offset_of!(RawIControlStaticsVtbl, get_background_property)
            == 21 * size_of::<usize>()
    );
    assert!(
        core::mem::offset_of!(RawIControlStaticsVtbl, get_border_brush_property)
            == 23 * size_of::<usize>()
    );
    assert!(
        core::mem::offset_of!(RawIControlStatics7Vtbl, get_corner_radius_property)
            == 7 * size_of::<usize>()
    );
    assert!(core::mem::offset_of!(RawIToolTipVtbl, get_is_open) == 8 * PTR_SIZE);
    assert!(
        core::mem::offset_of!(RawIElementCompositionPreviewStaticsVtbl, get_element_child_visual)
            == 7 * PTR_SIZE
    );
    assert!(
        core::mem::offset_of!(RawIElementCompositionPreviewStaticsVtbl, set_element_child_visual)
            == 8 * PTR_SIZE
    );
    assert!(core::mem::offset_of!(RawIGridStaticsVtbl, get_row) == 7 * PTR_SIZE);
    assert!(core::mem::offset_of!(RawIGridStaticsVtbl, set_row) == 8 * PTR_SIZE);
    assert!(core::mem::offset_of!(RawIGridStaticsVtbl, get_column) == 10 * PTR_SIZE);
    assert!(core::mem::offset_of!(RawIGridStaticsVtbl, set_column) == 11 * PTR_SIZE);
    assert!(core::mem::offset_of!(RawIGridStaticsVtbl, get_row_span) == 13 * PTR_SIZE);
    assert!(core::mem::offset_of!(RawIGridStaticsVtbl, set_row_span) == 14 * PTR_SIZE);
    assert!(core::mem::offset_of!(RawIGridStaticsVtbl, get_column_span) == 16 * PTR_SIZE);
    assert!(core::mem::offset_of!(RawIGridStaticsVtbl, set_column_span) == 17 * PTR_SIZE);
};

pub(super) const VISIBILITY_VISIBLE: i32 = 0;
pub(super) const VISIBILITY_COLLAPSED: i32 = 1;

pub(super) const DIAG_WATCHER_CALLED: u32 = 1 << 0;
pub(super) const DIAG_SAW_FRAME_OR_BG: u32 = 1 << 1;
pub(super) const DIAG_SAW_TARGET_NAME: u32 = 1 << 2;
pub(super) const DIAG_TARGET_MATCHED: u32 = 1 << 3;
pub(super) const DIAG_ENTER_ON_TARGET: u32 = 1 << 4;
pub(super) const DIAG_DIAG_FROM_GIT_OK: u32 = 1 << 5;
pub(super) const DIAG_GET_INSP_OK: u32 = 1 << 6;
pub(super) const DIAG_QI_UI_OK: u32 = 1 << 7;
pub(super) const DIAG_QI_WEAK_OK: u32 = 1 << 8;
pub(super) const DIAG_DISPATCHER_OK: u32 = 1 << 9;
pub(super) const DIAG_INSERTED_TARGET: u32 = 1 << 10;
pub(super) const DIAG_APPLY_RAN: u32 = 1 << 11;
pub(super) const DIAG_OVERFLOW_APPLIED: u32 = 1 << 12;
pub(super) const DIAG_ALTTAB_APPLIED: u32 = 1 << 13;
pub(super) const DIAG_ALTTAB_ITEM_APPLIED: u32 = 1 << 14;
pub(super) const DIAG_ALTTAB_FOCUS_APPLIED: u32 = 1 << 15;
pub(super) const DIAG_HOVER_FLYOUT_APPLIED: u32 = 1 << 16;
pub(super) const DIAG_HOVER_FLYOUT_SHADOW_APPLIED: u32 = 1 << 17;
pub(super) const DIAG_OSD_APPLIED: u32 = 1 << 18;
pub(super) const DIAG_OSD_SHADOW_APPLIED: u32 = 1 << 19;
pub(super) const DIAG_TOOLTIP_APPLIED: u32 = 1 << 20;
pub(super) const DIAG_TOOLTIP_SHADOW_APPLIED: u32 = 1 << 21;
pub(super) const DIAG_MENU_FLYOUT_APPLIED: u32 = 1 << 22;
pub(super) const DIAG_MENU_FLYOUT_SHADOW_APPLIED: u32 = 1 << 23;
pub(super) const DIAG_SHELL_CARD_APPLIED: u32 = 1 << 24;
pub(super) const DIAG_SHELL_CARD_SHADOW_APPLIED: u32 = 1 << 25;
pub(super) const DIAG_VIRTUAL_DESKTOP_BAR_APPLIED: u32 = 1 << 26;
pub(super) const DIAG_INPUT_SWITCHER_APPLIED: u32 = 1 << 27;
pub(super) const DIAG_INPUT_SWITCHER_SHADOW_APPLIED: u32 = 1 << 28;

#[repr(C)]
pub(super) struct RawIUIElement {
    pub(super) lp_vtbl: *const RawIUIElementVtbl,
}
