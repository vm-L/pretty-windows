pub mod layout;

#[cfg(windows)]
mod menu;

#[cfg(windows)]
pub use win::run;

#[cfg(windows)]
mod win {
    use super::layout::TrayClickFilter;
    use super::menu::{
        DeskBoxMenuState, TrayMenu, WM_APP_TRAY_DESKBOX_ACTION, WM_APP_TRAY_EXIT,
        WM_APP_TRAY_MENU_CLOSED, WM_APP_TRAY_SET_TASKBAR_STYLE, WM_APP_TRAY_TOGGLE_AUTOSTART,
        WM_APP_TRAY_TOGGLE_MEDIABAR, WM_APP_TRAY_TOGGLE_MONITORBAR,
    };
    use crate::color::Rgb;
    use crate::deskbox::{
        pick_folder_for_new_portal, DeskBoxController, DeskBoxTrayAction,
        HOST_CMD_NEW_FOLDER_PORTAL, WM_APP_DESKBOX_HOST_CMD, WM_APP_DESKBOX_ZORDER_SYNC,
    };
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
    use crate::ui::{GlassSurfaceStyle, POPUP_FLOW_INTERVAL_MS};
    use std::cell::RefCell;
    use std::mem::size_of;
    use std::path::PathBuf;
    use std::rc::Rc;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::Instant;
    use windows::core::{w, GUID, PCWSTR};
    use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM};
    use windows::Win32::Graphics::Direct2D::{
        D2D1CreateFactory, ID2D1Factory1, D2D1_FACTORY_TYPE_SINGLE_THREADED,
    };
    use windows::Win32::Graphics::DirectWrite::{
        DWriteCreateFactory, IDWriteFactory, DWRITE_FACTORY_TYPE_SHARED,
    };
    use windows::Win32::Graphics::Gdi::{MonitorFromPoint, MONITOR_DEFAULTTONEAREST};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::HiDpi::{
        GetDpiForMonitor, GetDpiForWindow, GetSystemMetricsForDpi, MDT_EFFECTIVE_DPI,
    };
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
        WM_APP, WM_DESTROY, WM_DISPLAYCHANGE, WM_ENDSESSION, WM_SETTINGCHANGE, WM_TIMER,
        WNDCLASSEXW, WS_EX_TOOLWINDOW, WS_POPUP,
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

    static TASKBAR_CREATED_MSG: AtomicU32 = AtomicU32::new(0);

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
        deskbox: DeskBoxController,
        taskbar: TaskbarController,
        taskbar_timer_ms: u32,
        overflow_hook: OverflowEventHook,
        menu: TrayMenu,
        started_at: Instant,
        click_filter: TrayClickFilter,
        folder_picker_open: bool,
        cleaned_up: bool,
    }

    impl TrayHostState {
        fn cleanup(&mut self) {
            if self.cleaned_up {
                return;
            }
            self.cleaned_up = true;
            unsafe {
                let _ = KillTimer(self.hwnd, TIMER_TASKBAR_MAINTAIN_ID);
            }
            self.mediabar.take();
            self.monitorbar.take();
            self.deskbox.shutdown_and_restore();
            crate::monitorbar::brightness::restore_software_gamma_on_exit();
            self.menu.hide();
            self.taskbar.restore_default_and_shutdown();
            delete_tray_icon(self.hwnd);
            if self.tray_icon_owned && !self.tray_icon.is_invalid() {
                unsafe {
                    let _ = DestroyIcon(self.tray_icon);
                }
            }
            self.tray_icon = HICON::default();
            self.tray_icon_owned = false;
        }

        fn new(hwnd: HWND, taskbar_created_msg: u32) -> windows::core::Result<Self> {
            unsafe {
                let d2d_factory: ID2D1Factory1 =
                    D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
                let dwrite_factory: IDWriteFactory =
                    DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;

                let light_taskbar = is_light_taskbar();
                let accent = system_accent_color();
                let dpi = current_shell_dpi(hwnd);
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
                let deskbox = DeskBoxController::new(
                    hwnd,
                    d2d_factory.clone(),
                    dwrite_factory.clone(),
                    GlassSurfaceStyle::from(taskbar.style()),
                    light_taskbar,
                )?;
                let (tray_icon, tray_icon_owned) = load_app_icon(tray_icon_size_for_dpi(dpi), false);
                let _ = register_tray_icon(hwnd, tray_icon);
                taskbar.set_overlay_notify_target(hwnd, WM_APP_TRAY_OVERFLOW_CHANGED);
                let mut overflow_hook = OverflowEventHook::new(hwnd, WM_APP_TRAY_OVERFLOW_CHANGED);
                overflow_hook.refresh_for_explorer_pid(taskbar.explorer_pid());
                overflow_hook
                    .refresh_for_shell_experience_pid(taskbar.shell_experience_pid());
                overflow_hook.refresh_for_shell_host_pid(taskbar.shell_host_pid());
                overflow_hook.refresh_for_start_menu_pid(taskbar.start_menu_pid());
                overflow_hook.refresh_for_search_host_pid(taskbar.search_host_pid());

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
                    deskbox,
                    taskbar,
                    taskbar_timer_ms: 0,
                    overflow_hook,
                    menu,
                    started_at: Instant::now(),
                    click_filter: TrayClickFilter::default(),
                    folder_picker_open: false,
                    cleaned_up: false,
                };
                host.sync_taskbar_timer();
                Ok(host)
            }
        }

        fn sync_taskbar_timer(&mut self) {
            let need_timer = self.taskbar.needs_timer() || self.deskbox.needs_timer();
            let need_fast = self.taskbar.needs_fast_tick() || self.deskbox.needs_fast_tick();
            let desired = if !need_timer {
                0
            } else if need_fast {
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

        fn deskbox_menu_state(&self) -> DeskBoxMenuState {
            DeskBoxMenuState {
                enabled: self.deskbox.is_enabled(),
                hide_desktop_icons: self.deskbox.hide_desktop_icons(),
                default_view_mode: self.deskbox.default_view_mode(),
            }
        }

        fn sync_menu_state(&mut self) {
            let mediabar_on = self.is_mediabar_enabled();
            let monitorbar_on = self.is_monitorbar_enabled();
            let deskbox_state = self.deskbox_menu_state();
            let style = self.taskbar.style();
            self.menu.set_state(
                mediabar_on,
                monitorbar_on,
                deskbox_state,
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
            let effective_style = self.taskbar.style();
            self.deskbox
                .refresh_surface_style(GlassSurfaceStyle::from(effective_style));
            self.overflow_hook
                .refresh_for_shell_experience_pid(self.taskbar.shell_experience_pid());
            self.overflow_hook
                .refresh_for_shell_host_pid(self.taskbar.shell_host_pid());
            self.overflow_hook
                .refresh_for_start_menu_pid(self.taskbar.start_menu_pid());
            self.overflow_hook
                .refresh_for_search_host_pid(self.taskbar.search_host_pid());
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
            let menu_dpi = monitor_dpi_at_point(anchor_pt, self.hwnd);
            let light = is_light_taskbar();
            let accent = system_accent_color();
            self.light_taskbar = light;
            self.accent = accent;

            self.taskbar.hide_overflow_backdrop();

            self.menu.show(
                anchor_pt,
                menu_dpi,
                light,
                accent,
                self.is_mediabar_enabled(),
                self.is_monitorbar_enabled(),
                self.deskbox_menu_state(),
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
            self.deskbox.refresh_theme(light);

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
            self.deskbox.on_taskbar_created();
            let reverted = self.taskbar.on_taskbar_created();
            self.overflow_hook
                .refresh_for_explorer_pid(self.taskbar.explorer_pid());
            self.overflow_hook
                .refresh_for_shell_experience_pid(self.taskbar.shell_experience_pid());
            self.overflow_hook
                .refresh_for_shell_host_pid(self.taskbar.shell_host_pid());
            self.overflow_hook
                .refresh_for_start_menu_pid(self.taskbar.start_menu_pid());
            self.overflow_hook
                .refresh_for_search_host_pid(self.taskbar.search_host_pid());
            self.sync_taskbar_timer();
            if reverted {
                self.deskbox
                    .refresh_surface_style(GlassSurfaceStyle::from(self.taskbar.style()));
                self.sync_taskbar_timer();
                self.sync_menu_state();
            }
        }
    }

    pub fn run() -> windows::core::Result<()> {
        unsafe {
            let hinst = GetModuleHandleW(None)?;
            let taskbar_created_msg = RegisterWindowMessageW(w!("TaskbarCreated"));
            TASKBAR_CREATED_MSG.store(taskbar_created_msg, Ordering::Relaxed);
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

            let host_state = match TrayHostState::new(hwnd, taskbar_created_msg) {
                Ok(s) => s,
                Err(e) => {
                    let _ = DestroyWindow(hwnd);
                    return Err(e);
                }
            };
            let host = Rc::new(RefCell::new(host_state));
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, Rc::into_raw(host) as isize);

            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        Ok(())
    }

    impl Drop for TrayHostState {
        fn drop(&mut self) {
            self.cleanup();
        }
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
                state.cleanup();
            }
            PostQuitMessage(0);
            return LRESULT(0);
        }

        Rc::increment_strong_count(raw_ptr);
        let rc = Rc::from_raw(raw_ptr);

        let cached_tb_msg = TASKBAR_CREATED_MSG.load(Ordering::Relaxed);
        let taskbar_created_msg = if cached_tb_msg != 0 {
            cached_tb_msg
        } else {
            rc.try_borrow().map(|s| s.taskbar_created_msg).unwrap_or(0)
        };
        if taskbar_created_msg != 0 && msg == taskbar_created_msg {
            if let Ok(mut state) = rc.try_borrow_mut() {
                state.re_register_tray_icon();
            } else {
                let _ = PostMessageW(hwnd, msg, wparam, lparam);
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
                } else {
                    let _ = PostMessageW(hwnd, msg, wparam, lparam);
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
                } else {
                    let _ = PostMessageW(hwnd, msg, wparam, lparam);
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
            WM_APP_TRAY_DESKBOX_ACTION => {
                if let Some(action) = DeskBoxTrayAction::from_u8(wparam.0 as u8) {
                    if let Ok(mut state) = rc.try_borrow_mut() {
                        state.deskbox.handle_tray_action(action);
                        state.sync_taskbar_timer();
                        state.sync_menu_state();
                    } else {
                        let _ = PostMessageW(hwnd, msg, wparam, lparam);
                    }
                }
                LRESULT(0)
            }
            WM_APP_DESKBOX_HOST_CMD => {
                let cmd = wparam.0;
                let box_id = lparam.0 as u64;
                if cmd == HOST_CMD_NEW_FOLDER_PORTAL {
                    let can_open = if let Ok(mut state) = rc.try_borrow_mut() {
                        if state.cleaned_up || state.folder_picker_open {
                            false
                        } else {
                            state.folder_picker_open = true;
                            state.menu.hide();
                            true
                        }
                    } else {
                        let _ = PostMessageW(hwnd, msg, wparam, lparam);
                        false
                    };
                    if can_open {
                        let picked = pick_folder_for_new_portal(hwnd);
                        if let Ok(mut state) = rc.try_borrow_mut() {
                            state.folder_picker_open = false;
                            if !state.cleaned_up {
                                if let Some(folder) = picked {
                                    state.deskbox.create_folder_portal_box(folder);
                                    state.sync_taskbar_timer();
                                    state.sync_menu_state();
                                }
                            }
                        }
                    }
                } else if let Ok(mut state) = rc.try_borrow_mut() {
                    let need_picker = state.deskbox.handle_host_cmd(cmd, box_id);
                    state.sync_taskbar_timer();
                    state.sync_menu_state();
                    if need_picker {
                        let _ = PostMessageW(
                            hwnd,
                            WM_APP_DESKBOX_HOST_CMD,
                            WPARAM(HOST_CMD_NEW_FOLDER_PORTAL),
                            LPARAM(box_id as isize),
                        );
                    }
                } else {
                    let _ = PostMessageW(hwnd, msg, wparam, lparam);
                }
                LRESULT(0)
            }
            WM_APP_DESKBOX_ZORDER_SYNC => {
                if let Ok(mut state) = rc.try_borrow_mut() {
                    state.deskbox.sync_zorder_now();
                    state.sync_taskbar_timer();
                } else {
                    let _ = PostMessageW(hwnd, msg, wparam, lparam);
                }
                LRESULT(0)
            }
            WM_TIMER => {
                if wparam.0 == TIMER_TASKBAR_MAINTAIN_ID {
                    if let Ok(mut state) = rc.try_borrow_mut() {
                        state.deskbox.tick();
                        let deskbox_animating = state.deskbox.is_animating();
                        if state.taskbar.tick(deskbox_animating) {
                            let style = GlassSurfaceStyle::from(state.taskbar.style());
                            state.deskbox.refresh_surface_style(style);
                            state.sync_taskbar_timer();
                            state.sync_menu_state();
                        }
                        let explorer_pid = state.taskbar.cached_explorer_pid();
                        if explorer_pid != 0 {
                            state.overflow_hook.refresh_for_explorer_pid(explorer_pid);
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
                        let search_host_pid = state.taskbar.search_host_pid();
                        state
                            .overflow_hook
                            .refresh_for_search_host_pid(search_host_pid);
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
            WM_SETTINGCHANGE => {
                if let Ok(mut state) = rc.try_borrow_mut() {
                    state.refresh_theme_and_icon();
                    // SPI_SETWORKAREA (0x002F) fires on taskbar resize / auto-hide toggle.
                    if wparam.0 == 0x002F {
                        state.deskbox.on_display_change();
                    }
                }
                LRESULT(0)
            }
            WM_DISPLAYCHANGE => {
                if let Ok(mut state) = rc.try_borrow_mut() {
                    state.refresh_theme_and_icon();
                    state.deskbox.on_display_change();
                }
                LRESULT(0)
            }
            WM_ENDSESSION => {
                if wparam.0 != 0 {
                    if let Ok(mut state) = rc.try_borrow_mut() {
                        state.cleanup();
                    }
                }
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }

    fn monitor_dpi_at_point(pt: POINT, fallback_hwnd: HWND) -> u32 {
        unsafe {
            let mon = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
            let mut dpi_x = 0u32;
            let mut dpi_y = 0u32;
            if GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y).is_ok()
                && dpi_x >= 96
            {
                return dpi_x;
            }
        }
        current_shell_dpi(fallback_hwnd)
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
