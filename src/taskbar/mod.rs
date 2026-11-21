pub mod config;
pub mod swca;
pub mod tap_host;

#[cfg(windows)]
mod liquid_glass;

#[allow(unused_imports)]
pub use config::TaskbarStyle;
#[cfg(windows)]
pub use win::TaskbarController;

#[cfg(windows)]
mod win {
    use super::config::{portable_config_path, TaskbarConfig, TaskbarStyle};
    use super::liquid_glass::{
        collect_tooltip_island_origins, find_visible_alttab_host_window,
        find_visible_hover_flyout_host_window, find_visible_menu_flyout_host_windows,
        find_visible_osd_host_window, find_visible_shell_experience_host_windows,
        find_visible_shell_host_windows, find_visible_tooltip_host_window, LiquidGlassManager,
    };
    use super::swca::{
        alt_tab_item_card_colors, enumerate_taskbar_windows, explorer_pid_from_taskbars,
        map_tooltip_card_rect_to_screen, map_xaml_card_rect_to_screen,
        map_xaml_card_rect_to_screen_with_min, match_menu_flyout_hosts_to_rects,
        match_shell_host_cards_to_screen, menu_flyout_fallback_card_colors,
        osd_fallback_card_colors, taskbar_thread_ids, tooltip_fallback_card_colors,
        MAX_MENU_FLYOUT_SLOTS, MAX_SHELL_CARD_SLOTS,
    };
    use super::tap_host::{
        find_shell_experience_host_pid, find_shell_host_pid, ShellFlyoutTapClient, TapClient,
    };
    use crate::color::Rgb;
    use crate::system::{
        is_exclusive_fullscreen_or_presentation, is_transparency_enabled,
        set_active_taskbar_surface_style, should_suppress_osd_backdrop,
        should_suppress_tooltip_backdrop,
    };
    use std::path::PathBuf;
    use std::time::{Duration, Instant};
    use windows::Win32::Foundation::{HWND, RECT};
    use windows::Win32::Graphics::Direct2D::ID2D1Factory1;

    struct ShellFlyoutHostState {
        tap: ShellFlyoutTapClient,
        cached_pid: u32,
        last_pid_scan: Instant,
        active: bool,
        last_query: Instant,
        last_sig: Vec<(isize, [i32; 4])>,
        suppressed: bool,
        find_pid: fn() -> u32,
    }

    impl ShellFlyoutHostState {
        fn new(tap: ShellFlyoutTapClient, find_pid: fn() -> u32) -> Self {
            let past = Instant::now() - Duration::from_secs(10);
            Self {
                tap,
                cached_pid: 0,
                last_pid_scan: past,
                active: false,
                last_query: past,
                last_sig: Vec::new(),
                suppressed: false,
                find_pid,
            }
        }

        fn resolve_pid(&mut self, force: bool) -> u32 {
            if force
                || self.cached_pid == 0
                || self.last_pid_scan.elapsed() >= Duration::from_secs(2)
            {
                self.cached_pid = (self.find_pid)();
                self.last_pid_scan = Instant::now();
            }
            self.cached_pid
        }

        fn reset_tracking(&mut self, clear_colors: bool) {
            self.active = false;
            self.last_sig.clear();
            self.suppressed = false;
            self.tap.clear_flyout_rects();
            self.tap.set_flyout_suppressed(0, false);
            if clear_colors {
                self.tap.set_flyout_fallback_colors(0, None);
            }
        }
    }

    /// Coordinates Windows 11 taskbar, overflow flyout, Alt+Tab switcher, hover flyout, hardware OSD,
    /// menu flyouts/JumpLists, Quick Settings, Notification Center, Floating Toasts, and ToolTips across all monitors.
    pub struct TaskbarController {
        config_path: PathBuf,
        style: TaskbarStyle,
        light_glass: bool,
        transparency_enabled: bool,
        tap: TapClient,
        glass: LiquidGlassManager,
        last_maintain: Instant,
        alt_tab_host_active: bool,
        last_alt_tab_query: Instant,
        hover_flyout_host_active: bool,
        last_hover_flyout_query: Instant,
        last_hover_flyout_host_rect: Option<RECT>,
        known_alt_tab_tid: Option<u32>,
        known_hover_flyout_tid: Option<u32>,
        osd_host_active: bool,
        last_osd_query: Instant,
        last_osd_host_rect: Option<RECT>,
        known_osd_tid: Option<u32>,
        osd_suppressed: bool,
        tooltip_host_active: bool,
        last_tooltip_query: Instant,
        last_tooltip_host_rect: Option<RECT>,
        known_tooltip_tid: Option<u32>,
        tooltip_suppressed: bool,
        menu_flyout_host_active: bool,
        last_menu_flyout_query: Instant,
        last_menu_flyout_sig: Vec<(isize, [i32; 4])>,
        known_menu_flyout_tids: Vec<u32>,
        menu_flyout_suppressed: bool,
        active_menu_flyout_hwnds: Vec<HWND>,
        shell_exp_host: ShellFlyoutHostState,
        shell_host: ShellFlyoutHostState,
    }

    impl TaskbarController {
        pub fn new(d2d_factory: ID2D1Factory1, light_glass: bool, _accent: Rgb) -> Self {
            let config_path = portable_config_path();
            let cfg = TaskbarConfig::load_from_path(&config_path);
            let transparency_enabled = is_transparency_enabled();
            let mut ctrl = Self {
                config_path,
                style: cfg.style,
                light_glass,
                transparency_enabled,
                tap: TapClient::new(),
                glass: LiquidGlassManager::new(d2d_factory, light_glass, transparency_enabled),
                last_maintain: Instant::now() - Duration::from_secs(10),
                alt_tab_host_active: false,
                last_alt_tab_query: Instant::now() - Duration::from_secs(10),
                hover_flyout_host_active: false,
                last_hover_flyout_query: Instant::now() - Duration::from_secs(10),
                last_hover_flyout_host_rect: None,
                known_alt_tab_tid: None,
                known_hover_flyout_tid: None,
                osd_host_active: false,
                last_osd_query: Instant::now() - Duration::from_secs(10),
                last_osd_host_rect: None,
                known_osd_tid: None,
                osd_suppressed: false,
                tooltip_host_active: false,
                last_tooltip_query: Instant::now() - Duration::from_secs(10),
                last_tooltip_host_rect: None,
                known_tooltip_tid: None,
                tooltip_suppressed: false,
                menu_flyout_host_active: false,
                last_menu_flyout_query: Instant::now() - Duration::from_secs(10),
                last_menu_flyout_sig: Vec::new(),
                known_menu_flyout_tids: Vec::new(),
                menu_flyout_suppressed: false,
                active_menu_flyout_hwnds: Vec::new(),
                shell_exp_host: ShellFlyoutHostState::new(
                    ShellFlyoutTapClient::new_shell_experience(),
                    find_shell_experience_host_pid,
                ),
                shell_host: ShellFlyoutHostState::new(
                    ShellFlyoutTapClient::new_shell_host(),
                    find_shell_host_pid,
                ),
            };
            set_active_taskbar_surface_style(ctrl.style.into());
            if ctrl.style != TaskbarStyle::Default {
                ctrl.apply_current_style(true, false);
            }
            ctrl
        }

        pub fn style(&self) -> TaskbarStyle {
            self.style
        }

        pub fn explorer_pid(&self) -> u32 {
            let taskbars = enumerate_taskbar_windows();
            explorer_pid_from_taskbars(&taskbars).unwrap_or(0)
        }

        pub fn shell_experience_pid(&self) -> u32 {
            self.shell_exp_host.cached_pid
        }

        pub fn shell_host_pid(&self) -> u32 {
            self.shell_host.cached_pid
        }

        pub fn set_overlay_notify_target(&mut self, hwnd: HWND, msg: u32) {
            self.tap.set_notify_target(hwnd, msg);
            self.shell_exp_host.tap.set_notify_target(hwnd, msg);
            self.shell_host.tap.set_notify_target(hwnd, msg);
        }

        pub fn needs_fast_tick(&self) -> bool {
            self.style == TaskbarStyle::LiquidGlass
                || self.glass.is_overflow_visible()
                || self.alt_tab_host_active
                || self.hover_flyout_host_active
                || self.osd_host_active
                || self.tooltip_host_active
                || self.menu_flyout_host_active
                || self.shell_exp_host.active
                || self.shell_host.active
        }

        pub fn needs_timer(&self) -> bool {
            self.style != TaskbarStyle::Default
        }

        pub fn sync_overflow_now(&mut self) {
            if self.style == TaskbarStyle::Default {
                self.alt_tab_host_active = false;
                self.hover_flyout_host_active = false;
                self.last_hover_flyout_host_rect = None;
                self.osd_host_active = false;
                self.last_osd_host_rect = None;
                self.osd_suppressed = false;
                self.tooltip_host_active = false;
                self.last_tooltip_host_rect = None;
                self.tooltip_suppressed = false;
                self.menu_flyout_host_active = false;
                self.last_menu_flyout_sig.clear();
                self.menu_flyout_suppressed = false;
                self.active_menu_flyout_hwnds.clear();
                self.shell_exp_host.reset_tracking(true);
                self.shell_host.reset_tracking(true);
                self.tap.clear_alt_tab_rect();
                self.tap.clear_hover_flyout_rect();
                self.tap.clear_osd_rect();
                self.tap.clear_tooltip_rect();
                self.tap.clear_menu_flyout_rects();
                self.tap.set_osd_suppressed(0, false);
                self.tap.set_osd_fallback_colors(0, None);
                self.tap.set_tooltip_suppressed(0, false);
                self.tap.set_tooltip_fallback_colors(0, None);
                self.tap.set_menu_flyout_suppressed(0, false);
                self.tap.set_menu_flyout_fallback_colors(0, None);
                self.glass.hide_overflow_backdrop();
                self.glass.hide_alt_tab_backdrop();
                self.glass.hide_hover_flyout_backdrop();
                self.glass.hide_osd_backdrop();
                self.glass.hide_tooltip_backdrop();
                self.glass.hide_shell_cards();
                self.glass.hide_menu_flyout_backdrops();
                return;
            }
            let taskbars = enumerate_taskbar_windows();
            let explorer_pid = explorer_pid_from_taskbars(&taskbars).unwrap_or(0);
            let taskbar_tids = taskbar_thread_ids(&taskbars);
            if self.glass.sync_overflow() && explorer_pid != 0 {
                self.tap.reassert_hide(explorer_pid);
            }
            self.sync_alt_tab_state(explorer_pid, &taskbar_tids);
            self.sync_hover_flyout_state(explorer_pid, &taskbar_tids);
            self.sync_osd_state(explorer_pid, &taskbar_tids);
            self.sync_menu_flyout_state(explorer_pid, &taskbar_tids);
            self.sync_tooltip_state(explorer_pid, &taskbar_tids);
            self.sync_shell_flyouts_state();
        }

        pub fn hide_overflow_backdrop(&mut self) {
            self.glass.hide_overflow_backdrop();
        }

        pub fn set_style(&mut self, style: TaskbarStyle) {
            if self.tap.is_circuit_tripped() && style != TaskbarStyle::Default {
                let taskbars = enumerate_taskbar_windows();
                self.fallback_to_default_on_trip(&taskbars);
                return;
            }
            if self.style == style {
                return;
            }
            self.style = style;
            set_active_taskbar_surface_style(style.into());
            let _ = (TaskbarConfig { style }).save_to_path(&self.config_path);
            self.apply_current_style(true, true);
        }

        pub fn refresh_theme(&mut self, light_glass: bool, _accent: Rgb) {
            let transparency_enabled = is_transparency_enabled();
            self.light_glass = light_glass;
            self.transparency_enabled = transparency_enabled;
            self.glass.set_theme(light_glass);
            self.glass.set_transparency_enabled(transparency_enabled);
            if self.style != TaskbarStyle::Default {
                self.last_maintain = Instant::now() - Duration::from_secs(10);
                self.apply_current_style(false, false);
            }
        }

        /// Re-applies style after Explorer restarts; returns `true` if safety circuit breaker reverted to `Default`.
        pub fn on_taskbar_created(&mut self) -> bool {
            self.known_alt_tab_tid = None;
            self.known_hover_flyout_tid = None;
            self.known_osd_tid = None;
            self.known_tooltip_tid = None;
            self.known_menu_flyout_tids.clear();
            self.alt_tab_host_active = false;
            self.hover_flyout_host_active = false;
            self.last_hover_flyout_host_rect = None;
            self.osd_host_active = false;
            self.last_osd_host_rect = None;
            self.osd_suppressed = false;
            self.tooltip_host_active = false;
            self.last_tooltip_host_rect = None;
            self.tooltip_suppressed = false;
            self.menu_flyout_host_active = false;
            self.last_menu_flyout_sig.clear();
            self.menu_flyout_suppressed = false;
            self.active_menu_flyout_hwnds.clear();
            self.shell_exp_host.reset_tracking(true);
            self.shell_host.reset_tracking(true);
            self.shell_exp_host.resolve_pid(true);
            self.shell_host.resolve_pid(true);
            self.tap.clear_alt_tab_rect();
            self.tap.clear_hover_flyout_rect();
            self.tap.clear_osd_rect();
            self.tap.clear_tooltip_rect();
            self.tap.clear_menu_flyout_rects();
            self.tap.set_osd_suppressed(0, false);
            self.tap.set_osd_fallback_colors(0, None);
            self.tap.set_tooltip_suppressed(0, false);
            self.tap.set_tooltip_fallback_colors(0, None);
            self.tap.set_menu_flyout_suppressed(0, false);
            self.tap.set_menu_flyout_fallback_colors(0, None);
            self.tap.on_explorer_restarted();
            self.glass.reset_windows();
            if self.tap.is_circuit_tripped() {
                let taskbars = enumerate_taskbar_windows();
                self.fallback_to_default_on_trip(&taskbars);
                return true;
            }
            if self.style != TaskbarStyle::Default {
                self.apply_current_style(true, false);
            }
            false
        }

        /// Periodic maintenance; returns `true` if safety circuit breaker reverted to `Default`.
        pub fn tick(&mut self) -> bool {
            if self.style == TaskbarStyle::Default {
                return false;
            }
            let taskbars = enumerate_taskbar_windows();
            let explorer_pid = explorer_pid_from_taskbars(&taskbars).unwrap_or(0);
            let taskbar_tids = taskbar_thread_ids(&taskbars);

            if self.last_maintain.elapsed() >= Duration::from_millis(120) {
                self.tap.maintain(explorer_pid);
                if self.tap.is_circuit_tripped() {
                    self.fallback_to_default_on_trip(&taskbars);
                    return true;
                }
                let shell_exp_pid = self.shell_exp_host.resolve_pid(false);
                self.shell_exp_host.tap.maintain(shell_exp_pid);
                let shell_host_pid = self.shell_host.resolve_pid(false);
                self.shell_host.tap.maintain(shell_host_pid);
                let trans_now = is_transparency_enabled();
                let trans_changed = trans_now != self.transparency_enabled;
                if trans_changed {
                    self.transparency_enabled = trans_now;
                    self.glass.set_transparency_enabled(trans_now);
                }
                if (trans_changed && self.style != TaskbarStyle::LiquidGlass)
                    || matches!(self.style, TaskbarStyle::Blur | TaskbarStyle::Acrylic)
                {
                    self.glass.sync(&taskbars);
                }
                self.last_maintain = Instant::now();
            }

            if self.style == TaskbarStyle::LiquidGlass {
                self.glass.sync(&taskbars);
            }
            if self.glass.sync_overflow() && explorer_pid != 0 {
                self.tap.reassert_hide(explorer_pid);
            }
            self.sync_alt_tab_state(explorer_pid, &taskbar_tids);
            self.sync_hover_flyout_state(explorer_pid, &taskbar_tids);
            self.sync_osd_state(explorer_pid, &taskbar_tids);
            self.sync_menu_flyout_state(explorer_pid, &taskbar_tids);
            self.sync_tooltip_state(explorer_pid, &taskbar_tids);
            self.sync_shell_flyouts_state();
            false
        }

        pub fn restore_default_and_shutdown(&mut self) {
            let taskbars = enumerate_taskbar_windows();
            self.alt_tab_host_active = false;
            self.hover_flyout_host_active = false;
            self.last_hover_flyout_host_rect = None;
            self.osd_host_active = false;
            self.last_osd_host_rect = None;
            self.osd_suppressed = false;
            self.tooltip_host_active = false;
            self.last_tooltip_host_rect = None;
            self.tooltip_suppressed = false;
            self.menu_flyout_host_active = false;
            self.last_menu_flyout_sig.clear();
            self.menu_flyout_suppressed = false;
            self.active_menu_flyout_hwnds.clear();
            self.shell_exp_host.reset_tracking(true);
            self.shell_host.reset_tracking(true);
            self.tap.clear_alt_tab_rect();
            self.tap.clear_hover_flyout_rect();
            self.tap.clear_osd_rect();
            self.tap.clear_tooltip_rect();
            self.tap.clear_menu_flyout_rects();
            self.tap.set_osd_suppressed(0, false);
            self.tap.set_alt_tab_item_colors(0, None);
            self.tap.set_osd_fallback_colors(0, None);
            self.tap.set_tooltip_suppressed(0, false);
            self.tap.set_tooltip_fallback_colors(0, None);
            self.tap.set_menu_flyout_suppressed(0, false);
            self.tap.set_menu_flyout_fallback_colors(0, None);
            self.glass.set_style(TaskbarStyle::Default, &taskbars);
            set_active_taskbar_surface_style(TaskbarStyle::Default.into());
            self.shell_exp_host.tap.shutdown();
            self.shell_host.tap.shutdown();
            self.tap.shutdown();
        }

        fn fallback_to_default_on_trip(&mut self, taskbars: &[HWND]) {
            self.style = TaskbarStyle::Default;
            self.alt_tab_host_active = false;
            self.hover_flyout_host_active = false;
            self.last_hover_flyout_host_rect = None;
            self.osd_host_active = false;
            self.last_osd_host_rect = None;
            self.osd_suppressed = false;
            self.tooltip_host_active = false;
            self.last_tooltip_host_rect = None;
            self.tooltip_suppressed = false;
            self.menu_flyout_host_active = false;
            self.last_menu_flyout_sig.clear();
            self.menu_flyout_suppressed = false;
            self.active_menu_flyout_hwnds.clear();
            self.shell_exp_host.reset_tracking(true);
            self.shell_host.reset_tracking(true);
            self.shell_exp_host.tap.set_hide_background(0, false, false);
            self.shell_host.tap.set_hide_background(0, false, false);
            self.tap.clear_alt_tab_rect();
            self.tap.clear_hover_flyout_rect();
            self.tap.clear_osd_rect();
            self.tap.clear_tooltip_rect();
            self.tap.clear_menu_flyout_rects();
            self.tap.set_osd_suppressed(0, false);
            self.tap.set_alt_tab_item_colors(0, None);
            self.tap.set_osd_fallback_colors(0, None);
            self.tap.set_tooltip_suppressed(0, false);
            self.tap.set_tooltip_fallback_colors(0, None);
            self.tap.set_menu_flyout_suppressed(0, false);
            self.tap.set_menu_flyout_fallback_colors(0, None);
            set_active_taskbar_surface_style(TaskbarStyle::Default.into());
            self.glass.set_style(TaskbarStyle::Default, taskbars);
        }

        fn sync_alt_tab_state(&mut self, explorer_pid: u32, taskbar_tids: &[u32]) {
            if self.style == TaskbarStyle::Default || explorer_pid == 0 {
                if self.alt_tab_host_active {
                    self.alt_tab_host_active = false;
                    self.tap.clear_alt_tab_rect();
                }
                self.glass.hide_alt_tab_backdrop();
                return;
            }

            let cached = self.tap.cached_alt_tab_rect();
            if let Some(tid) = cached.map(|c| c.thread_id).filter(|&t| t != 0) {
                self.known_alt_tab_tid = Some(tid);
            }
            let preferred_tid = cached
                .map(|c| c.thread_id)
                .filter(|&t| t != 0)
                .or(self.known_alt_tab_tid);
            let mut excluded_tids = Vec::with_capacity(2);
            if let Some(t) = self
                .known_hover_flyout_tid
                .filter(|&t| Some(t) != self.known_alt_tab_tid)
            {
                excluded_tids.push(t);
            }
            if let Some(t) = self
                .known_osd_tid
                .filter(|&t| Some(t) != self.known_alt_tab_tid)
            {
                excluded_tids.push(t);
            }
            let Some((host_hwnd, host_rect, dpi, host_tid)) = find_visible_alttab_host_window(
                explorer_pid,
                preferred_tid,
                &excluded_tids,
                taskbar_tids,
            ) else {
                if self.alt_tab_host_active {
                    self.alt_tab_host_active = false;
                    self.tap.clear_alt_tab_rect();
                }
                self.glass.hide_alt_tab_backdrop();
                return;
            };

            let just_shown = !self.alt_tab_host_active;
            self.alt_tab_host_active = true;
            let retry_interval = if cached.is_none() {
                Duration::from_millis(25)
            } else {
                Duration::from_millis(120)
            };
            if just_shown || self.last_alt_tab_query.elapsed() >= retry_interval {
                self.tap.request_alt_tab_rect(explorer_pid);
                self.last_alt_tab_query = Instant::now();
            }

            if let Some(cache) = cached {
                if cache.thread_id == 0 || cache.thread_id == host_tid {
                    if let Some([l, t, r, b]) = map_xaml_card_rect_to_screen(
                        [
                            host_rect.left,
                            host_rect.top,
                            host_rect.right,
                            host_rect.bottom,
                        ],
                        dpi,
                        cache.rect_dip,
                    ) {
                        let card_rect = RECT {
                            left: l,
                            top: t,
                            right: r,
                            bottom: b,
                        };
                        let _ = self.glass.sync_alt_tab(host_hwnd, card_rect, dpi);
                        return;
                    }
                }
            }
            self.glass.hide_alt_tab_backdrop();
        }

        fn sync_hover_flyout_state(&mut self, explorer_pid: u32, taskbar_tids: &[u32]) {
            if self.style == TaskbarStyle::Default || explorer_pid == 0 {
                if self.hover_flyout_host_active {
                    self.hover_flyout_host_active = false;
                    self.last_hover_flyout_host_rect = None;
                    self.tap.clear_hover_flyout_rect();
                }
                self.glass.hide_hover_flyout_backdrop();
                return;
            }

            let cached = self.tap.cached_hover_flyout_rect();
            if let Some(tid) = cached.map(|c| c.thread_id).filter(|&t| t != 0) {
                self.known_hover_flyout_tid = Some(tid);
            }
            let preferred_tid = cached
                .map(|c| c.thread_id)
                .filter(|&t| t != 0)
                .or(self.known_hover_flyout_tid);
            let mut excluded_tids = Vec::with_capacity(2);
            if let Some(t) = self
                .known_alt_tab_tid
                .filter(|&t| Some(t) != self.known_hover_flyout_tid && !taskbar_tids.contains(&t))
            {
                excluded_tids.push(t);
            }
            if let Some(t) = self
                .known_osd_tid
                .filter(|&t| Some(t) != self.known_hover_flyout_tid && !taskbar_tids.contains(&t))
            {
                excluded_tids.push(t);
            }
            let Some((host_hwnd, host_rect, dpi, host_tid)) =
                find_visible_hover_flyout_host_window(
                    explorer_pid,
                    preferred_tid,
                    &excluded_tids,
                    taskbar_tids,
                )
            else {
                if self.hover_flyout_host_active {
                    self.hover_flyout_host_active = false;
                    self.last_hover_flyout_host_rect = None;
                    self.tap.clear_hover_flyout_rect();
                }
                self.glass.hide_hover_flyout_backdrop();
                return;
            };

            let just_shown = !self.hover_flyout_host_active;
            self.hover_flyout_host_active = true;
            let host_rect_changed = self.last_hover_flyout_host_rect.is_none_or(|prev| {
                prev.left != host_rect.left
                    || prev.top != host_rect.top
                    || prev.right != host_rect.right
                    || prev.bottom != host_rect.bottom
            });
            self.last_hover_flyout_host_rect = Some(host_rect);

            let retry_interval = if cached.is_none() {
                Duration::from_millis(20)
            } else {
                Duration::from_millis(30)
            };
            if just_shown
                || host_rect_changed
                || self.last_hover_flyout_query.elapsed() >= retry_interval
            {
                self.tap.request_hover_flyout_rect(explorer_pid);
                self.last_hover_flyout_query = Instant::now();
            }

            if let Some(cache) = cached {
                if cache.thread_id == 0 || cache.thread_id == host_tid {
                    if let Some([l, t, r, b]) = map_xaml_card_rect_to_screen_with_min(
                        [
                            host_rect.left,
                            host_rect.top,
                            host_rect.right,
                            host_rect.bottom,
                        ],
                        dpi,
                        cache.rect_dip,
                        48.0,
                        36.0,
                    ) {
                        let card_rect = RECT {
                            left: l,
                            top: t,
                            right: r,
                            bottom: b,
                        };
                        let _ = self.glass.sync_hover_flyout(host_hwnd, card_rect, dpi);
                        return;
                    }
                }
            }
            self.glass.hide_hover_flyout_backdrop();
        }

        fn sync_osd_state(&mut self, explorer_pid: u32, taskbar_tids: &[u32]) {
            if self.style == TaskbarStyle::Default || explorer_pid == 0 {
                if self.osd_host_active {
                    self.osd_host_active = false;
                    self.last_osd_host_rect = None;
                    self.tap.clear_osd_rect();
                }
                if self.osd_suppressed {
                    self.osd_suppressed = false;
                    self.tap.set_osd_suppressed(explorer_pid, false);
                }
                self.glass.hide_osd_backdrop();
                return;
            }

            let cached = self.tap.cached_osd_rect();
            if let Some(tid) = cached.map(|c| c.thread_id).filter(|&t| t != 0) {
                self.known_osd_tid = Some(tid);
            }
            let preferred_tid = cached
                .map(|c| c.thread_id)
                .filter(|&t| t != 0)
                .or(self.known_osd_tid);
            let mut excluded_tids = Vec::with_capacity(2);
            if let Some(t) = self
                .known_alt_tab_tid
                .filter(|&t| Some(t) != self.known_osd_tid)
            {
                excluded_tids.push(t);
            }
            if let Some(t) = self
                .known_hover_flyout_tid
                .filter(|&t| Some(t) != self.known_osd_tid)
            {
                excluded_tids.push(t);
            }
            let Some((host_hwnd, host_rect, dpi, host_tid)) = find_visible_osd_host_window(
                explorer_pid,
                preferred_tid,
                &excluded_tids,
                taskbar_tids,
            ) else {
                if self.osd_host_active {
                    self.osd_host_active = false;
                    self.last_osd_host_rect = None;
                    self.tap.clear_osd_rect();
                }
                self.glass.hide_osd_backdrop();
                let exclusive_fs = is_exclusive_fullscreen_or_presentation();
                if self.osd_suppressed != exclusive_fs {
                    self.osd_suppressed = exclusive_fs;
                    self.tap.set_osd_suppressed(explorer_pid, exclusive_fs);
                }
                return;
            };

            let just_shown = !self.osd_host_active;
            self.osd_host_active = true;
            let host_rect_changed = self.last_osd_host_rect.is_none_or(|prev| {
                prev.left != host_rect.left
                    || prev.top != host_rect.top
                    || prev.right != host_rect.right
                    || prev.bottom != host_rect.bottom
            });
            self.last_osd_host_rect = Some(host_rect);

            let cached_card_rect = cached.and_then(|cache| {
                if cache.thread_id != 0 && cache.thread_id != host_tid {
                    return None;
                }
                map_xaml_card_rect_to_screen_with_min(
                    [
                        host_rect.left,
                        host_rect.top,
                        host_rect.right,
                        host_rect.bottom,
                    ],
                    dpi,
                    cache.rect_dip,
                    80.0,
                    32.0,
                )
                .map(|[l, t, r, b]| RECT {
                    left: l,
                    top: t,
                    right: r,
                    bottom: b,
                })
            });

            if should_suppress_osd_backdrop(host_hwnd, cached_card_rect.unwrap_or(host_rect)) {
                if !self.osd_suppressed {
                    self.osd_suppressed = true;
                    self.tap.set_osd_suppressed(explorer_pid, true);
                }
                self.glass.hide_osd_backdrop();
                return;
            }

            if self.osd_suppressed {
                self.osd_suppressed = false;
                self.tap.set_osd_suppressed(explorer_pid, false);
            }

            let retry_interval = if cached.is_none() {
                Duration::from_millis(20)
            } else {
                Duration::from_millis(30)
            };
            if just_shown || host_rect_changed || self.last_osd_query.elapsed() >= retry_interval {
                self.tap.request_osd_rect(explorer_pid);
                self.last_osd_query = Instant::now();
            }

            if let Some(card_rect) = cached_card_rect {
                if should_suppress_osd_backdrop(host_hwnd, card_rect) {
                    if !self.osd_suppressed {
                        self.osd_suppressed = true;
                        self.tap.set_osd_suppressed(explorer_pid, true);
                    }
                    self.glass.hide_osd_backdrop();
                    return;
                }
                let _ = self.glass.sync_osd(host_hwnd, card_rect, dpi);
                return;
            }
            self.glass.hide_osd_backdrop();
        }

        fn sync_menu_flyout_state(&mut self, explorer_pid: u32, taskbar_tids: &[u32]) {
            if self.style == TaskbarStyle::Default || explorer_pid == 0 {
                if self.menu_flyout_host_active {
                    self.menu_flyout_host_active = false;
                    self.last_menu_flyout_sig.clear();
                    self.active_menu_flyout_hwnds.clear();
                    self.tap.clear_menu_flyout_rects();
                }
                if self.menu_flyout_suppressed {
                    self.menu_flyout_suppressed = false;
                    self.tap.set_menu_flyout_suppressed(explorer_pid, false);
                }
                self.glass.hide_menu_flyout_backdrops();
                return;
            }

            let cached = self.tap.cached_menu_flyout_rects();
            for c in &cached {
                if c.thread_id != 0 && !self.known_menu_flyout_tids.contains(&c.thread_id) {
                    if self.known_menu_flyout_tids.len() >= 8 {
                        self.known_menu_flyout_tids.remove(0);
                    }
                    self.known_menu_flyout_tids.push(c.thread_id);
                }
            }

            let hosts = find_visible_menu_flyout_host_windows(
                explorer_pid,
                &self.known_menu_flyout_tids,
                taskbar_tids,
                &[],
            );
            if hosts.is_empty() {
                if self.menu_flyout_host_active {
                    self.menu_flyout_host_active = false;
                    self.last_menu_flyout_sig.clear();
                    self.active_menu_flyout_hwnds.clear();
                    self.tap.clear_menu_flyout_rects();
                }
                self.glass.hide_menu_flyout_backdrops();
                let exclusive_fs = is_exclusive_fullscreen_or_presentation();
                if self.menu_flyout_suppressed != exclusive_fs {
                    self.menu_flyout_suppressed = exclusive_fs;
                    self.tap
                        .set_menu_flyout_suppressed(explorer_pid, exclusive_fs);
                }
                return;
            }

            let just_shown = !self.menu_flyout_host_active;
            self.menu_flyout_host_active = true;
            let sig: Vec<(isize, [i32; 4])> = hosts
                .iter()
                .map(|&(h, r, _, _)| (h.0 as isize, [r.left, r.top, r.right, r.bottom]))
                .collect();
            let sig_changed = self.last_menu_flyout_sig != sig;
            if sig_changed {
                self.last_menu_flyout_sig = sig;
            }

            let tap_pairs: Vec<(u32, [f32; 4])> = cached
                .iter()
                .map(|c| (c.thread_id, c.rect_dip))
                .collect();
            let host_tuples: Vec<([i32; 4], u32, u32)> = hosts
                .iter()
                .map(|&(_, r, dpi, tid)| ([r.left, r.top, r.right, r.bottom], dpi, tid))
                .collect();
            let origins_per_host: Vec<Vec<[i32; 2]>> = hosts
                .iter()
                .map(|&(h, _, _, tid)| collect_tooltip_island_origins(h, explorer_pid, tid))
                .collect();

            let matched_indices =
                match_menu_flyout_hosts_to_rects(&host_tuples, &tap_pairs, &origins_per_host);
            let mut matched_cards: Vec<(HWND, RECT, u32)> =
                Vec::with_capacity(matched_indices.len().min(MAX_MENU_FLYOUT_SLOTS));
            for (hi, [l, t, r, b]) in matched_indices {
                if let Some(&(host_hwnd, _, dpi, _)) = hosts.get(hi) {
                    matched_cards.push((
                        host_hwnd,
                        RECT {
                            left: l,
                            top: t,
                            right: r,
                            bottom: b,
                        },
                        dpi,
                    ));
                }
            }

            let any_suppressed = if !matched_cards.is_empty() {
                matched_cards
                    .iter()
                    .any(|&(hh, cr, _)| should_suppress_tooltip_backdrop(hh, cr))
            } else {
                is_exclusive_fullscreen_or_presentation()
                    || hosts
                        .iter()
                        .any(|&(hh, hr, _, _)| should_suppress_tooltip_backdrop(hh, hr))
            };

            if any_suppressed {
                self.active_menu_flyout_hwnds =
                    matched_cards.iter().map(|&(h, _, _)| h).collect();
                if !self.menu_flyout_suppressed {
                    self.menu_flyout_suppressed = true;
                    self.tap.set_menu_flyout_suppressed(explorer_pid, true);
                }
                self.glass.hide_menu_flyout_backdrops();
                return;
            }

            if self.menu_flyout_suppressed {
                self.menu_flyout_suppressed = false;
                self.tap.set_menu_flyout_suppressed(explorer_pid, false);
            }

            let retry_interval = if matched_cards.is_empty() {
                Duration::from_millis(20)
            } else {
                Duration::from_millis(30)
            };
            if just_shown || sig_changed || self.last_menu_flyout_query.elapsed() >= retry_interval
            {
                self.tap.request_menu_flyout_rects(explorer_pid);
                self.last_menu_flyout_query = Instant::now();
            }

            if !matched_cards.is_empty() {
                self.active_menu_flyout_hwnds =
                    matched_cards.iter().map(|&(h, _, _)| h).collect();
                let _ = self.glass.sync_menu_flyouts(&matched_cards);
            } else {
                self.active_menu_flyout_hwnds.clear();
                self.glass.hide_menu_flyout_backdrops();
            }
        }

        fn sync_tooltip_state(&mut self, explorer_pid: u32, taskbar_tids: &[u32]) {
            if self.style == TaskbarStyle::Default || explorer_pid == 0 {
                if self.tooltip_host_active {
                    self.tooltip_host_active = false;
                    self.last_tooltip_host_rect = None;
                    self.tap.clear_tooltip_rect();
                }
                if self.tooltip_suppressed {
                    self.tooltip_suppressed = false;
                    self.tap.set_tooltip_suppressed(explorer_pid, false);
                }
                self.glass.hide_tooltip_backdrop();
                return;
            }

            let cached = self.tap.cached_tooltip_rect();
            if let Some(tid) = cached.map(|c| c.thread_id).filter(|&t| t != 0) {
                self.known_tooltip_tid = Some(tid);
            }
            let preferred_tid = cached
                .map(|c| c.thread_id)
                .filter(|&t| t != 0)
                .or(self.known_tooltip_tid);
            let expected_size_dip = cached.and_then(|c| {
                let w = c.rect_dip[2] - c.rect_dip[0];
                let h = c.rect_dip[3] - c.rect_dip[1];
                (w.is_finite() && h.is_finite() && w >= 20.0 && h >= 16.0).then_some((w, h))
            });

            let (matched, cached) = if let Some(found) = find_visible_tooltip_host_window(
                explorer_pid,
                preferred_tid,
                taskbar_tids,
                expected_size_dip,
                &self.active_menu_flyout_hwnds,
            ) {
                (Some(found), cached)
            } else if expected_size_dip.is_some() {
                self.tap.clear_tooltip_rect();
                (
                    find_visible_tooltip_host_window(
                        explorer_pid,
                        preferred_tid,
                        taskbar_tids,
                        None,
                        &self.active_menu_flyout_hwnds,
                    ),
                    None,
                )
            } else {
                (None, cached)
            };

            let Some((host_hwnd, host_rect, dpi, host_tid)) = matched else {
                if self.tooltip_host_active {
                    self.tooltip_host_active = false;
                    self.last_tooltip_host_rect = None;
                    self.tap.clear_tooltip_rect();
                }
                self.glass.hide_tooltip_backdrop();
                let exclusive_fs = is_exclusive_fullscreen_or_presentation();
                if self.tooltip_suppressed != exclusive_fs {
                    self.tooltip_suppressed = exclusive_fs;
                    self.tap.set_tooltip_suppressed(explorer_pid, exclusive_fs);
                }
                return;
            };

            let just_shown = !self.tooltip_host_active;
            self.tooltip_host_active = true;
            let host_rect_changed = self.last_tooltip_host_rect.is_none_or(|prev| {
                prev.left != host_rect.left
                    || prev.top != host_rect.top
                    || prev.right != host_rect.right
                    || prev.bottom != host_rect.bottom
            });
            self.last_tooltip_host_rect = Some(host_rect);

            let cached_card_rect = cached.and_then(|cache| {
                if cache.thread_id != 0 && cache.thread_id != host_tid {
                    return None;
                }
                let island_origins =
                    collect_tooltip_island_origins(host_hwnd, explorer_pid, host_tid);
                map_tooltip_card_rect_to_screen(
                    [
                        host_rect.left,
                        host_rect.top,
                        host_rect.right,
                        host_rect.bottom,
                    ],
                    dpi,
                    cache.rect_dip,
                    &island_origins,
                )
                .map(|[l, t, r, b]| RECT {
                    left: l,
                    top: t,
                    right: r,
                    bottom: b,
                })
            });

            if should_suppress_tooltip_backdrop(host_hwnd, cached_card_rect.unwrap_or(host_rect))
            {
                if !self.tooltip_suppressed {
                    self.tooltip_suppressed = true;
                    self.tap.set_tooltip_suppressed(explorer_pid, true);
                }
                self.glass.hide_tooltip_backdrop();
                return;
            }

            if self.tooltip_suppressed {
                self.tooltip_suppressed = false;
                self.tap.set_tooltip_suppressed(explorer_pid, false);
            }

            let retry_interval = if cached.is_none() {
                Duration::from_millis(20)
            } else {
                Duration::from_millis(30)
            };
            if just_shown
                || host_rect_changed
                || self.last_tooltip_query.elapsed() >= retry_interval
            {
                self.tap.request_tooltip_rect(explorer_pid);
                self.last_tooltip_query = Instant::now();
            }

            if let Some(card_rect) = cached_card_rect {
                if should_suppress_tooltip_backdrop(host_hwnd, card_rect) {
                    if !self.tooltip_suppressed {
                        self.tooltip_suppressed = true;
                        self.tap.set_tooltip_suppressed(explorer_pid, true);
                    }
                    self.glass.hide_tooltip_backdrop();
                    return;
                }
                let _ = self.glass.sync_tooltip(host_hwnd, card_rect, dpi);
                return;
            }
            self.glass.hide_tooltip_backdrop();
        }

        fn sync_one_shell_flyout_host(
            style: TaskbarStyle,
            host: &mut ShellFlyoutHostState,
            enumerate_windows: fn(u32) -> Vec<(HWND, RECT, u32, u32, bool)>,
        ) -> Vec<(HWND, RECT, u32)> {
            let pid = host.resolve_pid(false);
            if style == TaskbarStyle::Default || pid == 0 || host.tap.is_circuit_tripped() {
                if host.active {
                    host.active = false;
                    host.last_sig.clear();
                    host.tap.clear_flyout_rects();
                }
                if host.suppressed {
                    host.suppressed = false;
                    host.tap.set_flyout_suppressed(pid, false);
                }
                return Vec::new();
            }

            let visible_windows = enumerate_windows(pid);
            if visible_windows.is_empty() {
                if host.active {
                    host.active = false;
                    host.last_sig.clear();
                    host.tap.clear_flyout_rects();
                }
                let exclusive_fs = is_exclusive_fullscreen_or_presentation();
                if host.suppressed != exclusive_fs {
                    host.suppressed = exclusive_fs;
                    host.tap.set_flyout_suppressed(pid, exclusive_fs);
                }
                return Vec::new();
            }

            let just_shown = !host.active;
            host.active = true;
            if just_shown {
                host.tap.retry_attach_on_flyout_open(pid);
            }

            let mut cur_sig: Vec<(isize, [i32; 4])> = visible_windows
                .iter()
                .map(|&(h, r, _, _, _)| (h.0 as isize, [r.left, r.top, r.right, r.bottom]))
                .collect();
            cur_sig.sort_unstable();
            let sig_changed = host.last_sig != cur_sig;
            host.last_sig = cur_sig;

            let cached = host.tap.cached_flyout_rects();
            let hosts_input: Vec<([i32; 4], u32, u32, isize, bool)> = visible_windows
                .iter()
                .map(|&(h, r, dpi, tid, is_popup)| {
                    (
                        [r.left, r.top, r.right, r.bottom],
                        dpi,
                        tid,
                        h.0 as isize,
                        is_popup,
                    )
                })
                .collect();
            let tap_rects: Vec<(u32, [f32; 4])> = cached
                .iter()
                .map(|c| (c.thread_id, c.rect_dip))
                .collect();
            let matched = match_shell_host_cards_to_screen(&hosts_input, &tap_rects);

            let occluded = if matched.is_empty() {
                visible_windows
                    .iter()
                    .any(|&(hwnd, rect, _, _, _)| should_suppress_osd_backdrop(hwnd, rect))
            } else {
                matched.iter().any(|&(hwnd_raw, [l, t, r, b])| {
                    should_suppress_osd_backdrop(
                        HWND(hwnd_raw as *mut std::ffi::c_void),
                        RECT {
                            left: l,
                            top: t,
                            right: r,
                            bottom: b,
                        },
                    )
                })
            };

            if occluded {
                if !host.suppressed {
                    host.suppressed = true;
                    host.tap.set_flyout_suppressed(pid, true);
                }
                return Vec::new();
            }

            if host.suppressed {
                host.suppressed = false;
                host.tap.set_flyout_suppressed(pid, false);
            }

            let retry_interval = if matched.is_empty() {
                Duration::from_millis(20)
            } else {
                Duration::from_millis(30)
            };
            if just_shown || sig_changed || host.last_query.elapsed() >= retry_interval {
                host.tap.request_flyout_rects(pid);
                host.last_query = Instant::now();
            }

            let mut out = Vec::with_capacity(matched.len());
            for (hwnd_raw, [l, t, r, b]) in matched {
                if let Some(&(hwnd, _, dpi, _, _)) = visible_windows
                    .iter()
                    .find(|&&(h, _, _, _, _)| (h.0 as isize) == hwnd_raw)
                {
                    out.push((
                        hwnd,
                        RECT {
                            left: l,
                            top: t,
                            right: r,
                            bottom: b,
                        },
                        dpi,
                    ));
                }
            }
            out
        }

        fn sync_shell_flyouts_state(&mut self) {
            if self.style == TaskbarStyle::Default {
                let _ = Self::sync_one_shell_flyout_host(
                    self.style,
                    &mut self.shell_exp_host,
                    find_visible_shell_experience_host_windows,
                );
                let _ = Self::sync_one_shell_flyout_host(
                    self.style,
                    &mut self.shell_host,
                    find_visible_shell_host_windows,
                );
                self.glass.hide_shell_cards();
                return;
            }

            let mut active_cards = Self::sync_one_shell_flyout_host(
                self.style,
                &mut self.shell_host,
                find_visible_shell_host_windows,
            );
            let mut exp_cards = Self::sync_one_shell_flyout_host(
                self.style,
                &mut self.shell_exp_host,
                find_visible_shell_experience_host_windows,
            );
            active_cards.append(&mut exp_cards);
            active_cards.truncate(MAX_SHELL_CARD_SLOTS);

            if active_cards.is_empty() {
                self.glass.hide_shell_cards();
            } else {
                let _ = self.glass.sync_shell_cards(&active_cards);
            }
        }

        fn apply_current_style(&mut self, update_tap: bool, user_initiated: bool) {
            let taskbars = enumerate_taskbar_windows();
            let explorer_pid = explorer_pid_from_taskbars(&taskbars).unwrap_or(0);
            let taskbar_tids = taskbar_thread_ids(&taskbars);

            if self.style == TaskbarStyle::Default {
                let shell_exp_pid = self.shell_exp_host.resolve_pid(false);
                let shell_host_pid = self.shell_host.resolve_pid(false);
                self.alt_tab_host_active = false;
                self.hover_flyout_host_active = false;
                self.last_hover_flyout_host_rect = None;
                self.osd_host_active = false;
                self.last_osd_host_rect = None;
                self.osd_suppressed = false;
                self.tooltip_host_active = false;
                self.last_tooltip_host_rect = None;
                self.tooltip_suppressed = false;
                self.menu_flyout_host_active = false;
                self.last_menu_flyout_sig.clear();
                self.menu_flyout_suppressed = false;
                self.active_menu_flyout_hwnds.clear();
                self.shell_exp_host.reset_tracking(true);
                self.shell_host.reset_tracking(true);
                self.tap.clear_alt_tab_rect();
                self.tap.clear_hover_flyout_rect();
                self.tap.clear_osd_rect();
                self.tap.clear_tooltip_rect();
                self.tap.clear_menu_flyout_rects();
                self.tap.set_osd_suppressed(explorer_pid, false);
                self.tap.set_alt_tab_item_colors(explorer_pid, None);
                self.tap.set_osd_fallback_colors(explorer_pid, None);
                self.tap.set_tooltip_suppressed(explorer_pid, false);
                self.tap.set_tooltip_fallback_colors(explorer_pid, None);
                self.tap.set_menu_flyout_suppressed(explorer_pid, false);
                self.tap
                    .set_menu_flyout_fallback_colors(explorer_pid, None);
                self.glass.set_style(TaskbarStyle::Default, &taskbars);
                if update_tap {
                    self.tap
                        .set_hide_background(explorer_pid, false, user_initiated);
                    self.shell_exp_host
                        .tap
                        .set_hide_background(shell_exp_pid, false, user_initiated);
                    self.shell_host
                        .tap
                        .set_hide_background(shell_host_pid, false, user_initiated);
                }
                return;
            }

            let shell_exp_pid = self.shell_exp_host.resolve_pid(true);
            let shell_host_pid = self.shell_host.resolve_pid(true);
            let item_colors = alt_tab_item_card_colors(self.style, self.light_glass)
                .map(|c| (c.bg_argb, c.border_argb));
            self.tap.set_alt_tab_item_colors(explorer_pid, item_colors);
            let osd_colors = osd_fallback_card_colors(self.style, self.light_glass)
                .map(|c| (c.bg_argb, c.border_argb));
            self.tap.set_osd_fallback_colors(explorer_pid, osd_colors);
            let tooltip_colors = tooltip_fallback_card_colors(self.style, self.light_glass)
                .map(|c| (c.bg_argb, c.border_argb));
            self.tap
                .set_tooltip_fallback_colors(explorer_pid, tooltip_colors);
            let menu_colors = menu_flyout_fallback_card_colors(self.style, self.light_glass)
                .map(|c| (c.bg_argb, c.border_argb));
            self.tap
                .set_menu_flyout_fallback_colors(explorer_pid, menu_colors);
            self.shell_exp_host
                .tap
                .set_flyout_fallback_colors(shell_exp_pid, menu_colors);
            self.shell_host
                .tap
                .set_flyout_fallback_colors(shell_host_pid, menu_colors);

            if update_tap {
                self.tap
                    .set_hide_background(explorer_pid, true, user_initiated);
                if self.tap.is_circuit_tripped() {
                    self.fallback_to_default_on_trip(&taskbars);
                    return;
                }
                self.shell_exp_host
                    .tap
                    .set_hide_background(shell_exp_pid, true, user_initiated);
                self.shell_host
                    .tap
                    .set_hide_background(shell_host_pid, true, user_initiated);
            }

            self.glass.set_style(self.style, &taskbars);
            if self.glass.sync_overflow() && explorer_pid != 0 {
                self.tap.reassert_hide(explorer_pid);
            }
            self.sync_alt_tab_state(explorer_pid, &taskbar_tids);
            self.sync_hover_flyout_state(explorer_pid, &taskbar_tids);
            self.sync_osd_state(explorer_pid, &taskbar_tids);
            self.sync_menu_flyout_state(explorer_pid, &taskbar_tids);
            self.sync_tooltip_state(explorer_pid, &taskbar_tids);
            self.sync_shell_flyouts_state();
            self.last_maintain = Instant::now();
        }
    }

    impl Drop for TaskbarController {
        fn drop(&mut self) {
            self.restore_default_and_shutdown();
        }
    }
}
