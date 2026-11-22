//! System performance sampling and formatting for MonitorBar (Network, CPU, Memory, GPU, Disk).

#[cfg(windows)]
use std::collections::HashMap;
#[cfg(windows)]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(windows)]
use std::sync::{Arc, Mutex};
#[cfg(windows)]
use std::thread::{self, JoinHandle};
#[cfg(windows)]
use std::time::{Duration, Instant};

#[cfg(windows)]
use windows::core::w;
#[cfg(windows)]
use windows::Win32::Foundation::{FILETIME, HWND, LPARAM, WPARAM};
#[cfg(windows)]
use windows::Win32::NetworkManagement::IpHelper::{
    FreeMibTable, GetIfTable2, IF_TYPE_SOFTWARE_LOOPBACK, MIB_IF_ROW2, MIB_IF_TABLE2,
};
#[cfg(windows)]
use windows::Win32::NetworkManagement::Ndis::{IfOperStatusUp, TUNNEL_TYPE_NONE};
#[cfg(windows)]
use windows::Win32::System::Performance::{
    PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData, PdhGetFormattedCounterArrayW,
    PdhGetFormattedCounterValue, PdhOpenQueryW, PDH_CSTATUS_NEW_DATA, PDH_CSTATUS_VALID_DATA,
    PDH_FMT_COUNTERVALUE, PDH_FMT_COUNTERVALUE_ITEM_W, PDH_FMT_DOUBLE, PDH_MORE_DATA,
};
#[cfg(windows)]
use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
#[cfg(windows)]
use windows::Win32::System::Threading::GetSystemTimes;
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

/// Percentage threshold (`>= 85%`) at which CPU/Memory/GPU/Disk values display warm alert styling.
#[cfg_attr(not(windows), allow(dead_code))]
pub const HIGH_LOAD_THRESHOLD: u8 = 85;

/// Latest sampled system metrics displayed by MonitorBar.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[cfg_attr(not(windows), allow(dead_code))]
pub struct MetricsSnapshot {
    pub up_bps: f64,
    pub down_bps: f64,
    pub cpu_pct: u8,
    pub mem_pct: u8,
    pub gpu_pct: u8,
    pub disk_pct: u8,
}

/// Formats throughput starting in `KB/s` and scaling up cleanly through `MB/s` and `GB/s`.
pub fn format_speed(bytes_per_sec: f64) -> String {
    let bps = if bytes_per_sec.is_finite() && bytes_per_sec > 0.0 {
        bytes_per_sec
    } else {
        0.0
    };

    let kb = bps / 1024.0;
    if kb < 1000.0 {
        return format!("{kb:.2} KB/s");
    }

    let mb = kb / 1024.0;
    if mb < 1000.0 {
        return format!("{mb:.2} MB/s");
    }

    let gb = mb / 1024.0;
    format!("{gb:.2} GB/s")
}

/// Formats integer percentage matching `7 %`, `37 %`, `100 %`.
pub fn format_percent(pct: u8) -> String {
    format!("{} %", pct.min(100))
}

/// Computes CPU utilization percentage from two `GetSystemTimes` snapshots.
///
/// On Windows, `kernel` includes `idle`, so busy = `(kernel - idle) + user`.
pub fn compute_cpu_percent(
    prev_idle: u64,
    prev_kernel: u64,
    prev_user: u64,
    cur_idle: u64,
    cur_kernel: u64,
    cur_user: u64,
) -> u8 {
    let d_idle = cur_idle.wrapping_sub(prev_idle);
    let d_kernel = cur_kernel.wrapping_sub(prev_kernel);
    let d_user = cur_user.wrapping_sub(prev_user);
    let total = d_kernel.saturating_add(d_user);
    if total == 0 {
        return 0;
    }
    let busy = total.saturating_sub(d_idle);
    ((busy as f64 / total as f64) * 100.0)
        .round()
        .clamp(0.0, 100.0) as u8
}

/// Converts PDH `\PhysicalDisk(_Total)\% Idle Time` into active busy percentage.
pub fn compute_disk_active_percent(idle_pct: f64) -> u8 {
    if !idle_pct.is_finite() {
        return 0;
    }
    (100.0 - idle_pct.clamp(0.0, 100.0)).round().clamp(0.0, 100.0) as u8
}

/// Aggregates Windows PDH `\GPU Engine(*)\Utilization Percentage` samples following Task Manager logic:
/// sum instances sharing the same `(adapter/phys, engtype)`, clamp each engine bucket to 100, and take the max.
pub fn aggregate_gpu_pdh_samples<'a>(samples: impl Iterator<Item = (&'a str, f64)>) -> u8 {
    let mut buckets: std::collections::HashMap<(&'a str, &'a str), f64> =
        std::collections::HashMap::new();

    for (name, val) in samples {
        if !val.is_finite() || val <= 0.0 {
            continue;
        }
        let key = parse_gpu_instance_bucket(name);
        let entry = buckets.entry(key).or_insert(0.0);
        *entry += val;
    }

    let max_val = buckets
        .into_values()
        .map(|v| v.clamp(0.0, 100.0))
        .fold(0.0_f64, f64::max);
    max_val.round().clamp(0.0, 100.0) as u8
}

fn parse_gpu_instance_bucket(instance_name: &str) -> (&str, &str) {
    // Typical PDH instance format:
    // "pid_1234_luid_0x00000000_0x00012ABC_phys_0_eng_0_engtype_3D"
    let adapter_part = if let Some(luid_idx) = instance_name.find("luid_") {
        let from_luid = &instance_name[luid_idx..];
        if let Some(eng_idx) = from_luid.find("_eng_") {
            &from_luid[..eng_idx]
        } else {
            from_luid
        }
    } else if let Some(eng_idx) = instance_name.find("_eng_") {
        &instance_name[..eng_idx]
    } else {
        instance_name
    };

    let engtype_part = if let Some(idx) = instance_name.rfind("_engtype_") {
        &instance_name[idx + "_engtype_".len()..]
    } else {
        ""
    };

    (adapter_part, engtype_part)
}

#[cfg(windows)]
#[derive(Clone)]
pub struct MetricsHandle {
    snapshot: Arc<Mutex<MetricsSnapshot>>,
    stop_flag: Arc<AtomicBool>,
    join: Arc<Mutex<Option<JoinHandle<()>>>>,
}

#[cfg(windows)]
impl MetricsHandle {
    pub fn start(target_hwnd: HWND, notify_msg: u32) -> Self {
        let initial = sample_instant_memory_only();
        let snapshot = Arc::new(Mutex::new(initial));
        let stop_flag = Arc::new(AtomicBool::new(false));

        let snap_clone = Arc::clone(&snapshot);
        let stop_clone = Arc::clone(&stop_flag);
        let hwnd_raw = target_hwnd.0 as usize;

        let handle = thread::spawn(move || {
            metrics_worker_loop(hwnd_raw, notify_msg, snap_clone, stop_clone);
        });

        Self {
            snapshot,
            stop_flag,
            join: Arc::new(Mutex::new(Some(handle))),
        }
    }

    pub fn snapshot(&self) -> MetricsSnapshot {
        self.snapshot
            .lock()
            .map(|guard| *guard)
            .unwrap_or_default()
    }

    pub fn stop(&self) {
        self.stop_flag.store(true, Ordering::SeqCst);
        if let Ok(mut guard) = self.join.lock() {
            if let Some(h) = guard.take() {
                let _ = h.join();
            }
        }
    }
}

#[cfg(windows)]
impl Drop for MetricsHandle {
    fn drop(&mut self) {
        // Avoid blocking indefinitely if multiple clones exist; only stop on last strong reference.
        if Arc::strong_count(&self.stop_flag) <= 2 {
            self.stop();
        }
    }
}

#[cfg(windows)]
fn filetime_to_u64(ft: FILETIME) -> u64 {
    ((ft.dwHighDateTime as u64) << 32) | (ft.dwLowDateTime as u64)
}

#[cfg(windows)]
fn sample_instant_memory_only() -> MetricsSnapshot {
    MetricsSnapshot {
        mem_pct: read_memory_load(),
        ..MetricsSnapshot::default()
    }
}

#[cfg(windows)]
fn read_memory_load() -> u8 {
    unsafe {
        let mut mem = MEMORYSTATUSEX {
            dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
            ..Default::default()
        };
        if GlobalMemoryStatusEx(&mut mem).is_ok() {
            (mem.dwMemoryLoad as u8).min(100)
        } else {
            0
        }
    }
}

#[cfg(windows)]
fn read_system_times() -> Option<(u64, u64, u64)> {
    unsafe {
        let mut idle = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        if GetSystemTimes(Some(&mut idle), Some(&mut kernel), Some(&mut user)).is_ok() {
            Some((
                filetime_to_u64(idle),
                filetime_to_u64(kernel),
                filetime_to_u64(user),
            ))
        } else {
            None
        }
    }
}

#[cfg(windows)]
fn read_network_octets() -> HashMap<u32, (u64, u64)> {
    let mut map = HashMap::new();
    unsafe {
        let mut table_ptr: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
        if GetIfTable2(&mut table_ptr).0 != 0 || table_ptr.is_null() {
            return map;
        }
        let num = (*table_ptr).NumEntries as usize;
        let rows: &[MIB_IF_ROW2] =
            std::slice::from_raw_parts((*table_ptr).Table.as_ptr(), num);
        for row in rows {
            if row.Type == IF_TYPE_SOFTWARE_LOOPBACK
                || row.TunnelType != TUNNEL_TYPE_NONE
                || row.OperStatus != IfOperStatusUp
            {
                continue;
            }
            // Bit 0 = HardwareInterface, Bit 1 = FilterInterface in MIB_IF_ROW2_0.
            let flags = row.InterfaceAndOperStatusFlags._bitfield;
            let is_hardware = (flags & 0x01) != 0;
            let is_filter = (flags & 0x02) != 0;
            if !is_hardware || is_filter {
                continue;
            }
            map.insert(row.InterfaceIndex, (row.InOctets, row.OutOctets));
        }
        FreeMibTable(table_ptr.cast());
    }
    map
}

#[cfg(windows)]
struct PdhSampler {
    query: isize,
    gpu_counter: isize,
    disk_counter: isize,
    gpu_buf: Vec<u64>,
}

#[cfg(windows)]
impl PdhSampler {
    fn new() -> Option<Self> {
        unsafe {
            let mut query: isize = 0;
            if PdhOpenQueryW(None, 0, &mut query) != 0 {
                return None;
            }
            let mut gpu_counter: isize = 0;
            if PdhAddEnglishCounterW(
                query,
                w!("\\GPU Engine(*)\\Utilization Percentage"),
                0,
                &mut gpu_counter,
            ) != 0
            {
                gpu_counter = 0;
            }
            let mut disk_counter: isize = 0;
            if PdhAddEnglishCounterW(
                query,
                w!("\\PhysicalDisk(_Total)\\% Idle Time"),
                0,
                &mut disk_counter,
            ) != 0
            {
                disk_counter = 0;
            }
            let _ = PdhCollectQueryData(query);
            Some(Self {
                query,
                gpu_counter,
                disk_counter,
                gpu_buf: Vec::new(),
            })
        }
    }

    fn collect(&mut self) -> (u8, u8) {
        unsafe {
            if PdhCollectQueryData(self.query) != 0 {
                return (0, 0);
            }
            let gpu_pct = if self.gpu_counter != 0 {
                self.sample_gpu()
            } else {
                0
            };
            let disk_pct = if self.disk_counter != 0 {
                self.sample_disk()
            } else {
                0
            };
            (gpu_pct, disk_pct)
        }
    }

    unsafe fn sample_disk(&self) -> u8 {
        let mut value = PDH_FMT_COUNTERVALUE::default();
        let status = PdhGetFormattedCounterValue(
            self.disk_counter,
            PDH_FMT_DOUBLE,
            None,
            &mut value,
        );
        if status == 0 && matches!(value.CStatus, PDH_CSTATUS_VALID_DATA | PDH_CSTATUS_NEW_DATA) {
            compute_disk_active_percent(value.Anonymous.doubleValue)
        } else {
            0
        }
    }

    unsafe fn sample_gpu(&mut self) -> u8 {
        let mut buf_size: u32 = 0;
        let mut item_count: u32 = 0;
        let status = PdhGetFormattedCounterArrayW(
            self.gpu_counter,
            PDH_FMT_DOUBLE,
            &mut buf_size,
            &mut item_count,
            None,
        );
        if status != PDH_MORE_DATA || buf_size == 0 {
            return 0;
        }
        let needed_u64 = (buf_size as usize).div_ceil(std::mem::size_of::<u64>());
        if self.gpu_buf.len() < needed_u64 {
            self.gpu_buf.resize(needed_u64, 0);
        }
        buf_size = (self.gpu_buf.len() * std::mem::size_of::<u64>()).min(u32::MAX as usize) as u32;

        let ptr = self.gpu_buf.as_mut_ptr().cast::<PDH_FMT_COUNTERVALUE_ITEM_W>();
        let status2 = PdhGetFormattedCounterArrayW(
            self.gpu_counter,
            PDH_FMT_DOUBLE,
            &mut buf_size,
            &mut item_count,
            Some(ptr),
        );
        if status2 != 0 || item_count == 0 {
            return 0;
        }

        let items = std::slice::from_raw_parts(ptr, item_count as usize);
        let mut decoded: Vec<(String, f64)> = Vec::with_capacity(items.len());
        for item in items {
            if !matches!(
                item.FmtValue.CStatus,
                PDH_CSTATUS_VALID_DATA | PDH_CSTATUS_NEW_DATA
            ) || item.szName.is_null()
            {
                continue;
            }
            let val = item.FmtValue.Anonymous.doubleValue;
            if val <= 0.0 || !val.is_finite() {
                continue;
            }
            let name = pwstr_to_string(item.szName.0);
            decoded.push((name, val));
        }

        aggregate_gpu_pdh_samples(decoded.iter().map(|(s, v)| (s.as_str(), *v)))
    }
}

#[cfg(windows)]
impl Drop for PdhSampler {
    fn drop(&mut self) {
        unsafe {
            let _ = PdhCloseQuery(self.query);
        }
    }
}

#[cfg(windows)]
unsafe fn pwstr_to_string(ptr: *const u16) -> String {
    if ptr.is_null() {
        return String::new();
    }
    let mut len = 0usize;
    while *ptr.add(len) != 0 && len < 512 {
        len += 1;
    }
    String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len))
}

#[cfg(windows)]
fn metrics_worker_loop(
    hwnd_raw: usize,
    notify_msg: u32,
    snapshot: Arc<Mutex<MetricsSnapshot>>,
    stop_flag: Arc<AtomicBool>,
) {
    let mut pdh = PdhSampler::new();
    let mut prev_cpu = read_system_times();
    let mut prev_net = read_network_octets();
    let mut prev_tick = Instant::now();

    while !stop_flag.load(Ordering::Relaxed) {
        for _ in 0..10 {
            if stop_flag.load(Ordering::Relaxed) {
                return;
            }
            thread::sleep(Duration::from_millis(100));
        }

        let now = Instant::now();
        let elapsed_s = now.duration_since(prev_tick).as_secs_f64().max(0.2);
        prev_tick = now;

        let cur_cpu = read_system_times();
        let cpu_pct = match (prev_cpu, cur_cpu) {
            (Some((pi, pk, pu)), Some((ci, ck, cu))) => compute_cpu_percent(pi, pk, pu, ci, ck, cu),
            _ => 0,
        };
        prev_cpu = cur_cpu;

        let cur_net = read_network_octets();
        let mut down_bytes: u64 = 0;
        let mut up_bytes: u64 = 0;
        for (idx, &(cur_in, cur_out)) in &cur_net {
            if let Some(&(prev_in, prev_out)) = prev_net.get(idx) {
                if cur_in >= prev_in {
                    down_bytes = down_bytes.saturating_add(cur_in - prev_in);
                }
                if cur_out >= prev_out {
                    up_bytes = up_bytes.saturating_add(cur_out - prev_out);
                }
            }
        }
        prev_net = cur_net;

        let mem_pct = read_memory_load();
        let (gpu_pct, disk_pct) = if let Some(ref mut sampler) = pdh {
            sampler.collect()
        } else {
            (0, 0)
        };

        let next = MetricsSnapshot {
            up_bps: (up_bytes as f64) / elapsed_s,
            down_bps: (down_bytes as f64) / elapsed_s,
            cpu_pct,
            mem_pct,
            gpu_pct,
            disk_pct,
        };

        if let Ok(mut guard) = snapshot.lock() {
            *guard = next;
        }

        unsafe {
            let hwnd = HWND(hwnd_raw as *mut _);
            let _ = PostMessageW(hwnd, notify_msg, WPARAM(0), LPARAM(0));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speed_formatting_scales_from_kb_to_gb() {
        assert_eq!(format_speed(0.0), "0.00 KB/s");
        assert_eq!(format_speed(460.8), "0.45 KB/s");
        assert_eq!(format_speed(1024.0 * 999.0), "999.00 KB/s");
        assert_eq!(format_speed(1024.0 * 1024.0 * 12.5), "12.50 MB/s");
        assert_eq!(format_speed(1024.0 * 1024.0 * 1024.0 * 1.25), "1.25 GB/s");
    }

    #[test]
    fn percent_formatting_matches_spaced_style() {
        assert_eq!(format_percent(7), "7 %");
        assert_eq!(format_percent(37), "37 %");
        assert_eq!(format_percent(140), "100 %");
    }

    #[test]
    fn cpu_and_disk_math_handles_deltas_and_clamping() {
        // kernel includes idle: 100_000 kernel (70_000 idle) + 20_000 user = 120_000 total, 50_000 busy => 42%
        let pct = compute_cpu_percent(1000, 2000, 500, 71000, 102000, 20500);
        assert_eq!(pct, 42);
        assert_eq!(compute_disk_active_percent(99.6), 0);
        assert_eq!(compute_disk_active_percent(12.2), 88);
        assert_eq!(compute_disk_active_percent(-3.0), 100);
    }

    #[test]
    fn gpu_pdh_aggregation_sums_same_engine_type_per_adapter_and_takes_max() {
        let samples = [
            ("pid_100_luid_0x0_0x1_phys_0_eng_0_engtype_3D", 18.4),
            ("pid_200_luid_0x0_0x1_phys_0_eng_1_engtype_3D", 10.6),
            ("pid_300_luid_0x0_0x1_phys_0_eng_2_engtype_Copy", 7.0),
            ("pid_400_luid_0x0_0x2_phys_0_eng_0_engtype_VideoDecode", 22.0),
        ];
        // Bucket (0x1, 3D) = 29.0, Bucket (0x1, Copy) = 7.0, Bucket (0x2, VideoDecode) = 22.0 => max 29%
        assert_eq!(aggregate_gpu_pdh_samples(samples.into_iter()), 29);
    }
}
