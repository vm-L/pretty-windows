#![cfg_attr(all(windows, not(test)), windows_subsystem = "windows")]

mod color;
mod mediabar;
mod monitorbar;
mod system;
mod taskbar;
mod tray;
mod ui;

#[cfg(windows)]
fn main() {
    use windows::core::w;
    use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS};
    use windows::Win32::System::Com::{
        CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::System::LibraryLoader::{
        SetDefaultDllDirectories, LOAD_LIBRARY_SEARCH_SYSTEM32,
    };
    use windows::Win32::System::Threading::CreateMutexW;
    use windows::Win32::UI::HiDpi::{
        SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
    };

    unsafe {
        let _ = SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_SYSTEM32);
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);

        let Ok(mutex) = CreateMutexW(None, false, w!("Local\\PrettyWindows_SingleInstance_Mutex"))
        else {
            return;
        };
        if GetLastError() == ERROR_ALREADY_EXISTS {
            let _ = CloseHandle(mutex);
            return;
        }

        let com_ok = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
        let _ = tray::run();
        if com_ok {
            CoUninitialize();
        }
        let _ = CloseHandle(mutex);
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("Pretty Windows is a Windows 11 application.");
}
