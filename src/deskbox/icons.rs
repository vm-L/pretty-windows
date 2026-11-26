//! Asynchronous pure-system-icon extractor and shared LRU cache (no content thumbnails).

use super::config::normalize_path_key;
use std::collections::{HashMap, HashSet, VecDeque};
use std::mem::size_of;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_PIXEL_FORMAT, D2D_SIZE_U,
};
use windows::Win32::Graphics::Direct2D::{
    ID2D1Bitmap, ID2D1DCRenderTarget, D2D1_BITMAP_PROPERTIES,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDIBits, GetObjectW,
    SelectObject, BITMAP, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HGDIOBJ,
};
use windows::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL};
use windows::Win32::System::Com::{
    CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED,
};
use windows::Win32::UI::Controls::{IImageList, ILD_TRANSPARENT};
use windows::Win32::UI::Shell::{
    SHGetFileInfoW, SHGetImageList, SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON, SHGFI_SYSICONINDEX,
    SHGFI_USEFILEATTRIBUTES, SHIL_EXTRALARGE, SHIL_JUMBO,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DestroyIcon, DrawIconEx, GetIconInfo, IsWindow, PostMessageW, DI_NORMAL, HICON, ICONINFO,
};

const MAX_DECODED_CACHE_ENTRIES: usize = 768;
const MAX_RT_ICON_BITMAPS: usize = 512;
const MAX_PENDING_ICON_JOBS: usize = 128;
const ICON_NOTIFY_BATCH_SIZE: usize = 8;
const ICON_NOTIFY_FLUSH_MS: u64 = 24;

/// Premultiplied BGRA8 system icon bitmap shared across DeskBox windows.
#[derive(Clone, Debug)]
pub struct DecodedIcon {
    pub width: u32,
    pub height: u32,
    pub pbgra: Vec<u8>,
}

struct IconJob {
    key: String,
    path: PathBuf,
    jumbo: bool,
}

struct IconResult {
    key: String,
    icon: Option<DecodedIcon>,
}

#[derive(Default)]
struct IconQueueState {
    /// Bounded LIFO stack so the most recently visible scrolled items decode first.
    stack: VecDeque<IconJob>,
    /// Keys currently queued or being extracted -> all subscribed `(notify_hwnd, notify_msg)` targets.
    pending_targets: HashMap<String, Vec<(isize, u32)>>,
    shutdown: bool,
}

/// Shared asynchronous system-icon extraction service owned by `DeskBoxController`.
pub struct IconService {
    queue: Arc<(Mutex<IconQueueState>, Condvar)>,
    result_rx: Receiver<IconResult>,
    worker: Option<JoinHandle<()>>,
    decoded: HashMap<String, (u64, Option<Arc<DecodedIcon>>)>,
    lru_clock: u64,
}

impl IconService {
    pub fn new() -> Self {
        let queue = Arc::new((Mutex::new(IconQueueState::default()), Condvar::new()));
        let (result_tx, result_rx) = mpsc::channel::<IconResult>();
        let worker_queue = Arc::clone(&queue);
        let worker = thread::Builder::new()
            .name("DeskBoxIcons".into())
            .spawn(move || icon_worker_loop(worker_queue, result_tx))
            .ok();

        Self {
            queue,
            result_rx,
            worker,
            decoded: HashMap::new(),
            lru_clock: 0,
        }
    }

    /// Computes both standard HD (`L|...`) and jumbo (`J|...`) cache keys from a single path pass.
    pub fn cache_key_pair(path: &Path) -> (String, String) {
        let norm = normalize_path_key(&path.to_string_lossy());
        (format!("L|{norm}"), format!("J|{norm}"))
    }

    /// Drains newly completed worker results into the shared decoded cache; returns `true` if any arrived.
    pub fn drain_completed(&mut self) -> bool {
        let mut any = false;
        while let Ok(res) = self.result_rx.try_recv() {
            if res.icon.is_some() {
                any = true;
            }
            self.insert_decoded(res.key, res.icon.map(Arc::new));
        }
        any
    }

    /// Returns a cached `DecodedIcon` by precomputed `(key, jumbo)` or schedules viewport-priority
    /// background extraction once for `notify_hwnd`, avoiding per-frame path normalization.
    pub fn request_or_get_by_key(
        &mut self,
        key: &str,
        jumbo: bool,
        path: &Path,
        notify_hwnd: HWND,
        notify_msg: u32,
    ) -> Option<Arc<DecodedIcon>> {
        if let Some((stamp, cached)) = self.decoded.get_mut(key) {
            self.lru_clock = self.lru_clock.wrapping_add(1);
            *stamp = self.lru_clock;
            return cached.as_ref().map(Arc::clone);
        }

        self.worker.as_ref()?;

        let (lock, cvar) = &*self.queue;
        if let Ok(mut state) = lock.lock() {
            let target = (notify_hwnd.0 as isize, notify_msg);
            if let Some(subs) = state.pending_targets.get_mut(key) {
                if !subs.contains(&target) {
                    subs.push(target);
                }
            } else {
                while state.stack.len() >= MAX_PENDING_ICON_JOBS {
                    if let Some(evicted) = state.stack.pop_front() {
                        state.pending_targets.remove(&evicted.key);
                    }
                }
                let owned_key = key.to_string();
                state
                    .pending_targets
                    .insert(owned_key.clone(), vec![target]);
                state.stack.push_back(IconJob {
                    key: owned_key,
                    path: path.to_path_buf(),
                    jumbo,
                });
                cvar.notify_one();
            }
        }
        None
    }

    fn insert_decoded(&mut self, key: String, icon: Option<Arc<DecodedIcon>>) {
        if !self.decoded.contains_key(&key) && self.decoded.len() >= MAX_DECODED_CACHE_ENTRIES {
            evict_oldest_quarter(&mut self.decoded, MAX_DECODED_CACHE_ENTRIES / 4);
        }
        self.lru_clock = self.lru_clock.wrapping_add(1);
        self.decoded.insert(key, (self.lru_clock, icon));
    }
}

impl Drop for IconService {
    fn drop(&mut self) {
        let (lock, cvar) = &*self.queue;
        if let Ok(mut state) = lock.lock() {
            state.shutdown = true;
            state.stack.clear();
            cvar.notify_all();
        }
        if let Some(handle) = self.worker.take() {
            let _ = handle.join();
        }
    }
}

/// Per-`ID2D1DCRenderTarget` GPU bitmap mirror backed by shared `DecodedIcon` bytes with true LRU eviction.
#[derive(Default)]
pub struct RtIconBitmaps {
    map: HashMap<String, (u64, ID2D1Bitmap)>,
    clock: u64,
}

impl RtIconBitmaps {
    pub fn clear(&mut self) {
        self.map.clear();
        self.clock = 0;
    }

    /// Fast O(1) lookup for already-uploaded per-RT icon bitmaps without touching `IconService`.
    pub fn get_cached(&mut self, key: &str) -> Option<ID2D1Bitmap> {
        if let Some((stamp, existing)) = self.map.get_mut(key) {
            self.clock = self.clock.wrapping_add(1);
            *stamp = self.clock;
            return Some(existing.clone());
        }
        None
    }

    pub fn get_or_upload(
        &mut self,
        rt: &ID2D1DCRenderTarget,
        key: &str,
        decoded: &DecodedIcon,
    ) -> Option<ID2D1Bitmap> {
        if let Some((stamp, existing)) = self.map.get_mut(key) {
            self.clock = self.clock.wrapping_add(1);
            *stamp = self.clock;
            return Some(existing.clone());
        }
        if self.map.len() >= MAX_RT_ICON_BITMAPS {
            evict_oldest_quarter(&mut self.map, MAX_RT_ICON_BITMAPS / 4);
        }
        unsafe {
            let props = D2D1_BITMAP_PROPERTIES {
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: 96.0,
                dpiY: 96.0,
            };
            let bmp = rt
                .CreateBitmap(
                    D2D_SIZE_U {
                        width: decoded.width,
                        height: decoded.height,
                    },
                    Some(decoded.pbgra.as_ptr().cast()),
                    decoded.width * 4,
                    &props,
                )
                .ok()?;
            self.clock = self.clock.wrapping_add(1);
            self.map.insert(key.to_string(), (self.clock, bmp.clone()));
            Some(bmp)
        }
    }
}

fn evict_oldest_quarter<V>(map: &mut HashMap<String, (u64, V)>, count: usize) {
    if count == 0 || map.is_empty() {
        return;
    }
    let mut entries: Vec<(u64, String)> = map
        .iter()
        .map(|(k, (stamp, _))| (*stamp, k.clone()))
        .collect();
    let k = count.min(entries.len().saturating_sub(1));
    if k == 0 {
        return;
    }
    entries.select_nth_unstable_by_key(k, |(stamp, _)| *stamp);
    for (_, key) in entries.into_iter().take(k) {
        map.remove(&key);
    }
}

fn icon_worker_loop(
    queue: Arc<(Mutex<IconQueueState>, Condvar)>,
    tx: Sender<IconResult>,
) {
    unsafe {
        let com_ok = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
        let (lock, cvar) = &*queue;
        let mut dirty_targets: HashSet<(isize, u32)> = HashSet::new();
        let mut batch_decoded = 0usize;
        let mut last_flush = Instant::now();

        let flush_notifications = |targets: &mut HashSet<(isize, u32)>| {
            for (hwnd_raw, msg) in targets.drain() {
                let notify_hwnd = HWND(hwnd_raw as *mut core::ffi::c_void);
                if !notify_hwnd.is_invalid() && IsWindow(notify_hwnd).as_bool() {
                    let _ = PostMessageW(notify_hwnd, msg, WPARAM(0), LPARAM(0));
                }
            }
        };

        let mut sys_icon_cache: HashMap<(bool, i32), DecodedIcon> = HashMap::new();
        let extralarge_list = SHGetImageList::<IImageList>(SHIL_EXTRALARGE as i32).ok();
        let jumbo_list = SHGetImageList::<IImageList>(SHIL_JUMBO as i32).ok();

        'outer: loop {
            let job = {
                let Ok(mut state) = lock.lock() else {
                    break 'outer;
                };
                while state.stack.is_empty() && !state.shutdown {
                    if !dirty_targets.is_empty() {
                        drop(state);
                        flush_notifications(&mut dirty_targets);
                        batch_decoded = 0;
                        last_flush = Instant::now();
                        let Ok(relocked) = lock.lock() else {
                            break 'outer;
                        };
                        state = relocked;
                        if !state.stack.is_empty() || state.shutdown {
                            break;
                        }
                    }
                    let Ok(next) = cvar.wait(state) else {
                        break 'outer;
                    };
                    state = next;
                }
                if state.shutdown {
                    break 'outer;
                }
                state.stack.pop_back()
            };

            let Some(job) = job else {
                continue;
            };

            let extracted = extract_system_icon(
                &job.path,
                job.jumbo,
                extralarge_list.as_ref(),
                jumbo_list.as_ref(),
                &mut sys_icon_cache,
            );
            let decoded_ok = extracted.is_some();
            let _ = tx.send(IconResult {
                key: job.key.clone(),
                icon: extracted,
            });

            let (targets, queue_empty) = if let Ok(mut state) = lock.lock() {
                let subs = state.pending_targets.remove(&job.key).unwrap_or_default();
                (subs, state.stack.is_empty())
            } else {
                (Vec::new(), true)
            };

            if decoded_ok {
                for target in targets {
                    dirty_targets.insert(target);
                }
                batch_decoded += 1;
            }

            if !dirty_targets.is_empty()
                && (queue_empty
                    || batch_decoded >= ICON_NOTIFY_BATCH_SIZE
                    || last_flush.elapsed() >= Duration::from_millis(ICON_NOTIFY_FLUSH_MS))
            {
                flush_notifications(&mut dirty_targets);
                batch_decoded = 0;
                last_flush = Instant::now();
            }
        }
        drop(jumbo_list);
        drop(extralarge_list);
        if com_ok {
            CoUninitialize();
        }
    }
}

fn can_use_extension_icon_fast_path(path: &Path) -> bool {
    let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
        return false;
    };
    if ext.is_empty() {
        return false;
    }
    !matches!(
        ext.to_ascii_lowercase().as_str(),
        "exe" | "lnk" | "ico" | "cur" | "ani" | "url" | "msi" | "scr" | "cpl" | "dll" | "ocx"
            | "pif" | "appref-ms" | "theme" | "deskthemepack"
    )
}

unsafe fn extract_system_icon(
    path: &Path,
    jumbo: bool,
    extralarge_list: Option<&IImageList>,
    jumbo_list: Option<&IImageList>,
    sys_icon_cache: &mut HashMap<(bool, i32), DecodedIcon>,
) -> Option<DecodedIcon> {
    let mut wide_path: Vec<u16> = path.as_os_str().encode_wide().collect();
    wide_path.push(0);

    let mut sfi = SHFILEINFOW::default();
    let mut exists_cached: Option<bool> = None;

    // Fast path for regular extension-typed files (e.g. .conf/.log/.txt/.json/.png): query by
    // file attributes without touching disk or triggering per-file shell/AV I/O, and reuse cached
    // rasterized bitmaps across files sharing the same system image list index `iIcon`.
    let (got_idx, fallback_hicon) = if can_use_extension_icon_fast_path(path)
        && !path.is_dir()
    {
        let ok = SHGetFileInfoW(
            PCWSTR(wide_path.as_ptr()),
            FILE_ATTRIBUTE_NORMAL,
            Some(&mut sfi),
            size_of::<SHFILEINFOW>() as u32,
            SHGFI_SYSICONINDEX | SHGFI_USEFILEATTRIBUTES,
        ) != 0;
        (ok, None)
    } else {
        let exists = std::fs::symlink_metadata(path).is_ok();
        exists_cached = Some(exists);
        if exists {
            let ok = SHGetFileInfoW(
                PCWSTR(wide_path.as_ptr()),
                FILE_ATTRIBUTE_NORMAL,
                Some(&mut sfi),
                size_of::<SHFILEINFOW>() as u32,
                SHGFI_SYSICONINDEX,
            ) != 0;
            (ok, None)
        } else {
            let looks_like_dir =
                path.extension().is_none() && path.to_string_lossy().ends_with('\\');
            let attr = if looks_like_dir {
                FILE_ATTRIBUTE_DIRECTORY
            } else {
                FILE_ATTRIBUTE_NORMAL
            };
            let ok = SHGetFileInfoW(
                PCWSTR(wide_path.as_ptr()),
                attr,
                Some(&mut sfi),
                size_of::<SHFILEINFOW>() as u32,
                SHGFI_SYSICONINDEX | SHGFI_USEFILEATTRIBUTES,
            ) != 0;
            if !ok {
                let mut sfi_ic = SHFILEINFOW::default();
                let got_ic = SHGetFileInfoW(
                    PCWSTR(wide_path.as_ptr()),
                    attr,
                    Some(&mut sfi_ic),
                    size_of::<SHFILEINFOW>() as u32,
                    SHGFI_ICON | SHGFI_LARGEICON | SHGFI_USEFILEATTRIBUTES,
                ) != 0;
                (false, got_ic.then_some(sfi_ic.hIcon))
            } else {
                (true, None)
            }
        }
    };

    if got_idx {
        if let Some(cached) = sys_icon_cache.get(&(jumbo, sfi.iIcon)) {
            return Some(cached.clone());
        }
    }

    let mut owned_hicon = fallback_hicon;
    let mut dim = 32u32;

    if got_idx {
        dim = if jumbo { 256 } else { 48 };
        let img_list = if jumbo { jumbo_list } else { extralarge_list };
        if let Some(list) = img_list {
            if let Ok(ic) = list.GetIcon(sfi.iIcon, ILD_TRANSPARENT.0) {
                if !ic.is_invalid() {
                    owned_hicon = Some(ic);
                }
            }
        } else {
            let list_id = if jumbo { SHIL_JUMBO } else { SHIL_EXTRALARGE };
            if let Ok(list) = SHGetImageList::<IImageList>(list_id as i32) {
                if let Ok(ic) = list.GetIcon(sfi.iIcon, ILD_TRANSPARENT.0) {
                    if !ic.is_invalid() {
                        owned_hicon = Some(ic);
                    }
                }
            }
        }
    }

    if owned_hicon.as_ref().is_none_or(|h| h.is_invalid()) {
        let exists = exists_cached.unwrap_or_else(|| std::fs::symlink_metadata(path).is_ok());
        let mut sfi_fb = SHFILEINFOW::default();
        let attr = if exists && path.is_dir() {
            FILE_ATTRIBUTE_DIRECTORY
        } else {
            FILE_ATTRIBUTE_NORMAL
        };
        let flags = if exists {
            SHGFI_ICON | SHGFI_LARGEICON
        } else {
            SHGFI_ICON | SHGFI_LARGEICON | SHGFI_USEFILEATTRIBUTES
        };
        if SHGetFileInfoW(
            PCWSTR(wide_path.as_ptr()),
            attr,
            Some(&mut sfi_fb),
            size_of::<SHFILEINFOW>() as u32,
            flags,
        ) != 0
            && !sfi_fb.hIcon.is_invalid()
        {
            owned_hicon = Some(sfi_fb.hIcon);
            dim = 32;
        }
    }

    let hicon = owned_hicon?;
    if hicon.is_invalid() {
        return None;
    }

    let decoded = rasterize_hicon_premul_bgra(hicon, dim);
    let _ = DestroyIcon(hicon);
    if got_idx {
        if let Some(ref icon) = decoded {
            if sys_icon_cache.len() >= 256 {
                sys_icon_cache.clear();
            }
            sys_icon_cache.insert((jumbo, sfi.iIcon), icon.clone());
        }
    }
    decoded
}

unsafe fn rasterize_hicon_premul_bgra(hicon: HICON, target_dim: u32) -> Option<DecodedIcon> {
    let dim = target_dim.max(16) as i32;
    let mem_dc = CreateCompatibleDC(None);
    if mem_dc.is_invalid() {
        return None;
    }

    let bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: dim,
            biHeight: -dim, // top-down
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };

    let mut raw_ptr = std::ptr::null_mut();
    let Ok(bmp) = CreateDIBSection(mem_dc, &bmi, DIB_RGB_COLORS, &mut raw_ptr, None, 0) else {
        let _ = DeleteDC(mem_dc);
        return None;
    };
    if raw_ptr.is_null() {
        let _ = DeleteObject(bmp);
        let _ = DeleteDC(mem_dc);
        return None;
    }

    let byte_len = (dim as usize) * (dim as usize) * 4;
    let slice = std::slice::from_raw_parts_mut(raw_ptr.cast::<u8>(), byte_len);
    slice.fill(0);

    let old_obj: HGDIOBJ = SelectObject(mem_dc, bmp);
    let draw_ok = DrawIconEx(mem_dc, 0, 0, hicon, dim, dim, 0, None, DI_NORMAL).is_ok();
    let _ = SelectObject(mem_dc, old_obj);

    let mut buf = if draw_ok { slice.to_vec() } else { Vec::new() };
    let _ = DeleteObject(bmp);
    let _ = DeleteDC(mem_dc);

    if buf.is_empty() {
        return None;
    }

    // Legacy 16/24-bit icons drawn via DrawIconEx sometimes leave alpha at 0 for opaque pixels.
    let mut any_alpha = false;
    let mut any_color = false;
    for px in buf.chunks_exact(4) {
        if px[3] != 0 {
            any_alpha = true;
            break;
        }
        if px[0] != 0 || px[1] != 0 || px[2] != 0 {
            any_color = true;
        }
    }

    if !any_alpha && any_color {
        apply_icon_mask_alpha(hicon, &mut buf, dim);
    }

    // Crop legacy 48px icons sitting in the top-left corner of a 256px SHIL_JUMBO canvas.
    let (out_w, out_h, out_straight) = if dim >= 128 {
        maybe_crop_corner_legacy_jumbo(&buf, dim as u32)
    } else {
        (dim as u32, dim as u32, buf)
    };

    // Convert straight BGRA to premultiplied BGRA for Direct2D.
    let mut pbgra = out_straight;
    for px in pbgra.chunks_exact_mut(4) {
        let a = u16::from(px[3]);
        if a < 255 {
            px[0] = ((u16::from(px[0]) * a + 127) / 255) as u8;
            px[1] = ((u16::from(px[1]) * a + 127) / 255) as u8;
            px[2] = ((u16::from(px[2]) * a + 127) / 255) as u8;
        }
    }

    Some(DecodedIcon {
        width: out_w,
        height: out_h,
        pbgra,
    })
}

unsafe fn apply_icon_mask_alpha(hicon: HICON, bgra: &mut [u8], dim: i32) {
    let fallback_opaque = |buf: &mut [u8]| {
        for px in buf.chunks_exact_mut(4) {
            if px[0] != 0 || px[1] != 0 || px[2] != 0 {
                px[3] = 255;
            }
        }
    };

    let mut ii = ICONINFO::default();
    if GetIconInfo(hicon, &mut ii).is_err() {
        fallback_opaque(bgra);
        return;
    }

    let mut applied = false;
    let dc = CreateCompatibleDC(None);
    if !dc.is_invalid() && !ii.hbmMask.is_invalid() && dim > 0 {
        let mut bm = BITMAP::default();
        let got_obj = GetObjectW(
            ii.hbmMask,
            size_of::<BITMAP>() as i32,
            Some((&mut bm as *mut BITMAP).cast()),
        );
        let mask_w = bm.bmWidth;
        let full_mask_h = bm.bmHeight.abs();
        let and_mask_h = if ii.hbmColor.is_invalid() {
            full_mask_h / 2
        } else {
            full_mask_h
        };
        if got_obj > 0 && mask_w > 0 && and_mask_h > 0 && full_mask_h > 0 {
            let mut bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: mask_w,
                    biHeight: -full_mask_h,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mask_len = (mask_w as usize)
                .saturating_mul(full_mask_h as usize)
                .saturating_mul(4);
            let mut mask_buf = vec![0u8; mask_len];
            let scan = GetDIBits(
                dc,
                ii.hbmMask,
                0,
                full_mask_h as u32,
                Some(mask_buf.as_mut_ptr().cast()),
                &mut bmi,
                DIB_RGB_COLORS,
            );
            if scan > 0 {
                let d = dim as usize;
                let mw = mask_w as usize;
                let mh = and_mask_h as usize;
                for y in 0..d {
                    let sy = (y * mh) / d;
                    let dst_row = y * d;
                    let src_row = sy * mw;
                    for x in 0..d {
                        let sx = (x * mw) / d;
                        let si = (src_row + sx) * 4;
                        let di = (dst_row + x) * 4;
                        if si + 2 < mask_buf.len() && di + 3 < bgra.len() {
                            let is_transparent =
                                mask_buf[si] > 128 && mask_buf[si + 1] > 128 && mask_buf[si + 2] > 128;
                            bgra[di + 3] = if is_transparent { 0 } else { 255 };
                        }
                    }
                }
                applied = true;
            }
        }
    }
    if !applied {
        fallback_opaque(bgra);
    }
    if !dc.is_invalid() {
        let _ = DeleteDC(dc);
    }
    if !ii.hbmMask.is_invalid() {
        let _ = DeleteObject(ii.hbmMask);
    }
    if !ii.hbmColor.is_invalid() {
        let _ = DeleteObject(ii.hbmColor);
    }
}

/// Detects old icons padded inside a 256×256 `SHIL_JUMBO` slot where all non-zero pixels
/// occupy only a small `[0..<=64, 0..<=64]` top-left region and crops cleanly to that box.
fn maybe_crop_corner_legacy_jumbo(buf: &[u8], dim: u32) -> (u32, u32, Vec<u8>) {
    let d = dim as usize;
    let mut max_x = 0usize;
    let mut max_y = 0usize;
    let mut has_any = false;
    for y in 0..d {
        let row = y * d;
        for x in 0..d {
            let a = buf[(row + x) * 4 + 3];
            if a > 4 {
                has_any = true;
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            }
        }
    }
    let bound = max_x.max(max_y) + 1;
    if has_any && bound <= 64 {
        let crop_dim = bound.max(32);
        let mut cropped = vec![0u8; crop_dim * crop_dim * 4];
        for y in 0..crop_dim {
            let src_start = (y * d) * 4;
            let dst_start = (y * crop_dim) * 4;
            let row_bytes = crop_dim * 4;
            cropped[dst_start..dst_start + row_bytes]
                .copy_from_slice(&buf[src_start..src_start + row_bytes]);
        }
        return (crop_dim as u32, crop_dim as u32, cropped);
    }
    (dim, dim, buf.to_vec())
}
