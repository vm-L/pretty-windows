//! Asynchronous, debounced `ReadDirectoryChangesW` directory watcher for Folder Portal boxes.

use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::thread::{self, JoinHandle};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{
    CloseHandle, ERROR_IO_PENDING, HANDLE, HWND, LPARAM, WAIT_OBJECT_0, WPARAM,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, ReadDirectoryChangesW, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OVERLAPPED,
    FILE_LIST_DIRECTORY, FILE_NOTIFY_CHANGE_DIR_NAME, FILE_NOTIFY_CHANGE_FILE_NAME,
    FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::Threading::{
    CreateEventW, SetEvent, WaitForMultipleObjects, WaitForSingleObject, INFINITE,
};
use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
use windows::Win32::UI::WindowsAndMessaging::{IsWindow, PostMessageW};

const DEBOUNCE_MS: u32 = 180;

struct SendHandle(HANDLE);
unsafe impl Send for SendHandle {}

struct SendHwnd(HWND);
unsafe impl Send for SendHwnd {}

/// Watches one directory subtree non-recursively on a background thread and posts
/// `notify_msg(WPARAM(watch_gen), 0)` after coalescing burst filesystem events.
pub struct FolderWatcher {
    stop_event: HANDLE,
    dir_handle: HANDLE,
    worker: Option<JoinHandle<()>>,
}

impl FolderWatcher {
    pub fn start(dir: &Path, notify_hwnd: HWND, notify_msg: u32, watch_gen: usize) -> Option<Self> {
        let mut wide: Vec<u16> = dir.as_os_str().encode_wide().collect();
        wide.push(0);

        unsafe {
            let dir_handle = CreateFileW(
                PCWSTR(wide.as_ptr()),
                FILE_LIST_DIRECTORY.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                None,
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OVERLAPPED,
                None,
            )
            .ok()?;

            let Ok(stop_event) = CreateEventW(None, true, false, None) else {
                let _ = CloseHandle(dir_handle);
                return None;
            };
            let Ok(io_event) = CreateEventW(None, true, false, None) else {
                let _ = CloseHandle(stop_event);
                let _ = CloseHandle(dir_handle);
                return None;
            };

            let s_dir = SendHandle(dir_handle);
            let s_stop = SendHandle(stop_event);
            let s_io = SendHandle(io_event);
            let s_hwnd = SendHwnd(notify_hwnd);

            let worker = thread::Builder::new()
                .name("DeskBoxDirWatch".into())
                .spawn(move || {
                    watch_loop(s_dir, s_stop, s_io, s_hwnd, notify_msg, watch_gen);
                })
                .ok();

            let Some(handle) = worker else {
                let _ = CloseHandle(io_event);
                let _ = CloseHandle(stop_event);
                let _ = CloseHandle(dir_handle);
                return None;
            };

            Some(Self {
                stop_event,
                dir_handle,
                worker: Some(handle),
            })
        }
    }
}

impl Drop for FolderWatcher {
    fn drop(&mut self) {
        unsafe {
            let _ = SetEvent(self.stop_event);
            let _ = CancelIoEx(self.dir_handle, None);
        }
        if let Some(handle) = self.worker.take() {
            let _ = handle.join();
        }
        unsafe {
            let _ = CloseHandle(self.stop_event);
            let _ = CloseHandle(self.dir_handle);
        }
    }
}

fn watch_loop(
    s_dir: SendHandle,
    s_stop: SendHandle,
    s_io: SendHandle,
    s_hwnd: SendHwnd,
    notify_msg: u32,
    watch_gen: usize,
) {
    let dir_handle = s_dir.0;
    let stop_event = s_stop.0;
    let io_event = s_io.0;
    let notify_hwnd = s_hwnd.0;

    let mut buffer = [0u32; 2048];
    let byte_len = (buffer.len() * size_of::<u32>()) as u32;
    // Only watch structural directory changes (create/delete/rename) so background log/temp writes
    // inside linked folders do not trigger constant full-directory rescans and UI redraws.
    let notify_filter = FILE_NOTIFY_CHANGE_FILE_NAME | FILE_NOTIFY_CHANGE_DIR_NAME;

    let wait_handles = [stop_event, io_event];

    unsafe {
        loop {
            let _ = windows::Win32::System::Threading::ResetEvent(io_event);
            let mut overlapped = OVERLAPPED {
                hEvent: io_event,
                ..Default::default()
            };
            let mut bytes_returned = 0u32;
            let ok = ReadDirectoryChangesW(
                dir_handle,
                buffer.as_mut_ptr().cast(),
                byte_len,
                false,
                notify_filter,
                Some(&mut bytes_returned),
                Some(&mut overlapped),
                None,
            )
            .is_ok();

            if !ok {
                let err = windows::Win32::Foundation::GetLastError();
                if err != ERROR_IO_PENDING {
                    break;
                }
            }

            let wait = WaitForMultipleObjects(&wait_handles, false, INFINITE);
            if wait == WAIT_OBJECT_0 || wait.0 != WAIT_OBJECT_0.0 + 1 {
                // Cancel the in-flight overlapped request and block until the kernel finishes
                // touching `overlapped` and `buffer` before unwinding this stack frame.
                let _ = CancelIoEx(dir_handle, Some(&overlapped));
                let mut cancelled_bytes = 0u32;
                let _ = GetOverlappedResult(dir_handle, &overlapped, &mut cancelled_bytes, true);
                break;
            }

            let mut transferred = 0u32;
            if GetOverlappedResult(dir_handle, &overlapped, &mut transferred, true).is_err() {
                break;
            }

            // Coalesce burst file operations cleanly before waking UI thread
            let debounce = WaitForSingleObject(stop_event, DEBOUNCE_MS);
            if debounce == WAIT_OBJECT_0 {
                break;
            }

            if !notify_hwnd.is_invalid() && IsWindow(notify_hwnd).as_bool() {
                let _ = PostMessageW(notify_hwnd, notify_msg, WPARAM(watch_gen), LPARAM(0));
            }
        }
        let _ = CloseHandle(io_event);
    }
}
