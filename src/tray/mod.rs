pub mod layout;

#[cfg(windows)]
mod menu;

#[cfg(windows)]
pub use win::run;

#[cfg(windows)]
mod win {
    use super::menu::{
        TrayMenu, WM_APP_TRAY_EXIT, WM_APP_TRAY_MENU_CLOSED, WM_APP_TRAY_SET_TASKBAR_STYLE,
        WM_APP_TRAY_TOGGLE_AUTOSTART, WM_APP_TRAY_TOGGLE_MEDIABAR, WM_APP_TRAY_TOGGLE_MONITORBAR,
    };
    use crate::color::{adjust_accent_for_theme, Rgb};
    use crate::mediabar::config::{portable_config_path, Config};
    use crate::mediabar::MediaBarInstance;
    use crate::monitorbar::config::{
        load_or_create as load_monitor_config, portable_config_path as portable_monitor_config_path,
        save_config as save_monitor_config,
    };
    use crate::monitorbar::MonitorBarInstance;
    use crate::system::{
        is_autostart_enabled, is_light_taskbar, set_autostart_enabled, system_accent_color,
    };
    use crate::taskbar::{TaskbarController, TaskbarStyle};
    use crate::ui::glass::{d2d_color_rgb, LayeredSurface};
    use crate::ui::{POPUP_FLOW_INTERVAL_MS, POPUP_REOPEN_DEBOUNCE_MS};
    use std::cell::RefCell;
    use std::mem::size_of;
    use std::path::PathBuf;
    use std::rc::Rc;
    use std::time::{Duration, Instant};
    use windows::core::{w, GUID, PCWSTR};
    use windows::Win32::Foundation::{BOOL, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
    use windows::Win32::Graphics::Direct2D::Common::{
        D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_FIGURE_BEGIN_FILLED,
        D2D1_FIGURE_END_CLOSED, D2D1_PIXEL_FORMAT, D2D_POINT_2F, D2D_RECT_F,
    };
    use windows::Win32::Graphics::Direct2D::{
        D2D1CreateFactory, ID2D1Factory1, D2D1_ELLIPSE, D2D1_FACTORY_TYPE_SINGLE_THREADED,
        D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE_DEFAULT,
        D2D1_RENDER_TARGET_USAGE_NONE, D2D1_ROUNDED_RECT,
    };
    use windows::Win32::Graphics::DirectWrite::{
        DWriteCreateFactory, IDWriteFactory, DWRITE_FACTORY_TYPE_SHARED,
    };
    use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
    use windows::Win32::Graphics::Gdi::{CreateBitmap, DeleteObject};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::HiDpi::GetDpiForWindow;
    use windows::Win32::UI::Shell::{
        Shell_NotifyIconGetRect, Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP,
        NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION, NIN_SELECT, NOTIFYICONDATAW,
        NOTIFYICONDATAW_0, NOTIFYICONIDENTIFIER, NOTIFYICON_VERSION_4,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateIconIndirect, CreateWindowExW, DefWindowProcW, DestroyIcon, DestroyWindow,
        DispatchMessageW, FindWindowW, GetCursorPos, GetMessageW, GetWindowLongPtrW, KillTimer,
        LoadCursorW, PostQuitMessage, RegisterClassExW, RegisterWindowMessageW, SetTimer,
        SetWindowLongPtrW, TranslateMessage, GWLP_USERDATA, HICON, ICONINFO, IDC_ARROW, MSG,
        WM_APP, WM_CONTEXTMENU, WM_DESTROY, WM_DISPLAYCHANGE, WM_LBUTTONUP, WM_RBUTTONUP,
        WM_SETTINGCHANGE, WM_TIMER, WNDCLASSEXW, WS_EX_TOOLWINDOW, WS_POPUP,
    };

    const HOST_CLASS_NAME: PCWSTR = w!("PrettyWindowsTrayHostWnd");
    const TRAY_ICON_ID: u32 = 1;
    const NIN_KEYSELECT: u32 = NIN_SELECT + 1;
    const TIMER_TASKBAR_MAINTAIN_ID: usize = 1;
    const TASKBAR_STEADY_MAINTAIN_MS: u32 = 150;

    const WM_APP_TRAY_ICON: u32 = WM_APP + 20;
    const WM_APP_MEDIABAR_CLOSE_REQUEST: u32 = WM_APP + 25;
    const WM_APP_MONITORBAR_CLOSE_REQUEST: u32 = WM_APP + 28;

    struct TrayHostState {
        hwnd: HWND,
        taskbar_created_msg: u32,
        d2d_factory: ID2D1Factory1,
        dwrite_factory: IDWriteFactory,
        tray_icon: HICON,
        v4_mode: bool,
        light_taskbar: bool,
        accent: Rgb,
        dpi: u32,
        config_path: PathBuf,
        mediabar: Option<MediaBarInstance>,
        monitorbar_config_path: PathBuf,
        monitorbar: Option<MonitorBarInstance>,
        taskbar: TaskbarController,
        taskbar_timer_ms: u32,
        menu: TrayMenu,
        menu_closed_at: Option<Instant>,
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
                let tray_icon = create_tray_icon(&d2d_factory, dpi, light_taskbar, accent)?;
                let v4_mode = register_tray_icon(hwnd, tray_icon);

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

                let taskbar = TaskbarController::new(d2d_factory.clone(), light_taskbar, accent);

                let mut host = Self {
                    hwnd,
                    taskbar_created_msg,
                    d2d_factory,
                    dwrite_factory,
                    tray_icon,
                    v4_mode,
                    light_taskbar,
                    accent,
                    dpi,
                    config_path,
                    mediabar,
                    monitorbar_config_path,
                    monitorbar,
                    taskbar,
                    taskbar_timer_ms: 0,
                    menu,
                    menu_closed_at: None,
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
            if let Some(closed_at) = self.menu_closed_at.take() {
                if closed_at.elapsed() < Duration::from_millis(POPUP_REOPEN_DEBOUNCE_MS) {
                    return;
                }
            }

            let anchor_pt = tray_anchor_point(self.hwnd, use_cursor);
            let dpi = current_shell_dpi(self.hwnd);
            self.dpi = dpi;
            let light = is_light_taskbar();
            let accent = system_accent_color();
            self.light_taskbar = light;
            self.accent = accent;

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
            let need_icon =
                light != self.light_taskbar || accent != self.accent || dpi != self.dpi;

            self.light_taskbar = light;
            self.accent = accent;
            self.dpi = dpi;
            self.menu.set_system_theme(light, accent);
            self.taskbar.refresh_theme(light, accent);

            if need_icon {
                if let Ok(new_icon) = create_tray_icon(&self.d2d_factory, dpi, light, accent) {
                    modify_tray_icon(self.hwnd, new_icon);
                    if !self.tray_icon.is_invalid() {
                        unsafe {
                            let _ = DestroyIcon(self.tray_icon);
                        }
                    }
                    self.tray_icon = new_icon;
                }
            }
        }

        fn re_register_tray_icon(&mut self) {
            self.v4_mode = register_tray_icon(self.hwnd, self.tray_icon);
            let reverted = self.taskbar.on_taskbar_created();
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

            let wc = WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(tray_host_wnd_proc),
                hInstance: hinst.into(),
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
                state.menu.hide();
                state.taskbar.restore_default_and_shutdown();
                delete_tray_icon(hwnd);
                if !state.tray_icon.is_invalid() {
                    let _ = DestroyIcon(state.tray_icon);
                    state.tray_icon = HICON::default();
                }
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
                let v4_mode = rc.try_borrow().map(|s| s.v4_mode).unwrap_or(true);
                let trigger = if v4_mode {
                    matches!(event, WM_CONTEXTMENU | NIN_SELECT | NIN_KEYSELECT)
                } else {
                    matches!(event, WM_RBUTTONUP | WM_LBUTTONUP)
                };
                if trigger {
                    let use_cursor = event != NIN_KEYSELECT;
                    if let Ok(mut state) = rc.try_borrow_mut() {
                        state.open_or_toggle_menu(use_cursor);
                    }
                }
                LRESULT(0)
            }
            WM_APP_TRAY_TOGGLE_MEDIABAR => {
                if let Ok(mut state) = rc.try_borrow_mut() {
                    let next = !state.is_mediabar_enabled();
                    state.set_mediabar_enabled(next);
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
                }
                LRESULT(0)
            }
            WM_APP_TRAY_SET_TASKBAR_STYLE => {
                if let Some(style) = TaskbarStyle::from_u8(wparam.0 as u8) {
                    if let Ok(mut state) = rc.try_borrow_mut() {
                        state.set_taskbar_style(style);
                    }
                }
                LRESULT(0)
            }
            WM_TIMER => {
                if wparam.0 == TIMER_TASKBAR_MAINTAIN_ID {
                    if let Ok(mut state) = rc.try_borrow_mut() {
                        if state.taskbar.tick() {
                            state.sync_taskbar_timer();
                            state.sync_menu_state();
                        }
                    }
                }
                LRESULT(0)
            }
            WM_APP_TRAY_EXIT => {
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_APP_TRAY_MENU_CLOSED => {
                if let Ok(mut state) = rc.try_borrow_mut() {
                    state.menu_closed_at = Some(Instant::now());
                }
                LRESULT(0)
            }
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

    /// Generates a DPI-aware Liquid Glass vector tray icon (`HICON`) adapted to dark/light taskbar & system accent.
    fn create_tray_icon(
        d2d_factory: &ID2D1Factory1,
        dpi: u32,
        light_taskbar: bool,
        accent: Rgb,
    ) -> windows::core::Result<HICON> {
        let scale = (dpi.max(96) as f32) / 96.0;
        let size = ((16.0 * scale).round() as i32).clamp(16, 64);
        let u = (size as f32) / 16.0;

        let rt_props = D2D1_RENDER_TARGET_PROPERTIES {
            r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0,
            dpiY: 96.0,
            usage: D2D1_RENDER_TARGET_USAGE_NONE,
            minLevel: Default::default(),
        };

        unsafe {
            let rt = d2d_factory.CreateDCRenderTarget(&rt_props)?;
            let mut surf = LayeredSurface::new()?;
            surf.ensure_size(size, size)?;
            let bind_rect = RECT {
                left: 0,
                top: 0,
                right: size,
                bottom: size,
            };

            rt.BindDC(surf.dc(), &bind_rect)?;
            rt.BeginDraw();
            rt.Clear(Some(&D2D1_COLOR_F {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.0,
            }));

            let badge = D2D1_ROUNDED_RECT {
                rect: D2D_RECT_F {
                    left: 1.4 * u,
                    top: 1.4 * u,
                    right: 14.6 * u,
                    bottom: 14.6 * u,
                },
                radiusX: 3.6 * u,
                radiusY: 3.6 * u,
            };
            let fill_col = if light_taskbar {
                D2D1_COLOR_F {
                    r: 0.08,
                    g: 0.09,
                    b: 0.12,
                    a: 0.10,
                }
            } else {
                D2D1_COLOR_F {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: 0.14,
                }
            };
            let rim_col = if light_taskbar {
                D2D1_COLOR_F {
                    r: 0.08,
                    g: 0.09,
                    b: 0.12,
                    a: 0.84,
                }
            } else {
                D2D1_COLOR_F {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: 0.90,
                }
            };
            if let Ok(fb) = rt.CreateSolidColorBrush(&fill_col, None) {
                rt.FillRoundedRectangle(&badge, &fb);
            }
            if let Ok(rb) = rt.CreateSolidColorBrush(&rim_col, None) {
                rt.DrawRoundedRectangle(&badge, &rb, 1.15 * u, None);
            }

            // Four-point sparkle star in center-left
            let star_col = if light_taskbar {
                D2D1_COLOR_F {
                    r: 0.08,
                    g: 0.09,
                    b: 0.12,
                    a: 0.92,
                }
            } else {
                D2D1_COLOR_F {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: 0.96,
                }
            };
            if let (Ok(star_geo), Ok(star_brush)) = (
                d2d_factory.CreatePathGeometry(),
                rt.CreateSolidColorBrush(&star_col, None),
            ) {
                if let Ok(sink) = star_geo.Open() {
                    let cx = 7.4 * u;
                    let cy = 8.4 * u;
                    let outer = 4.1 * u;
                    let inner = 1.25 * u;
                    sink.BeginFigure(
                        D2D_POINT_2F {
                            x: cx,
                            y: cy - outer,
                        },
                        D2D1_FIGURE_BEGIN_FILLED,
                    );
                    sink.AddLine(D2D_POINT_2F {
                        x: cx + inner,
                        y: cy - inner,
                    });
                    sink.AddLine(D2D_POINT_2F {
                        x: cx + outer,
                        y: cy,
                    });
                    sink.AddLine(D2D_POINT_2F {
                        x: cx + inner,
                        y: cy + inner,
                    });
                    sink.AddLine(D2D_POINT_2F {
                        x: cx,
                        y: cy + outer,
                    });
                    sink.AddLine(D2D_POINT_2F {
                        x: cx - inner,
                        y: cy + inner,
                    });
                    sink.AddLine(D2D_POINT_2F {
                        x: cx - outer,
                        y: cy,
                    });
                    sink.AddLine(D2D_POINT_2F {
                        x: cx - inner,
                        y: cy - inner,
                    });
                    sink.EndFigure(D2D1_FIGURE_END_CLOSED);
                    let _ = sink.Close();
                    rt.FillGeometry(&star_geo, &star_brush, None);
                }
            }

            // Calibrated accent dot in upper-right
            let calibrated_accent = adjust_accent_for_theme(accent, light_taskbar);
            if let Ok(ab) = rt.CreateSolidColorBrush(&d2d_color_rgb(calibrated_accent, 0.98), None)
            {
                rt.FillEllipse(
                    &D2D1_ELLIPSE {
                        point: D2D_POINT_2F {
                            x: 11.4 * u,
                            y: 4.6 * u,
                        },
                        radiusX: 1.35 * u,
                        radiusY: 1.35 * u,
                    },
                    &ab,
                );
            }

            rt.EndDraw(None, None)?;

            let row_words = (size as usize).div_ceil(16);
            let mask_bytes = vec![0u8; row_words * 2 * (size as usize)];
            let mask_bmp = CreateBitmap(size, size, 1, 1, Some(mask_bytes.as_ptr().cast()));
            if mask_bmp.is_invalid() {
                return Err(windows::core::Error::from_win32());
            }

            let info = ICONINFO {
                fIcon: BOOL(1),
                xHotspot: 0,
                yHotspot: 0,
                hbmMask: mask_bmp,
                hbmColor: surf.bitmap_handle(),
            };
            let res = CreateIconIndirect(&info);
            let _ = DeleteObject(mask_bmp);
            res
        }
    }
}
