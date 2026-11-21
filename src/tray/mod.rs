pub mod layout;

#[cfg(windows)]
mod menu;

#[cfg(windows)]
pub use win::run;

#[cfg(windows)]
mod win {
    use super::layout::TrayClickFilter;
    use super::menu::{
        TrayMenu, WM_APP_TRAY_EXIT, WM_APP_TRAY_MENU_CLOSED, WM_APP_TRAY_SET_TASKBAR_STYLE,
        WM_APP_TRAY_TOGGLE_AUTOSTART, WM_APP_TRAY_TOGGLE_MEDIABAR, WM_APP_TRAY_TOGGLE_MONITORBAR,
    };
    use crate::color::Rgb;
    use crate::mediabar::config::{portable_config_path, Config};
    use crate::mediabar::MediaBarInstance;
    use crate::monitorbar::config::{
        load_or_create as load_monitor_config, portable_config_path as portable_monitor_config_path,
        save_config as save_monitor_config,
    };
    use crate::monitorbar::MonitorBarInstance;
    use crate::system::{
        clear_overflow_notify_pending, is_autostart_enabled, is_light_taskbar,
        set_autostart_enabled, system_accent_color, OverflowEventHook,
    };
    use crate::taskbar::{TaskbarController, TaskbarStyle};
    use crate::ui::POPUP_FLOW_INTERVAL_MS;
    use std::cell::RefCell;
    use std::mem::size_of;
    use std::path::PathBuf;
    use std::rc::Rc;
    use std::time::Instant;
    use windows::core::{w, GUID, PCWSTR};
    use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM};
    use windows::Win32::Graphics::Direct2D::{
        D2D1CreateFactory, ID2D1Factory1, D2D1_FACTORY_TYPE_SINGLE_THREADED,
    };
    use windows::Win32::Graphics::DirectWrite::{
        DWriteCreateFactory, IDWriteFactory, DWRITE_FACTORY_TYPE_SHARED,
    };
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
    use windows::Win32::UI::Shell::{
        Shell_NotifyIconGetRect, Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP,
        NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION, NOTIFYICONDATAW, NOTIFYICONDATAW_0,
        NOTIFYICONIDENTIFIER, NOTIFYICON_VERSION_4,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyIcon, DestroyWindow, DispatchMessageW, FindWindowW,
        GetCursorPos, GetMessageW, GetWindowLongPtrW, KillTimer, LoadCursorW, LoadIconW,
        LoadImageW, PostMessageW, PostQuitMessage, RegisterClassExW, RegisterWindowMessageW,
        SetTimer, SetWindowLongPtrW, TranslateMessage, GWLP_USERDATA, HICON, IDC_ARROW,
        IDI_APPLICATION, IMAGE_ICON, LR_DEFAULTCOLOR, LR_SHARED, MSG, SM_CXICON, SM_CXSMICON,
        WM_APP, WM_DESTROY, WM_DISPLAYCHANGE, WM_SETTINGCHANGE, WM_TIMER, WNDCLASSEXW,
        WS_EX_TOOLWINDOW, WS_POPUP,
    };

    const HOST_CLASS_NAME: PCWSTR = w!("PrettyWindowsTrayHostWnd");
    const TRAY_ICON_ID: u32 = 1;
    const APP_ICON_RES_ID: u16 = 1;
    const TIMER_TASKBAR_MAINTAIN_ID: usize = 1;
    const TASKBAR_STEADY_MAINTAIN_MS: u32 = 150;

    const WM_APP_TRAY_ICON: u32 = WM_APP + 20;
    const WM_APP_MEDIABAR_CLOSE_REQUEST: u32 = WM_APP + 25;
    const WM_APP_MONITORBAR_CLOSE_REQUEST: u32 = WM_APP + 28;
    const WM_APP_TRAY_OVERFLOW_CHANGED: u32 = WM_APP + 29;

    struct TrayHostState {
        hwnd: HWND,
        taskbar_created_msg: u32,
        d2d_factory: ID2D1Factory1,
        dwrite_factory: IDWriteFactory,
        tray_icon: HICON,
        tray_icon_owned: bool,
        light_taskbar: bool,
        accent: Rgb,
        dpi: u32,
        config_path: PathBuf,
        mediabar: Option<MediaBarInstance>,
        monitorbar_config_path: PathBuf,
        monitorbar: Option<MonitorBarInstance>,
        taskbar: TaskbarController,
        taskbar_timer_ms: u32,
        overflow_hook: OverflowEventHook,
        menu: TrayMenu,
        started_at: Instant,
        click_filter: TrayClickFilter,
    }

    impl TrayHostState {
        fn new(hwnd: HWND, taskbar_created_msg: u32) -> windows::core::Result<Self> {
            unsafe {
                let d2d_factory: ID2D1Factory1 =
                    D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
                let dwrite_factory: IDWriteFactory =
                    DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;

                let light_taskbar = is_light_taskbar();
                let accent = system_accent_color();
                let dpi = current_shell_dpi(hwnd);
                let (tray_icon, tray_icon_owned) = load_app_icon(tray_icon_size_for_dpi(dpi), false);
                let _ = register_tray_icon(hwnd, tray_icon);

                let menu = TrayMenu::new(hwnd, d2d_factory.clone(), dwrite_factory.clone())?;
                let config_path = portable_config_path();
                let cfg = Config::load_from_path(&config_path);

                let mediabar = if cfg.enabled {
                    MediaBarInstance::start(
                        hwnd,
                        WM_APP_MEDIABAR_CLOSE_REQUEST,
                        d2d_factory.clone(),
                        dwrite_factory.clone(),
                    )
                    .ok()
                } else {
                    None
                };

                let monitorbar_config_path = portable_monitor_config_path();
                let monitor_cfg = load_monitor_config(&monitorbar_config_path);
                let monitorbar = if monitor_cfg.enabled {
                    MonitorBarInstance::start(
                        hwnd,
                        WM_APP_MONITORBAR_CLOSE_REQUEST,
                        monitorbar_config_path.clone(),
                    )
                    .ok()
                } else {
                    None
                };

                let mut taskbar = TaskbarController::new(d2d_factory.clone(), light_taskbar, accent);
                taskbar.set_overlay_notify_target(hwnd, WM_APP_TRAY_OVERFLOW_CHANGED);
                let mut overflow_hook = OverflowEventHook::new(hwnd, WM_APP_TRAY_OVERFLOW_CHANGED);
                overflow_hook.refresh_for_explorer_pid(taskbar.explorer_pid());
                overflow_hook
                    .refresh_for_shell_experience_pid(taskbar.shell_experience_pid());
                overflow_hook.refresh_for_shell_host_pid(taskbar.shell_host_pid());
                overflow_hook.refresh_for_start_menu_pid(taskbar.start_menu_pid());

                let mut host = Self {
                    hwnd,
                    taskbar_created_msg,
                    d2d_factory,
                    dwrite_factory,
                    tray_icon,
                    tray_icon_owned,
                    light_taskbar,
                    accent,
                    dpi,
                    config_path,
                    mediabar,
                    monitorbar_config_path,
                    monitorbar,
                    taskbar,
                    taskbar_timer_ms: 0,
                    overflow_hook,
                    menu,
                    started_at: Instant::now(),
                    click_filter: TrayClickFilter::default(),
                };
                host.sync_taskbar_timer();
                Ok(host)
            }
        }

        fn sync_taskbar_timer(&mut self) {
            let desired = if !self.taskbar.needs_timer() {
                0
            } else if self.taskbar.needs_fast_tick() {
                POPUP_FLOW_INTERVAL_MS
            } else {
                TASKBAR_STEADY_MAINTAIN_MS
            };
            if self.taskbar_timer_ms == desired {
                return;
            }
            unsafe {
                if desired == 0 {
                    let _ = KillTimer(self.hwnd, TIMER_TASKBAR_MAINTAIN_ID);
                } else {
                    SetTimer(self.hwnd, TIMER_TASKBAR_MAINTAIN_ID, desired, None);
                }
            }
            self.taskbar_timer_ms = desired;
        }

        fn is_mediabar_enabled(&self) -> bool {
            self.mediabar.is_some()
        }

        fn is_monitorbar_enabled(&self) -> bool {
            self.monitorbar.is_some()
        }

        fn sync_menu_state(&mut self) {
            let mediabar_on = self.is_mediabar_enabled();
            let monitorbar_on = self.is_monitorbar_enabled();
            let style = self.taskbar.style();
            self.menu.set_state(
                mediabar_on,
                monitorbar_on,
                style,
                is_autostart_enabled(),
            );
        }

        fn set_mediabar_enabled(&mut self, enable: bool) {
            if enable == self.is_mediabar_enabled() {
                return;
            }

            if enable {
                if let Ok(inst) = MediaBarInstance::start(
                    self.hwnd,
                    WM_APP_MEDIABAR_CLOSE_REQUEST,
                    self.d2d_factory.clone(),
                    self.dwrite_factory.clone(),
                ) {
                    self.mediabar = Some(inst);
                }
            } else {
                self.mediabar = None;
            }

            let mut cfg = Config::load_from_path(&self.config_path);
            cfg.enabled = self.is_mediabar_enabled();
            let _ = cfg.save_to_path(&self.config_path);

            self.sync_menu_state();
            self.menu.ensure_topmost();
        }

        fn set_monitorbar_enabled(&mut self, enable: bool) {
            if enable == self.is_monitorbar_enabled() {
                return;
            }

            if enable {
                if let Ok(inst) = MonitorBarInstance::start(
                    self.hwnd,
                    WM_APP_MONITORBAR_CLOSE_REQUEST,
                    self.monitorbar_config_path.clone(),
                ) {
                    self.monitorbar = Some(inst);
                }
            } else {
                self.monitorbar = None;
            }

            let mut cfg = load_monitor_config(&self.monitorbar_config_path);
            cfg.enabled = self.is_monitorbar_enabled();
            let _ = save_monitor_config(&self.monitorbar_config_path, &cfg);

            self.sync_menu_state();
            self.menu.ensure_topmost();
        }

        fn set_taskbar_style(&mut self, style: TaskbarStyle) {
            self.taskbar.set_style(style);
            self.overflow_hook
                .refresh_for_shell_experience_pid(self.taskbar.shell_experience_pid());
            self.overflow_hook
                .refresh_for_shell_host_pid(self.taskbar.shell_host_pid());
            self.overflow_hook
                .refresh_for_start_menu_pid(self.taskbar.start_menu_pid());
            self.sync_taskbar_timer();
            self.sync_menu_state();
        }

        fn toggle_autostart(&mut self) {
            let next = !is_autostart_enabled();
            let _ = set_autostart_enabled(next);
            self.sync_menu_state();
        }

        fn open_or_toggle_menu(&mut self, use_cursor: bool) {
            if self.menu.is_open() {
                self.menu.hide_animated();
                return;
            }
            if self.menu.is_closing_recently() {
                return;
            }

            let anchor_pt = tray_anchor_point(self.hwnd, use_cursor);
            let dpi = current_shell_dpi(self.hwnd);
            self.dpi = dpi;
            let light = is_light_taskbar();
            let accent = system_accent_color();
            self.light_taskbar = light;
            self.accent = accent;

            self.taskbar.hide_overflow_backdrop();

            self.menu.show(
                anchor_pt,
                dpi,
                light,
                accent,
                self.is_mediabar_enabled(),
                self.is_monitorbar_enabled(),
                self.taskbar.style(),
                is_autostart_enabled(),
            );
        }

        fn refresh_theme_and_icon(&mut self) {
            let light = is_light_taskbar();
            let accent = system_accent_color();
            let dpi = current_shell_dpi(self.hwnd);
            let need_icon = dpi != self.dpi;

            self.light_taskbar = light;
            self.accent = accent;
            self.dpi = dpi;
            self.menu.set_system_theme(light, accent);
            self.taskbar.refresh_theme(light, accent);

            if need_icon {
                let (new_icon, new_owned) = load_app_icon(tray_icon_size_for_dpi(dpi), false);
                if !new_icon.is_invalid() {
                    modify_tray_icon(self.hwnd, new_icon);
                    if self.tray_icon_owned && !self.tray_icon.is_invalid() {
                        unsafe {
                            let _ = DestroyIcon(self.tray_icon);
                        }
                    }
                    self.tray_icon = new_icon;
                    self.tray_icon_owned = new_owned;
                }
            }
        }

        fn re_register_tray_icon(&mut self) {
            let _ = register_tray_icon(self.hwnd, self.tray_icon);
            let reverted = self.taskbar.on_taskbar_created();
            self.overflow_hook
                .refresh_for_explorer_pid(self.taskbar.explorer_pid());
            self.overflow_hook
                .refresh_for_shell_experience_pid(self.taskbar.shell_experience_pid());
            self.overflow_hook
                .refresh_for_shell_host_pid(self.taskbar.shell_host_pid());
            self.overflow_hook
                .refresh_for_start_menu_pid(self.taskbar.start_menu_pid());
            self.sync_taskbar_timer();
            if reverted {
                self.sync_menu_state();
            }
        }
    }

    pub fn run() -> windows::core::Result<()> {
        unsafe {
            let hinst = GetModuleHandleW(None)?;
            let taskbar_created_msg = RegisterWindowMessageW(w!("TaskbarCreated"));
            let arrow_cursor = LoadCursorW(None, IDC_ARROW)?;
            let init_dpi = current_shell_dpi(HWND::default());
            let (app_icon_lg, _) = load_app_icon(GetSystemMetricsForDpi(SM_CXICON, init_dpi).max(32), true);
            let (app_icon_sm, _) = load_app_icon(tray_icon_size_for_dpi(init_dpi), true);

            let wc = WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(tray_host_wnd_proc),
                hInstance: hinst.into(),
                hIcon: app_icon_lg,
                hIconSm: app_icon_sm,
                hCursor: arrow_cursor,
                lpszClassName: HOST_CLASS_NAME,
                ..Default::default()
            };
            RegisterClassExW(&wc);

            // Hidden top-level host window (must NOT be HWND_MESSAGE so it receives TaskbarCreated/WM_SETTINGCHANGE).
            let hwnd = CreateWindowExW(
                WS_EX_TOOLWINDOW,
                HOST_CLASS_NAME,
                w!("Pretty Windows"),
                WS_POPUP,
                0,
                0,
                0,
                0,
                None,
                None,
                hinst,
                None,
            )?;

            let host = Rc::new(RefCell::new(TrayHostState::new(hwnd, taskbar_created_msg)?));
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, Rc::into_raw(host) as isize);

            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        Ok(())
    }

    unsafe extern "system" fn tray_host_wnd_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        let raw_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const RefCell<TrayHostState>;
        if raw_ptr.is_null() {
            return DefWindowProcW(hwnd, msg, wparam, lparam);
        }

        if msg == WM_DESTROY {
            let rc = Rc::from_raw(raw_ptr);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            if let Ok(mut state) = rc.try_borrow_mut() {
                let _ = KillTimer(hwnd, TIMER_TASKBAR_MAINTAIN_ID);
                state.mediabar.take();
                state.monitorbar.take();
                crate::monitorbar::brightness::restore_software_gamma_on_exit();
                state.menu.hide();
                state.taskbar.restore_default_and_shutdown();
                delete_tray_icon(hwnd);
                if state.tray_icon_owned && !state.tray_icon.is_invalid() {
                    let _ = DestroyIcon(state.tray_icon);
                }
                state.tray_icon = HICON::default();
                state.tray_icon_owned = false;
            }
            PostQuitMessage(0);
            return LRESULT(0);
        }

        Rc::increment_strong_count(raw_ptr);
        let rc = Rc::from_raw(raw_ptr);

        let taskbar_created_msg = rc
            .try_borrow()
            .map(|s| s.taskbar_created_msg)
            .unwrap_or(0);
        if taskbar_created_msg != 0 && msg == taskbar_created_msg {
            if let Ok(mut state) = rc.try_borrow_mut() {
                state.re_register_tray_icon();
            }
            return LRESULT(0);
        }

        match msg {
            WM_APP_TRAY_ICON => {
                let event = (lparam.0 & 0xFFFF) as u32;
                if let Ok(mut state) = rc.try_borrow_mut() {
                    let elapsed_ms = state.started_at.elapsed().as_millis() as u64;
                    if let Some(decision) = state.click_filter.evaluate(event, elapsed_ms) {
                        state.open_or_toggle_menu(decision.use_cursor);
                    }
                } else {
                    let _ = PostMessageW(hwnd, msg, wparam, lparam);
                }
                LRESULT(0)
            }
            WM_APP_TRAY_TOGGLE_MEDIABAR => {
                if let Ok(mut state) = rc.try_borrow_mut() {
                    let next = !state.is_mediabar_enabled();
                    state.set_mediabar_enabled(next);
                } else {
                    let _ = PostMessageW(hwnd, msg, wparam, lparam);
                }
                LRESULT(0)
            }
            WM_APP_MEDIABAR_CLOSE_REQUEST => {
                if let Ok(mut state) = rc.try_borrow_mut() {
                    state.set_mediabar_enabled(false);
                }
                LRESULT(0)
            }
            WM_APP_TRAY_TOGGLE_MONITORBAR => {
                if let Ok(mut state) = rc.try_borrow_mut() {
                    let next = !state.is_monitorbar_enabled();
                    state.set_monitorbar_enabled(next);
                } else {
                    let _ = PostMessageW(hwnd, msg, wparam, lparam);
                }
                LRESULT(0)
            }
            WM_APP_MONITORBAR_CLOSE_REQUEST => {
                if let Ok(mut state) = rc.try_borrow_mut() {
                    state.set_monitorbar_enabled(false);
                }
                LRESULT(0)
            }
            WM_APP_TRAY_TOGGLE_AUTOSTART => {
                if let Ok(mut state) = rc.try_borrow_mut() {
                    state.toggle_autostart();
                } else {
                    let _ = PostMessageW(hwnd, msg, wparam, lparam);
                }
                LRESULT(0)
            }
            WM_APP_TRAY_SET_TASKBAR_STYLE => {
                if let Some(style) = TaskbarStyle::from_u8(wparam.0 as u8) {
                    if let Ok(mut state) = rc.try_borrow_mut() {
                        state.set_taskbar_style(style);
                    } else {
                        let _ = PostMessageW(hwnd, msg, wparam, lparam);
                    }
                }
                LRESULT(0)
            }
            WM_TIMER => {
                if wparam.0 == TIMER_TASKBAR_MAINTAIN_ID {
                    if let Ok(mut state) = rc.try_borrow_mut() {
                        if state.taskbar.tick() {
                            state.sync_menu_state();
                        }
                        let shell_exp_pid = state.taskbar.shell_experience_pid();
                        state
                            .overflow_hook
                            .refresh_for_shell_experience_pid(shell_exp_pid);
                        let shell_host_pid = state.taskbar.shell_host_pid();
                        state
                            .overflow_hook
                            .refresh_for_shell_host_pid(shell_host_pid);
                        let start_menu_pid = state.taskbar.start_menu_pid();
                        state
                            .overflow_hook
                            .refresh_for_start_menu_pid(start_menu_pid);
                        state.sync_taskbar_timer();
                    }
                }
                LRESULT(0)
            }
            WM_APP_TRAY_OVERFLOW_CHANGED => {
                if let Ok(mut state) = rc.try_borrow_mut() {
                    clear_overflow_notify_pending();
                    state.taskbar.sync_overflow_now();
                    state.sync_taskbar_timer();
                } else {
                    let _ = PostMessageW(hwnd, msg, wparam, lparam);
                }
                LRESULT(0)
            }
            WM_APP_TRAY_EXIT => {
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_APP_TRAY_MENU_CLOSED => LRESULT(0),
            WM_SETTINGCHANGE | WM_DISPLAYCHANGE => {
                if let Ok(mut state) = rc.try_borrow_mut() {
                    state.refresh_theme_and_icon();
                }
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }

    fn current_shell_dpi(fallback_hwnd: HWND) -> u32 {
        unsafe {
            if let Ok(tb) = FindWindowW(w!("Shell_TrayWnd"), None) {
                if !tb.is_invalid() {
                    let dpi = GetDpiForWindow(tb);
                    if dpi >= 96 {
                        return dpi;
                    }
                }
            }
            GetDpiForWindow(fallback_hwnd).max(96)
        }
    }

    fn tray_anchor_point(hwnd: HWND, use_cursor: bool) -> POINT {
        unsafe {
            let mut pt = POINT::default();
            if use_cursor && GetCursorPos(&mut pt).is_ok() {
                return pt;
            }
            let id = NOTIFYICONIDENTIFIER {
                cbSize: size_of::<NOTIFYICONIDENTIFIER>() as u32,
                hWnd: hwnd,
                uID: TRAY_ICON_ID,
                guidItem: GUID::zeroed(),
            };
            if let Ok(rect) = Shell_NotifyIconGetRect(&id) {
                return POINT {
                    x: (rect.left + rect.right) / 2,
                    y: rect.top,
                };
            }
            let _ = GetCursorPos(&mut pt);
            pt
        }
    }

    fn build_nid(hwnd: HWND, icon: HICON) -> NOTIFYICONDATAW {
        let mut nid = NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: TRAY_ICON_ID,
            uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP,
            uCallbackMessage: WM_APP_TRAY_ICON,
            hIcon: icon,
            ..Default::default()
        };
        let tip: Vec<u16> = "Pretty Windows".encode_utf16().collect();
        let copy_len = tip.len().min(nid.szTip.len() - 1);
        nid.szTip[..copy_len].copy_from_slice(&tip[..copy_len]);
        nid.szTip[copy_len] = 0;
        nid
    }

    fn register_tray_icon(hwnd: HWND, icon: HICON) -> bool {
        unsafe {
            let mut nid = build_nid(hwnd, icon);
            let _ = Shell_NotifyIconW(NIM_ADD, &nid);
            nid.Anonymous = NOTIFYICONDATAW_0 {
                uVersion: NOTIFYICON_VERSION_4,
            };
            Shell_NotifyIconW(NIM_SETVERSION, &nid).as_bool()
        }
    }

    fn modify_tray_icon(hwnd: HWND, icon: HICON) {
        unsafe {
            let mut nid = build_nid(hwnd, icon);
            nid.uFlags = NIF_ICON | NIF_TIP | NIF_SHOWTIP;
            let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
        }
    }

    fn delete_tray_icon(hwnd: HWND) {
        unsafe {
            let nid = NOTIFYICONDATAW {
                cbSize: size_of::<NOTIFYICONDATAW>() as u32,
                hWnd: hwnd,
                uID: TRAY_ICON_ID,
                ..Default::default()
            };
            let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
        }
    }

    /// Returns the exact system small-icon dimension in physical pixels for `dpi`.
    fn tray_icon_size_for_dpi(dpi: u32) -> i32 {
        let clamped_dpi = dpi.max(96);
        let sm = unsafe { GetSystemMetricsForDpi(SM_CXSMICON, clamped_dpi) };
        if sm > 0 {
            sm
        } else {
            ((16.0 * (clamped_dpi as f32) / 96.0).round() as i32).clamp(16, 64)
        }
    }

    /// Loads the embedded multi-resolution `PrettyWindows` icon resource at `size_px`.
    /// Returns `(hicon, owned_by_caller)`.
    fn load_app_icon(size_px: i32, shared: bool) -> (HICON, bool) {
        unsafe {
            if let Ok(hmod) = GetModuleHandleW(None) {
                let hinst: HINSTANCE = hmod.into();
                let flags = if shared {
                    LR_DEFAULTCOLOR | LR_SHARED
                } else {
                    LR_DEFAULTCOLOR
                };
                let dim = size_px.max(16);
                if let Ok(handle) = LoadImageW(
                    hinst,
                    PCWSTR(APP_ICON_RES_ID as *const u16),
                    IMAGE_ICON,
                    dim,
                    dim,
                    flags,
                ) {
                    if !handle.is_invalid() {
                        return (HICON(handle.0), !shared);
                    }
                }
            }
            let fallback = LoadIconW(None, IDI_APPLICATION).unwrap_or_default();
            (fallback, false)
        }
    }
}
