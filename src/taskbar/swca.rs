use super::config::TaskbarStyle;

#[allow(dead_code)]
pub const ACCENT_DISABLED: u32 = 0;
#[allow(dead_code)]
pub const ACCENT_ENABLE_TRANSPARENTGRADIENT: u32 = 2;
pub const ACCENT_ENABLE_BLURBEHIND: u32 = 3;
pub const ACCENT_ENABLE_ACRYLICBLURBEHIND: u32 = 4;

/// Flag enabling custom `gradient_color` tint inside `ACCENT_POLICY`.
pub const ACCENT_FLAG_USE_GRADIENT: u32 = 2;

/// Packs `(a, r, g, b)` into Win32 `ACCENT_POLICY.GradientColor` (`0xAABBGGRR`).
#[inline]
pub const fn abgr(a: u8, r: u8, g: u8, b: u8) -> u32 {
    ((a as u32) << 24) | ((b as u32) << 16) | ((g as u32) << 8) | (r as u32)
}

/// DWM accent policy specification applied to our behind-taskbar backdrop windows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AccentPolicySpec {
    pub state: u32,
    pub flags: u32,
    pub gradient_color: u32,
}

/// Maps `TaskbarStyle` + system light/dark theme to the `ACCENT_POLICY` of our behind-taskbar backdrop window.
/// `Default` and `Transparent` need no backdrop window (`None`).
#[cfg_attr(not(windows), allow(dead_code))]
pub const fn backdrop_blur_policy(
    style: TaskbarStyle,
    light_taskbar: bool,
) -> Option<AccentPolicySpec> {
    match style {
        TaskbarStyle::Default | TaskbarStyle::Transparent => None,
        TaskbarStyle::Blur => Some(AccentPolicySpec {
            state: ACCENT_ENABLE_BLURBEHIND,
            flags: 0,
            gradient_color: 0,
        }),
        TaskbarStyle::Acrylic => Some(AccentPolicySpec {
            state: ACCENT_ENABLE_ACRYLICBLURBEHIND,
            flags: ACCENT_FLAG_USE_GRADIENT,
            gradient_color: if light_taskbar {
                abgr(0x78, 0xF6, 0xF6, 0xF8)
            } else {
                abgr(0x66, 0x18, 0x18, 0x1A)
            },
        }),
        TaskbarStyle::LiquidGlass => Some(AccentPolicySpec {
            state: ACCENT_ENABLE_ACRYLICBLURBEHIND,
            flags: ACCENT_FLAG_USE_GRADIENT,
            gradient_color: if light_taskbar {
                abgr(0x14, 0xF6, 0xF6, 0xF8)
            } else {
                abgr(0x0A, 0x18, 0x18, 0x1A)
            },
        }),
    }
}

#[cfg(windows)]
mod win {
    use super::AccentPolicySpec;
    use std::ffi::c_void;
    use std::mem::size_of;
    use std::sync::OnceLock;
    use windows::core::{s, w};
    use windows::Win32::Foundation::{BOOL, HWND};
    use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryW};
    use windows::Win32::UI::WindowsAndMessaging::{
        FindWindowExW, FindWindowW, GetWindowThreadProcessId,
    };

    const WCA_ACCENT_POLICY: u32 = 19;

    #[repr(C)]
    struct AccentPolicy {
        accent_state: u32,
        accent_flags: u32,
        gradient_color: u32,
        animation_id: u32,
    }

    #[repr(C)]
    struct WindowCompositionAttribData {
        attrib: u32,
        pv_data: *mut c_void,
        cb_data: usize,
    }

    type SetWindowCompositionAttributeFn =
        unsafe extern "system" fn(HWND, *mut WindowCompositionAttribData) -> BOOL;

    fn load_swca() -> Option<SetWindowCompositionAttributeFn> {
        static CACHE: OnceLock<Option<SetWindowCompositionAttributeFn>> = OnceLock::new();
        *CACHE.get_or_init(|| unsafe {
            let hmod = GetModuleHandleW(w!("user32.dll"))
                .ok()
                .or_else(|| LoadLibraryW(w!("user32.dll")).ok())?;
            let proc = GetProcAddress(hmod, s!("SetWindowCompositionAttribute"))?;
            Some(std::mem::transmute::<_, SetWindowCompositionAttributeFn>(
                proc,
            ))
        })
    }

    pub fn apply_window_accent(hwnd: HWND, spec: AccentPolicySpec) -> bool {
        if hwnd.is_invalid() {
            return false;
        }
        let Some(swca) = load_swca() else {
            return false;
        };
        let mut policy = AccentPolicy {
            accent_state: spec.state,
            accent_flags: spec.flags,
            gradient_color: spec.gradient_color,
            animation_id: 0,
        };
        let mut data = WindowCompositionAttribData {
            attrib: WCA_ACCENT_POLICY,
            pv_data: (&mut policy as *mut AccentPolicy).cast(),
            cb_data: size_of::<AccentPolicy>(),
        };
        unsafe { swca(hwnd, &mut data).as_bool() }
    }

    pub fn enumerate_taskbar_windows() -> Vec<HWND> {
        let mut result = Vec::new();
        unsafe {
            if let Ok(primary) = FindWindowW(w!("Shell_TrayWnd"), None) {
                if !primary.is_invalid() {
                    result.push(primary);
                }
            }
            let mut prev = HWND::default();
            loop {
                let Ok(sec) = FindWindowExW(None, prev, w!("Shell_SecondaryTrayWnd"), None) else {
                    break;
                };
                if sec.is_invalid() {
                    break;
                }
                if !result.contains(&sec) {
                    result.push(sec);
                }
                prev = sec;
            }
        }
        result
    }

    pub fn explorer_pid_from_taskbars(hwnds: &[HWND]) -> Option<u32> {
        for &hwnd in hwnds {
            if hwnd.is_invalid() {
                continue;
            }
            let mut pid = 0u32;
            unsafe {
                let _ = GetWindowThreadProcessId(hwnd, Some(&mut pid));
            }
            if pid != 0 {
                return Some(pid);
            }
        }
        None
    }
}

#[cfg(windows)]
pub use win::{apply_window_accent, enumerate_taskbar_windows, explorer_pid_from_taskbars};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abgr_packs_bytes_in_dwm_byte_order() {
        assert_eq!(abgr(0x6E, 0x18, 0x2A, 0x3C), 0x6E3C_2A18);
        assert_eq!(abgr(0x00, 0xFF, 0x00, 0x10), 0x0010_00FF);
    }

    #[test]
    fn backdrop_policies_map_distinctly_across_all_five_styles() {
        for light in [false, true] {
            assert_eq!(backdrop_blur_policy(TaskbarStyle::Default, light), None);
            assert_eq!(backdrop_blur_policy(TaskbarStyle::Transparent, light), None);

            let blur = backdrop_blur_policy(TaskbarStyle::Blur, light)
                .expect("Blur must produce backdrop policy");
            assert_eq!(blur.state, ACCENT_ENABLE_BLURBEHIND);
            assert_eq!(blur.flags, 0);

            let acrylic = backdrop_blur_policy(TaskbarStyle::Acrylic, light)
                .expect("Acrylic must produce backdrop policy");
            assert_eq!(acrylic.state, ACCENT_ENABLE_ACRYLICBLURBEHIND);
            assert_ne!(acrylic.gradient_color >> 24, 0);

            let glass = backdrop_blur_policy(TaskbarStyle::LiquidGlass, light)
                .expect("LiquidGlass must produce backdrop policy");
            assert_eq!(glass.state, ACCENT_ENABLE_ACRYLICBLURBEHIND);
            assert_ne!(glass.gradient_color >> 24, 0);
            assert_ne!(glass.gradient_color, acrylic.gradient_color);
        }
    }
}
