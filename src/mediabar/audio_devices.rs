use std::ffi::c_void;
use windows::core::{GUID, HRESULT, IUnknown, Interface, PCWSTR, PWSTR};
use windows::Win32::Media::Audio::{
    eCapture, eCommunications, eConsole, eMultimedia, eRender, DEVICE_STATE_ACTIVE, ERole,
    IMMDeviceEnumerator, MMDeviceEnumerator,
};
use windows::Win32::System::Com::StructuredStorage::PropVariantToStringAlloc;
use windows::Win32::System::Com::{CoCreateInstance, CoTaskMemFree, CLSCTX_ALL, STGM_READ};
use windows::Win32::UI::Shell::PropertiesSystem::{IPropertyStore, PROPERTYKEY};

/// Active system playback or recording audio endpoint entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioEndpointItem {
    pub id: String,
    pub main_name: String,
    pub sub_name: String,
    pub is_default: bool,
}

const PKEY_DEVICE_DEVICE_DESC: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0xa45c254e_df1c_4efd_8020_67d146a850e0),
    pid: 2,
};
const PKEY_DEVICE_FRIENDLY_NAME: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0xa45c254e_df1c_4efd_8020_67d146a850e0),
    pid: 14,
};
const PKEY_DEVICE_INTERFACE_FRIENDLY_NAME: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0x026e516e_b814_414b_83cd_856d6fef4822),
    pid: 2,
};

const CLSID_POLICY_CONFIG_CLIENT: GUID = GUID::from_u128(0x870af99c_171d_4f9e_af0d_e63df40c2bc9);
const IID_IPOLICY_CONFIG: GUID = GUID::from_u128(0xf8679f50_850a_41cf_9c72_430f290290c8);
const IID_IPOLICY_CONFIG_VISTA: GUID = GUID::from_u128(0x568b9108_44bf_40b4_9006_86afe5b5a620);

#[repr(C)]
struct PolicyConfigVtbl {
    base: windows::core::IUnknown_Vtbl,
    _pad3: usize,
    _pad4: usize,
    _pad5: usize,
    _pad6: usize,
    _pad7: usize,
    _pad8: usize,
    _pad9: usize,
    _pad10: usize,
    _pad11: usize,
    _pad12: usize,
    set_default_endpoint:
        unsafe extern "system" fn(this: *mut c_void, device_id: PCWSTR, role: ERole) -> HRESULT,
    _pad14: usize,
}

struct OwnedPolicyConfig(*mut c_void);

impl Drop for OwnedPolicyConfig {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                let vtbl = *(self.0 as *const *const PolicyConfigVtbl);
                ((*vtbl).base.Release)(self.0);
            }
        }
    }
}

unsafe fn take_pwstr(ptr: PWSTR) -> String {
    if ptr.is_null() {
        return String::new();
    }
    let s = ptr.to_string().unwrap_or_default();
    CoTaskMemFree(Some(ptr.0.cast()));
    s
}

unsafe fn read_prop_string(store: &IPropertyStore, key: &PROPERTYKEY) -> String {
    let Ok(val) = store.GetValue(key) else {
        return String::new();
    };
    match PropVariantToStringAlloc(&val) {
        Ok(pwstr) => take_pwstr(pwstr).trim().to_string(),
        Err(_) => String::new(),
    }
}

/// Lists all active enabled playback (`capture = false`) or recording (`capture = true`) endpoints.
pub fn enumerate_audio_endpoints(capture: bool) -> Vec<AudioEndpointItem> {
    unsafe {
        let Ok(enumerator): windows::core::Result<IMMDeviceEnumerator> =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
        else {
            return Vec::new();
        };

        let flow = if capture { eCapture } else { eRender };
        let default_id = enumerator
            .GetDefaultAudioEndpoint(flow, eMultimedia)
            .or_else(|_| enumerator.GetDefaultAudioEndpoint(flow, eConsole))
            .ok()
            .and_then(|dev| dev.GetId().ok())
            .map(|p| take_pwstr(p))
            .unwrap_or_default();

        let Ok(collection) = enumerator.EnumAudioEndpoints(flow, DEVICE_STATE_ACTIVE) else {
            return Vec::new();
        };
        let count = collection.GetCount().unwrap_or(0);
        let mut items = Vec::with_capacity(count as usize);

        for i in 0..count {
            let Ok(dev) = collection.Item(i) else {
                continue;
            };
            let id = dev
                .GetId()
                .ok()
                .map(|p| take_pwstr(p))
                .unwrap_or_default();
            if id.is_empty() {
                continue;
            }

            let (main_name, sub_name) = match dev.OpenPropertyStore(STGM_READ) {
                Ok(store) => {
                    let desc = read_prop_string(&store, &PKEY_DEVICE_DEVICE_DESC);
                    let iface = read_prop_string(&store, &PKEY_DEVICE_INTERFACE_FRIENDLY_NAME);
                    let friendly = read_prop_string(&store, &PKEY_DEVICE_FRIENDLY_NAME);
                    if !desc.is_empty() {
                        (desc, iface)
                    } else if !friendly.is_empty() {
                        (friendly, iface)
                    } else {
                        ("音频设备".to_string(), iface)
                    }
                }
                Err(_) => ("音频设备".to_string(), String::new()),
            };

            items.push(AudioEndpointItem {
                is_default: !default_id.is_empty() && id == default_id,
                id,
                main_name,
                sub_name,
            });
        }

        items
    }
}

/// Switches the Windows default playback/recording endpoint for all roles
/// (`eConsole`, `eMultimedia`, and `eCommunications`).
pub fn set_default_audio_endpoint(device_id: &str) -> bool {
    if device_id.is_empty() {
        return false;
    }

    unsafe {
        let Ok(client): windows::core::Result<IUnknown> =
            CoCreateInstance(&CLSID_POLICY_CONFIG_CLIENT, None, CLSCTX_ALL)
        else {
            return false;
        };

        let mut policy_ptr: *mut c_void = std::ptr::null_mut();
        let mut hr = (client.vtable().QueryInterface)(
            client.as_raw(),
            &IID_IPOLICY_CONFIG,
            &mut policy_ptr,
        );
        if hr.is_err() || policy_ptr.is_null() {
            hr = (client.vtable().QueryInterface)(
                client.as_raw(),
                &IID_IPOLICY_CONFIG_VISTA,
                &mut policy_ptr,
            );
        }
        if hr.is_err() || policy_ptr.is_null() {
            return false;
        }

        let _policy = OwnedPolicyConfig(policy_ptr);
        let vtbl = *(policy_ptr as *const *const PolicyConfigVtbl);
        let wide: Vec<u16> = device_id.encode_utf16().chain(std::iter::once(0)).collect();
        let pcw = PCWSTR(wide.as_ptr());

        let mut any_ok = false;
        for role in [eConsole, eMultimedia, eCommunications] {
            if ((*vtbl).set_default_endpoint)(policy_ptr, pcw, role).is_ok() {
                any_ok = true;
            }
        }
        any_ok
    }
}
