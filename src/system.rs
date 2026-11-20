use crate::ui::GlassSurfaceStyle;
use std::sync::atomic::{AtomicU8, Ordering};

static ACTIVE_TASKBAR_SURFACE_STYLE: AtomicU8 =
    AtomicU8::new(GlassSurfaceStyle::LiquidGlass.to_u8());

/// Publishes the active taskbar visual style so all popup surfaces harmonize with the taskbar.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn set_active_taskbar_surface_style(style: GlassSurfaceStyle) {
    ACTIVE_TASKBAR_SURFACE_STYLE.store(style.to_u8(), Ordering::Relaxed);
}

/// Returns the active taskbar visual style published by `TaskbarController`.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn active_taskbar_surface_style() -> GlassSurfaceStyle {
    GlassSurfaceStyle::from_u8(ACTIVE_TASKBAR_SURFACE_STYLE.load(Ordering::Relaxed))
}

/// Extracts the executable filename from a Windows/POSIX path.
pub(crate) fn extract_exe_basename(path: &str) -> &str {
    path.rsplit(['\\', '/']).next().unwrap_or(path).trim()
}

/// Returns `true` if `class_name` belongs to Windows desktop surfaces or modern shell flyouts
/// (Start, Quick Settings, Search, System Tray overflow, Input switcher, etc.).
pub(crate) fn is_shell_or_flyout_class_name(class_name: &str) -> bool {
    matches!(
        class_name,
        "Progman"
            | "WorkerW"
            | "Shell_TrayWnd"
            | "Shell_SecondaryTrayWnd"
            | "ControlCenterWindow"
            | "TopLevelWindowForOverflowXamlIsland"
            | "XamlExplorerHostIslandWindow"
            | "NotifyIconOverflowWindow"
            | "Shell_InputSwitchTopLevelWindow"
            | "Windows.UI.Core.CoreWindow"
            | "Windows.UI.Input.InputSite.WindowClass"
    )
}

/// Returns `true` if `exe_name` is a known Windows shell flyout host process.
///
/// Intentionally excludes `explorer.exe` because fullscreen File Explorer (`CabinetWClass`)
/// should still count as fullscreen occlusion.
pub(crate) fn is_shell_flyout_process_name(exe_name: &str) -> bool {
    let name = exe_name.trim();
    name.eq_ignore_ascii_case("shellhost.exe")
        || name.eq_ignore_ascii_case("shellexperiencehost.exe")
        || name.eq_ignore_ascii_case("startmenuexperiencehost.exe")
        || name.eq_ignore_ascii_case("searchhost.exe")
        || name.eq_ignore_ascii_case("searchapp.exe")
        || name.eq_ignore_ascii_case("textinputhost.exe")
        || name.eq_ignore_ascii_case("widgets.exe")
}

/// Returns `true` if `class_name` represents a transient desktop shell flyout or tooltip window
/// that may vanish right after a popup activates and should be excluded from backdrop captures.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn is_transient_backdrop_occluder_class(class_name: &str) -> bool {
    matches!(
        class_name,
        "TopLevelWindowForOverflowXamlIsland"
            | "NotifyIconOverflowWindow"
            | "XamlExplorerHostIslandWindow"
            | "ControlCenterWindow"
            | "tooltips_class32"
            | "Xaml_WindowedPopupClass"
            | "TaskListThumbnailWnd"
    )
}

/// Picks the lowest (largest `z_rank`) same-band window among the taskbar and visible taskbar
/// overlay widgets on the same monitor so a behind-taskbar backdrop never jumps above widgets
/// even when Windows temporarily elevates `Shell_TrayWnd` to a higher Z-order band (e.g. Start Menu).
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn pick_lowest_backdrop_anchor(
    taskbar_id: usize,
    taskbar_band: Option<u32>,
    backdrop_band: Option<u32>,
    taskbar_z_rank: Option<usize>,
    same_band_overlays_with_rank: &[(usize, usize)],
) -> usize {
    let mut best: Option<(usize, usize)> = None;
    let tb_same_band = match (taskbar_band, backdrop_band) {
        (Some(tb), Some(bg)) => tb == bg,
        _ => true,
    };
    if tb_same_band {
        if let Some(rank) = taskbar_z_rank {
            best = Some((taskbar_id, rank));
        }
    }
    for &(id, rank) in same_band_overlays_with_rank {
        if best.is_none_or(|(_, best_rank)| rank > best_rank) {
            best = Some((id, rank));
        }
    }
    best.map(|(id, _)| id).unwrap_or(taskbar_id)
}

/// Returns `true` when two axis-aligned screen rectangles `[left, top, right, bottom]` have positive intersection area.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn rects_intersect(a: [i32; 4], b: [i32; 4]) -> bool {
    a[0].max(b[0]) < a[2].min(b[2]) && a[1].max(b[1]) < a[3].min(b[3])
}

/// Horizontal gap (in DIPs) enforced between MediaBar and MonitorBar when adjacent on the taskbar.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) const TASKBAR_BAR_GAP_DIP: f32 = 2.0;

/// Deadband (in DIPs) past the peer's horizontal midpoint before dragging swaps to the opposite side.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) const TASKBAR_BAR_SWAP_HYSTERESIS_DIP: f32 = 8.0;

/// Identifies which taskbar widget occupies a registered horizontal collision slot.
#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TaskbarBarSlot {
    MediaBar = 0,
    MonitorBar = 1,
}

impl TaskbarBarSlot {
    #[cfg(windows)]
    const fn index(self) -> usize {
        self as usize
    }

    #[cfg(windows)]
    const fn peer(self) -> Self {
        match self {
            Self::MediaBar => Self::MonitorBar,
            Self::MonitorBar => Self::MediaBar,
        }
    }
}

/// Strategy used when resolving a bar's horizontal position against an active peer bar.
#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BarCollisionMode {
    /// Passive repositioning (startup, peer resize, display/taskbar change, Reset Position).
    Passive,
    /// Interactive horizontal drag: holds flush at `gap_px` and swaps across peer midpoint
    /// only after exceeding `hysteresis_px`.
    Drag,
}

#[cfg_attr(not(windows), allow(dead_code))]
fn horizontal_overlap_px(x: i32, width: i32, peer_left: i32, peer_right: i32) -> i64 {
    let left = i64::from(x).max(i64::from(peer_left));
    let right = (i64::from(x) + i64::from(width)).min(i64::from(peer_right));
    (right - left).max(0)
}

/// Resolves a bar's horizontal screen X coordinate inside `[min_x, max_x]` so it avoids
/// overlapping `peer_span: Option<(peer_left, peer_right)>` with at least `gap_px` spacing.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn resolve_taskbar_bar_x(
    desired_x: i32,
    bar_width_px: i32,
    min_x: i32,
    max_x: i32,
    current_x: Option<i32>,
    peer_span: Option<(i32, i32)>,
    gap_px: i32,
    hysteresis_px: i32,
    mode: BarCollisionMode,
) -> i32 {
    let width = bar_width_px.max(1);
    let effective_max = max_x.max(min_x);
    let base_x = desired_x.clamp(min_x, effective_max);

    let Some((peer_left, peer_right)) = peer_span else {
        return base_x;
    };
    if peer_right <= peer_left {
        return base_x;
    }

    let gap = i64::from(gap_px.max(0));
    let left_candidate64 = i64::from(peer_left) - gap - i64::from(width);
    let right_candidate64 = i64::from(peer_right) + gap;

    if i64::from(base_x) <= left_candidate64 || i64::from(base_x) >= right_candidate64 {
        return base_x;
    }

    let left_fits = left_candidate64 >= i64::from(min_x);
    let right_fits = right_candidate64 <= i64::from(effective_max);

    if left_fits || right_fits {
        let peer_mid2 = i64::from(peer_left) + i64::from(peer_right);
        let desired_mid2 = 2 * i64::from(desired_x) + i64::from(width);
        let choose_left = match (left_fits, right_fits) {
            (true, false) => true,
            (false, true) => false,
            (true, true) => match mode {
                BarCollisionMode::Passive => {
                    if desired_mid2 != peer_mid2 {
                        desired_mid2 < peer_mid2
                    } else if let Some(cur) = current_x {
                        2 * i64::from(cur) + i64::from(width) <= peer_mid2
                    } else {
                        true
                    }
                }
                BarCollisionMode::Drag => {
                    let hyst2 = 2 * i64::from(hysteresis_px.max(0));
                    let cur_left = current_x
                        .map(|cur| 2 * i64::from(cur) + i64::from(width) <= peer_mid2)
                        .unwrap_or(desired_mid2 <= peer_mid2);
                    if cur_left {
                        desired_mid2 <= peer_mid2 + hyst2
                    } else {
                        desired_mid2 < peer_mid2 - hyst2
                    }
                }
            },
            (false, false) => unreachable!(),
        };

        let target64 = if choose_left {
            left_candidate64
        } else {
            right_candidate64
        };
        return target64.clamp(i64::from(min_x), i64::from(effective_max)) as i32;
    }

    // Narrow taskbar fallback: neither clear side fits inside [min_x, effective_max];
    // pick the boundary that yields strictly smaller overlap with the peer.
    let left_overlap = horizontal_overlap_px(min_x, width, peer_left, peer_right);
    let right_overlap = horizontal_overlap_px(effective_max, width, peer_left, peer_right);
    if left_overlap < right_overlap {
        min_x
    } else if right_overlap < left_overlap {
        effective_max
    } else {
        let cur = current_x.unwrap_or(base_x);
        if (i64::from(cur) - i64::from(min_x)).abs()
            <= (i64::from(cur) - i64::from(effective_max)).abs()
        {
            min_x
        } else {
            effective_max
        }
    }
}

#[cfg(windows)]
mod win {
    use crate::color::{derive_background_accent, enforce_ui_accent_deadzone, Rgb};
    use std::cell::RefCell;
    use std::mem::size_of;
    use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering as WinOrdering};
    use std::sync::{Mutex, OnceLock};
    use windows::core::{s, w, PCWSTR, PWSTR};
    use windows::UI::ViewManagement::{UIColorType, UISettings};
    use windows::Win32::Foundation::{CloseHandle, BOOL, HMODULE, HWND, LPARAM, RECT, WPARAM};
    use windows::Win32::Graphics::Dwm::{
        DwmGetWindowAttribute, DwmSetWindowAttribute, DWMWA_CLOAKED, DWMWA_EXCLUDED_FROM_PEEK,
        DWMWA_EXTENDED_FRAME_BOUNDS,
    };
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };
    use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryW};
    use windows::Win32::System::Registry::{
        RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY,
        HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_DWORD, REG_SZ, REG_VALUE_TYPE,
    };
    use windows::Win32::System::Threading::{
        GetCurrentProcessId, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
    use windows::Win32::UI::Shell::{
        SHQueryUserNotificationState, QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, FindWindowW, GetAncestor, GetClassNameW, GetDesktopWindow,
        GetForegroundWindow, GetShellWindow, GetTopWindow, GetWindow, GetWindowLongW,
        GetWindowRect, GetWindowThreadProcessId, IsIconic, IsWindow, IsWindowVisible,
        PostMessageW, SetWindowPos, EVENT_OBJECT_CLOAKED, EVENT_OBJECT_FOCUS, EVENT_OBJECT_HIDE,
        EVENT_OBJECT_LOCATIONCHANGE, EVENT_OBJECT_REORDER, EVENT_OBJECT_SHOW,
        EVENT_OBJECT_UNCLOAKED, EVENT_SYSTEM_CAPTUREEND, EVENT_SYSTEM_CAPTURESTART,
        EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_MINIMIZEEND, EVENT_SYSTEM_MINIMIZESTART, GA_ROOT,
        GWL_EXSTYLE, GW_HWNDNEXT, GW_HWNDPREV, GW_OWNER, HWND_TOPMOST, SWP_NOACTIVATE,
        SWP_NOMOVE, SWP_NOOWNERZORDER, SWP_NOSIZE, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS,
        WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT,
    };

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

    /// Returns `true` if Pretty Windows is configured to launch at Windows startup.
    pub fn is_autostart_enabled() -> bool {
        unsafe {
            let mut hkey = HKEY::default();
            if RegOpenKeyExW(HKEY_CURRENT_USER, RUN_SUBKEY, 0, KEY_READ, &mut hkey).0 != 0 {
                return false;
            }
            let mut size = 0u32;
            let res = RegQueryValueExW(hkey, RUN_VALUE_NAME, None, None, None, Some(&mut size));
            let _ = RegCloseKey(hkey);
            res.0 == 0 && size > 0
        }
    }

    /// Enables or disables auto-start in HKCU Run registry key.
    pub fn set_autostart_enabled(enable: bool) -> bool {
        unsafe {
            let mut hkey = HKEY::default();
            if RegOpenKeyExW(HKEY_CURRENT_USER, RUN_SUBKEY, 0, KEY_WRITE, &mut hkey).0 != 0 {
                return false;
            }
            let ok = if enable {
                let Ok(exe_path) = std::env::current_exe() else {
                    let _ = RegCloseKey(hkey);
                    return false;
                };
                let quoted = format!("\"{}\"", exe_path.display());
                let wide: Vec<u16> = quoted.encode_utf16().chain(std::iter::once(0)).collect();
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

    /// Exempts a floating taskbar widget from Windows Aero Peek / Show Desktop fade.
    pub fn exclude_from_peek(hwnd: HWND) {
        let val: u32 = 1;
        unsafe {
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_EXCLUDED_FROM_PEEK,
                (&val as *const u32).cast(),
                size_of::<u32>() as u32,
            );
        }
    }

    /// Returns `true` if `hwnd` belongs to Windows desktop shell or modern tray/start/search flyouts.
    pub fn is_shell_or_flyout_window(hwnd: HWND) -> bool {
        !hwnd.is_invalid() && (is_shell_or_flyout_class(hwnd) || is_shell_flyout_process(hwnd))
    }

    fn is_shell_or_flyout_class(hwnd: HWND) -> bool {
        let mut buf = [0u16; 96];
        let len = unsafe { GetClassNameW(hwnd, &mut buf) };
        if len <= 0 {
            return false;
        }
        let name = String::from_utf16_lossy(&buf[..len as usize]);
        super::is_shell_or_flyout_class_name(&name)
    }

    fn is_shell_flyout_process(hwnd: HWND) -> bool {
        unsafe {
            let mut pid = 0u32;
            let _ = GetWindowThreadProcessId(hwnd, Some(&mut pid));
            if pid == 0 {
                return false;
            }
            let Ok(hproc) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
                return false;
            };
            let mut buf = [0u16; 512];
            let mut len = buf.len() as u32;
            let ok = QueryFullProcessImageNameW(
                hproc,
                PROCESS_NAME_WIN32,
                PWSTR(buf.as_mut_ptr()),
                &mut len,
            )
            .is_ok();
            let _ = CloseHandle(hproc);
            if !ok || len == 0 {
                return false;
            }
            let path = String::from_utf16_lossy(&buf[..len as usize]);
            super::is_shell_flyout_process_name(super::extract_exe_basename(&path))
        }
    }

    type GetWindowBandFn = unsafe extern "system" fn(HWND, *mut u32) -> BOOL;

    fn load_get_window_band() -> Option<GetWindowBandFn> {
        static CACHE: OnceLock<Option<GetWindowBandFn>> = OnceLock::new();
        *CACHE.get_or_init(|| unsafe {
            let hmod = GetModuleHandleW(w!("user32.dll"))
                .ok()
                .or_else(|| LoadLibraryW(w!("user32.dll")).ok())?;
            let proc = GetProcAddress(hmod, s!("GetWindowBand"))?;
            Some(std::mem::transmute::<_, GetWindowBandFn>(proc))
        })
    }

    fn window_band(hwnd: HWND) -> Option<u32> {
        if hwnd.is_invalid() {
            return None;
        }
        let f = load_get_window_band()?;
        let mut band = 0u32;
        unsafe { f(hwnd, &mut band).as_bool().then_some(band) }
    }

    fn is_window_above(upper: HWND, lower: HWND) -> bool {
        if upper.is_invalid() || lower.is_invalid() || upper == lower {
            return false;
        }
        unsafe {
            let mut cur = lower;
            for _ in 0..512 {
                match GetWindow(cur, GW_HWNDPREV) {
                    Ok(prev) if !prev.is_invalid() => {
                        if prev == upper {
                            return true;
                        }
                        cur = prev;
                    }
                    _ => break,
                }
            }
            false
        }
    }

    fn is_taskbar_above_window(bar_hwnd: HWND, taskbar_hwnd: HWND) -> bool {
        is_window_above(taskbar_hwnd, bar_hwnd)
    }

    unsafe fn is_window_above_or_equal_check(target: HWND, taskbar_hwnd: HWND) -> bool {
        !is_taskbar_above_window(target, taskbar_hwnd)
    }

    enum FullscreenActive {
        None,
        GlobalNotification,
        Window(HWND),
    }

    unsafe fn detect_fullscreen_on_monitor(anchor_hwnd: HWND) -> FullscreenActive {
        if let Ok(state) = SHQueryUserNotificationState() {
            if state == QUNS_RUNNING_D3D_FULL_SCREEN || state == QUNS_PRESENTATION_MODE {
                return FullscreenActive::GlobalNotification;
            }
        }

        let fg = GetForegroundWindow();
        if fg.is_invalid() {
            return FullscreenActive::None;
        }
        let root = GetAncestor(fg, GA_ROOT);
        let target = if root.is_invalid() { fg } else { root };

        if target == anchor_hwnd
            || target == GetShellWindow()
            || target == GetDesktopWindow()
            || !IsWindowVisible(target).as_bool()
            || IsIconic(target).as_bool()
        {
            return FullscreenActive::None;
        }

        let mut cloaked: u32 = 0;
        if DwmGetWindowAttribute(
            target,
            DWMWA_CLOAKED,
            (&mut cloaked as *mut u32).cast(),
            size_of::<u32>() as u32,
        )
        .is_ok()
            && cloaked != 0
        {
            return FullscreenActive::None;
        }

        if is_shell_or_flyout_class(target) || is_shell_flyout_process(target) {
            return FullscreenActive::None;
        }

        let ex_style = GetWindowLongW(target, GWL_EXSTYLE) as u32;
        if (ex_style & (WS_EX_TOOLWINDOW.0 | WS_EX_TRANSPARENT.0)) != 0 {
            return FullscreenActive::None;
        }

        if let Ok(owner) = GetWindow(target, GW_OWNER) {
            if !owner.is_invalid()
                && (is_shell_or_flyout_class(owner) || is_shell_flyout_process(owner))
            {
                return FullscreenActive::None;
            }
        }

        let anchor_mon = MonitorFromWindow(anchor_hwnd, MONITOR_DEFAULTTONEAREST);
        let fg_mon = MonitorFromWindow(target, MONITOR_DEFAULTTONEAREST);
        if anchor_mon != fg_mon {
            return FullscreenActive::None;
        }

        let mut mi = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !GetMonitorInfoW(anchor_mon, &mut mi).as_bool() {
            return FullscreenActive::None;
        }

        let mut fg_rect = RECT::default();
        if GetWindowRect(target, &mut fg_rect).is_err() {
            return FullscreenActive::None;
        }
        let covers_monitor = fg_rect.left <= mi.rcMonitor.left
            && fg_rect.top <= mi.rcMonitor.top
            && fg_rect.right >= mi.rcMonitor.right
            && fg_rect.bottom >= mi.rcMonitor.bottom;
        if covers_monitor {
            FullscreenActive::Window(target)
        } else {
            FullscreenActive::None
        }
    }

    /// Returns `true` when the active session is in exclusive D3D fullscreen or presentation mode
    /// (`QUNS_RUNNING_D3D_FULL_SCREEN` / `QUNS_PRESENTATION_MODE`).
    ///
    /// Regular borderless fullscreen windows and desktop video players intentionally return `false`
    /// so custom translucent overlays (such as the hardware OSD capsule backdrop) can remain active
    /// unless occluded by an external topmost window.
    pub fn is_exclusive_fullscreen_or_presentation() -> bool {
        unsafe {
            SHQueryUserNotificationState().is_ok_and(|state| {
                state == QUNS_RUNNING_D3D_FULL_SCREEN || state == QUNS_PRESENTATION_MODE
            })
        }
    }

    /// Detects whether a genuine fullscreen/presentation window is currently occluding `taskbar_hwnd`.
    ///
    /// Desktop surfaces (`Progman`/`WorkerW`), system shell flyouts (Quick Settings, Start Menu,
    /// Search, TextInputHost, `Windows.UI.Core.CoreWindow`), DWM-cloaked/minimized windows, and
    /// windows sitting below `taskbar_hwnd` in Z-order (such as after `Win+D`) do not count.
    pub fn is_fullscreen_occluding_taskbar(taskbar_hwnd: HWND) -> bool {
        unsafe {
            match detect_fullscreen_on_monitor(taskbar_hwnd) {
                FullscreenActive::None => false,
                FullscreenActive::GlobalNotification => true,
                // If `taskbar_hwnd` sits above `target` in Z-order (e.g. `Win+D` or desktop companion),
                // the taskbar is visible and not occluded.
                FullscreenActive::Window(target) => {
                    is_window_above_or_equal_check(target, taskbar_hwnd)
                }
            }
        }
    }

    unsafe fn is_external_topmost_window_intersecting(
        cur: HWND,
        my_pid: u32,
        card_arr: [i32; 4],
    ) -> bool {
        if !IsWindowVisible(cur).as_bool() {
            return false;
        }
        let mut cloaked: u32 = 0;
        let is_cloaked = DwmGetWindowAttribute(
            cur,
            DWMWA_CLOAKED,
            (&mut cloaked as *mut u32).cast(),
            size_of::<u32>() as u32,
        )
        .is_ok()
            && cloaked != 0;
        if is_cloaked {
            return false;
        }
        let mut pid = 0u32;
        let _ = GetWindowThreadProcessId(cur, Some(&mut pid));
        if pid == 0
            || pid == my_pid
            || is_shell_or_flyout_class(cur)
            || is_shell_flyout_process(cur)
        {
            return false;
        }
        let mut r = RECT::default();
        let got_rect = DwmGetWindowAttribute(
            cur,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            (&mut r as *mut RECT).cast(),
            size_of::<RECT>() as u32,
        )
        .is_ok()
            || GetWindowRect(cur, &mut r).is_ok();
        got_rect && super::rects_intersect([r.left, r.top, r.right, r.bottom], card_arr)
    }

    /// Returns `true` when the `band=1` `ShellCardGlassBackdrop` behind the OSD would fail to
    /// composite cleanly above foreground content: either the system is in D3D exclusive fullscreen /
    /// presentation mode, or an external `WS_EX_TOPMOST` window overlaps `card_rect`.
    ///
    /// In those cases the OSD switches to calibrated inline XAML translucent glass (`band=18`)
    /// instead of hiding its backdrop altogether.
    pub fn should_suppress_osd_backdrop(osd_host_hwnd: HWND, card_rect: RECT) -> bool {
        if is_exclusive_fullscreen_or_presentation() {
            return true;
        }
        unsafe {
            let my_pid = GetCurrentProcessId();
            let card_arr = [
                card_rect.left,
                card_rect.top,
                card_rect.right,
                card_rect.bottom,
            ];
            let mut cur = GetTopWindow(None).unwrap_or_default();
            for _ in 0..64 {
                if cur.is_invalid() {
                    break;
                }
                let ex = GetWindowLongW(cur, GWL_EXSTYLE) as u32;
                if (ex & WS_EX_TOPMOST.0) == 0 {
                    break;
                }
                if cur != osd_host_hwnd
                    && is_external_topmost_window_intersecting(cur, my_pid, card_arr)
                {
                    return true;
                }
                cur = GetWindow(cur, GW_HWNDNEXT).unwrap_or_default();
            }
        }
        false
    }

    /// Returns `true` when the behind-ToolTip `ShellCardGlassBackdrop` would be occluded or fail to
    /// composite cleanly: either the session is in D3D exclusive fullscreen / presentation mode, or an
    /// external `WS_EX_TOPMOST` window strictly above `tooltip_host_hwnd` overlaps `card_rect`.
    ///
    /// Windows ordered below `tooltip_host_hwnd` in the topmost band (such as the taskbar overflow
    /// island or taskbar itself) do not occlude the backdrop anchored right behind `tooltip_host_hwnd`.
    pub fn should_suppress_tooltip_backdrop(tooltip_host_hwnd: HWND, card_rect: RECT) -> bool {
        if is_exclusive_fullscreen_or_presentation() {
            return true;
        }
        unsafe {
            let my_pid = GetCurrentProcessId();
            let card_arr = [
                card_rect.left,
                card_rect.top,
                card_rect.right,
                card_rect.bottom,
            ];
            let mut cur = GetTopWindow(None).unwrap_or_default();
            for _ in 0..64 {
                if cur.is_invalid() || cur == tooltip_host_hwnd {
                    break;
                }
                let ex = GetWindowLongW(cur, GWL_EXSTYLE) as u32;
                if (ex & WS_EX_TOPMOST.0) == 0 {
                    break;
                }
                if is_external_topmost_window_intersecting(cur, my_pid, card_arr) {
                    return true;
                }
                cur = GetWindow(cur, GW_HWNDNEXT).unwrap_or_default();
            }
        }
        false
    }

    /// Places `bar_hwnd` immediately above `taskbar_hwnd` without covering higher shell flyouts,
    /// falling back to `HWND_TOPMOST` when needed (and avoiding cross-band thrashing when the
    /// taskbar is temporarily elevated by shell UI such as the Start Menu).
    pub fn ensure_window_above_taskbar(bar_hwnd: HWND, taskbar_hwnd: HWND) {
        unsafe {
            if bar_hwnd.is_invalid()
                || taskbar_hwnd.is_invalid()
                || !IsWindowVisible(bar_hwnd).as_bool()
            {
                return;
            }

            let bar_band = window_band(bar_hwnd);
            let tb_band = window_band(taskbar_hwnd);
            if matches!((bar_band, tb_band), (Some(bb), Some(tb)) if tb > bb) {
                let is_topmost =
                    (GetWindowLongW(bar_hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST.0) != 0;
                if is_topmost && !is_any_backdrop_above(bar_hwnd) {
                    return;
                }
                let _ = SetWindowPos(
                    bar_hwnd,
                    HWND_TOPMOST,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
                );
                return;
            }

            if !is_taskbar_above_window(bar_hwnd, taskbar_hwnd) {
                if !is_any_backdrop_above(bar_hwnd) {
                    return;
                }
                let _ = SetWindowPos(
                    bar_hwnd,
                    HWND_TOPMOST,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
                );
                return;
            }

            if let Ok(above_tb) = GetWindow(taskbar_hwnd, GW_HWNDPREV) {
                if !above_tb.is_invalid() && above_tb != bar_hwnd {
                    let same_band = match (bar_band, window_band(above_tb)) {
                        (Some(bb), Some(ab)) => bb == ab,
                        _ => true,
                    };
                    if same_band {
                        let _ = SetWindowPos(
                            bar_hwnd,
                            above_tb,
                            0,
                            0,
                            0,
                            0,
                            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
                        );
                        if !is_taskbar_above_window(bar_hwnd, taskbar_hwnd) {
                            return;
                        }
                    }
                }
            }

            let _ = SetWindowPos(
                bar_hwnd,
                HWND_TOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
            );
        }
    }

    struct TaskbarOverlayGuard {
        widgets: Vec<HWND>,
        backdrops: Vec<HWND>,
        in_callback: bool,
        global_fg_hook: HWINEVENTHOOK,
        global_min_hook: HWINEVENTHOOK,
        taskbar_capture_hook: HWINEVENTHOOK,
        taskbar_reorder_hook: HWINEVENTHOOK,
        bound_tb_pid: u32,
        bound_tb_tid: u32,
    }

    impl Default for TaskbarOverlayGuard {
        fn default() -> Self {
            Self {
                widgets: Vec::new(),
                backdrops: Vec::new(),
                in_callback: false,
                global_fg_hook: HWINEVENTHOOK::default(),
                global_min_hook: HWINEVENTHOOK::default(),
                taskbar_capture_hook: HWINEVENTHOOK::default(),
                taskbar_reorder_hook: HWINEVENTHOOK::default(),
                bound_tb_pid: 0,
                bound_tb_tid: 0,
            }
        }
    }

    impl TaskbarOverlayGuard {
        fn unhook_taskbar_specific(&mut self) {
            unsafe {
                if !self.taskbar_capture_hook.is_invalid() {
                    let _ = UnhookWinEvent(self.taskbar_capture_hook);
                    self.taskbar_capture_hook = HWINEVENTHOOK::default();
                }
                if !self.taskbar_reorder_hook.is_invalid() {
                    let _ = UnhookWinEvent(self.taskbar_reorder_hook);
                    self.taskbar_reorder_hook = HWINEVENTHOOK::default();
                }
            }
            self.bound_tb_pid = 0;
            self.bound_tb_tid = 0;
        }

        fn unhook_all(&mut self) {
            unsafe {
                if !self.global_fg_hook.is_invalid() {
                    let _ = UnhookWinEvent(self.global_fg_hook);
                    self.global_fg_hook = HWINEVENTHOOK::default();
                }
                if !self.global_min_hook.is_invalid() {
                    let _ = UnhookWinEvent(self.global_min_hook);
                    self.global_min_hook = HWINEVENTHOOK::default();
                }
            }
            self.unhook_taskbar_specific();
        }

        fn rebind_taskbar_hooks(&mut self, pid: u32, tid: u32) {
            if self.bound_tb_pid == pid
                && self.bound_tb_tid == tid
                && !self.taskbar_capture_hook.is_invalid()
                && !self.taskbar_reorder_hook.is_invalid()
            {
                return;
            }
            self.unhook_taskbar_specific();
            if pid == 0 || tid == 0 {
                return;
            }
            let flags = WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS;
            unsafe {
                self.taskbar_capture_hook = SetWinEventHook(
                    EVENT_SYSTEM_CAPTURESTART,
                    EVENT_SYSTEM_CAPTUREEND,
                    HMODULE::default(),
                    Some(taskbar_zorder_win_event_proc),
                    pid,
                    tid,
                    flags,
                );
                self.taskbar_reorder_hook = SetWinEventHook(
                    EVENT_OBJECT_REORDER,
                    EVENT_OBJECT_FOCUS,
                    HMODULE::default(),
                    Some(taskbar_zorder_win_event_proc),
                    pid,
                    tid,
                    flags,
                );
            }
            self.bound_tb_pid = pid;
            self.bound_tb_tid = tid;
        }

        fn ensure_hooks(&mut self) {
            if self.widgets.is_empty() {
                self.unhook_all();
                return;
            }
            let flags = WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS;
            unsafe {
                if self.global_fg_hook.is_invalid() {
                    self.global_fg_hook = SetWinEventHook(
                        EVENT_SYSTEM_FOREGROUND,
                        EVENT_SYSTEM_FOREGROUND,
                        HMODULE::default(),
                        Some(taskbar_zorder_win_event_proc),
                        0,
                        0,
                        flags,
                    );
                }
                if self.global_min_hook.is_invalid() {
                    self.global_min_hook = SetWinEventHook(
                        EVENT_SYSTEM_MINIMIZESTART,
                        EVENT_SYSTEM_MINIMIZEEND,
                        HMODULE::default(),
                        Some(taskbar_zorder_win_event_proc),
                        0,
                        0,
                        flags,
                    );
                }
            }
            if let Ok(tb_hwnd) = unsafe { FindWindowW(w!("Shell_TrayWnd"), None) } {
                if !tb_hwnd.is_invalid() {
                    let mut pid = 0u32;
                    let tid = unsafe { GetWindowThreadProcessId(tb_hwnd, Some(&mut pid)) };
                    self.rebind_taskbar_hooks(pid, tid);
                }
            }
        }
    }

    impl Drop for TaskbarOverlayGuard {
        fn drop(&mut self) {
            self.unhook_all();
        }
    }

    thread_local! {
        static OVERLAY_GUARD: RefCell<TaskbarOverlayGuard> = RefCell::new(TaskbarOverlayGuard::default());
    }

    struct ReentryResetGuard;

    impl Drop for ReentryResetGuard {
        fn drop(&mut self) {
            OVERLAY_GUARD.with(|cell| {
                if let Ok(mut guard) = cell.try_borrow_mut() {
                    guard.in_callback = false;
                }
            });
        }
    }

    unsafe extern "system" fn taskbar_zorder_win_event_proc(
        _hwineventhook: HWINEVENTHOOK,
        _event: u32,
        _hwnd: HWND,
        _idobject: i32,
        _idchild: i32,
        _ideventthread: u32,
        _dwmseventtime: u32,
    ) {
        let Some(widgets) = OVERLAY_GUARD.with(|cell| {
            let Ok(mut guard) = cell.try_borrow_mut() else {
                return None;
            };
            if guard.in_callback || guard.widgets.is_empty() {
                return None;
            }
            guard.in_callback = true;
            Some(guard.widgets.clone())
        }) else {
            return;
        };
        let _reset = ReentryResetGuard;

        let Ok(tb_hwnd) = FindWindowW(w!("Shell_TrayWnd"), None) else {
            return;
        };
        if tb_hwnd.is_invalid() {
            return;
        }

        let tb_band = window_band(tb_hwnd);
        let any_below = widgets
            .iter()
            .copied()
            .any(|w| IsWindowVisible(w).as_bool() && is_taskbar_above_window(w, tb_hwnd));
        if any_below && !is_fullscreen_occluding_taskbar(tb_hwnd) {
            for w in widgets {
                if !IsWindowVisible(w).as_bool() {
                    continue;
                }
                if let (Some(wb), Some(tb)) = (window_band(w), tb_band) {
                    if tb > wb {
                        let is_topmost =
                            (GetWindowLongW(w, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST.0) != 0;
                        if is_topmost && !is_any_backdrop_above(w) {
                            continue;
                        }
                    }
                }
                ensure_window_above_taskbar(w, tb_hwnd);
            }
        }

        let mut cur_pid = 0u32;
        let cur_tid = GetWindowThreadProcessId(tb_hwnd, Some(&mut cur_pid));
        let need_rebind = OVERLAY_GUARD.with(|cell| {
            cell.try_borrow().is_ok_and(|g| {
                !g.widgets.is_empty()
                    && (g.bound_tb_pid != cur_pid || g.bound_tb_tid != cur_tid)
            })
        });
        if need_rebind {
            OVERLAY_GUARD.with(|cell| {
                if let Ok(mut guard) = cell.try_borrow_mut() {
                    guard.rebind_taskbar_hooks(cur_pid, cur_tid);
                }
            });
        }
    }

    fn is_any_backdrop_above(bar_hwnd: HWND) -> bool {
        if bar_hwnd.is_invalid() {
            return false;
        }
        let backdrops = OVERLAY_GUARD.with(|cell| {
            cell.try_borrow()
                .map(|g| g.backdrops.clone())
                .unwrap_or_default()
        });
        backdrops.into_iter().any(|bg| unsafe {
            !bg.is_invalid()
                && IsWindow(bg).as_bool()
                && IsWindowVisible(bg).as_bool()
                && is_window_above(bg, bar_hwnd)
        })
    }

    /// Registers a behind-taskbar backdrop HWND so Z-order coordination keeps it below widgets.
    pub fn register_taskbar_backdrop(hwnd: HWND) {
        if hwnd.is_invalid() {
            return;
        }
        OVERLAY_GUARD.with(|cell| {
            if let Ok(mut guard) = cell.try_borrow_mut() {
                guard
                    .backdrops
                    .retain(|&w| !w.is_invalid() && unsafe { IsWindow(w).as_bool() });
                if !guard.backdrops.contains(&hwnd) {
                    guard.backdrops.push(hwnd);
                }
            }
        });
    }

    /// Unregisters a behind-taskbar backdrop HWND before destruction.
    pub fn unregister_taskbar_backdrop(hwnd: HWND) {
        OVERLAY_GUARD.with(|cell| {
            if let Ok(mut guard) = cell.try_borrow_mut() {
                guard
                    .backdrops
                    .retain(|&w| w != hwnd && !w.is_invalid() && unsafe { IsWindow(w).as_bool() });
            }
        });
    }

    /// Resolves the Z-order anchor (`insert_after`) for a behind-taskbar backdrop window on the
    /// same monitor as `tb_hwnd`, placing it below all visible same-band taskbar widgets and the
    /// taskbar itself even if Windows elevates `tb_hwnd` to a higher band.
    pub fn taskbar_backdrop_anchor(tb_hwnd: HWND, backdrop_hwnd: HWND) -> HWND {
        if tb_hwnd.is_invalid() {
            return tb_hwnd;
        }
        unsafe {
            let tb_mon = MonitorFromWindow(tb_hwnd, MONITOR_DEFAULTTONEAREST);
            let tb_band = window_band(tb_hwnd);
            let bg_band = window_band(backdrop_hwnd);

            let candidates: Vec<HWND> = OVERLAY_GUARD.with(|cell| {
                let Ok(guard) = cell.try_borrow() else {
                    return Vec::new();
                };
                guard
                    .widgets
                    .iter()
                    .copied()
                    .filter(|&w| {
                        !w.is_invalid()
                            && w != backdrop_hwnd
                            && IsWindow(w).as_bool()
                            && IsWindowVisible(w).as_bool()
                            && MonitorFromWindow(w, MONITOR_DEFAULTTONEAREST) == tb_mon
                            && match (bg_band, window_band(w)) {
                                (Some(bg), Some(wb)) => bg == wb,
                                _ => true,
                            }
                    })
                    .collect()
            });

            if candidates.is_empty() {
                return tb_hwnd;
            }

            let mut taskbar_z_rank: Option<usize> = None;
            let mut matched_overlays: Vec<(usize, usize)> = Vec::with_capacity(candidates.len());
            let mut rank = 0usize;
            let mut cur = GetTopWindow(None).unwrap_or_default();
            while !cur.is_invalid() && rank < 1024 {
                if cur == tb_hwnd && taskbar_z_rank.is_none() {
                    taskbar_z_rank = Some(rank);
                }
                if candidates.contains(&cur)
                    && !matched_overlays.iter().any(|&(id, _)| id == cur.0 as usize)
                {
                    matched_overlays.push((cur.0 as usize, rank));
                }
                if taskbar_z_rank.is_some() && matched_overlays.len() == candidates.len() {
                    break;
                }
                rank += 1;
                cur = GetWindow(cur, GW_HWNDNEXT).unwrap_or_default();
            }

            let anchor_id = super::pick_lowest_backdrop_anchor(
                tb_hwnd.0 as usize,
                tb_band,
                bg_band,
                taskbar_z_rank,
                &matched_overlays,
            );
            HWND(anchor_id as *mut core::ffi::c_void)
        }
    }

    /// Registers a taskbar widget HWND with the thread-local event-driven Z-order guard.
    pub fn register_taskbar_overlay(hwnd: HWND) {
        if hwnd.is_invalid() {
            return;
        }
        OVERLAY_GUARD.with(|cell| {
            if let Ok(mut guard) = cell.try_borrow_mut() {
                guard
                    .widgets
                    .retain(|&w| !w.is_invalid() && unsafe { IsWindow(w).as_bool() });
                if !guard.widgets.contains(&hwnd) {
                    guard.widgets.push(hwnd);
                }
                guard.ensure_hooks();
            }
        });
    }

    /// Unregisters a taskbar widget HWND and unhooks global/taskbar listeners when no widgets remain.
    pub fn unregister_taskbar_overlay(hwnd: HWND) {
        OVERLAY_GUARD.with(|cell| {
            if let Ok(mut guard) = cell.try_borrow_mut() {
                guard
                    .widgets
                    .retain(|&w| w != hwnd && !w.is_invalid() && unsafe { IsWindow(w).as_bool() });
                if guard.widgets.is_empty() {
                    guard.unhook_all();
                }
            }
        });
    }

    /// Refreshes thread-scoped WinEvent hooks after Explorer recreates the taskbar (`TaskbarCreated`).
    pub fn refresh_taskbar_overlay_hooks() {
        OVERLAY_GUARD.with(|cell| {
            if let Ok(mut guard) = cell.try_borrow_mut() {
                guard.ensure_hooks();
            }
        });
    }

    /// Collects screen bounding rectangles (with a 16px DWM shadow margin) of visible transient
    /// shell flyouts or tooltips intersecting `capture_rect`, so popup backdrop captures can
    /// cleanly inpaint them out before they fade/dismiss on activation.
    pub fn visible_transient_overlay_rects(capture_rect: RECT) -> Vec<RECT> {
        struct CollectCtx {
            capture: RECT,
            rects: Vec<RECT>,
        }

        unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
            let ctx = &mut *(lparam.0 as *mut CollectCtx);
            if hwnd.is_invalid() || !IsWindowVisible(hwnd).as_bool() {
                return BOOL(1);
            }
            let mut cls_buf = [0u16; 96];
            let len = GetClassNameW(hwnd, &mut cls_buf);
            if len <= 0 {
                return BOOL(1);
            }
            let class_name = String::from_utf16_lossy(&cls_buf[..len as usize]);
            if !crate::system::is_transient_backdrop_occluder_class(&class_name) {
                return BOOL(1);
            }

            let mut cloaked: u32 = 0;
            if DwmGetWindowAttribute(
                hwnd,
                DWMWA_CLOAKED,
                (&mut cloaked as *mut u32).cast(),
                size_of::<u32>() as u32,
            )
            .is_ok()
                && cloaked != 0
            {
                return BOOL(1);
            }

            let mut win_rect = RECT::default();
            if GetWindowRect(hwnd, &mut win_rect).is_err() {
                return BOOL(1);
            }
            let mut ext_rect = RECT::default();
            let bounds = if DwmGetWindowAttribute(
                hwnd,
                DWMWA_EXTENDED_FRAME_BOUNDS,
                (&mut ext_rect as *mut RECT).cast(),
                size_of::<RECT>() as u32,
            )
            .is_ok()
            {
                RECT {
                    left: win_rect.left.min(ext_rect.left),
                    top: win_rect.top.min(ext_rect.top),
                    right: win_rect.right.max(ext_rect.right),
                    bottom: win_rect.bottom.max(ext_rect.bottom),
                }
            } else {
                win_rect
            };

            let w = bounds.right - bounds.left;
            let h = bounds.bottom - bounds.top;
            if w <= 0 || h <= 0 || w > 960 || h > 720 {
                return BOOL(1);
            }

            const SHADOW_PAD_PX: i32 = 16;
            let inter = RECT {
                left: (bounds.left - SHADOW_PAD_PX).max(ctx.capture.left),
                top: (bounds.top - SHADOW_PAD_PX).max(ctx.capture.top),
                right: (bounds.right + SHADOW_PAD_PX).min(ctx.capture.right),
                bottom: (bounds.bottom + SHADOW_PAD_PX).min(ctx.capture.bottom),
            };
            if inter.right > inter.left && inter.bottom > inter.top {
                ctx.rects.push(inter);
            }
            BOOL(1)
        }

        if capture_rect.right <= capture_rect.left || capture_rect.bottom <= capture_rect.top {
            return Vec::new();
        }
        let mut ctx = CollectCtx {
            capture: capture_rect,
            rects: Vec::new(),
        };
        unsafe {
            let _ = EnumWindows(Some(enum_proc), LPARAM((&mut ctx as *mut CollectCtx) as isize));
        }
            ctx.rects
    }

    static OVERFLOW_NOTIFY_HWND: AtomicUsize = AtomicUsize::new(0);
    static OVERFLOW_NOTIFY_MSG: AtomicU32 = AtomicU32::new(0);
    static OVERFLOW_NOTIFY_PENDING: AtomicBool = AtomicBool::new(false);

    /// Clears the coalescing flag so subsequent overflow WinEvents can schedule another host sync.
    pub fn clear_overflow_notify_pending() {
        OVERFLOW_NOTIFY_PENDING.store(false, WinOrdering::Release);
    }

    unsafe extern "system" fn overflow_win_event_proc(
        _hwineventhook: HWINEVENTHOOK,
        _event: u32,
        hwnd: HWND,
        idobject: i32,
        _idchild: i32,
        _ideventthread: u32,
        _dwmseventtime: u32,
    ) {
        if hwnd.is_invalid() || idobject != 0 {
            return;
        }
        let target_raw = OVERFLOW_NOTIFY_HWND.load(WinOrdering::Acquire);
        let notify_msg = OVERFLOW_NOTIFY_MSG.load(WinOrdering::Acquire);
        if target_raw == 0 || notify_msg == 0 {
            return;
        }
        let mut cls_buf = [0u16; 64];
        let len = GetClassNameW(hwnd, &mut cls_buf);
        if len <= 0 {
            return;
        }
        let cls = String::from_utf16_lossy(&cls_buf[..len as usize]);
        if matches!(
            cls.as_str(),
            "TopLevelWindowForOverflowXamlIsland"
                | "NotifyIconOverflowWindow"
                | "XamlExplorerHostIslandWindow"
                | "XamlExplorerHostIslandWindow_WASDK"
                | "Xaml_WindowedPopupClass"
        ) && !OVERFLOW_NOTIFY_PENDING.swap(true, WinOrdering::AcqRel)
        {
            if PostMessageW(
                HWND(target_raw as *mut core::ffi::c_void),
                notify_msg,
                WPARAM(0),
                LPARAM(0),
            )
            .is_err()
            {
                OVERFLOW_NOTIFY_PENDING.store(false, WinOrdering::Release);
            }
        }
    }

    /// Resolves the Z-order anchor (`insert_after`) for the overflow flyout backdrop window.
    /// Same-band overlays insert right after `ov_hwnd`; higher-band shell flyouts sit above
    /// normal topmost windows via `HWND_TOPMOST` while staying underneath the flyout's own band.
    pub fn overflow_backdrop_anchor(ov_hwnd: HWND, backdrop_hwnd: HWND) -> HWND {
        if ov_hwnd.is_invalid() {
            return HWND_TOPMOST;
        }
        match (window_band(ov_hwnd), window_band(backdrop_hwnd)) {
            (Some(ob), Some(bb)) if ob == bb => ov_hwnd,
            _ => HWND_TOPMOST,
        }
    }

    /// Returns `true` when any visible external topmost window in the same ZBID band sits above `hwnd`.
    pub fn is_occluded_by_same_band_topmost(hwnd: HWND) -> bool {
        if hwnd.is_invalid() {
            return false;
        }
        let my_pid = unsafe { GetCurrentProcessId() };
        let my_band = window_band(hwnd);
        let mut cur = hwnd;
        for _ in 0..32 {
            let Ok(prev) = (unsafe { GetWindow(cur, GW_HWNDPREV) }) else {
                break;
            };
            if prev.is_invalid() || prev == cur {
                break;
            }
            cur = prev;
            if !unsafe { IsWindowVisible(cur).as_bool() } {
                continue;
            }
            let ex = unsafe { GetWindowLongW(cur, GWL_EXSTYLE) as u32 };
            if (ex & WS_EX_TOPMOST.0) == 0 {
                continue;
            }
            if window_band(cur) != my_band {
                continue;
            }
            let mut pid = 0u32;
            unsafe {
                let _ = GetWindowThreadProcessId(cur, Some(&mut pid));
            }
            if pid != 0 && pid != my_pid {
                return true;
            }
        }
        false
    }

    /// RAII process-scoped WinEvent hook notifying the tray host immediately when Explorer
    /// tray overflow flyouts or the Alt+Tab switcher (`XamlExplorerHostIslandWindow`) show, move,
    /// cloak, or hide.
    pub struct OverflowEventHook {
        host_hwnd: HWND,
        notify_msg: u32,
        explorer_pid: u32,
        show_hide_hook: HWINEVENTHOOK,
        location_hook: HWINEVENTHOOK,
        cloak_hook: HWINEVENTHOOK,
    }

    impl OverflowEventHook {
        pub fn new(host_hwnd: HWND, notify_msg: u32) -> Self {
            OVERFLOW_NOTIFY_PENDING.store(false, WinOrdering::Release);
            OVERFLOW_NOTIFY_HWND.store(host_hwnd.0 as usize, WinOrdering::Release);
            OVERFLOW_NOTIFY_MSG.store(notify_msg, WinOrdering::Release);
            Self {
                host_hwnd,
                notify_msg,
                explorer_pid: 0,
                show_hide_hook: HWINEVENTHOOK::default(),
                location_hook: HWINEVENTHOOK::default(),
                cloak_hook: HWINEVENTHOOK::default(),
            }
        }

        pub fn refresh_for_explorer_pid(&mut self, pid: u32) {
            OVERFLOW_NOTIFY_HWND.store(self.host_hwnd.0 as usize, WinOrdering::Release);
            OVERFLOW_NOTIFY_MSG.store(self.notify_msg, WinOrdering::Release);
            if self.explorer_pid == pid
                && (!self.show_hide_hook.is_invalid() || pid == 0)
            {
                return;
            }
            self.unhook_all();
            self.explorer_pid = pid;
            if pid == 0 {
                return;
            }
            let flags = WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS;
            unsafe {
                self.show_hide_hook = SetWinEventHook(
                    EVENT_OBJECT_SHOW,
                    EVENT_OBJECT_HIDE,
                    HMODULE::default(),
                    Some(overflow_win_event_proc),
                    pid,
                    0,
                    flags,
                );
                self.location_hook = SetWinEventHook(
                    EVENT_OBJECT_LOCATIONCHANGE,
                    EVENT_OBJECT_LOCATIONCHANGE,
                    HMODULE::default(),
                    Some(overflow_win_event_proc),
                    pid,
                    0,
                    flags,
                );
                self.cloak_hook = SetWinEventHook(
                    EVENT_OBJECT_CLOAKED,
                    EVENT_OBJECT_UNCLOAKED,
                    HMODULE::default(),
                    Some(overflow_win_event_proc),
                    pid,
                    0,
                    flags,
                );
            }
        }

        fn unhook_all(&mut self) {
            unsafe {
                for hook in [
                    &mut self.show_hide_hook,
                    &mut self.location_hook,
                    &mut self.cloak_hook,
                ] {
                    if !hook.is_invalid() {
                        let _ = UnhookWinEvent(*hook);
                        *hook = HWINEVENTHOOK::default();
                    }
                }
            }
            self.explorer_pid = 0;
        }
    }

    impl Drop for OverflowEventHook {
        fn drop(&mut self) {
            self.unhook_all();
            OVERFLOW_NOTIFY_PENDING.store(false, WinOrdering::Release);
            OVERFLOW_NOTIFY_HWND.store(0, WinOrdering::Release);
            OVERFLOW_NOTIFY_MSG.store(0, WinOrdering::Release);
        }
    }

    #[derive(Clone, Copy, Default)]
    struct TaskbarBarSlotEntry {
        hwnd_raw: isize,
        resync_msg: u32,
        span: Option<(i32, i32)>,
    }

    fn taskbar_bar_registry() -> &'static Mutex<[TaskbarBarSlotEntry; 2]> {
        static REGISTRY: OnceLock<Mutex<[TaskbarBarSlotEntry; 2]>> = OnceLock::new();
        REGISTRY.get_or_init(|| Mutex::new([TaskbarBarSlotEntry::default(); 2]))
    }

    /// Registers a taskbar widget window handle and its async resync `WM_APP_*` message ID.
    /// Preserves any bounds span published during window creation before handle registration.
    pub fn register_taskbar_bar(slot: super::TaskbarBarSlot, hwnd: HWND, resync_msg: u32) {
        let mut guard = match taskbar_bar_registry().lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        let entry = &mut guard[slot.index()];
        entry.hwnd_raw = hwnd.0 as isize;
        entry.resync_msg = resync_msg;
    }

    /// Unregisters a taskbar widget handle (only if matching `hwnd`) and notifies its peer
    /// to re-evaluate whether it can return toward its saved position.
    pub fn unregister_taskbar_bar(slot: super::TaskbarBarSlot, hwnd: HWND) {
        let peer_to_notify = {
            let mut guard = match taskbar_bar_registry().lock() {
                Ok(g) => g,
                Err(poisoned) => poisoned.into_inner(),
            };
            let idx = slot.index();
            if guard[idx].hwnd_raw == hwnd.0 as isize {
                guard[idx] = TaskbarBarSlotEntry::default();
            }
            let peer = guard[slot.peer().index()];
            if peer.hwnd_raw != 0 && peer.resync_msg != 0 {
                Some((
                    HWND(peer.hwnd_raw as *mut core::ffi::c_void),
                    peer.resync_msg,
                ))
            } else {
                None
            }
        };
        if let Some((peer_hwnd, msg)) = peer_to_notify {
            unsafe {
                if IsWindow(peer_hwnd).as_bool() {
                    let _ = PostMessageW(peer_hwnd, msg, WPARAM(0), LPARAM(0));
                }
            }
        }
    }

    /// Updates the current horizontal screen span `(left, right)` of `slot` (or `None` when hidden).
    pub fn update_taskbar_bar_bounds(slot: super::TaskbarBarSlot, span: Option<(i32, i32)>) {
        let mut guard = match taskbar_bar_registry().lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        guard[slot.index()].span = span.filter(|&(l, r)| r > l);
    }

    /// Returns the peer bar's active horizontal screen span `(left, right)`, ignoring stale
    /// or hidden windows once the peer handle has been registered.
    pub fn peer_taskbar_bar_span(slot: super::TaskbarBarSlot) -> Option<(i32, i32)> {
        let peer = {
            let guard = match taskbar_bar_registry().lock() {
                Ok(g) => g,
                Err(poisoned) => poisoned.into_inner(),
            };
            guard[slot.peer().index()]
        };
        let span = peer.span?;
        if peer.hwnd_raw != 0 {
            let hwnd = HWND(peer.hwnd_raw as *mut core::ffi::c_void);
            let alive_and_visible =
                unsafe { IsWindow(hwnd).as_bool() && IsWindowVisible(hwnd).as_bool() };
            if !alive_and_visible {
                return None;
            }
        }
        Some(span)
    }

    /// Asynchronously notifies the active peer bar to re-evaluate its position.
    /// Must only be called after user drag commit, position reset, config width change, or close.
    pub fn notify_peer_taskbar_bar_resync(slot: super::TaskbarBarSlot) {
        let peer_to_notify = {
            let guard = match taskbar_bar_registry().lock() {
                Ok(g) => g,
                Err(poisoned) => poisoned.into_inner(),
            };
            let peer = guard[slot.peer().index()];
            if peer.hwnd_raw != 0 && peer.resync_msg != 0 {
                Some((
                    HWND(peer.hwnd_raw as *mut core::ffi::c_void),
                    peer.resync_msg,
                ))
            } else {
                None
            }
        };
        if let Some((peer_hwnd, msg)) = peer_to_notify {
            unsafe {
                if IsWindow(peer_hwnd).as_bool() {
                    let _ = PostMessageW(peer_hwnd, msg, WPARAM(0), LPARAM(0));
                }
            }
        }
    }
}

#[cfg(windows)]
pub use win::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_exe_basename_handles_windows_and_posix_paths() {
        assert_eq!(
            extract_exe_basename(r"C:\Windows\SystemApps\ShellHost.exe"),
            "ShellHost.exe"
        );
        assert_eq!(
            extract_exe_basename("C:/Windows/System32/explorer.exe"),
            "explorer.exe"
        );
        assert_eq!(extract_exe_basename("SearchHost.exe"), "SearchHost.exe");
        assert_eq!(extract_exe_basename(""), "");
    }

    #[test]
    fn shell_and_flyout_class_filter_matches_expected_windows_only() {
        assert!(is_shell_or_flyout_class_name("Progman"));
        assert!(is_shell_or_flyout_class_name("WorkerW"));
        assert!(is_shell_or_flyout_class_name("Shell_TrayWnd"));
        assert!(is_shell_or_flyout_class_name("Shell_SecondaryTrayWnd"));
        assert!(is_shell_or_flyout_class_name("ControlCenterWindow"));
        assert!(is_shell_or_flyout_class_name(
            "TopLevelWindowForOverflowXamlIsland"
        ));
        assert!(is_shell_or_flyout_class_name("XamlExplorerHostIslandWindow"));
        assert!(is_shell_or_flyout_class_name("Windows.UI.Core.CoreWindow"));

        assert!(!is_shell_or_flyout_class_name("Chrome_WidgetWin_1"));
        assert!(!is_shell_or_flyout_class_name("ApplicationFrameWindow"));
        assert!(!is_shell_or_flyout_class_name("CabinetWClass"));
    }

    #[test]
    fn shell_flyout_process_filter_is_case_insensitive_and_preserves_explorer() {
        assert!(is_shell_flyout_process_name("ShellHost.exe"));
        assert!(is_shell_flyout_process_name("SHELLEXPERIENCEHOST.EXE"));
        assert!(is_shell_flyout_process_name("StartMenuExperienceHost.exe"));
        assert!(is_shell_flyout_process_name("SearchHost.exe"));
        assert!(is_shell_flyout_process_name("TextInputHost.exe"));
        assert!(is_shell_flyout_process_name("Widgets.exe"));

        assert!(!is_shell_flyout_process_name("explorer.exe"));
        assert!(!is_shell_flyout_process_name("chrome.exe"));
    }

    #[test]
    fn backdrop_anchor_stays_below_widgets_when_taskbar_elevates_band() {
        const TB: usize = 0x1001;
        const MEDIA: usize = 0x2001;
        const MONITOR: usize = 0x2002;
        const SEC_TB: usize = 0x1002;

        // Normal same-band state: MediaBar (5) & MonitorBar (6) above Shell_TrayWnd (7) -> anchor = Shell_TrayWnd.
        assert_eq!(
            pick_lowest_backdrop_anchor(TB, Some(1), Some(1), Some(7), &[(MEDIA, 5), (MONITOR, 6)]),
            TB
        );

        // Start Menu open: Shell_TrayWnd elevated to band 16 while widgets & backdrop remain in band 1
        // -> must anchor below the lowest visible widget (MonitorBar), NEVER Shell_TrayWnd.
        assert_eq!(
            pick_lowest_backdrop_anchor(TB, Some(16), Some(1), Some(1), &[(MEDIA, 5), (MONITOR, 6)]),
            MONITOR
        );

        // Transient same-band reorder where Shell_TrayWnd (4) sits above widgets (5, 6)
        // -> still picks lowest widget (MonitorBar) so backdrop stays beneath all widgets.
        assert_eq!(
            pick_lowest_backdrop_anchor(TB, Some(1), Some(1), Some(4), &[(MEDIA, 5), (MONITOR, 6)]),
            MONITOR
        );

        // Secondary monitor with no widgets falls back to its own Shell_SecondaryTrayWnd.
        assert_eq!(
            pick_lowest_backdrop_anchor(SEC_TB, Some(16), Some(1), Some(2), &[]),
            SEC_TB
        );
    }

    #[test]
    fn transient_backdrop_occluder_class_filter_matches_expected_windows() {
        for cls in [
            "TopLevelWindowForOverflowXamlIsland",
            "NotifyIconOverflowWindow",
            "XamlExplorerHostIslandWindow",
            "ControlCenterWindow",
            "tooltips_class32",
            "Xaml_WindowedPopupClass",
            "TaskListThumbnailWnd",
        ] {
            assert!(
                is_transient_backdrop_occluder_class(cls),
                "expected transient occluder match for {cls}"
            );
        }

        for keep in [
            "Progman",
            "WorkerW",
            "Shell_TrayWnd",
            "Chrome_WidgetWin_1",
            "CabinetWClass",
            "PrettyWindowsTrayMenuWnd",
        ] {
            assert!(
                !is_transient_backdrop_occluder_class(keep),
                "should NOT mark {keep} as transient occluder"
            );
        }
    }

    #[test]
    fn rects_intersect_checks_positive_overlap_strictly() {
        let osd = [875, 971, 1044, 1032];
        assert!(rects_intersect(osd, [800, 950, 920, 1000]));
        assert!(rects_intersect(osd, [0, 0, 1920, 1080]));
        // Edge-touching only must not count as overlapping
        assert!(!rects_intersect(osd, [0, 0, 875, 1080]));
        assert!(!rects_intersect(osd, [1044, 971, 1200, 1032]));
        assert!(!rects_intersect(osd, [875, 1032, 1044, 1080]));
        // Degenerate / empty rect must not intersect
        assert!(!rects_intersect(osd, [900, 980, 900, 1020]));
    }

    #[test]
    fn resolve_bar_x_returns_clamped_desired_without_valid_peer() {
        assert_eq!(
            resolve_taskbar_bar_x(100, 256, 4, 1200, None, None, 2, 8, BarCollisionMode::Passive),
            100
        );
        assert_eq!(
            resolve_taskbar_bar_x(-50, 256, 4, 1200, None, Some((500, 500)), 2, 8, BarCollisionMode::Passive),
            4
        );
        assert_eq!(
            resolve_taskbar_bar_x(1500, 256, 4, 1200, None, Some((700, 600)), 2, 8, BarCollisionMode::Drag),
            1200
        );
    }

    #[test]
    fn resolve_bar_x_passive_yields_to_peer_without_moving_peer() {
        // Right-anchored MonitorBar collides on startup/enable with MediaBar parked at right edge:
        // right side does not fit, so MonitorBar yields flush to the left of MediaBar.
        assert_eq!(
            resolve_taskbar_bar_x(
                1516,
                200,
                4,
                1516,
                None,
                Some((1400, 1656)),
                2,
                8,
                BarCollisionMode::Passive,
            ),
            1198
        );

        // MediaBar Reset Position when MonitorBar occupies the left anchor:
        // left side does not fit (< min_x), so MediaBar yields flush to the right of MonitorBar.
        assert_eq!(
            resolve_taskbar_bar_x(
                164,
                256,
                4,
                1460,
                Some(600),
                Some((150, 350)),
                2,
                8,
                BarCollisionMode::Passive,
            ),
            352
        );

        // When both sides fit, Passive mode picks the nearer side cleanly.
        assert_eq!(
            resolve_taskbar_bar_x(
                410,
                256,
                4,
                1200,
                Some(200),
                Some((500, 700)),
                2,
                8,
                BarCollisionMode::Passive,
            ),
            242
        );
        assert_eq!(
            resolve_taskbar_bar_x(
                520,
                256,
                4,
                1200,
                Some(800),
                Some((500, 700)),
                2,
                8,
                BarCollisionMode::Passive,
            ),
            702
        );
    }

    #[test]
    fn resolve_bar_x_passive_narrow_taskbar_minimizes_overlap() {
        // Peer [180, 420], range [4, 400], bar_w = 256: neither side clears peer.
        // Overlap at min_x = 4 ([4, 260]) is 80px; overlap at max_x = 400 ([400, 656]) is 20px.
        assert_eq!(
            resolve_taskbar_bar_x(
                120,
                256,
                4,
                400,
                None,
                Some((180, 420)),
                2,
                8,
                BarCollisionMode::Passive,
            ),
            400
        );

        // Peer [200, 440], range [4, 400], bar_w = 256:
        // Overlap at min_x = 4 ([4, 260]) is 60px; overlap at max_x = 400 ([400, 656]) is 40px -> 400.
        // Symmetric left-win case: Peer [340, 620], range [4, 400], bar_w = 256:
        // Overlap at min_x = 4 ([4, 260]) is 0 -> already clear, or Peer [240, 620]:
        // min_x overlap = 20px vs max_x overlap = 220px -> picks min_x = 4.
        assert_eq!(
            resolve_taskbar_bar_x(
                180,
                256,
                4,
                400,
                None,
                Some((240, 620)),
                2,
                8,
                BarCollisionMode::Passive,
            ),
            4
        );
    }

    #[test]
    fn resolve_bar_x_drag_stops_flush_and_swaps_with_symmetric_hysteresis() {
        let peer = Some((500, 700)); // peer_mid = 600, bar_w = 256 => mid at desired_x = 472
        // Left candidate = 500 - 2 - 256 = 242, right candidate = 700 + 2 = 702.

        // Dragging from left to right: holds flush at 242 until desired center > 600 + 8.
        assert_eq!(
            resolve_taskbar_bar_x(260, 256, 4, 1200, Some(200), peer, 2, 8, BarCollisionMode::Drag),
            242
        );
        assert_eq!(
            resolve_taskbar_bar_x(472, 256, 4, 1200, Some(242), peer, 2, 8, BarCollisionMode::Drag),
            242
        );
        assert_eq!(
            resolve_taskbar_bar_x(480, 256, 4, 1200, Some(242), peer, 2, 8, BarCollisionMode::Drag),
            242
        );
        assert_eq!(
            resolve_taskbar_bar_x(481, 256, 4, 1200, Some(242), peer, 2, 8, BarCollisionMode::Drag),
            702
        );

        // Dragging back from right to left: holds flush at 702 until desired center < 600 - 8.
        assert_eq!(
            resolve_taskbar_bar_x(472, 256, 4, 1200, Some(702), peer, 2, 8, BarCollisionMode::Drag),
            702
        );
        assert_eq!(
            resolve_taskbar_bar_x(464, 256, 4, 1200, Some(702), peer, 2, 8, BarCollisionMode::Drag),
            702
        );
        assert_eq!(
            resolve_taskbar_bar_x(463, 256, 4, 1200, Some(702), peer, 2, 8, BarCollisionMode::Drag),
            242
        );
    }

    #[test]
    fn resolve_bar_x_drag_stays_flush_when_opposite_side_cannot_fit() {
        // Peer parked on far right [1200, 1400], dragged bar max_x = 1184:
        // right_candidate = 1402 > max_x, so dragging past peer midpoint stays flush at 942.
        assert_eq!(
            resolve_taskbar_bar_x(
                1184,
                256,
                4,
                1184,
                Some(942),
                Some((1200, 1400)),
                2,
                8,
                BarCollisionMode::Drag,
            ),
            942
        );
    }
}
