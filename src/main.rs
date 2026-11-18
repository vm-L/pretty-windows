#![cfg_attr(all(windows, not(test)), windows_subsystem = "windows")]

mod arbitration;
mod color;
mod config;
mod dsp;
mod layout;
mod marquee;
mod volume;

#[cfg(windows)]
mod app;
#[cfg(windows)]
mod audio;
#[cfg(windows)]
mod audio_devices;
#[cfg(windows)]
mod popup;
#[cfg(windows)]
mod process_util;
#[cfg(windows)]
mod render;
#[cfg(windows)]
mod smtc;
#[cfg(windows)]
mod taskbar;

#[cfg(windows)]
fn main() {
    use windows::core::w;
    use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS};
    use windows::Win32::System::Com::{
        CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::System::Threading::CreateMutexW;
    use windows::Win32::UI::HiDpi::{
        SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
    };

    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);

        let Ok(mutex) = CreateMutexW(None, false, w!("Local\\MediaBar_SingleInstance_Mutex"))
        else {
            return;
        };
        if GetLastError() == ERROR_ALREADY_EXISTS {
            let _ = CloseHandle(mutex);
            return;
        }

        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let _ = app::run();
        CoUninitialize();
        let _ = CloseHandle(mutex);
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("MediaBar is a Windows 11 application.");
}
