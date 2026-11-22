use super::super::tree_match::{TargetKind, CLSID_PRETTY_WINDOWS_TAP_U128};
use std::ffi::c_void;
use std::mem::size_of;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};
use windows::core::{IInspectable, IUnknown, Interface, GUID, HRESULT, HSTRING};
use windows::System::{DispatcherQueue};
use windows::Win32::Foundation::{E_NOINTERFACE, E_POINTER, S_OK};
use windows::Win32::System::Com::{CoCreateFreeThreadedMarshaler, CoCreateInstance, IGlobalInterfaceTable, CLSCTX_INPROC_SERVER};
use windows::Win32::System::WinRT::{RoGetActivationFactory};
use super::brush_store::ensure_cached_com;

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

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct RawUiColor {
    pub(super) a: u8,
    pub(super) r: u8,
    pub(super) g: u8,
    pub(super) b: u8,
}

impl RawUiColor {
    pub(super) const TRANSPARENT: Self = Self {
        a: 0,
        r: 0,
        g: 0,
        b: 0,
    };

    pub(super) const fn from_argb(argb: u32) -> Self {
        Self {
            a: ((argb >> 24) & 0xFF) as u8,
            r: ((argb >> 16) & 0xFF) as u8,
            g: ((argb >> 8) & 0xFF) as u8,
            b: (argb & 0xFF) as u8,
        }
    }

    pub(super) const fn to_argb(self) -> u32 {
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
pub(super) struct RawIBorderVtbl {
    pub(super) query_interface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    pub(super) add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(super) release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(super) get_iids: unsafe extern "system" fn(*mut c_void),
    pub(super) get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
    pub(super) get_trust_level: unsafe extern "system" fn(*mut c_void),
    pub(super) get_border_brush: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    pub(super) put_border_brush: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
    pub(super) get_border_thickness: unsafe extern "system" fn(*mut c_void, *mut RawThickness) -> HRESULT,
    pub(super) put_border_thickness: unsafe extern "system" fn(*mut c_void, RawThickness) -> HRESULT,
    pub(super) get_background: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    pub(super) put_background: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
    pub(super) get_corner_radius: unsafe extern "system" fn(*mut c_void, *mut RawCornerRadius) -> HRESULT,
    pub(super) put_corner_radius: unsafe extern "system" fn(*mut c_void, RawCornerRadius) -> HRESULT,
}

#[repr(C)]
pub(super) struct RawIBorder {
    pub(super) lp_vtbl: *const RawIBorderVtbl,
}

#[repr(C)]
#[allow(dead_code)]
pub(super) struct RawIPanelVtbl {
    pub(super) query_interface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    pub(super) add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(super) release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(super) get_iids: unsafe extern "system" fn(*mut c_void),
    pub(super) get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
    pub(super) get_trust_level: unsafe extern "system" fn(*mut c_void),
    pub(super) get_children: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    pub(super) get_background: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    pub(super) put_background: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
}

#[repr(C)]
pub(super) struct RawIPanel {
    pub(super) lp_vtbl: *const RawIPanelVtbl,
}

#[repr(C)]
#[allow(dead_code)]
pub(super) struct RawIUIElementVectorVtbl {
    pub(super) query_interface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    pub(super) add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(super) release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(super) get_iids: unsafe extern "system" fn(*mut c_void),
    pub(super) get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
    pub(super) get_trust_level: unsafe extern "system" fn(*mut c_void),
    pub(super) get_at: unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> HRESULT,
    pub(super) get_size: unsafe extern "system" fn(*mut c_void, *mut u32) -> HRESULT,
    pub(super) _pad8_10: [*const c_void; 3],
    pub(super) insert_at: unsafe extern "system" fn(*mut c_void, u32, *mut c_void) -> HRESULT,
    pub(super) remove_at: unsafe extern "system" fn(*mut c_void, u32) -> HRESULT,
}

#[repr(C)]
pub(super) struct RawIUIElementVector {
    pub(super) lp_vtbl: *const RawIUIElementVectorVtbl,
}

#[repr(C)]
#[allow(dead_code)]
pub(super) struct RawIElementCompositionPreviewStaticsVtbl {
    pub(super) query_interface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    pub(super) add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(super) release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(super) get_iids: unsafe extern "system" fn(*mut c_void),
    pub(super) get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
    pub(super) get_trust_level: unsafe extern "system" fn(*mut c_void),
    pub(super) get_element_visual:
        unsafe extern "system" fn(*mut c_void, *mut c_void, *mut *mut c_void) -> HRESULT,
}

#[repr(C)]
pub(super) struct RawIElementCompositionPreviewStatics {
    pub(super) lp_vtbl: *const RawIElementCompositionPreviewStaticsVtbl,
}

#[repr(C)]
#[allow(dead_code)]
pub(super) struct RawIVisualVtbl {
    pub(super) query_interface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    pub(super) add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(super) release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(super) get_iids: unsafe extern "system" fn(*mut c_void),
    pub(super) get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
    pub(super) get_trust_level: unsafe extern "system" fn(*mut c_void),
    pub(super) _pad6_17: [*const c_void; 12],
    pub(super) get_is_visible: unsafe extern "system" fn(*mut c_void, *mut u8) -> HRESULT,
    pub(super) put_is_visible: unsafe extern "system" fn(*mut c_void, u8) -> HRESULT,
}

#[repr(C)]
pub(super) struct RawIVisual {
    pub(super) lp_vtbl: *const RawIVisualVtbl,
}

#[repr(C)]
#[allow(dead_code)]
pub(super) struct RawISolidColorBrushVtbl {
    pub(super) query_interface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    pub(super) add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(super) release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(super) get_iids: unsafe extern "system" fn(*mut c_void),
    pub(super) get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
    pub(super) get_trust_level: unsafe extern "system" fn(*mut c_void),
    pub(super) get_color: unsafe extern "system" fn(*mut c_void, *mut RawUiColor) -> HRESULT,
    pub(super) put_color: unsafe extern "system" fn(*mut c_void, RawUiColor) -> HRESULT,
}

#[repr(C)]
pub(super) struct RawISolidColorBrush {
    pub(super) lp_vtbl: *const RawISolidColorBrushVtbl,
}

#[repr(C)]
#[allow(dead_code)]
pub(super) struct RawISolidColorBrushFactoryVtbl {
    pub(super) query_interface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    pub(super) add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(super) release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(super) get_iids: unsafe extern "system" fn(*mut c_void),
    pub(super) get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
    pub(super) get_trust_level: unsafe extern "system" fn(*mut c_void),
    pub(super) create_instance_with_color:
        unsafe extern "system" fn(*mut c_void, RawUiColor, *mut *mut c_void) -> HRESULT,
}

#[repr(C)]
pub(super) struct RawISolidColorBrushFactory {
    pub(super) lp_vtbl: *const RawISolidColorBrushFactoryVtbl,
}

#[repr(C)]
#[allow(dead_code)]
pub(super) struct RawIDependencyObjectVtbl {
    pub(super) query_interface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    pub(super) add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(super) release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(super) get_iids: unsafe extern "system" fn(*mut c_void),
    pub(super) get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
    pub(super) get_trust_level: unsafe extern "system" fn(*mut c_void),
    pub(super) get_value: unsafe extern "system" fn(*mut c_void),
    pub(super) set_value: unsafe extern "system" fn(*mut c_void),
    pub(super) clear_value: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
    pub(super) read_local_value:
        unsafe extern "system" fn(*mut c_void, *mut c_void, *mut *mut c_void) -> HRESULT,
    pub(super) get_animation_base_value: unsafe extern "system" fn(*mut c_void),
    pub(super) get_dispatcher: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
}

#[repr(C)]
pub(super) struct RawIDependencyObject {
    pub(super) lp_vtbl: *const RawIDependencyObjectVtbl,
}

#[repr(C)]
pub(super) struct RawIDependencyObject2Vtbl {
    pub(super) query_interface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    pub(super) add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(super) release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(super) get_iids: unsafe extern "system" fn(*mut c_void),
    pub(super) get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
    pub(super) get_trust_level: unsafe extern "system" fn(*mut c_void),
    pub(super) register_property_changed_callback: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        *mut c_void,
        *mut i64,
    ) -> HRESULT,
    pub(super) unregister_property_changed_callback:
        unsafe extern "system" fn(*mut c_void, *mut c_void, i64) -> HRESULT,
}

#[repr(C)]
pub(super) struct RawIDependencyObject2 {
    pub(super) lp_vtbl: *const RawIDependencyObject2Vtbl,
}

#[repr(C)]
#[allow(dead_code)]
pub(super) struct RawIBorderStaticsVtbl {
    pub(super) query_interface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    pub(super) add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(super) release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(super) get_iids: unsafe extern "system" fn(*mut c_void),
    pub(super) get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
    pub(super) get_trust_level: unsafe extern "system" fn(*mut c_void),
    pub(super) get_border_brush_property:
        unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    pub(super) get_border_thickness_property:
        unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    pub(super) get_background_property:
        unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
}

#[repr(C)]
pub(super) struct RawIBorderStatics {
    pub(super) lp_vtbl: *const RawIBorderStaticsVtbl,
}

#[repr(C)]
pub(super) struct RawIPanelStaticsVtbl {
    pub(super) query_interface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    pub(super) add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(super) release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(super) get_iids: unsafe extern "system" fn(*mut c_void),
    pub(super) get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
    pub(super) get_trust_level: unsafe extern "system" fn(*mut c_void),
    pub(super) get_background_property:
        unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
}

#[repr(C)]
pub(super) struct RawIPanelStatics {
    pub(super) lp_vtbl: *const RawIPanelStaticsVtbl,
}

#[repr(C)]
#[allow(dead_code)]
pub(super) struct RawIGrid2Vtbl {
    pub(super) query_interface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    pub(super) add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(super) release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(super) get_iids: unsafe extern "system" fn(*mut c_void),
    pub(super) get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
    pub(super) get_trust_level: unsafe extern "system" fn(*mut c_void),
    pub(super) get_border_brush: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    pub(super) put_border_brush: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
}

#[repr(C)]
pub(super) struct RawIGrid2 {
    pub(super) lp_vtbl: *const RawIGrid2Vtbl,
}

#[repr(C)]
#[allow(dead_code)]
pub(super) struct RawIGridStatics2Vtbl {
    pub(super) query_interface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    pub(super) add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(super) release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(super) get_iids: unsafe extern "system" fn(*mut c_void),
    pub(super) get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
    pub(super) get_trust_level: unsafe extern "system" fn(*mut c_void),
    pub(super) get_border_brush_property:
        unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
}

#[repr(C)]
pub(super) struct RawIGridStatics2 {
    pub(super) lp_vtbl: *const RawIGridStatics2Vtbl,
}

#[repr(C)]
#[allow(dead_code)]
pub(super) struct RawIContentPresenter4Vtbl {
    pub(super) query_interface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    pub(super) add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(super) release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(super) get_iids: unsafe extern "system" fn(*mut c_void),
    pub(super) get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
    pub(super) get_trust_level: unsafe extern "system" fn(*mut c_void),
    pub(super) _pad6_13: [*const c_void; 8],
    pub(super) get_border_brush: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    pub(super) put_border_brush: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
    pub(super) get_border_thickness: unsafe extern "system" fn(*mut c_void, *mut RawThickness) -> HRESULT,
    pub(super) put_border_thickness: unsafe extern "system" fn(*mut c_void, RawThickness) -> HRESULT,
    pub(super) get_corner_radius: unsafe extern "system" fn(*mut c_void, *mut RawCornerRadius) -> HRESULT,
    pub(super) put_corner_radius: unsafe extern "system" fn(*mut c_void, RawCornerRadius) -> HRESULT,
    pub(super) get_padding: unsafe extern "system" fn(*mut c_void, *mut RawThickness) -> HRESULT,
    pub(super) put_padding: unsafe extern "system" fn(*mut c_void, RawThickness) -> HRESULT,
    pub(super) get_background: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    pub(super) put_background: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
}

#[repr(C)]
pub(super) struct RawIContentPresenter4 {
    pub(super) lp_vtbl: *const RawIContentPresenter4Vtbl,
}

#[repr(C)]
#[allow(dead_code)]
pub(super) struct RawIContentPresenterStatics4Vtbl {
    pub(super) query_interface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    pub(super) add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(super) release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(super) get_iids: unsafe extern "system" fn(*mut c_void),
    pub(super) get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
    pub(super) get_trust_level: unsafe extern "system" fn(*mut c_void),
    pub(super) _pad6_9: [*const c_void; 4],
    pub(super) get_border_brush_property:
        unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    pub(super) _pad11_13: [*const c_void; 3],
    pub(super) get_background_property:
        unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
}

#[repr(C)]
pub(super) struct RawIContentPresenterStatics4 {
    pub(super) lp_vtbl: *const RawIContentPresenterStatics4Vtbl,
}

#[repr(C)]
#[allow(dead_code)]
pub(super) struct RawIToolTipVtbl {
    pub(super) query_interface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    pub(super) add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(super) release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(super) get_iids: unsafe extern "system" fn(*mut c_void),
    pub(super) get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
    pub(super) get_trust_level: unsafe extern "system" fn(*mut c_void),
    pub(super) get_horizontal_offset: unsafe extern "system" fn(*mut c_void, *mut f64) -> HRESULT,
    pub(super) put_horizontal_offset: unsafe extern "system" fn(*mut c_void, f64) -> HRESULT,
    pub(super) get_is_open: unsafe extern "system" fn(*mut c_void, *mut u8) -> HRESULT,
}

#[repr(C)]
pub(super) struct RawIToolTip {
    pub(super) lp_vtbl: *const RawIToolTipVtbl,
}

#[repr(C)]
#[allow(dead_code)]
pub(super) struct RawIControlVtbl {
    pub(super) query_interface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    pub(super) add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(super) release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(super) get_iids: unsafe extern "system" fn(*mut c_void),
    pub(super) get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
    pub(super) get_trust_level: unsafe extern "system" fn(*mut c_void),
    pub(super) _pad6_35: [*const c_void; 30],
    pub(super) get_background: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    pub(super) put_background: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
    pub(super) _pad38_39: [*const c_void; 2],
    pub(super) get_border_brush: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    pub(super) put_border_brush: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
}

#[repr(C)]
pub(super) struct RawIControl {
    pub(super) lp_vtbl: *const RawIControlVtbl,
}

#[repr(C)]
#[allow(dead_code)]
pub(super) struct RawIControl7Vtbl {
    pub(super) query_interface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    pub(super) add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(super) release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(super) get_iids: unsafe extern "system" fn(*mut c_void),
    pub(super) get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
    pub(super) get_trust_level: unsafe extern "system" fn(*mut c_void),
    pub(super) _pad6_7: [*const c_void; 2],
    pub(super) get_corner_radius: unsafe extern "system" fn(*mut c_void, *mut RawCornerRadius) -> HRESULT,
    pub(super) put_corner_radius: unsafe extern "system" fn(*mut c_void, RawCornerRadius) -> HRESULT,
}

#[repr(C)]
pub(super) struct RawIControl7 {
    pub(super) lp_vtbl: *const RawIControl7Vtbl,
}

#[repr(C)]
#[allow(dead_code)]
pub(super) struct RawIControlStaticsVtbl {
    pub(super) query_interface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    pub(super) add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(super) release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(super) get_iids: unsafe extern "system" fn(*mut c_void),
    pub(super) get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
    pub(super) get_trust_level: unsafe extern "system" fn(*mut c_void),
    pub(super) _pad6_20: [*const c_void; 15],
    pub(super) get_background_property:
        unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    pub(super) _pad22: *const c_void,
    pub(super) get_border_brush_property:
        unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
}

#[repr(C)]
pub(super) struct RawIControlStatics {
    pub(super) lp_vtbl: *const RawIControlStaticsVtbl,
}

#[repr(C)]
#[allow(dead_code)]
pub(super) struct RawIControlStatics7Vtbl {
    pub(super) query_interface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    pub(super) add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(super) release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(super) get_iids: unsafe extern "system" fn(*mut c_void),
    pub(super) get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
    pub(super) get_trust_level: unsafe extern "system" fn(*mut c_void),
    pub(super) _pad6: *const c_void,
    pub(super) get_corner_radius_property:
        unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
}

#[repr(C)]
pub(super) struct RawIControlStatics7 {
    pub(super) lp_vtbl: *const RawIControlStatics7Vtbl,
}

#[repr(C)]
pub(super) struct RawIDependencyPropertyStaticsVtbl {
    pub(super) query_interface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    pub(super) add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(super) release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(super) get_iids: unsafe extern "system" fn(*mut c_void),
    pub(super) get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
    pub(super) get_trust_level: unsafe extern "system" fn(*mut c_void),
    pub(super) get_unset_value: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
}

#[repr(C)]
pub(super) struct RawIDependencyPropertyStatics {
    pub(super) lp_vtbl: *const RawIDependencyPropertyStaticsVtbl,
}

#[repr(C)]
pub(super) struct RawICoreDispatcherVtbl {
    pub(super) query_interface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    pub(super) add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(super) release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(super) get_iids: unsafe extern "system" fn(*mut c_void),
    pub(super) get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
    pub(super) get_trust_level: unsafe extern "system" fn(*mut c_void),
    pub(super) get_has_thread_access: unsafe extern "system" fn(*mut c_void),
    pub(super) process_events: unsafe extern "system" fn(*mut c_void),
    pub(super) run_async: unsafe extern "system" fn(
        *mut c_void,
        i32,
        *mut c_void,
        *mut *mut c_void,
    ) -> HRESULT,
}

#[repr(C)]
pub(super) struct RawICoreDispatcher {
    pub(super) lp_vtbl: *const RawICoreDispatcherVtbl,
}

#[repr(C)]
pub(super) struct RawIWeakReferenceSourceVtbl {
    pub(super) query_interface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    pub(super) add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(super) release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(super) get_weak_reference: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
}

#[repr(C)]
pub(super) struct RawIWeakReferenceSource {
    pub(super) lp_vtbl: *const RawIWeakReferenceSourceVtbl,
}

#[repr(C)]
pub(super) struct RawIWeakReferenceVtbl {
    pub(super) query_interface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    pub(super) add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(super) release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(super) resolve:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
}

#[repr(C)]
pub(super) struct RawIWeakReference {
    pub(super) lp_vtbl: *const RawIWeakReferenceVtbl,
}

pub(super) struct ComRaw(pub(super) *mut c_void);

impl ComRaw {
    #[inline]
    pub(super) fn null() -> Self {
        Self(null_mut())
    }

    #[inline]
    pub(super) fn as_ptr(&self) -> *mut c_void {
        self.0
    }

    #[inline]
    pub(super) fn is_null(&self) -> bool {
        self.0.is_null()
    }

    #[inline]
    pub(super) fn clone_ref(&self) -> Self {
        add_ref_raw_com_addr(self.0 as usize);
        Self(self.0)
    }

    #[inline]
    pub(super) fn into_raw(mut self) -> *mut c_void {
        let ptr = self.0;
        self.0 = null_mut();
        ptr
    }
}

impl Drop for ComRaw {
    fn drop(&mut self) {
        release_raw_com_addr(self.0 as usize);
        self.0 = null_mut();
    }
}

#[inline]
pub(super) fn release_raw_com_addr(addr: usize) {
    if addr != 0 {
        unsafe {
            let unk = addr as *mut RawIUnknown;
            ((*(*unk).lp_vtbl).release)(unk.cast());
        }
    }
}

#[inline]
pub(super) fn add_ref_raw_com_addr(addr: usize) {
    if addr != 0 {
        unsafe {
            let unk = addr as *mut RawIUnknown;
            ((*(*unk).lp_vtbl).add_ref)(unk.cast());
        }
    }
}

pub(super) unsafe fn create_git() -> windows::core::Result<IGlobalInterfaceTable> {
    CoCreateInstance(&CLSID_STD_GLOBAL_INTERFACE_TABLE, None, CLSCTX_INPROC_SERVER)
}

/// Aggregates the Free-Threaded Marshaler onto `outer_unk` lazily so COM can marshal safely across apartments.
pub(super) unsafe fn delegate_ftm_qi(
    outer_unk: *mut c_void,
    ftm_slot: &Mutex<Option<usize>>,
    riid: *const GUID,
    ppv: *mut *mut c_void,
) -> HRESULT {
    let mut ftm_addr = ftm_slot.lock().ok().and_then(|g| *g);
    if ftm_addr.is_none() {
        if let Some(outer) = IUnknown::from_raw_borrowed(&outer_unk) {
            if let Ok(new_ftm) = CoCreateFreeThreadedMarshaler(outer) {
                let new_addr = new_ftm.into_raw() as usize;
                if let Ok(mut g) = ftm_slot.lock() {
                    if g.is_none() {
                        *g = Some(new_addr);
                        ftm_addr = Some(new_addr);
                    } else {
                        ftm_addr = *g;
                        drop(g);
                        release_raw_com_addr(new_addr);
                    }
                } else {
                    release_raw_com_addr(new_addr);
                }
            }
        }
    }
    if let Some(addr) = ftm_addr {
        let unk = addr as *mut RawIUnknown;
        ((*(*unk).lp_vtbl).query_interface)(unk.cast(), riid, ppv)
    } else {
        E_NOINTERFACE
    }
}

// ---------- Agile IDispatchedHandler for ICoreDispatcher fallback ----------

#[repr(C)]
pub(super) struct RawIDispatchedHandlerVtbl {
    pub(super) query_interface: unsafe extern "system" fn(
        *mut CoreDispatchedHandler,
        *const GUID,
        *mut *mut c_void,
    ) -> HRESULT,
    pub(super) add_ref: unsafe extern "system" fn(*mut CoreDispatchedHandler) -> u32,
    pub(super) release: unsafe extern "system" fn(*mut CoreDispatchedHandler) -> u32,
    pub(super) invoke: unsafe extern "system" fn(*mut CoreDispatchedHandler) -> HRESULT,
}

pub(super) static DISPATCHED_HANDLER_VTBL: RawIDispatchedHandlerVtbl = RawIDispatchedHandlerVtbl {
    query_interface: dispatched_qi,
    add_ref: dispatched_add_ref,
    release: dispatched_release,
    invoke: dispatched_invoke,
};

#[repr(C)]
pub(super) struct CoreDispatchedHandler {
    pub(super) lp_vtbl: *const RawIDispatchedHandlerVtbl,
    pub(super) ref_count: AtomicU32,
    pub(super) callback: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}

impl CoreDispatchedHandler {
    pub(super) fn new_com_ptr(cb: impl FnOnce() + Send + 'static) -> *mut Self {
        Box::into_raw(Box::new(Self {
            lp_vtbl: &DISPATCHED_HANDLER_VTBL,
            ref_count: AtomicU32::new(1),
            callback: Mutex::new(Some(Box::new(cb))),
        }))
    }
}

pub(super) unsafe extern "system" fn dispatched_qi(
    this: *mut CoreDispatchedHandler,
    riid: *const GUID,
    ppv: *mut *mut c_void,
) -> HRESULT {
    if ppv.is_null() || riid.is_null() {
        return E_POINTER;
    }
    *ppv = null_mut();
    let iid = *riid;
    if iid == IID_IUNKNOWN || iid == IID_IDISPATCHEDHANDLER || iid == IID_IAGILEOBJECT {
        *ppv = this.cast();
        dispatched_add_ref(this);
        S_OK
    } else {
        E_NOINTERFACE
    }
}

pub(super) unsafe extern "system" fn dispatched_add_ref(this: *mut CoreDispatchedHandler) -> u32 {
    (*this).ref_count.fetch_add(1, Ordering::Relaxed) + 1
}

pub(super) unsafe extern "system" fn dispatched_release(this: *mut CoreDispatchedHandler) -> u32 {
    let prev = (*this).ref_count.fetch_sub(1, Ordering::Release);
    if prev == 1 {
        std::sync::atomic::fence(Ordering::Acquire);
        drop(Box::from_raw(this));
    }
    prev - 1
}

pub(super) unsafe extern "system" fn dispatched_invoke(this: *mut CoreDispatchedHandler) -> HRESULT {
    let cb = (*this).callback.lock().ok().and_then(|mut g| g.take());
    if let Some(cb) = cb {
        cb();
    }
    S_OK
}

// ---------- Tracked Taskbar Targets & Runtime ----------

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum OwnedDispatcher {
    Queue(usize), // Owned raw pointer of agile `windows::System::DispatcherQueue`
    Core(u32),    // GIT cookie for `ICoreDispatcher`
}

pub(super) struct TrackedTarget {
    pub(super) thread_id: Option<u32>,
    pub(super) weak_ref_raw: usize,
    pub(super) kind: TargetKind,
    pub(super) orig_opacity: f64,
    pub(super) orig_visibility: i32,
    pub(super) dispatcher: OwnedDispatcher,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum DispatchFilter {
    QueueTid(u32),
    CoreCookie(u32),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum DispatchKey {
    ThreadQueue(u32),
    CoreCookie(u32),
}

pub(super) enum DispatchTarget {
    Queue {
        thread_id: u32,
        queue: DispatcherQueue,
    },
    Core {
        cookie: u32,
    },
}

pub(super) struct PendingBarrier {
    pub(super) remaining: Mutex<usize>,
    pub(super) cv: Condvar,
}

impl PendingBarrier {
    pub(super) fn new(count: usize) -> Arc<Self> {
        Arc::new(Self {
            remaining: Mutex::new(count),
            cv: Condvar::new(),
        })
    }

    pub(super) fn done_one(&self) {
        if let Ok(mut rem) = self.remaining.lock() {
            if *rem > 0 {
                *rem -= 1;
            }
            if *rem == 0 {
                self.cv.notify_all();
            }
        }
    }

    pub(super) fn wait_timeout(&self, max_wait: Duration) {
        let Ok(mut rem) = self.remaining.lock() else {
            return;
        };
        let deadline = Instant::now() + max_wait;
        while *rem > 0 {
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            if let Ok((next, _)) = self.cv.wait_timeout(rem, deadline - now) {
                rem = next;
            } else {
                break;
            }
        }
    }
}

pub(super) struct BarrierGuard {
    pub(super) barrier: Option<Arc<PendingBarrier>>,
}

impl Drop for BarrierGuard {
    fn drop(&mut self) {
        if let Some(b) = self.barrier.take() {
            b.done_one();
        }
    }
}

#[inline]
pub(super) fn revoke_git_cookie(git: Option<&IGlobalInterfaceTable>, cookie: u32) {
    if cookie != 0 {
        if let Some(g) = git {
            unsafe {
                let _ = g.RevokeInterfaceFromGlobal(cookie);
            }
        }
    }
}

pub(super) unsafe fn qi_interface(ptr: *mut c_void, iid: &GUID) -> ComRaw {
    if ptr.is_null() {
        return ComRaw::null();
    }
    let mut out = ComRaw::null();
    let unk = ptr.cast::<RawIUnknown>();
    if ((*(*unk).lp_vtbl).query_interface)(ptr, iid, &mut out.0).is_ok() {
        out
    } else {
        ComRaw::null()
    }
}

pub(super) unsafe fn com_same_identity(a: *mut c_void, b: *mut c_void) -> bool {
    if a.is_null() || b.is_null() {
        return false;
    }
    let ua = qi_interface(a, &IID_IUNKNOWN);
    let ub = qi_interface(b, &IID_IUNKNOWN);
    !ua.is_null() && ua.as_ptr() == ub.as_ptr()
}

pub(super) unsafe fn activate_factory_iface(class_name: &str, iid: &GUID) -> Option<ComRaw> {
    let class_id = HSTRING::from(class_name);
    let factory_insp: IInspectable = RoGetActivationFactory(&class_id).ok()?;
    let out = qi_interface(factory_insp.as_raw(), iid);
    (!out.is_null()).then_some(out)
}

pub(super) unsafe fn fetch_solid_brush(color: RawUiColor) -> Option<ComRaw> {
    let factory_com = activate_factory_iface(
        "Windows.UI.Xaml.Media.SolidColorBrush",
        &IID_ISOLIDCOLORBRUSHFACTORY,
    )?;
    let fac = factory_com.as_ptr().cast::<RawISolidColorBrushFactory>();
    let mut solid_com = ComRaw::null();
    if ((*(*fac).lp_vtbl).create_instance_with_color)(
        factory_com.as_ptr(),
        color,
        &mut solid_com.0,
    )
    .is_err()
        || solid_com.is_null()
    {
        return None;
    }
    let brush_com = qi_interface(solid_com.as_ptr(), &IID_IBRUSH);
    (!brush_com.is_null()).then_some(brush_com)
}

pub(super) unsafe fn fetch_transparent_brush() -> Option<ComRaw> {
    fetch_solid_brush(RawUiColor::TRANSPARENT)
}

pub(super) unsafe fn fetch_unset_value() -> Option<ComRaw> {
    let statics_com = activate_factory_iface(
        "Windows.UI.Xaml.DependencyProperty",
        &IID_IDEPENDENCYPROPERTYSTATICS,
    )?;
    let statics = statics_com
        .as_ptr()
        .cast::<RawIDependencyPropertyStatics>();
    let mut unset = ComRaw::null();
    if ((*(*statics).lp_vtbl).get_unset_value)(statics_com.as_ptr(), &mut unset.0).is_ok()
        && !unset.is_null()
    {
        Some(unset)
    } else {
        None
    }
}

pub(super) unsafe fn fetch_border_bg_dp() -> Option<ComRaw> {
    let statics_com =
        activate_factory_iface("Windows.UI.Xaml.Controls.Border", &IID_IBORDERSTATICS)?;
    let statics = statics_com.as_ptr().cast::<RawIBorderStatics>();
    let mut dp = ComRaw::null();
    if ((*(*statics).lp_vtbl).get_background_property)(statics_com.as_ptr(), &mut dp.0).is_ok()
        && !dp.is_null()
    {
        Some(dp)
    } else {
        None
    }
}

pub(super) unsafe fn fetch_border_brush_dp() -> Option<ComRaw> {
    let statics_com =
        activate_factory_iface("Windows.UI.Xaml.Controls.Border", &IID_IBORDERSTATICS)?;
    let statics = statics_com.as_ptr().cast::<RawIBorderStatics>();
    let mut dp = ComRaw::null();
    if ((*(*statics).lp_vtbl).get_border_brush_property)(statics_com.as_ptr(), &mut dp.0)
        .is_ok()
        && !dp.is_null()
    {
        Some(dp)
    } else {
        None
    }
}

pub(super) unsafe fn fetch_panel_bg_dp() -> Option<ComRaw> {
    let statics_com =
        activate_factory_iface("Windows.UI.Xaml.Controls.Panel", &IID_IPANELSTATICS)?;
    let statics = statics_com.as_ptr().cast::<RawIPanelStatics>();
    let mut dp = ComRaw::null();
    if ((*(*statics).lp_vtbl).get_background_property)(statics_com.as_ptr(), &mut dp.0).is_ok()
        && !dp.is_null()
    {
        Some(dp)
    } else {
        None
    }
}

pub(super) unsafe fn fetch_grid_border_brush_dp() -> Option<ComRaw> {
    let statics_com =
        activate_factory_iface("Windows.UI.Xaml.Controls.Grid", &IID_IGRIDSTATICS2)?;
    let statics = statics_com.as_ptr().cast::<RawIGridStatics2>();
    let mut dp = ComRaw::null();
    if ((*(*statics).lp_vtbl).get_border_brush_property)(statics_com.as_ptr(), &mut dp.0)
        .is_ok()
        && !dp.is_null()
    {
        Some(dp)
    } else {
        None
    }
}

pub(super) unsafe fn fetch_visual_tree_helper() -> Option<ComRaw> {
    activate_factory_iface(
        "Windows.UI.Xaml.Media.VisualTreeHelper",
        &IID_IVISUALTREEHELPERSTATICS,
    )
}

pub(super) unsafe fn cached_visual_tree_helper() -> Option<ComRaw> {
    ensure_cached_com(
        |s| s.visual_tree_helper.as_ref(),
        |s, v| s.visual_tree_helper = Some(v),
        fetch_visual_tree_helper,
    )
}

pub(super) unsafe fn fetch_element_composition_preview() -> Option<ComRaw> {
    activate_factory_iface(
        "Windows.UI.Xaml.Hosting.ElementCompositionPreview",
        &IID_IELEMENTCOMPOSITIONPREVIEWSTATICS,
    )
}

pub(super) unsafe fn cached_element_composition_preview() -> Option<ComRaw> {
    ensure_cached_com(
        |s| s.element_comp_preview.as_ref(),
        |s, v| s.element_comp_preview = Some(v),
        fetch_element_composition_preview,
    )
}

pub(super) unsafe fn fetch_focus_secondary_brush_dp() -> Option<ComRaw> {
    let statics_com = activate_factory_iface(
        "Windows.UI.Xaml.FrameworkElement",
        &IID_IFRAMEWORKELEMENTSTATICS4,
    )?;
    let statics = statics_com.as_ptr().cast::<RawIFrameworkElementStatics4>();
    let mut dp = ComRaw::null();
    if ((*(*statics).lp_vtbl).get_focus_visual_secondary_brush_property)(
        statics_com.as_ptr(),
        &mut dp.0,
    )
    .is_ok()
        && !dp.is_null()
    {
        Some(dp)
    } else {
        None
    }
}

pub(super) unsafe fn fetch_focus_secondary_thickness_dp() -> Option<ComRaw> {
    let statics_com = activate_factory_iface(
        "Windows.UI.Xaml.FrameworkElement",
        &IID_IFRAMEWORKELEMENTSTATICS4,
    )?;
    let statics = statics_com.as_ptr().cast::<RawIFrameworkElementStatics4>();
    let mut dp = ComRaw::null();
    if ((*(*statics).lp_vtbl).get_focus_visual_secondary_thickness_property)(
        statics_com.as_ptr(),
        &mut dp.0,
    )
    .is_ok()
        && !dp.is_null()
    {
        Some(dp)
    } else {
        None
    }
}

pub(super) unsafe fn fetch_element_shadow_dp() -> Option<ComRaw> {
    let statics_com =
        activate_factory_iface("Windows.UI.Xaml.UIElement", &IID_IUIELEMENTSTATICS10)?;
    let statics = statics_com.as_ptr().cast::<RawIUIElementStatics10>();
    let mut dp = ComRaw::null();
    if ((*(*statics).lp_vtbl).get_shadow_property)(statics_com.as_ptr(), &mut dp.0).is_ok()
        && !dp.is_null()
    {
        Some(dp)
    } else {
        None
    }
}

pub(super) unsafe fn fetch_content_presenter_bg_dp() -> Option<ComRaw> {
    let statics_com = activate_factory_iface(
        "Windows.UI.Xaml.Controls.ContentPresenter",
        &IID_ICONTENTPRESENTERSTATICS4,
    )?;
    let statics = statics_com
        .as_ptr()
        .cast::<RawIContentPresenterStatics4>();
    let mut dp = ComRaw::null();
    if ((*(*statics).lp_vtbl).get_background_property)(statics_com.as_ptr(), &mut dp.0)
        .is_ok()
        && !dp.is_null()
    {
        Some(dp)
    } else {
        None
    }
}

pub(super) unsafe fn fetch_content_presenter_border_brush_dp() -> Option<ComRaw> {
    let statics_com = activate_factory_iface(
        "Windows.UI.Xaml.Controls.ContentPresenter",
        &IID_ICONTENTPRESENTERSTATICS4,
    )?;
    let statics = statics_com
        .as_ptr()
        .cast::<RawIContentPresenterStatics4>();
    let mut dp = ComRaw::null();
    if ((*(*statics).lp_vtbl).get_border_brush_property)(statics_com.as_ptr(), &mut dp.0)
        .is_ok()
        && !dp.is_null()
    {
        Some(dp)
    } else {
        None
    }
}

pub(super) unsafe fn fetch_control_bg_dp() -> Option<ComRaw> {
    let statics_com =
        activate_factory_iface("Windows.UI.Xaml.Controls.Control", &IID_ICONTROLSTATICS)?;
    let statics = statics_com.as_ptr().cast::<RawIControlStatics>();
    let mut dp = ComRaw::null();
    if ((*(*statics).lp_vtbl).get_background_property)(statics_com.as_ptr(), &mut dp.0).is_ok()
        && !dp.is_null()
    {
        Some(dp)
    } else {
        None
    }
}

pub(super) unsafe fn fetch_control_border_brush_dp() -> Option<ComRaw> {
    let statics_com =
        activate_factory_iface("Windows.UI.Xaml.Controls.Control", &IID_ICONTROLSTATICS)?;
    let statics = statics_com.as_ptr().cast::<RawIControlStatics>();
    let mut dp = ComRaw::null();
    if ((*(*statics).lp_vtbl).get_border_brush_property)(statics_com.as_ptr(), &mut dp.0)
        .is_ok()
        && !dp.is_null()
    {
        Some(dp)
    } else {
        None
    }
}

pub(super) unsafe fn fetch_control_corner_radius_dp() -> Option<ComRaw> {
    let statics_com =
        activate_factory_iface("Windows.UI.Xaml.Controls.Control", &IID_ICONTROLSTATICS7)?;
    let statics = statics_com.as_ptr().cast::<RawIControlStatics7>();
    let mut dp = ComRaw::null();
    if ((*(*statics).lp_vtbl).get_corner_radius_property)(statics_com.as_ptr(), &mut dp.0)
        .is_ok()
        && !dp.is_null()
    {
        Some(dp)
    } else {
        None
    }
}

pub(super) unsafe fn com_identity_key(ptr: *mut c_void) -> u64 {
    let unk = qi_interface(ptr, &IID_IUNKNOWN);
    let raw = if !unk.is_null() {
        unk.as_ptr() as u64
    } else {
        ptr as u64
    };
    (1u64 << 63) | (raw & !(1u64 << 63))
}

pub(super) unsafe fn element_framework_name(ptr: *mut c_void) -> Option<String> {
    let fe_com = qi_interface(ptr, &IID_IFRAMEWORKELEMENT);
    if fe_com.is_null() {
        return None;
    }
    let fe = fe_com.as_ptr().cast::<RawIFrameworkElement>();
    let mut hs = HSTRING::new();
    if ((*(*fe).lp_vtbl).get_name)(fe_com.as_ptr(), &mut hs).is_ok() {
        Some(hs.to_string_lossy())
    } else {
        None
    }
}

pub(super) unsafe fn element_runtime_class_name(ptr: *mut c_void) -> Option<String> {
    let insp_com = qi_interface(ptr, &IID_IINSPECTABLE);
    if insp_com.is_null() {
        return None;
    }
    let insp = insp_com.as_ptr().cast::<RawIInspectable>();
    let mut hs = HSTRING::new();
    if ((*(*insp).lp_vtbl).get_runtime_class_name)(insp_com.as_ptr(), &mut hs).is_ok() {
        Some(hs.to_string_lossy())
    } else {
        None
    }
}

pub(super) unsafe fn is_transparent_solid_brush(brush_ptr: *mut c_void) -> bool {
    if brush_ptr.is_null() {
        return false;
    }
    let solid_com = qi_interface(brush_ptr, &IID_ISOLIDCOLORBRUSH);
    if solid_com.is_null() {
        return false;
    }
    let solid = solid_com.as_ptr().cast::<RawISolidColorBrush>();
    let mut col = RawUiColor {
        a: 255,
        r: 0,
        g: 0,
        b: 0,
    };
    ((*(*solid).lp_vtbl).get_color)(solid_com.as_ptr(), &mut col).is_ok() && col.a == 0
}

pub(super) unsafe fn is_solid_brush_with_color(brush_ptr: *mut c_void, target: RawUiColor) -> bool {
    if brush_ptr.is_null() {
        return false;
    }
    let solid_com = qi_interface(brush_ptr, &IID_ISOLIDCOLORBRUSH);
    if solid_com.is_null() {
        return false;
    }
    let solid = solid_com.as_ptr().cast::<RawISolidColorBrush>();
    let mut col = RawUiColor {
        a: target.a.wrapping_add(1),
        r: 0,
        g: 0,
        b: 0,
    };
    ((*(*solid).lp_vtbl).get_color)(solid_com.as_ptr(), &mut col).is_ok() && col == target
}

