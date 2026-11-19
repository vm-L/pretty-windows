//! Async WiFi & Bluetooth radio/device discovery and control for MonitorBar.

#[cfg(windows)]
use super::actions::open_shell_target;
use std::collections::{HashMap, HashSet};
#[cfg(windows)]
use std::sync::mpsc::{self, Receiver, Sender};
#[cfg(windows)]
use std::sync::{Arc, Mutex};
#[cfg(windows)]
use std::thread::{self, JoinHandle};
#[cfg(windows)]
use std::time::Duration;

#[cfg(windows)]
use windows::core::{GUID, HSTRING, PCWSTR};
#[cfg(windows)]
use windows::Devices::Bluetooth::{BluetoothConnectionStatus, BluetoothLEDevice};
#[cfg(windows)]
use windows::Devices::Enumeration::DeviceInformation;
#[cfg(windows)]
use windows::Devices::Radios::{Radio, RadioKind, RadioState};
#[cfg(windows)]
use windows::Networking::Connectivity::{NetworkConnectivityLevel, NetworkInformation};
#[cfg(windows)]
use windows::Win32::Devices::Bluetooth::{
    BluetoothEnumerateInstalledServices, BluetoothFindDeviceClose, BluetoothFindFirstDevice,
    BluetoothFindFirstRadio, BluetoothFindNextDevice, BluetoothFindRadioClose,
    BluetoothSetServiceState, BLUETOOTH_DEVICE_INFO, BLUETOOTH_DEVICE_SEARCH_PARAMS,
    BLUETOOTH_FIND_RADIO_PARAMS,
};
#[cfg(windows)]
use windows::Win32::Foundation::{CloseHandle, BOOL, HANDLE, HWND, LPARAM, RPC_E_CHANGED_MODE, WPARAM};
#[cfg(windows)]
use windows::Win32::NetworkManagement::WiFi::{
    dot11_BSS_type_any, wlan_connection_mode_profile, WlanCloseHandle, WlanConnect, WlanDisconnect,
    WlanEnumInterfaces, WlanFreeMemory, WlanGetAvailableNetworkList, WlanGetProfileList,
    WlanOpenHandle, WlanScan, WLAN_AVAILABLE_NETWORK, WLAN_AVAILABLE_NETWORK_CONNECTED,
    WLAN_AVAILABLE_NETWORK_HAS_PROFILE, WLAN_AVAILABLE_NETWORK_LIST, WLAN_CONNECTION_PARAMETERS,
    WLAN_INTERFACE_INFO, WLAN_INTERFACE_INFO_LIST, WLAN_PROFILE_INFO, WLAN_PROFILE_INFO_LIST,
};
#[cfg(windows)]
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

/// Single discovered WiFi network merged across BSSIDs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WifiNetworkItem {
    pub ssid: String,
    pub profile_name: String,
    pub signal_quality: u8,
    pub is_connected: bool,
    pub has_profile: bool,
    pub is_secured: bool,
}

/// Transport classification for paired Bluetooth devices.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BtDeviceKind {
    Classic,
    Ble,
}

/// Single paired Bluetooth device shown in MonitorBar's Bluetooth submenu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BluetoothDeviceItem {
    pub name: String,
    pub address: u64,
    pub device_id: String,
    pub kind: BtDeviceKind,
    pub is_connected: bool,
}

/// Complete wireless status snapshot consumed by MonitorBar popup rendering.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(not(windows), allow(dead_code))]
pub struct WirelessSnapshot {
    pub wifi_available: bool,
    pub wifi_enabled: bool,
    pub wifi_busy: bool,
    pub wifi_networks: Vec<WifiNetworkItem>,
    pub bt_available: bool,
    pub bt_enabled: bool,
    pub bt_busy: bool,
    pub bt_devices: Vec<BluetoothDeviceItem>,
}

/// Parses a 6-byte Bluetooth MAC address embedded in a Windows BLE DeviceInformation Id.
///
/// Example id: `"BluetoothLE#BluetoothLEac:ed:5c:11:22:33-d4:e5:f6:aa:bb:cc"`.
pub fn parse_ble_id_address(device_id: &str) -> u64 {
    let Some(dash_pos) = device_id.rfind('-') else {
        return 0;
    };
    let tail = &device_id[dash_pos + 1..];
    let hex_only: String = tail.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    if hex_only.len() != 12 {
        return 0;
    }
    u64::from_str_radix(&hex_only, 16).unwrap_or(0)
}

/// Deduplicates available WLAN entries by SSID, preferring connected -> profiled -> strongest signal.
pub fn dedup_and_sort_wifi(raw: Vec<WifiNetworkItem>) -> Vec<WifiNetworkItem> {
    let mut by_ssid: HashMap<String, WifiNetworkItem> = HashMap::new();
    for item in raw {
        let ssid = item.ssid.trim();
        if ssid.is_empty() {
            continue;
        }
        let normalized = WifiNetworkItem {
            ssid: ssid.to_string(),
            profile_name: item.profile_name,
            signal_quality: item.signal_quality.min(100),
            is_connected: item.is_connected,
            has_profile: item.has_profile,
            is_secured: item.is_secured,
        };
        match by_ssid.get(ssid) {
            Some(existing) => {
                let better = (
                    normalized.is_connected,
                    normalized.has_profile,
                    normalized.signal_quality,
                ) > (
                    existing.is_connected,
                    existing.has_profile,
                    existing.signal_quality,
                );
                if better {
                    by_ssid.insert(ssid.to_string(), normalized);
                }
            }
            None => {
                by_ssid.insert(ssid.to_string(), normalized);
            }
        }
    }

    let mut list: Vec<WifiNetworkItem> = by_ssid.into_values().collect();
    list.sort_by(|a, b| {
        b.is_connected
            .cmp(&a.is_connected)
            .then_with(|| b.has_profile.cmp(&a.has_profile))
            .then_with(|| b.signal_quality.cmp(&a.signal_quality))
            .then_with(|| a.ssid.to_lowercase().cmp(&b.ssid.to_lowercase()))
    });
    list
}

/// Merges Classic BR/EDR and BLE paired device lists without duplicate dual-mode entries.
pub fn dedup_and_sort_bluetooth(
    classic: Vec<BluetoothDeviceItem>,
    ble: Vec<BluetoothDeviceItem>,
) -> Vec<BluetoothDeviceItem> {
    let mut result: Vec<BluetoothDeviceItem> = Vec::new();
    let mut seen_addrs: HashSet<u64> = HashSet::new();
    let mut seen_names: HashSet<String> = HashSet::new();

    for mut item in classic.into_iter().chain(ble.into_iter()) {
        item.name = item.name.trim().to_string();
        if item.name.is_empty() {
            continue;
        }
        let name_key = item.name.to_lowercase();
        let dup_addr = item.address != 0 && seen_addrs.contains(&item.address);
        let dup_name = seen_names.contains(&name_key);
        if dup_addr || dup_name {
            if let Some(existing) = result.iter_mut().find(|e| {
                (item.address != 0 && e.address == item.address)
                    || e.name.to_lowercase() == name_key
            }) {
                existing.is_connected |= item.is_connected;
            }
            continue;
        }
        if item.address != 0 {
            seen_addrs.insert(item.address);
        }
        seen_names.insert(name_key);
        result.push(item);
    }

    result.sort_by(|a, b| {
        b.is_connected
            .cmp(&a.is_connected)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    result
}

#[cfg(windows)]
enum WirelessCmd {
    Refresh,
    ScanAndRefresh,
    SetWifiEnabled(bool),
    ToggleWifiNetwork(WifiNetworkItem),
    SetBluetoothEnabled(bool),
    ToggleBluetoothDevice(BluetoothDeviceItem),
    Shutdown,
}

/// Handle owning the MTA background worker communicating with WinRT Radios & Win32 WLAN/Bluetooth APIs.
#[cfg(windows)]
#[derive(Clone)]
pub struct WirelessHandle {
    snapshot: Arc<Mutex<WirelessSnapshot>>,
    tx: Sender<WirelessCmd>,
    join: Arc<Mutex<Option<JoinHandle<()>>>>,
}

#[cfg(windows)]
impl WirelessHandle {
    pub fn start(target_hwnd: HWND, notify_msg: u32) -> Self {
        let snapshot = Arc::new(Mutex::new(WirelessSnapshot::default()));
        let (tx, rx) = mpsc::channel();
        let snap_clone = Arc::clone(&snapshot);
        let hwnd_raw = target_hwnd.0 as usize;

        let handle = thread::spawn(move || {
            wireless_worker_loop(hwnd_raw, notify_msg, rx, snap_clone);
        });

        let _ = tx.send(WirelessCmd::Refresh);

        Self {
            snapshot,
            tx,
            join: Arc::new(Mutex::new(Some(handle))),
        }
    }

    pub fn snapshot(&self) -> WirelessSnapshot {
        self.snapshot
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default()
    }

    pub fn request_refresh(&self, scan_wifi: bool) {
        let _ = self.tx.send(if scan_wifi {
            WirelessCmd::ScanAndRefresh
        } else {
            WirelessCmd::Refresh
        });
    }

    pub fn toggle_wifi_radio(&self) {
        let current = self.snapshot().wifi_enabled;
        let _ = self.tx.send(WirelessCmd::SetWifiEnabled(!current));
    }

    pub fn activate_wifi_network(&self, item: WifiNetworkItem) {
        let _ = self.tx.send(WirelessCmd::ToggleWifiNetwork(item));
    }

    pub fn toggle_bluetooth_radio(&self) {
        let current = self.snapshot().bt_enabled;
        let _ = self.tx.send(WirelessCmd::SetBluetoothEnabled(!current));
    }

    pub fn activate_bluetooth_device(&self, item: BluetoothDeviceItem) {
        let _ = self.tx.send(WirelessCmd::ToggleBluetoothDevice(item));
    }

    pub fn stop(&self) {
        let _ = self.tx.send(WirelessCmd::Shutdown);
        if let Ok(mut guard) = self.join.lock() {
            if let Some(h) = guard.take() {
                let _ = h.join();
            }
        }
    }
}

#[cfg(windows)]
impl Drop for WirelessHandle {
    fn drop(&mut self) {
        if Arc::strong_count(&self.snapshot) <= 2 {
            self.stop();
        }
    }
}

#[cfg(windows)]
fn wireless_worker_loop(
    hwnd_raw: usize,
    notify_msg: u32,
    rx: Receiver<WirelessCmd>,
    snapshot: Arc<Mutex<WirelessSnapshot>>,
) {
    let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    let need_uninit = hr.is_ok();
    let _ = hr != RPC_E_CHANGED_MODE;

    let notify = |hwnd_raw: usize, notify_msg: u32| unsafe {
        let hwnd = HWND(hwnd_raw as *mut _);
        let _ = PostMessageW(hwnd, notify_msg, WPARAM(0), LPARAM(0));
    };

    while let Ok(cmd) = rx.recv() {
        // Coalesce back-to-back refresh requests cleanly.
        let mut coalesced = cmd;
        while let Ok(next_cmd) = rx.try_recv() {
            match next_cmd {
                WirelessCmd::Shutdown => {
                    coalesced = WirelessCmd::Shutdown;
                    break;
                }
                WirelessCmd::Refresh | WirelessCmd::ScanAndRefresh
                    if matches!(coalesced, WirelessCmd::Refresh) =>
                {
                    coalesced = next_cmd;
                }
                other => {
                    execute_cmd(other, hwnd_raw, notify_msg, &snapshot, &notify);
                }
            }
        }

        if matches!(coalesced, WirelessCmd::Shutdown) {
            break;
        }
        execute_cmd(coalesced, hwnd_raw, notify_msg, &snapshot, &notify);
    }

    if need_uninit {
        unsafe {
            CoUninitialize();
        }
    }
}

#[cfg(windows)]
fn execute_cmd(
    cmd: WirelessCmd,
    hwnd_raw: usize,
    notify_msg: u32,
    snapshot: &Arc<Mutex<WirelessSnapshot>>,
    notify: &dyn Fn(usize, u32),
) {
    match cmd {
        WirelessCmd::Refresh => {
            refresh_into(snapshot);
            notify(hwnd_raw, notify_msg);
        }
        WirelessCmd::ScanAndRefresh => {
            trigger_wlan_scan();
            refresh_into(snapshot);
            notify(hwnd_raw, notify_msg);
        }
        WirelessCmd::SetWifiEnabled(enable) => {
            set_busy(snapshot, Some(true), None);
            notify(hwnd_raw, notify_msg);
            let _ = set_radio_kind_state(RadioKind::WiFi, enable);
            thread::sleep(Duration::from_millis(if enable { 550 } else { 250 }));
            if enable {
                trigger_wlan_scan();
            }
            refresh_into(snapshot);
            set_busy(snapshot, Some(false), None);
            notify(hwnd_raw, notify_msg);
            if enable {
                thread::sleep(Duration::from_millis(1400));
                refresh_into(snapshot);
                notify(hwnd_raw, notify_msg);
            }
        }
        WirelessCmd::ToggleWifiNetwork(item) => {
            if !item.has_profile {
                open_shell_target("ms-settings:network-wifi");
                return;
            }
            set_busy(snapshot, Some(true), None);
            notify(hwnd_raw, notify_msg);
            let handled = if item.is_connected {
                wlan_disconnect_all()
            } else {
                wlan_connect_profile(&item.ssid, &item.profile_name)
            };
            if !handled {
                open_shell_target("ms-settings:network-wifi");
            }
            thread::sleep(Duration::from_millis(650));
            refresh_into(snapshot);
            set_busy(snapshot, Some(false), None);
            notify(hwnd_raw, notify_msg);
        }
        WirelessCmd::SetBluetoothEnabled(enable) => {
            set_busy(snapshot, None, Some(true));
            notify(hwnd_raw, notify_msg);
            let _ = set_radio_kind_state(RadioKind::Bluetooth, enable);
            thread::sleep(Duration::from_millis(450));
            refresh_into(snapshot);
            set_busy(snapshot, None, Some(false));
            notify(hwnd_raw, notify_msg);
        }
        WirelessCmd::ToggleBluetoothDevice(item) => {
            if item.kind != BtDeviceKind::Classic || item.address == 0 {
                open_shell_target("ms-settings:bluetooth");
                return;
            }
            set_busy(snapshot, None, Some(true));
            notify(hwnd_raw, notify_msg);
            let ok = toggle_classic_bt_device(item.address, !item.is_connected);
            thread::sleep(Duration::from_millis(600));
            refresh_into(snapshot);
            set_busy(snapshot, None, Some(false));
            notify(hwnd_raw, notify_msg);
            if !ok {
                open_shell_target("ms-settings:bluetooth");
            }
        }
        WirelessCmd::Shutdown => {}
    }
}

#[cfg(windows)]
fn set_busy(snapshot: &Arc<Mutex<WirelessSnapshot>>, wifi_busy: Option<bool>, bt_busy: Option<bool>) {
    if let Ok(mut guard) = snapshot.lock() {
        if let Some(b) = wifi_busy {
            guard.wifi_busy = b;
        }
        if let Some(b) = bt_busy {
            guard.bt_busy = b;
        }
    }
}

#[cfg(windows)]
fn refresh_into(snapshot: &Arc<Mutex<WirelessSnapshot>>) {
    let (wifi_available, wifi_enabled, bt_available, bt_enabled) = query_radio_states();
    let wifi_networks = if wifi_available && wifi_enabled {
        enumerate_wlan_networks()
    } else {
        Vec::new()
    };
    let bt_devices = if bt_available && bt_enabled {
        let classic = enumerate_classic_bt_devices();
        let ble = enumerate_ble_bt_devices();
        dedup_and_sort_bluetooth(classic, ble)
    } else {
        Vec::new()
    };

    if let Ok(mut guard) = snapshot.lock() {
        guard.wifi_available = wifi_available;
        guard.wifi_enabled = wifi_enabled;
        guard.wifi_networks = wifi_networks;
        guard.bt_available = bt_available;
        guard.bt_enabled = bt_enabled;
        guard.bt_devices = bt_devices;
    }
}

#[cfg(windows)]
fn query_radio_states() -> (bool, bool, bool, bool) {
    let Ok(op) = Radio::GetRadiosAsync() else {
        return (false, false, false, false);
    };
    let Ok(radios) = op.get() else {
        return (false, false, false, false);
    };
    let count = radios.Size().unwrap_or(0);
    let mut wifi_avail = false;
    let mut wifi_on = false;
    let mut bt_avail = false;
    let mut bt_on = false;

    for i in 0..count {
        let Ok(radio) = radios.GetAt(i) else {
            continue;
        };
        let Ok(kind) = radio.Kind() else {
            continue;
        };
        let on = radio.State().ok() == Some(RadioState::On);
        if kind == RadioKind::WiFi {
            wifi_avail = true;
            wifi_on |= on;
        } else if kind == RadioKind::Bluetooth {
            bt_avail = true;
            bt_on |= on;
        }
    }
    (wifi_avail, wifi_on, bt_avail, bt_on)
}

#[cfg(windows)]
fn set_radio_kind_state(target: RadioKind, enable: bool) -> bool {
    let Ok(op) = Radio::GetRadiosAsync() else {
        return false;
    };
    let Ok(radios) = op.get() else {
        return false;
    };
    let count = radios.Size().unwrap_or(0);
    let desired = if enable {
        RadioState::On
    } else {
        RadioState::Off
    };
    let mut any = false;
    for i in 0..count {
        let Ok(radio) = radios.GetAt(i) else {
            continue;
        };
        if radio.Kind().ok() == Some(target) {
            if let Ok(set_op) = radio.SetStateAsync(desired) {
                let _ = set_op.get();
                any = true;
            }
        }
    }
    any
}

#[cfg(windows)]
fn trigger_wlan_scan() {
    unsafe {
        let mut neg_ver = 0u32;
        let mut handle = HANDLE::default();
        if WlanOpenHandle(2, None, &mut neg_ver, &mut handle) != 0 || handle.is_invalid() {
            return;
        }
        let mut iface_list: *mut WLAN_INTERFACE_INFO_LIST = std::ptr::null_mut();
        if WlanEnumInterfaces(handle, None, &mut iface_list) == 0 && !iface_list.is_null() {
            let count = (*iface_list).dwNumberOfItems as usize;
            let ifaces: &[WLAN_INTERFACE_INFO] =
                std::slice::from_raw_parts((*iface_list).InterfaceInfo.as_ptr(), count);
            for iface in ifaces {
                let _ = WlanScan(handle, &iface.InterfaceGuid, None, None, None);
            }
            WlanFreeMemory(iface_list.cast());
        }
        let _ = WlanCloseHandle(handle, None);
    }
}

#[cfg(windows)]
fn collect_connected_winrt_wlan_profiles() -> Vec<WifiNetworkItem> {
    let mut items = Vec::new();
    let mut inspect_profile = |prof: windows::Networking::Connectivity::ConnectionProfile| {
        if !prof.IsWlanConnectionProfile().unwrap_or(false) {
            return;
        }
        let level = prof
            .GetNetworkConnectivityLevel()
            .ok()
            .unwrap_or(NetworkConnectivityLevel::None);
        let connected_ssid = prof
            .WlanConnectionProfileDetails()
            .ok()
            .and_then(|d| d.GetConnectedSsid().ok())
            .map(|s| s.to_string_lossy().trim().to_string())
            .unwrap_or_default();
        if connected_ssid.is_empty() && level == NetworkConnectivityLevel::None {
            return;
        }
        let profile_name = prof
            .ProfileName()
            .map(|s| s.to_string_lossy().trim().to_string())
            .unwrap_or_default();
        let ssid = if !connected_ssid.is_empty() {
            connected_ssid
        } else {
            profile_name.clone()
        };
        if ssid.is_empty() {
            return;
        }
        let bars = prof
            .GetSignalBars()
            .ok()
            .and_then(|r| r.Value().ok())
            .unwrap_or(4);
        let signal_quality = ((bars as u16).min(5) * 20) as u8;
        items.push(WifiNetworkItem {
            ssid,
            profile_name,
            signal_quality,
            is_connected: true,
            has_profile: true,
            is_secured: true,
        });
    };

    if let Ok(profiles) = NetworkInformation::GetConnectionProfiles() {
        let count = profiles.Size().unwrap_or(0);
        for i in 0..count {
            if let Ok(prof) = profiles.GetAt(i) {
                inspect_profile(prof);
            }
        }
    } else if let Ok(prof) = NetworkInformation::GetInternetConnectionProfile() {
        inspect_profile(prof);
    }

    items
}

#[cfg(windows)]
fn enumerate_wlan_networks() -> Vec<WifiNetworkItem> {
    let mut collected = collect_connected_winrt_wlan_profiles();
    let mut scan_succeeded = false;

    unsafe {
        let mut neg_ver = 0u32;
        let mut handle = HANDLE::default();
        if WlanOpenHandle(2, None, &mut neg_ver, &mut handle) != 0 || handle.is_invalid() {
            return dedup_and_sort_wifi(collected);
        }
        let mut iface_list: *mut WLAN_INTERFACE_INFO_LIST = std::ptr::null_mut();
        if WlanEnumInterfaces(handle, None, &mut iface_list) == 0 && !iface_list.is_null() {
            let count = (*iface_list).dwNumberOfItems as usize;
            let ifaces: &[WLAN_INTERFACE_INFO] =
                std::slice::from_raw_parts((*iface_list).InterfaceInfo.as_ptr(), count);
            for iface in ifaces {
                let mut net_list: *mut WLAN_AVAILABLE_NETWORK_LIST = std::ptr::null_mut();
                let err = WlanGetAvailableNetworkList(
                    handle,
                    &iface.InterfaceGuid,
                    0,
                    None,
                    &mut net_list,
                );
                if err == 0 && !net_list.is_null() {
                    scan_succeeded = true;
                    let n_count = (*net_list).dwNumberOfItems as usize;
                    let nets: &[WLAN_AVAILABLE_NETWORK] =
                        std::slice::from_raw_parts((*net_list).Network.as_ptr(), n_count);
                    for net in nets {
                        let ssid_len = (net.dot11Ssid.uSSIDLength as usize).min(32);
                        if ssid_len == 0 {
                            continue;
                        }
                        let ssid = String::from_utf8_lossy(&net.dot11Ssid.ucSSID[..ssid_len])
                            .trim_matches('\0')
                            .to_string();
                        if ssid.is_empty() {
                            continue;
                        }
                        let profile_name = utf16_z_to_string(&net.strProfileName);
                        collected.push(WifiNetworkItem {
                            ssid,
                            profile_name,
                            signal_quality: (net.wlanSignalQuality as u8).min(100),
                            is_connected: (net.dwFlags & WLAN_AVAILABLE_NETWORK_CONNECTED) != 0,
                            has_profile: (net.dwFlags & WLAN_AVAILABLE_NETWORK_HAS_PROFILE) != 0,
                            is_secured: net.bSecurityEnabled.as_bool(),
                        });
                    }
                    WlanFreeMemory(net_list.cast());
                }
            }

            if !scan_succeeded {
                for iface in ifaces {
                    let mut prof_list: *mut WLAN_PROFILE_INFO_LIST = std::ptr::null_mut();
                    if WlanGetProfileList(handle, &iface.InterfaceGuid, None, &mut prof_list) == 0
                        && !prof_list.is_null()
                    {
                        let p_count = (*prof_list).dwNumberOfItems as usize;
                        let profiles: &[WLAN_PROFILE_INFO] =
                            std::slice::from_raw_parts((*prof_list).ProfileInfo.as_ptr(), p_count);
                        for prof in profiles {
                            let prof_name = utf16_z_to_string(&prof.strProfileName);
                            if prof_name.is_empty() {
                                continue;
                            }
                            collected.push(WifiNetworkItem {
                                ssid: prof_name.clone(),
                                profile_name: prof_name,
                                signal_quality: 0,
                                is_connected: false,
                                has_profile: true,
                                is_secured: true,
                            });
                        }
                        WlanFreeMemory(prof_list.cast());
                    }
                }
            }

            WlanFreeMemory(iface_list.cast());
        }
        let _ = WlanCloseHandle(handle, None);
    }

    dedup_and_sort_wifi(collected)
}

#[cfg(windows)]
fn wlan_disconnect_all() -> bool {
    let mut ok = false;
    unsafe {
        let mut neg_ver = 0u32;
        let mut handle = HANDLE::default();
        if WlanOpenHandle(2, None, &mut neg_ver, &mut handle) != 0 || handle.is_invalid() {
            return false;
        }
        let mut iface_list: *mut WLAN_INTERFACE_INFO_LIST = std::ptr::null_mut();
        if WlanEnumInterfaces(handle, None, &mut iface_list) == 0 && !iface_list.is_null() {
            let count = (*iface_list).dwNumberOfItems as usize;
            let ifaces: &[WLAN_INTERFACE_INFO] =
                std::slice::from_raw_parts((*iface_list).InterfaceInfo.as_ptr(), count);
            for iface in ifaces {
                if WlanDisconnect(handle, &iface.InterfaceGuid, None) == 0 {
                    ok = true;
                }
            }
            WlanFreeMemory(iface_list.cast());
        }
        let _ = WlanCloseHandle(handle, None);
    }
    ok
}

#[cfg(windows)]
fn wlan_connect_profile(target_ssid: &str, preferred_profile: &str) -> bool {
    unsafe {
        let mut neg_ver = 0u32;
        let mut handle = HANDLE::default();
        if WlanOpenHandle(2, None, &mut neg_ver, &mut handle) != 0 || handle.is_invalid() {
            return false;
        }
        let mut connected_ok = false;
        let mut iface_list: *mut WLAN_INTERFACE_INFO_LIST = std::ptr::null_mut();
        if WlanEnumInterfaces(handle, None, &mut iface_list) == 0 && !iface_list.is_null() {
            let count = (*iface_list).dwNumberOfItems as usize;
            let ifaces: &[WLAN_INTERFACE_INFO] =
                std::slice::from_raw_parts((*iface_list).InterfaceInfo.as_ptr(), count);

            'iface_loop: for iface in ifaces {
                let mut net_list: *mut WLAN_AVAILABLE_NETWORK_LIST = std::ptr::null_mut();
                if WlanGetAvailableNetworkList(handle, &iface.InterfaceGuid, 0, None, &mut net_list)
                    != 0
                    || net_list.is_null()
                {
                    continue;
                }
                let n_count = (*net_list).dwNumberOfItems as usize;
                let nets: &[WLAN_AVAILABLE_NETWORK] =
                    std::slice::from_raw_parts((*net_list).Network.as_ptr(), n_count);
                for net in nets {
                    if (net.dwFlags & WLAN_AVAILABLE_NETWORK_HAS_PROFILE) == 0 {
                        continue;
                    }
                    let ssid_len = (net.dot11Ssid.uSSIDLength as usize).min(32);
                    let ssid = String::from_utf8_lossy(&net.dot11Ssid.ucSSID[..ssid_len])
                        .trim_matches('\0')
                        .to_string();
                    let prof = utf16_z_to_string(&net.strProfileName);
                    let matches = (!preferred_profile.is_empty() && prof == preferred_profile)
                        || (ssid == target_ssid && !prof.is_empty());
                    if matches {
                        let prof_h = HSTRING::from(prof);
                        let params = WLAN_CONNECTION_PARAMETERS {
                            wlanConnectionMode: wlan_connection_mode_profile,
                            strProfile: PCWSTR(prof_h.as_ptr()),
                            pDot11Ssid: std::ptr::null_mut(),
                            pDesiredBssidList: std::ptr::null_mut(),
                            dot11BssType: net.dot11BssType,
                            dwFlags: 0,
                        };
                        if WlanConnect(handle, &iface.InterfaceGuid, &params, None) == 0 {
                            connected_ok = true;
                        }
                        WlanFreeMemory(net_list.cast());
                        break 'iface_loop;
                    }
                }
                WlanFreeMemory(net_list.cast());
            }
            if !connected_ok {
                let fallback_prof = if !preferred_profile.is_empty() {
                    preferred_profile
                } else {
                    target_ssid
                };
                if !fallback_prof.is_empty() {
                    let prof_h = HSTRING::from(fallback_prof);
                    let params = WLAN_CONNECTION_PARAMETERS {
                        wlanConnectionMode: wlan_connection_mode_profile,
                        strProfile: PCWSTR(prof_h.as_ptr()),
                        pDot11Ssid: std::ptr::null_mut(),
                        pDesiredBssidList: std::ptr::null_mut(),
                        dot11BssType: dot11_BSS_type_any,
                        dwFlags: 0,
                    };
                    for iface in ifaces {
                        if WlanConnect(handle, &iface.InterfaceGuid, &params, None) == 0 {
                            connected_ok = true;
                            break;
                        }
                    }
                }
            }
            WlanFreeMemory(iface_list.cast());
        }
        let _ = WlanCloseHandle(handle, None);
        connected_ok
    }
}

#[cfg(windows)]
fn enumerate_classic_bt_devices() -> Vec<BluetoothDeviceItem> {
    let mut list = Vec::new();
    unsafe {
        let params = BLUETOOTH_DEVICE_SEARCH_PARAMS {
            dwSize: std::mem::size_of::<BLUETOOTH_DEVICE_SEARCH_PARAMS>() as u32,
            fReturnAuthenticated: BOOL(1),
            fReturnRemembered: BOOL(1),
            fReturnUnknown: BOOL(0),
            fReturnConnected: BOOL(1),
            fIssueInquiry: BOOL(0),
            cTimeoutMultiplier: 0,
            hRadio: HANDLE::default(),
        };
        let mut info = BLUETOOTH_DEVICE_INFO {
            dwSize: std::mem::size_of::<BLUETOOTH_DEVICE_INFO>() as u32,
            ..Default::default()
        };
        let Ok(hfind) = BluetoothFindFirstDevice(&params, &mut info) else {
            return list;
        };
        loop {
            let name = utf16_z_to_string(&info.szName);
            let addr = info.Address.Anonymous.ullLong;
            if !name.is_empty() {
                list.push(BluetoothDeviceItem {
                    name,
                    address: addr,
                    device_id: format!("{addr:012X}"),
                    kind: BtDeviceKind::Classic,
                    is_connected: info.fConnected.as_bool(),
                });
            }
            info = BLUETOOTH_DEVICE_INFO {
                dwSize: std::mem::size_of::<BLUETOOTH_DEVICE_INFO>() as u32,
                ..Default::default()
            };
            if BluetoothFindNextDevice(hfind, &mut info).is_err() {
                break;
            }
        }
        let _ = BluetoothFindDeviceClose(hfind);
    }
    list
}

#[cfg(windows)]
fn enumerate_ble_bt_devices() -> Vec<BluetoothDeviceItem> {
    let mut connected_ids = HashSet::new();
    if let Ok(conn_sel) =
        BluetoothLEDevice::GetDeviceSelectorFromConnectionStatus(BluetoothConnectionStatus::Connected)
    {
        if let Ok(op) = DeviceInformation::FindAllAsyncAqsFilter(&conn_sel) {
            if let Ok(col) = op.get() {
                let n = col.Size().unwrap_or(0);
                for i in 0..n {
                    if let Ok(dev) = col.GetAt(i) {
                        if let Ok(id) = dev.Id() {
                            connected_ids.insert(id.to_string_lossy());
                        }
                    }
                }
            }
        }
    }

    let mut list = Vec::new();
    let Ok(paired_sel) = BluetoothLEDevice::GetDeviceSelectorFromPairingState(true) else {
        return list;
    };
    let Ok(op) = DeviceInformation::FindAllAsyncAqsFilter(&paired_sel) else {
        return list;
    };
    let Ok(col) = op.get() else {
        return list;
    };
    let n = col.Size().unwrap_or(0);
    for i in 0..n {
        let Ok(dev) = col.GetAt(i) else {
            continue;
        };
        let name = dev.Name().map(|s| s.to_string_lossy()).unwrap_or_default();
        let id = dev.Id().map(|s| s.to_string_lossy()).unwrap_or_default();
        if name.trim().is_empty() {
            continue;
        }
        let addr = parse_ble_id_address(&id);
        let is_connected = connected_ids.contains(&id);
        list.push(BluetoothDeviceItem {
            name,
            address: addr,
            device_id: id,
            kind: BtDeviceKind::Ble,
            is_connected,
        });
    }
    list
}

#[cfg(windows)]
fn toggle_classic_bt_device(target_addr: u64, connect: bool) -> bool {
    const STANDARD_PROFILE_GUIDS: [GUID; 4] = [
        // AudioSink (A2DP)
        GUID::from_u128(0x0000110b_0000_1000_8000_00805f9b34fb),
        // Handsfree
        GUID::from_u128(0x0000111e_0000_1000_8000_00805f9b34fb),
        // Headset
        GUID::from_u128(0x00001108_0000_1000_8000_00805f9b34fb),
        // HumanInterfaceDeviceService
        GUID::from_u128(0x00001124_0000_1000_8000_00805f9b34fb),
    ];

    unsafe {
        let params = BLUETOOTH_DEVICE_SEARCH_PARAMS {
            dwSize: std::mem::size_of::<BLUETOOTH_DEVICE_SEARCH_PARAMS>() as u32,
            fReturnAuthenticated: BOOL(1),
            fReturnRemembered: BOOL(1),
            fReturnUnknown: BOOL(0),
            fReturnConnected: BOOL(1),
            fIssueInquiry: BOOL(0),
            cTimeoutMultiplier: 0,
            hRadio: HANDLE::default(),
        };
        let mut target_info: Option<BLUETOOTH_DEVICE_INFO> = None;
        let mut info = BLUETOOTH_DEVICE_INFO {
            dwSize: std::mem::size_of::<BLUETOOTH_DEVICE_INFO>() as u32,
            ..Default::default()
        };
        if let Ok(hfind) = BluetoothFindFirstDevice(&params, &mut info) {
            loop {
                if info.Address.Anonymous.ullLong == target_addr {
                    target_info = Some(info);
                    break;
                }
                info = BLUETOOTH_DEVICE_INFO {
                    dwSize: std::mem::size_of::<BLUETOOTH_DEVICE_INFO>() as u32,
                    ..Default::default()
                };
                if BluetoothFindNextDevice(hfind, &mut info).is_err() {
                    break;
                }
            }
            let _ = BluetoothFindDeviceClose(hfind);
        }

        let Some(dev_info) = target_info else {
            return false;
        };

        let mut hradio = HANDLE::default();
        let radio_params = BLUETOOTH_FIND_RADIO_PARAMS {
            dwSize: std::mem::size_of::<BLUETOOTH_FIND_RADIO_PARAMS>() as u32,
        };
        let radio_find = BluetoothFindFirstRadio(&radio_params, &mut hradio).ok();

        let mut service_guids = vec![GUID::zeroed(); 16];
        let mut svc_count = service_guids.len() as u32;
        let enum_res = BluetoothEnumerateInstalledServices(
            hradio,
            &dev_info,
            &mut svc_count,
            Some(service_guids.as_mut_ptr()),
        );
        let guids_to_apply: Vec<GUID> = if enum_res == 0 && svc_count > 0 {
            service_guids[..(svc_count as usize).min(service_guids.len())].to_vec()
        } else {
            STANDARD_PROFILE_GUIDS.to_vec()
        };

        let flag = if connect { 1u32 } else { 0u32 };
        let mut any_ok = false;
        for guid in &guids_to_apply {
            if BluetoothSetServiceState(hradio, &dev_info, guid, flag) == 0 {
                any_ok = true;
            }
        }

        if !hradio.is_invalid() {
            let _ = CloseHandle(hradio);
        }
        if let Some(rf) = radio_find {
            let _ = BluetoothFindRadioClose(rf);
        }
        any_ok
    }
}

#[cfg(windows)]
fn utf16_z_to_string(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end]).trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ble_device_id_extracts_mac_address() {
        assert_eq!(
            parse_ble_id_address("BluetoothLE#BluetoothLEac:ed:5c:11:22:33-d4:e5:f6:aa:bb:cc"),
            0xd4e5f6aabbcc
        );
        assert_eq!(parse_ble_id_address("invalid"), 0);
    }

    #[test]
    fn wifi_dedup_keeps_connected_and_profiled_then_strongest() {
        let items = vec![
            WifiNetworkItem {
                ssid: "Office".to_string(),
                profile_name: String::new(),
                signal_quality: 75,
                is_connected: false,
                has_profile: false,
                is_secured: true,
            },
            WifiNetworkItem {
                ssid: "Office".to_string(),
                profile_name: "Office".to_string(),
                signal_quality: 68,
                is_connected: true,
                has_profile: true,
                is_secured: true,
            },
            WifiNetworkItem {
                ssid: "CafeGuest".to_string(),
                profile_name: String::new(),
                signal_quality: 92,
                is_connected: false,
                has_profile: false,
                is_secured: false,
            },
        ];
        let deduped = dedup_and_sort_wifi(items);
        assert_eq!(deduped.len(), 2);
        assert_eq!(deduped[0].ssid, "Office");
        assert!(deduped[0].is_connected);
        assert_eq!(deduped[1].ssid, "CafeGuest");
    }

    #[test]
    fn wifi_dedup_prefers_winrt_connected_over_saved_profile_duplicate() {
        let items = vec![
            WifiNetworkItem {
                ssid: "Home5G".to_string(),
                profile_name: "Home5G".to_string(),
                signal_quality: 0,
                is_connected: false,
                has_profile: true,
                is_secured: true,
            },
            WifiNetworkItem {
                ssid: "Backup".to_string(),
                profile_name: "Backup".to_string(),
                signal_quality: 0,
                is_connected: false,
                has_profile: true,
                is_secured: true,
            },
            WifiNetworkItem {
                ssid: "Home5G".to_string(),
                profile_name: "Home5G".to_string(),
                signal_quality: 80,
                is_connected: true,
                has_profile: true,
                is_secured: true,
            },
        ];
        let deduped = dedup_and_sort_wifi(items);
        assert_eq!(deduped.len(), 2);
        assert_eq!(deduped[0].ssid, "Home5G");
        assert!(deduped[0].is_connected);
        assert_eq!(deduped[0].signal_quality, 80);
        assert_eq!(deduped[1].ssid, "Backup");
        assert!(!deduped[1].is_connected);
        assert_eq!(deduped[1].signal_quality, 0);
    }

    #[test]
    fn bluetooth_dedup_merges_classic_and_ble_dual_mode_device() {
        let classic = vec![BluetoothDeviceItem {
            name: "Sony WH-1000XM5".to_string(),
            address: 0x112233445566,
            device_id: "112233445566".to_string(),
            kind: BtDeviceKind::Classic,
            is_connected: false,
        }];
        let ble = vec![
            BluetoothDeviceItem {
                name: "Sony WH-1000XM5".to_string(),
                address: 0x112233445566,
                device_id: "BluetoothLE#BluetoothLE00:00:00:00:00:00-11:22:33:44:55:66".to_string(),
                kind: BtDeviceKind::Ble,
                is_connected: true,
            },
            BluetoothDeviceItem {
                name: "MX Master 3S".to_string(),
                address: 0xaabbccddeeff,
                device_id: "BluetoothLE#BluetoothLE00:00:00:00:00:00-aa:bb:cc:dd:ee:ff".to_string(),
                kind: BtDeviceKind::Ble,
                is_connected: false,
            },
        ];
        let merged = dedup_and_sort_bluetooth(classic, ble);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].name, "Sony WH-1000XM5");
        assert!(merged[0].is_connected);
        assert_eq!(merged[0].kind, BtDeviceKind::Classic);
    }
}
