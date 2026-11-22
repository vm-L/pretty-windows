use crate::color::{derive_background_accent, enforce_ui_accent_deadzone, Rgb};
use std::mem::size_of;
use std::os::windows::ffi::OsStrExt;
use std::sync::{Mutex, OnceLock};
use windows::core::{w, PCWSTR};
use windows::UI::ViewManagement::{UIColorType, UISettings};
use windows::Win32::System::Registry::{RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_DWORD, REG_EXPAND_SZ, REG_OPTION_NON_VOLATILE, REG_SZ, REG_VALUE_TYPE};

const RUN_SUBKEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const RUN_VALUE_NAME: PCWSTR = w!("PrettyWindows");
const LEGACY_RUN_VALUE_NAME: PCWSTR = w!("MediaBar");

/// Reads a `REG_DWORD` value from the Windows Registry.
pub(crate) fn read_reg_dword(root: HKEY, subkey: PCWSTR, value_name: PCWSTR) -> Option<u32> {
    unsafe {
        let mut hkey = HKEY::default();
        if RegOpenKeyExW(root, subkey, 0, KEY_READ, &mut hkey).0 != 0 {
            return None;
        }
        let mut val = 0u32;
        let mut val_type = REG_VALUE_TYPE::default();
        let mut size = size_of::<u32>() as u32;
        let q = RegQueryValueExW(
            hkey,
            value_name,
            None,
            Some(&mut val_type),
            Some((&mut val as *mut u32).cast::<u8>()),
            Some(&mut size),
        );
        let _ = RegCloseKey(hkey);
        (q.0 == 0 && val_type == REG_DWORD).then_some(val)
    }
}

/// Reads Windows 11 taskbar/system theme (`true` = light theme, `false` = dark theme).
pub fn is_light_taskbar() -> bool {
    read_reg_dword(
        HKEY_CURRENT_USER,
        w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
        w!("SystemUsesLightTheme"),
    ) == Some(1)
}

/// Returns `true` when Windows "Transparency effects" (`EnableTransparency`) is enabled.
/// Defaults to `true` if the registry value is absent.
pub fn is_transparency_enabled() -> bool {
    read_reg_dword(
        HKEY_CURRENT_USER,
        w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
        w!("EnableTransparency"),
    ) != Some(0)
}

/// Reads the current Windows System Accent Color, falling back to classic Win11 blue.
pub fn system_accent_color() -> Rgb {
    if let Ok(settings) = UISettings::new() {
        if let Ok(c) = settings.GetColorValue(UIColorType::Accent) {
            return Rgb::new(c.R, c.G, c.B);
        }
    }
    Rgb::new(0, 120, 212)
}

fn mediabar_shared_accent_slot() -> &'static Mutex<Option<(Rgb, Rgb)>> {
    static SHARED_ACCENT: OnceLock<Mutex<Option<(Rgb, Rgb)>>> = OnceLock::new();
    SHARED_ACCENT.get_or_init(|| Mutex::new(None))
}

/// Publishes MediaBar's active cover accent pair `(light, dark)` for Tray & MonitorBar synchronization.
/// Pass `None` when MediaBar is stopped, auto-color is disabled, or no active cover exists.
pub fn set_mediabar_shared_accent(accent: Option<(Rgb, Rgb)>) {
    if let Ok(mut guard) = mediabar_shared_accent_slot().lock() {
        *guard = accent;
    }
}

/// Returns MediaBar's currently published `(accent_light, accent_dark)`, if active.
pub fn get_mediabar_shared_accent() -> Option<(Rgb, Rgb)> {
    mediabar_shared_accent_slot().lock().ok().and_then(|g| *g)
}

/// Resolves `(ui_accent, halo_dark_base)` across Tray, MediaBar, and MonitorBar:
/// prefers MediaBar's published album accent when active; otherwise lifts backdrop lightness preserving hue.
/// Applies deadzone protection only to `ui_accent`, keeping silk halo base colors soft.
pub fn resolve_unified_accent(backdrop_bg: Rgb, light_glass: bool) -> (Rgb, Rgb) {
    if let Some((al, ad)) = get_mediabar_shared_accent() {
        let ui_accent =
            enforce_ui_accent_deadzone(if light_glass { al } else { ad }, light_glass);
        return (ui_accent, ad);
    }
    (
        enforce_ui_accent_deadzone(
            derive_background_accent(backdrop_bg, light_glass),
            light_glass,
        ),
        derive_background_accent(backdrop_bg, false),
    )
}

/// Returns `true` if Pretty Windows is configured to launch the current executable at Windows startup.
pub fn is_autostart_enabled() -> bool {
    let Ok(exe_path) = std::env::current_exe() else {
        return false;
    };
    let exe_str = exe_path.to_string_lossy();
    unsafe {
        let mut hkey = HKEY::default();
        if RegOpenKeyExW(HKEY_CURRENT_USER, RUN_SUBKEY, 0, KEY_READ, &mut hkey).0 != 0 {
            return false;
        }
        let mut val_type = REG_VALUE_TYPE::default();
        let mut size = 0u32;
        let q1 = RegQueryValueExW(
            hkey,
            RUN_VALUE_NAME,
            None,
            Some(&mut val_type),
            None,
            Some(&mut size),
        );
        if q1.0 != 0 || size < 2 || (val_type != REG_SZ && val_type != REG_EXPAND_SZ) {
            let _ = RegCloseKey(hkey);
            return false;
        }
        let mut buf = vec![0u16; (size as usize).div_ceil(2)];
        let mut byte_len = (buf.len() * size_of::<u16>()) as u32;
        let q2 = RegQueryValueExW(
            hkey,
            RUN_VALUE_NAME,
            None,
            Some(&mut val_type),
            Some(buf.as_mut_ptr().cast::<u8>()),
            Some(&mut byte_len),
        );
        let _ = RegCloseKey(hkey);
        if q2.0 != 0 || (val_type != REG_SZ && val_type != REG_EXPAND_SZ) {
            return false;
        }
        let u16_len = ((byte_len as usize) / 2).min(buf.len());
        let decoded = String::from_utf16_lossy(&buf[..u16_len]);
        let trimmed = decoded.trim_end_matches('\0');
        super::autostart_value_matches_exe(trimmed, &exe_str)
    }
}

/// Enables or disables auto-start in HKCU Run registry key.
pub fn set_autostart_enabled(enable: bool) -> bool {
    unsafe {
        let mut hkey = HKEY::default();
        let open_res = if enable {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                RUN_SUBKEY,
                0,
                None,
                REG_OPTION_NON_VOLATILE,
                KEY_WRITE,
                None,
                &mut hkey,
                None,
            )
        } else {
            RegOpenKeyExW(HKEY_CURRENT_USER, RUN_SUBKEY, 0, KEY_WRITE, &mut hkey)
        };
        if open_res.0 != 0 {
            return false;
        }
        let ok = if enable {
            let Ok(exe_path) = std::env::current_exe() else {
                let _ = RegCloseKey(hkey);
                return false;
            };
            let wide: Vec<u16> = std::iter::once(u16::from(b'"'))
                .chain(exe_path.as_os_str().encode_wide())
                .chain([u16::from(b'"'), 0])
                .collect();
            let byte_slice = std::slice::from_raw_parts(
                wide.as_ptr().cast::<u8>(),
                wide.len() * size_of::<u16>(),
            );
            let set_ok =
                RegSetValueExW(hkey, RUN_VALUE_NAME, 0, REG_SZ, Some(byte_slice)).0 == 0;
            if set_ok {
                let _ = RegDeleteValueW(hkey, LEGACY_RUN_VALUE_NAME);
            }
            set_ok
        } else {
            let err = RegDeleteValueW(hkey, RUN_VALUE_NAME);
            let _ = RegDeleteValueW(hkey, LEGACY_RUN_VALUE_NAME);
            err.0 == 0 || err.0 == 2 // ERROR_FILE_NOT_FOUND
        };
        let _ = RegCloseKey(hkey);
        ok
    }
}
