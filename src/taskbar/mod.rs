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
        find_visible_hover_flyout_host_window, find_visible_osd_host_window,
        find_visible_tooltip_host_window, LiquidGlassManager,
    };
    use super::swca::{
        alt_tab_item_card_colors, enumerate_taskbar_windows, explorer_pid_from_taskbars,
        map_tooltip_card_rect_to_screen, map_xaml_card_rect_to_screen,
        map_xaml_card_rect_to_screen_with_min, osd_fallback_card_colors, taskbar_thread_ids,
        tooltip_fallback_card_colors,
    };
    use super::tap_host::TapClient;
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

    /// Coordinates Windows 11 taskbar, overflow flyout, Alt+Tab switcher, hover flyout, hardware OSD, and ToolTip appearance across all monitors.
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

        pub fn set_overlay_notify_target(&mut self, hwnd: HWND, msg: u32) {
            self.tap.set_notify_target(hwnd, msg);
        }

        pub fn needs_fast_tick(&self) -> bool {
            self.style == TaskbarStyle::LiquidGlass
                || self.glass.is_overflow_visible()
                || self.alt_tab_host_active
                || self.hover_flyout_host_active
                || self.osd_host_active
                || self.tooltip_host_active
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
                self.tap.clear_alt_tab_rect();
                self.tap.clear_hover_flyout_rect();
                self.tap.clear_osd_rect();
                self.tap.clear_tooltip_rect();
                self.tap.set_osd_suppressed(0, false);
                self.tap.set_osd_fallback_colors(0, None);
                self.tap.set_tooltip_suppressed(0, false);
                self.tap.set_tooltip_fallback_colors(0, None);
                self.glass.hide_overflow_backdrop();
                self.glass.hide_alt_tab_backdrop();
                self.glass.hide_hover_flyout_backdrop();
                self.glass.hide_osd_backdrop();
                self.glass.hide_tooltip_backdrop();
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
            self.sync_tooltip_state(explorer_pid, &taskbar_tids);
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
            self.alt_tab_host_active = false;
            self.hover_flyout_host_active = false;
            self.last_hover_flyout_host_rect = None;
            self.osd_host_active = false;
            self.last_osd_host_rect = None;
            self.osd_suppressed = false;
            self.tooltip_host_active = false;
            self.last_tooltip_host_rect = None;
            self.tooltip_suppressed = false;
            self.tap.clear_alt_tab_rect();
            self.tap.clear_hover_flyout_rect();
            self.tap.clear_osd_rect();
            self.tap.clear_tooltip_rect();
            self.tap.set_osd_suppressed(0, false);
            self.tap.set_osd_fallback_colors(0, None);
            self.tap.set_tooltip_suppressed(0, false);
            self.tap.set_tooltip_fallback_colors(0, None);
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
            self.sync_tooltip_state(explorer_pid, &taskbar_tids);
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
            self.tap.clear_alt_tab_rect();
            self.tap.clear_hover_flyout_rect();
            self.tap.clear_osd_rect();
            self.tap.clear_tooltip_rect();
            self.tap.set_osd_suppressed(0, false);
            self.tap.set_alt_tab_item_colors(0, None);
            self.tap.set_osd_fallback_colors(0, None);
            self.tap.set_tooltip_suppressed(0, false);
            self.tap.set_tooltip_fallback_colors(0, None);
            self.glass.set_style(TaskbarStyle::Default, &taskbars);
            set_active_taskbar_surface_style(TaskbarStyle::Default.into());
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
            self.tap.clear_alt_tab_rect();
            self.tap.clear_hover_flyout_rect();
            self.tap.clear_osd_rect();
            self.tap.clear_tooltip_rect();
            self.tap.set_osd_suppressed(0, false);
            self.tap.set_alt_tab_item_colors(0, None);
            self.tap.set_osd_fallback_colors(0, None);
            self.tap.set_tooltip_suppressed(0, false);
            self.tap.set_tooltip_fallback_colors(0, None);
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
            ) {
                (Some(found), cached)
            } else if expected_size_dip.is_some() {
                self.tap.clear_tooltip_rect();
                (
                    find_visible_tooltip_host_window(explorer_pid, preferred_tid, taskbar_tids, None),
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

        fn apply_current_style(&mut self, update_tap: bool, user_initiated: bool) {
            let taskbars = enumerate_taskbar_windows();
            let explorer_pid = explorer_pid_from_taskbars(&taskbars).unwrap_or(0);
            let taskbar_tids = taskbar_thread_ids(&taskbars);

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
                self.tap.clear_alt_tab_rect();
                self.tap.clear_hover_flyout_rect();
                self.tap.clear_osd_rect();
                self.tap.clear_tooltip_rect();
                self.tap.set_osd_suppressed(explorer_pid, false);
                self.tap.set_alt_tab_item_colors(explorer_pid, None);
                self.tap.set_osd_fallback_colors(explorer_pid, None);
                self.tap.set_tooltip_suppressed(explorer_pid, false);
                self.tap.set_tooltip_fallback_colors(explorer_pid, None);
                self.glass.set_style(TaskbarStyle::Default, &taskbars);
                if update_tap {
                    self.tap
                        .set_hide_background(explorer_pid, false, user_initiated);
                }
                return;
            }

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

            if update_tap {
                self.tap
                    .set_hide_background(explorer_pid, true, user_initiated);
                if self.tap.is_circuit_tripped() {
                    self.fallback_to_default_on_trip(&taskbars);
                    return;
                }
            }

            self.glass.set_style(self.style, &taskbars);
            if self.glass.sync_overflow() && explorer_pid != 0 {
                self.tap.reassert_hide(explorer_pid);
            }
            self.sync_alt_tab_state(explorer_pid, &taskbar_tids);
            self.sync_hover_flyout_state(explorer_pid, &taskbar_tids);
            self.sync_osd_state(explorer_pid, &taskbar_tids);
            self.sync_tooltip_state(explorer_pid, &taskbar_tids);
            self.last_maintain = Instant::now();
        }
    }

    impl Drop for TaskbarController {
        fn drop(&mut self) {
            self.restore_default_and_shutdown();
        }
    }
}
