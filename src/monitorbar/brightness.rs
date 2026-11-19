//! Async multi-monitor brightness enumeration and hardware/software control for MonitorBar.

use std::collections::{HashMap, HashSet};
#[cfg(windows)]
use std::mem::size_of;
#[cfg(windows)]
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(windows)]
use std::sync::mpsc::{self, Receiver, Sender};
#[cfg(windows)]
use std::sync::{Arc, Mutex, OnceLock};
#[cfg(windows)]
use std::thread::{self, JoinHandle};

#[cfg(windows)]
use windows::core::{w, PCWSTR};
#[cfg(windows)]
use windows::Win32::Devices::Display::{
    DestroyPhysicalMonitors, DisplayConfigGetDeviceInfo, GetDisplayConfigBufferSizes,
    GetMonitorBrightness, GetNumberOfPhysicalMonitorsFromHMONITOR,
    GetPhysicalMonitorsFromHMONITOR, GetVCPFeatureAndVCPFeatureReply, QueryDisplayConfig,
    SetMonitorBrightness, SetVCPFeature, DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
    DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME, DISPLAYCONFIG_MODE_INFO,
    DISPLAYCONFIG_OUTPUT_TECHNOLOGY_DISPLAYPORT_EMBEDDED,
    DISPLAYCONFIG_OUTPUT_TECHNOLOGY_INTERNAL, DISPLAYCONFIG_OUTPUT_TECHNOLOGY_UDI_EMBEDDED,
    DISPLAYCONFIG_PATH_INFO, DISPLAYCONFIG_SOURCE_DEVICE_NAME, DISPLAYCONFIG_TARGET_DEVICE_NAME,
    DISPLAY_BRIGHTNESS, IOCTL_VIDEO_QUERY_DISPLAY_BRIGHTNESS, IOCTL_VIDEO_SET_DISPLAY_BRIGHTNESS,
    PHYSICAL_MONITOR, QDC_ONLY_ACTIVE_PATHS,
};
#[cfg(windows)]
use windows::Win32::Foundation::{
    CloseHandle, BOOL, GENERIC_READ, GENERIC_WRITE, HWND, LPARAM, RECT, WPARAM,
};
#[cfg(windows)]
use windows::Win32::Graphics::Gdi::{
    CreateDCW, DeleteDC, EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFOEXW,
};
#[cfg(windows)]
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
#[cfg(windows)]
use windows::Win32::System::IO::DeviceIoControl;
#[cfg(windows)]
use windows::Win32::UI::ColorSystem::{GetDeviceGammaRamp, SetDeviceGammaRamp};
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, MONITORINFOF_PRIMARY};

/// Minimum perceptual luminance factor for software brightness mode (0% slider -> 20% minimum).
pub const SOFTWARE_BRIGHTNESS_FLOOR: f32 = 0.20;

/// Fallback minimum luminance factor when Windows ICM enforces the default 50% gamma ramp bound.
pub const SOFTWARE_ICM_FALLBACK_FLOOR: f32 = 0.50;

/// VCP code for display Luminance per MCCS specification.
#[cfg(windows)]
const VCP_LUMINANCE: u8 = 0x10;

/// Active brightness control mechanism for a display.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(windows), allow(dead_code))]
pub enum BrightnessMode {
    /// Hardware backlight / DDC-CI brightness control.
    Hardware,
    /// Software gamma-ramp brightness fallback.
    Software,
}

/// Snapshot metadata and current brightness level (`0..=100`) for one display output.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(not(windows), allow(dead_code))]
pub struct DisplayBrightnessItem {
    pub id: String,
    pub name: String,
    pub brightness: u8,
    pub mode: BrightnessMode,
    pub is_primary: bool,
}

/// Snapshot of all active displays consumed by MonitorBar's Brightness submenu.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(not(windows), allow(dead_code))]
pub struct BrightnessSnapshot {
    pub busy: bool,
    pub displays: Vec<DisplayBrightnessItem>,
}

/// Formats brightness percentage label displayed on the slider header row.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn format_brightness_pct(pct: u8) -> String {
    format!("{}%", pct.min(100))
}

/// Normalizes a hardware brightness value in `[min, max]` to `0..=100` percentage.
pub fn normalize_hardware_to_pct(min: u32, cur: u32, max: u32) -> u8 {
    if max <= min {
        return 100;
    }
    let clamped = cur.clamp(min, max);
    let ratio = (clamped - min) as f32 / (max - min) as f32;
    (ratio * 100.0).round().clamp(0.0, 100.0) as u8
}

/// Maps a `0..=100` percentage back into hardware units `[min, max]`.
pub fn pct_to_hardware_value(min: u32, max: u32, pct: u8) -> u32 {
    if max <= min {
        return min;
    }
    let t = (pct.min(100) as f32) / 100.0;
    let val = (min as f32 + (max - min) as f32 * t).round();
    (val as u32).clamp(min, max)
}

/// Returns the standard linear identity 16-bit RGB gamma ramp.
pub fn identity_gamma_ramp() -> [[u16; 256]; 3] {
    let mut ramp = [[0u16; 256]; 3];
    for ch in &mut ramp {
        for (i, slot) in ch.iter_mut().enumerate() {
            *slot = (i * 257) as u16;
        }
    }
    ramp
}

/// Computes a scaled gamma ramp preserving `original` verbatim at 100% and clamping 0% to `min_floor`.
pub fn build_software_gamma_ramp(
    original: &[[u16; 256]; 3],
    pct: u8,
    min_floor: f32,
) -> [[u16; 256]; 3] {
    let clamped = pct.min(100);
    if clamped == 100 {
        return *original;
    }
    let floor = min_floor.clamp(0.05, 1.0);
    let scale = floor + (1.0 - floor) * (f32::from(clamped) / 100.0);
    let mut scaled = *original;
    for ch in &mut scaled {
        for slot in ch.iter_mut() {
            *slot = (f32::from(*slot) * scale).round().clamp(0.0, 65535.0) as u16;
        }
    }
    scaled
}

/// Returns `true` when the reported monitor name is empty or a generic Windows placeholder.
pub fn is_generic_monitor_name(name: &str) -> bool {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return true;
    }
    let lower = trimmed.to_ascii_lowercase();
    lower == "generic pnp monitor"
        || lower == "generic non-pnp monitor"
        || trimmed == "通用即插即用监视器"
        || trimmed == "通用非即插即用监视器"
}

/// Ensures every display has a clear, user-readable unique label (`显示器 N` fallback and `(1)` suffixes on duplicates).
pub fn disambiguate_display_names(items: &mut [DisplayBrightnessItem]) {
    for (idx, item) in items.iter_mut().enumerate() {
        let trimmed = item.name.trim();
        if is_generic_monitor_name(trimmed) {
            item.name = format!("显示器 {}", idx + 1);
        } else {
            item.name = trimmed.to_string();
        }
    }

    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for item in items.iter() {
        *counts.entry(item.name.clone()).or_insert(0) += 1;
    }

    let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for item in items.iter_mut() {
        if counts.get(&item.name).copied().unwrap_or(0) > 1 {
            let ordinal = seen.entry(item.name.clone()).or_insert(0);
            *ordinal += 1;
            item.name = format!("{} ({})", item.name, *ordinal);
        }
    }
}

/// Process-lifetime store of original and current software gamma state across MonitorBar restarts.
#[derive(Default)]
pub struct SoftwareGammaStore {
    original_ramps: HashMap<String, [[u16; 256]; 3]>,
    software_pcts: HashMap<String, u8>,
    dirty_software_ids: HashSet<String>,
}

impl SoftwareGammaStore {
    /// Registers a software-controlled display if unseen and returns its remembered percentage.
    /// Does not overwrite the original ramp on subsequent opens in the same process.
    pub fn ensure_registered(&mut self, id: &str, read_orig: impl FnOnce() -> [[u16; 256]; 3]) -> u8 {
        if !self.original_ramps.contains_key(id) {
            self.original_ramps.insert(id.to_string(), read_orig());
        }
        *self.software_pcts.entry(id.to_string()).or_insert(100)
    }

    /// Returns the initially captured original gamma ramp, registering it first if needed.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn original_or_register(
        &mut self,
        id: &str,
        read_orig: impl FnOnce() -> [[u16; 256]; 3],
    ) -> [[u16; 256]; 3] {
        if let Some(existing) = self.original_ramps.get(id) {
            return *existing;
        }
        let orig = read_orig();
        self.original_ramps.insert(id.to_string(), orig);
        orig
    }

    /// Updates tracked software percentage and dirty state (`< 100` marks display for exit restore).
    pub fn record_applied(&mut self, id: &str, pct: u8) {
        let clamped = pct.min(100);
        self.software_pcts.insert(id.to_string(), clamped);
        if clamped < 100 {
            self.dirty_software_ids.insert(id.to_string());
        } else {
            self.dirty_software_ids.remove(id);
        }
    }

    /// Drains all displays with non-100% software brightness so the host can restore them on exit.
    pub fn drain_dirty_for_restore(&mut self) -> Vec<(String, [[u16; 256]; 3])> {
        let mut out = Vec::new();
        for id in self.dirty_software_ids.drain() {
            self.software_pcts.insert(id.clone(), 100);
            if let Some(orig) = self.original_ramps.get(&id) {
                out.push((id, *orig));
            }
        }
        out
    }
}

#[cfg(windows)]
#[derive(Clone, Debug)]
enum BrightnessCmd {
    Refresh,
    Set { id: String, pct: u8 },
    Shutdown,
}

/// Thread-safe handle to the background monitor brightness worker.
#[cfg(windows)]
#[derive(Clone)]
pub struct BrightnessHandle {
    tx: Sender<BrightnessCmd>,
    snapshot: Arc<Mutex<BrightnessSnapshot>>,
    write_seq: Arc<AtomicU64>,
    join: Arc<Mutex<Option<JoinHandle<()>>>>,
}

#[cfg(windows)]
impl BrightnessHandle {
    /// Spawns the background brightness worker and performs an initial non-blocking display probe.
    pub fn start(notify_hwnd: HWND, notify_msg: u32) -> Self {
        let (tx, rx) = mpsc::channel();
        let snapshot = Arc::new(Mutex::new(BrightnessSnapshot {
            busy: true,
            displays: Vec::new(),
        }));
        let write_seq = Arc::new(AtomicU64::new(0));
        let snap_worker = Arc::clone(&snapshot);
        let seq_worker = Arc::clone(&write_seq);
        let hwnd_raw = notify_hwnd.0 as usize;

        let handle = thread::spawn(move || {
            brightness_worker_loop(hwnd_raw, notify_msg, rx, snap_worker, seq_worker);
        });

        let this = Self {
            tx,
            snapshot,
            write_seq,
            join: Arc::new(Mutex::new(Some(handle))),
        };
        this.request_refresh();
        this
    }

    /// Returns the latest cached multi-display brightness snapshot.
    pub fn snapshot(&self) -> BrightnessSnapshot {
        self.snapshot
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default()
    }

    /// Queues an asynchronous display enumeration and brightness query.
    pub fn request_refresh(&self) {
        let _ = self.tx.send(BrightnessCmd::Refresh);
    }

    /// Optimistically updates the target display percentage and queues an async write.
    pub fn set_brightness(&self, id: &str, pct: u8) {
        let clamped = pct.min(100);
        self.write_seq.fetch_add(1, Ordering::AcqRel);
        if let Ok(mut guard) = self.snapshot.lock() {
            if let Some(item) = guard.displays.iter_mut().find(|d| d.id == id) {
                item.brightness = clamped;
            }
        }
        let _ = self.tx.send(BrightnessCmd::Set {
            id: id.to_string(),
            pct: clamped,
        });
    }

    /// Stops the worker thread cleanly while preserving any active software brightness on screen.
    pub fn stop(&self) {
        let _ = self.tx.send(BrightnessCmd::Shutdown);
        if let Ok(mut guard) = self.join.lock() {
            if let Some(h) = guard.take() {
                let _ = h.join();
            }
        }
    }
}

#[cfg(windows)]
fn software_gamma_store() -> &'static Mutex<SoftwareGammaStore> {
    static STORE: OnceLock<Mutex<SoftwareGammaStore>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(SoftwareGammaStore::default()))
}

/// Restores original display colors for all software-adjusted screens when exiting the application.
#[cfg(windows)]
pub fn restore_software_gamma_on_exit() {
    let pending = if let Ok(mut store) = software_gamma_store().lock() {
        store.drain_dirty_for_restore()
    } else {
        Vec::new()
    };
    for (id, orig) in pending {
        let _ = apply_gdi_gamma_ramp(&id, &orig);
    }
}

#[cfg(windows)]
impl Drop for BrightnessHandle {
    fn drop(&mut self) {
        if Arc::strong_count(&self.snapshot) <= 2 {
            self.stop();
        }
    }
}

#[cfg(windows)]
#[derive(Clone, Copy, Debug)]
enum DisplayBackend {
    Dxva2HighLevel { min: u32, max: u32 },
    Dxva2Vcp { max: u32 },
    InternalLcd,
    SoftwareGamma,
}

#[cfg(windows)]
#[derive(Default)]
struct WorkerState {
    backends: HashMap<String, DisplayBackend>,
}

#[cfg(windows)]
struct DisplayConfigMeta {
    friendly_name: String,
    is_internal: bool,
}

#[cfg(windows)]
struct EnumMonitorInfo {
    hmonitor: HMONITOR,
    device_name: String,
    is_primary: bool,
    left: i32,
    top: i32,
}

#[cfg(windows)]
fn coalesce_brightness_cmd(
    cmd: BrightnessCmd,
    need_refresh: &mut bool,
    pending_sets: &mut Vec<(String, u8)>,
    shutdown: &mut bool,
) {
    match cmd {
        BrightnessCmd::Refresh => *need_refresh = true,
        BrightnessCmd::Set { id, pct } => {
            if let Some(existing) = pending_sets.iter_mut().find(|(eid, _)| eid == &id) {
                existing.1 = pct;
            } else {
                pending_sets.push((id, pct));
            }
        }
        BrightnessCmd::Shutdown => *shutdown = true,
    }
}

#[cfg(windows)]
fn post_notify(hwnd_raw: usize, notify_msg: u32) {
    unsafe {
        let hwnd = HWND(hwnd_raw as *mut _);
        let _ = PostMessageW(hwnd, notify_msg, WPARAM(0), LPARAM(0));
    }
}

#[cfg(windows)]
fn commit_refreshed_displays(
    snapshot: &Arc<Mutex<BrightnessSnapshot>>,
    mut merged: Vec<DisplayBrightnessItem>,
    seq_before: u64,
    write_seq: &AtomicU64,
) {
    if let Ok(mut guard) = snapshot.lock() {
        let written_concurrently = write_seq.load(Ordering::Acquire) != seq_before;
        if written_concurrently {
            for item in &mut merged {
                if let Some(prev) = guard.displays.iter().find(|p| p.id == item.id) {
                    item.brightness = prev.brightness;
                }
            }
        }
        guard.busy = false;
        guard.displays = merged;
    }
}

#[cfg(windows)]
fn brightness_worker_loop(
    hwnd_raw: usize,
    notify_msg: u32,
    rx: Receiver<BrightnessCmd>,
    snapshot: Arc<Mutex<BrightnessSnapshot>>,
    write_seq: Arc<AtomicU64>,
) {
    let mut state = WorkerState::default();

    while let Ok(first) = rx.recv() {
        let mut need_refresh = false;
        let mut pending_sets: Vec<(String, u8)> = Vec::new();
        let mut shutdown = false;

        coalesce_brightness_cmd(first, &mut need_refresh, &mut pending_sets, &mut shutdown);
        while !shutdown {
            match rx.try_recv() {
                Ok(next) => coalesce_brightness_cmd(
                    next,
                    &mut need_refresh,
                    &mut pending_sets,
                    &mut shutdown,
                ),
                Err(_) => break,
            }
        }

        if shutdown {
            break;
        }

        if need_refresh && pending_sets.is_empty() {
            let seq_before = write_seq.load(Ordering::Acquire);
            if let Ok(mut guard) = snapshot.lock() {
                guard.busy = true;
            }
            let displays = refresh_displays(&mut state);
            commit_refreshed_displays(&snapshot, displays, seq_before, &write_seq);
            post_notify(hwnd_raw, notify_msg);
            continue;
        }

        for (id, pct) in pending_sets {
            let clamped = pct.min(100);
            apply_display_brightness(&id, clamped, &mut state);
        }

        if need_refresh {
            let seq_before = write_seq.load(Ordering::Acquire);
            let displays = refresh_displays(&mut state);
            commit_refreshed_displays(&snapshot, displays, seq_before, &write_seq);
            post_notify(hwnd_raw, notify_msg);
        }
    }
}

#[cfg(windows)]
fn utf16_z_to_string(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end]).trim().to_string()
}

#[cfg(windows)]
fn query_display_config_metadata() -> HashMap<String, DisplayConfigMeta> {
    let mut map = HashMap::new();
    unsafe {
        let mut num_paths = 0u32;
        let mut num_modes = 0u32;
        if GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut num_paths, &mut num_modes).0 != 0
            || num_paths == 0
        {
            return map;
        }
        let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); num_paths as usize];
        let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); num_modes as usize];
        if QueryDisplayConfig(
            QDC_ONLY_ACTIVE_PATHS,
            &mut num_paths,
            paths.as_mut_ptr(),
            &mut num_modes,
            modes.as_mut_ptr(),
            None,
        )
        .0 != 0
        {
            return map;
        }

        for path in paths.into_iter().take(num_paths as usize) {
            let mut src = DISPLAYCONFIG_SOURCE_DEVICE_NAME::default();
            src.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME;
            src.header.size = size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32;
            src.header.adapterId = path.sourceInfo.adapterId;
            src.header.id = path.sourceInfo.id;
            if DisplayConfigGetDeviceInfo(&mut src.header) != 0 {
                continue;
            }
            let gdi_name = utf16_z_to_string(&src.viewGdiDeviceName);
            if gdi_name.is_empty() {
                continue;
            }

            let mut tgt = DISPLAYCONFIG_TARGET_DEVICE_NAME::default();
            tgt.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME;
            tgt.header.size = size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>() as u32;
            tgt.header.adapterId = path.targetInfo.adapterId;
            tgt.header.id = path.targetInfo.id;
            let friendly = if DisplayConfigGetDeviceInfo(&mut tgt.header) == 0 {
                utf16_z_to_string(&tgt.monitorFriendlyDeviceName)
            } else {
                String::new()
            };

            let tech = path.targetInfo.outputTechnology;
            let is_internal = tech == DISPLAYCONFIG_OUTPUT_TECHNOLOGY_INTERNAL
                || tech == DISPLAYCONFIG_OUTPUT_TECHNOLOGY_DISPLAYPORT_EMBEDDED
                || tech == DISPLAYCONFIG_OUTPUT_TECHNOLOGY_UDI_EMBEDDED;

            map.entry(gdi_name).or_insert(DisplayConfigMeta {
                friendly_name: friendly,
                is_internal,
            });
        }
    }
    map
}

#[cfg(windows)]
unsafe extern "system" fn enum_monitor_cb(
    hmonitor: HMONITOR,
    _hdc: HDC,
    _lprc: *mut RECT,
    lparam: LPARAM,
) -> BOOL {
    let list = &mut *(lparam.0 as *mut Vec<EnumMonitorInfo>);
    let mut mi = MONITORINFOEXW::default();
    mi.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
    if GetMonitorInfoW(hmonitor, &mut mi.monitorInfo as *mut _).as_bool() {
        let device_name = utf16_z_to_string(&mi.szDevice);
        let is_primary = (mi.monitorInfo.dwFlags & MONITORINFOF_PRIMARY) != 0;
        list.push(EnumMonitorInfo {
            hmonitor,
            device_name,
            is_primary,
            left: mi.monitorInfo.rcMonitor.left,
            top: mi.monitorInfo.rcMonitor.top,
        });
    }
    BOOL(1)
}

#[cfg(windows)]
fn enumerate_gdi_monitors() -> Vec<EnumMonitorInfo> {
    let mut list: Vec<EnumMonitorInfo> = Vec::new();
    unsafe {
        let ptr = (&mut list as *mut Vec<EnumMonitorInfo>) as isize;
        let _ = EnumDisplayMonitors(None, None, Some(enum_monitor_cb), LPARAM(ptr));
    }
    list.sort_by(|a, b| {
        (!a.is_primary)
            .cmp(&(!b.is_primary))
            .then_with(|| a.left.cmp(&b.left))
            .then_with(|| a.top.cmp(&b.top))
            .then_with(|| a.device_name.cmp(&b.device_name))
    });
    list
}

#[cfg(windows)]
fn open_physical_monitors(hmonitor: HMONITOR) -> Vec<PHYSICAL_MONITOR> {
    unsafe {
        let mut count = 0u32;
        if GetNumberOfPhysicalMonitorsFromHMONITOR(hmonitor, &mut count).is_err() || count == 0 {
            return Vec::new();
        }
        let mut phys = vec![PHYSICAL_MONITOR::default(); count as usize];
        if GetPhysicalMonitorsFromHMONITOR(hmonitor, &mut phys).is_err() {
            return Vec::new();
        }
        phys
    }
}

#[cfg(windows)]
fn close_physical_monitors(phys: &[PHYSICAL_MONITOR]) {
    if !phys.is_empty() {
        unsafe {
            let _ = DestroyPhysicalMonitors(phys);
        }
    }
}

#[cfg(windows)]
fn query_internal_lcd_brightness() -> Option<u8> {
    unsafe {
        let handle = CreateFileW(
            w!(r"\\.\LCD"),
            GENERIC_READ.0 | GENERIC_WRITE.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            None,
        )
        .ok()?;
        let mut out = DISPLAY_BRIGHTNESS::default();
        let mut returned = 0u32;
        let ok = DeviceIoControl(
            handle,
            IOCTL_VIDEO_QUERY_DISPLAY_BRIGHTNESS,
            None,
            0,
            Some((&mut out as *mut DISPLAY_BRIGHTNESS).cast()),
            size_of::<DISPLAY_BRIGHTNESS>() as u32,
            Some(&mut returned),
            None,
        )
        .is_ok();
        let _ = CloseHandle(handle);
        if !ok {
            return None;
        }
        // Policy 1 = AC, 2 = DC; fallback to AC if policy is combined.
        let level = if out.ucDisplayPolicy == 2 {
            out.ucDCBrightness
        } else {
            out.ucACBrightness
        };
        Some(level.min(100))
    }
}

#[cfg(windows)]
fn set_internal_lcd_brightness(pct: u8) -> bool {
    unsafe {
        let Ok(handle) = CreateFileW(
            w!(r"\\.\LCD"),
            GENERIC_READ.0 | GENERIC_WRITE.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            None,
        ) else {
            return false;
        };
        let clamped = pct.min(100);
        let input = DISPLAY_BRIGHTNESS {
            ucDisplayPolicy: 0x03, // Apply to both AC and DC power states
            ucACBrightness: clamped,
            ucDCBrightness: clamped,
        };
        let mut returned = 0u32;
        let ok = DeviceIoControl(
            handle,
            IOCTL_VIDEO_SET_DISPLAY_BRIGHTNESS,
            Some((&input as *const DISPLAY_BRIGHTNESS).cast()),
            size_of::<DISPLAY_BRIGHTNESS>() as u32,
            None,
            0,
            Some(&mut returned),
            None,
        )
        .is_ok();
        let _ = CloseHandle(handle);
        ok
    }
}

#[cfg(windows)]
fn read_gdi_gamma_ramp(device_name: &str) -> Option<[[u16; 256]; 3]> {
    let wide: Vec<u16> = device_name
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        let hdc = CreateDCW(
            PCWSTR(w!("DISPLAY").as_ptr()),
            PCWSTR(wide.as_ptr()),
            None,
            None,
        );
        if hdc.is_invalid() {
            return None;
        }
        let mut ramp = [[0u16; 256]; 3];
        let ok = GetDeviceGammaRamp(hdc, ramp.as_mut_ptr().cast()).as_bool();
        let _ = DeleteDC(hdc);
        if ok {
            Some(ramp)
        } else {
            None
        }
    }
}

#[cfg(windows)]
fn apply_gdi_gamma_ramp(device_name: &str, ramp: &[[u16; 256]; 3]) -> bool {
    let wide: Vec<u16> = device_name
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        let hdc = CreateDCW(
            PCWSTR(w!("DISPLAY").as_ptr()),
            PCWSTR(wide.as_ptr()),
            None,
            None,
        );
        if hdc.is_invalid() {
            return false;
        }
        let ok = SetDeviceGammaRamp(hdc, ramp.as_ptr().cast()).as_bool();
        let _ = DeleteDC(hdc);
        ok
    }
}

#[cfg(windows)]
fn refresh_displays(state: &mut WorkerState) -> Vec<DisplayBrightnessItem> {
    let meta_map = query_display_config_metadata();
    let monitors = enumerate_gdi_monitors();
    let mut items = Vec::with_capacity(monitors.len());

    for mon in monitors {
        let id = mon.device_name.clone();
        let meta = meta_map.get(&id);
        let mut display_name = meta
            .map(|m| m.friendly_name.clone())
            .unwrap_or_default();
        let is_internal = meta.is_some_and(|m| m.is_internal);

        // If DisplayConfig didn't yield a non-generic name, fall back to HMONITOR physical monitor desc before generic check.
        let phys = open_physical_monitors(mon.hmonitor);
        if is_generic_monitor_name(&display_name) {
            if let Some(first) = phys.first() {
                let raw_desc = first.szPhysicalMonitorDescription;
                let pdesc = utf16_z_to_string(&raw_desc);
                if !is_generic_monitor_name(&pdesc) {
                    display_name = pdesc;
                }
            }
        }

        let mut probed: Option<(u8, BrightnessMode, DisplayBackend)> = None;

        // 1. High-level DDC/CI brightness
        for pm in &phys {
            let mut min = 0u32;
            let mut cur = 0u32;
            let mut max = 0u32;
            let ok = unsafe { GetMonitorBrightness(pm.hPhysicalMonitor, &mut min, &mut cur, &mut max) != 0 };
            if ok && max > min {
                let pct = normalize_hardware_to_pct(min, cur, max);
                probed = Some((
                    pct,
                    BrightnessMode::Hardware,
                    DisplayBackend::Dxva2HighLevel { min, max },
                ));
                break;
            }
        }

        // 2. Fallback to raw VCP Luminance (0x10)
        if probed.is_none() {
            for pm in &phys {
                let mut cur = 0u32;
                let mut max = 0u32;
                let ok = unsafe {
                    GetVCPFeatureAndVCPFeatureReply(
                        pm.hPhysicalMonitor,
                        VCP_LUMINANCE,
                        None,
                        &mut cur,
                        Some(&mut max),
                    ) != 0
                };
                if ok && max > 0 {
                    let pct = normalize_hardware_to_pct(0, cur, max);
                    probed = Some((
                        pct,
                        BrightnessMode::Hardware,
                        DisplayBackend::Dxva2Vcp { max },
                    ));
                    break;
                }
            }
        }

        close_physical_monitors(&phys);

        // 3. Internal laptop LCD IOCTL fallback
        if probed.is_none() && is_internal {
            if let Some(pct) = query_internal_lcd_brightness() {
                probed = Some((pct, BrightnessMode::Hardware, DisplayBackend::InternalLcd));
            }
        }

        // 4. Software Gamma Ramp fallback (marked with a red dot in the UI)
        let (brightness, mode, backend) = if let Some(hit) = probed {
            hit
        } else {
            let already_tracked = software_gamma_store()
                .lock()
                .map(|g| g.original_ramps.contains_key(&id))
                .unwrap_or(false);
            let probed_ramp = if already_tracked {
                None
            } else {
                Some(read_gdi_gamma_ramp(&id).unwrap_or_else(identity_gamma_ramp))
            };
            let pct = if let Ok(mut store) = software_gamma_store().lock() {
                store.ensure_registered(&id, || probed_ramp.unwrap_or_else(identity_gamma_ramp))
            } else {
                100
            };
            (pct, BrightnessMode::Software, DisplayBackend::SoftwareGamma)
        };

        state.backends.insert(id.clone(), backend);
        items.push(DisplayBrightnessItem {
            id,
            name: display_name,
            brightness,
            mode,
            is_primary: mon.is_primary,
        });
    }

    disambiguate_display_names(&mut items);
    items
}

#[cfg(windows)]
fn apply_display_brightness(id: &str, pct: u8, state: &mut WorkerState) {
    let clamped = pct.min(100);
    let Some(backend) = state.backends.get(id).copied() else {
        return;
    };

    match backend {
        DisplayBackend::Dxva2HighLevel { min, max } => {
            if let Some(mon) = enumerate_gdi_monitors().into_iter().find(|m| m.device_name == id) {
                let phys = open_physical_monitors(mon.hmonitor);
                let target = pct_to_hardware_value(min, max, clamped);
                for pm in &phys {
                    let ok = unsafe { SetMonitorBrightness(pm.hPhysicalMonitor, target) != 0 };
                    if ok {
                        break;
                    }
                }
                close_physical_monitors(&phys);
            }
        }
        DisplayBackend::Dxva2Vcp { max } => {
            if let Some(mon) = enumerate_gdi_monitors().into_iter().find(|m| m.device_name == id) {
                let phys = open_physical_monitors(mon.hmonitor);
                let target = pct_to_hardware_value(0, max, clamped);
                for pm in &phys {
                    let ok = unsafe { SetVCPFeature(pm.hPhysicalMonitor, VCP_LUMINANCE, target) != 0 };
                    if ok {
                        break;
                    }
                }
                close_physical_monitors(&phys);
            }
        }
        DisplayBackend::InternalLcd => {
            let _ = set_internal_lcd_brightness(clamped);
        }
        DisplayBackend::SoftwareGamma => {
            let cached_orig = software_gamma_store()
                .lock()
                .ok()
                .and_then(|g| g.original_ramps.get(id).copied());
            let original = match cached_orig {
                Some(orig) => orig,
                None => {
                    let read = read_gdi_gamma_ramp(id).unwrap_or_else(identity_gamma_ramp);
                    if let Ok(mut store) = software_gamma_store().lock() {
                        store.original_or_register(id, || read)
                    } else {
                        read
                    }
                }
            };
            let ramp = build_software_gamma_ramp(&original, clamped, SOFTWARE_BRIGHTNESS_FLOOR);
            let mut applied = apply_gdi_gamma_ramp(id, &ramp);
            if !applied {
                // Retry with 50% floor if Windows ICM rejects ramps darker than half intensity.
                let fallback =
                    build_software_gamma_ramp(&original, clamped, SOFTWARE_ICM_FALLBACK_FLOOR);
                applied = apply_gdi_gamma_ramp(id, &fallback);
            }
            if applied {
                if let Ok(mut store) = software_gamma_store().lock() {
                    store.record_applied(id, clamped);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hardware_percentage_roundtrip_and_bounds() {
        assert_eq!(normalize_hardware_to_pct(0, 75, 100), 75);
        assert_eq!(normalize_hardware_to_pct(20, 60, 100), 50);
        assert_eq!(normalize_hardware_to_pct(50, 50, 50), 100);
        assert_eq!(normalize_hardware_to_pct(10, 0, 100), 0);
        assert_eq!(normalize_hardware_to_pct(10, 200, 100), 100);

        assert_eq!(pct_to_hardware_value(0, 100, 0), 0);
        assert_eq!(pct_to_hardware_value(20, 100, 50), 60);
        assert_eq!(pct_to_hardware_value(20, 100, 100), 100);
        assert_eq!(pct_to_hardware_value(50, 50, 30), 50);
    }

    #[test]
    fn software_gamma_preserves_100_and_enforces_floor() {
        let orig = identity_gamma_ramp();
        assert_eq!(
            build_software_gamma_ramp(&orig, 100, SOFTWARE_BRIGHTNESS_FLOOR),
            orig
        );
        assert_eq!(
            build_software_gamma_ramp(&orig, 120, SOFTWARE_BRIGHTNESS_FLOOR),
            orig
        );

        let min_ramp = build_software_gamma_ramp(&orig, 0, SOFTWARE_BRIGHTNESS_FLOOR);
        assert_eq!(min_ramp[0][0], 0);
        assert_eq!(min_ramp[0][255], (65535.0 * 0.20f32).round() as u16);

        let fallback_ramp = build_software_gamma_ramp(&orig, 0, SOFTWARE_ICM_FALLBACK_FLOOR);
        assert_eq!(fallback_ramp[1][255], (65535.0 * 0.50f32).round() as u16);

        let mid_ramp = build_software_gamma_ramp(&orig, 50, SOFTWARE_BRIGHTNESS_FLOOR);
        // scale = 0.20 + 0.80 * 0.50 = 0.60
        assert_eq!(mid_ramp[2][255], (65535.0 * 0.60f32).round() as u16);
    }

    #[test]
    fn display_name_fallback_and_disambiguation() {
        let mut items = vec![
            DisplayBrightnessItem {
                id: "D1".into(),
                name: "Generic PnP Monitor".into(),
                brightness: 80,
                mode: BrightnessMode::Hardware,
                is_primary: true,
            },
            DisplayBrightnessItem {
                id: "D2".into(),
                name: "DELL U2723QE".into(),
                brightness: 50,
                mode: BrightnessMode::Hardware,
                is_primary: false,
            },
            DisplayBrightnessItem {
                id: "D3".into(),
                name: "DELL U2723QE".into(),
                brightness: 65,
                mode: BrightnessMode::Software,
                is_primary: false,
            },
        ];
        disambiguate_display_names(&mut items);
        assert_eq!(items[0].name, "显示器 1");
        assert_eq!(items[1].name, "DELL U2723QE (1)");
        assert_eq!(items[2].name, "DELL U2723QE (2)");
    }

    #[test]
    fn format_brightness_pct_clamps_and_formats() {
        assert_eq!(format_brightness_pct(0), "0%");
        assert_eq!(format_brightness_pct(75), "75%");
        assert_eq!(format_brightness_pct(150), "100%");
    }

    #[test]
    fn software_gamma_store_persists_across_reopen_and_restores_on_exit() {
        let mut store = SoftwareGammaStore::default();
        let initial_ramp = identity_gamma_ramp();
        let initial_pct = store.ensure_registered(r"\\.\DISPLAY2", || initial_ramp);
        assert_eq!(initial_pct, 100);
        assert!(store.drain_dirty_for_restore().is_empty());

        store.record_applied(r"\\.\DISPLAY2", 60);

        // Simulate closing MonitorBar and reopening it later: reading GDI now would yield a dimmed ramp.
        let dimmed_ramp = build_software_gamma_ramp(&initial_ramp, 60, SOFTWARE_BRIGHTNESS_FLOOR);
        let pct_after_reopen = store.ensure_registered(r"\\.\DISPLAY2", || dimmed_ramp);
        assert_eq!(pct_after_reopen, 60);

        let orig = store.original_or_register(r"\\.\DISPLAY2", || dimmed_ramp);
        assert_eq!(orig, initial_ramp);

        let restored = store.drain_dirty_for_restore();
        assert_eq!(restored, vec![(r"\\.\DISPLAY2".to_string(), initial_ramp)]);
        assert!(store.drain_dirty_for_restore().is_empty());
        assert_eq!(
            store.ensure_registered(r"\\.\DISPLAY2", || dimmed_ramp),
            100
        );
    }
}
