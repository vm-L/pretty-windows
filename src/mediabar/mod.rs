pub mod arbitration;
pub mod config;
pub mod dsp;
pub mod layout;
pub mod marquee;
pub mod volume;

#[cfg(windows)]
pub mod app;
#[cfg(windows)]
pub mod audio;
#[cfg(windows)]
pub mod audio_devices;
#[cfg(windows)]
pub mod popup;
#[cfg(windows)]
pub mod process_util;
#[cfg(windows)]
pub mod render;
#[cfg(windows)]
pub mod smtc;
#[cfg(windows)]
pub mod taskbar;

#[cfg(windows)]
pub use app::MediaBarInstance;
