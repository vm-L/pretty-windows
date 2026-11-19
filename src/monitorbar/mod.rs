//! MonitorBar: right-aligned taskbar performance monitor widget with interactive controls.

pub mod brightness;
pub mod config;
pub mod layout;
pub mod metrics;
pub mod wireless;

#[cfg(windows)]
pub mod actions;
#[cfg(windows)]
mod app;
#[cfg(windows)]
mod popup;
#[cfg(windows)]
mod render;
#[cfg(windows)]
mod taskbar;

#[cfg(windows)]
pub use app::MonitorBarInstance;

#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::WM_APP;

/// Reserved `WM_APP` message IDs (`+40..+49`) internal to MonitorBar components.
#[cfg(windows)]
pub(crate) const WM_APP_MONITOR_METRICS_UPDATED: u32 = WM_APP + 40;
#[cfg(windows)]
pub(crate) const WM_APP_MONITOR_WIRELESS_UPDATED: u32 = WM_APP + 41;
#[cfg(windows)]
pub(crate) const WM_APP_MONITOR_POPUP_CLOSED: u32 = WM_APP + 42;
#[cfg(windows)]
pub(crate) const WM_APP_MONITOR_CONFIG_CHANGED: u32 = WM_APP + 43;
#[cfg(windows)]
pub(crate) const WM_APP_MONITOR_TOGGLE_LOCK: u32 = WM_APP + 44;
#[cfg(windows)]
pub(crate) const WM_APP_MONITOR_RESET_POS: u32 = WM_APP + 45;
#[cfg(windows)]
pub(crate) const WM_APP_MONITOR_PICK_CUSTOM_APP: u32 = WM_APP + 46;
#[cfg(windows)]
pub(crate) const WM_APP_MONITOR_CLOSE_SELF: u32 = WM_APP + 47;
#[cfg(windows)]
pub(crate) const WM_APP_MONITOR_BRIGHTNESS_UPDATED: u32 = WM_APP + 48;
