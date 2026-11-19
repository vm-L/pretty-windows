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
    use super::liquid_glass::LiquidGlassManager;
    use super::swca::{enumerate_taskbar_windows, explorer_pid_from_taskbars};
    use super::tap_host::TapClient;
    use crate::color::Rgb;
    use std::path::PathBuf;
    use std::time::{Duration, Instant};
    use windows::Win32::Graphics::Direct2D::ID2D1Factory1;
    use windows::Win32::Foundation::HWND;

    /// Coordinates Windows 11 taskbar appearance across all monitors (XAML TAP + behind-taskbar backdrop).
    pub struct TaskbarController {
        config_path: PathBuf,
        style: TaskbarStyle,
        light_glass: bool,
        tap: TapClient,
        glass: LiquidGlassManager,
        last_maintain: Instant,
    }

    impl TaskbarController {
        pub fn new(d2d_factory: ID2D1Factory1, light_glass: bool, accent: Rgb) -> Self {
            let config_path = portable_config_path();
            let cfg = TaskbarConfig::load_from_path(&config_path);
            let mut ctrl = Self {
                config_path,
                style: cfg.style,
                light_glass,
                tap: TapClient::new(),
                glass: LiquidGlassManager::new(d2d_factory, light_glass, accent),
                last_maintain: Instant::now() - Duration::from_secs(10),
            };
            if ctrl.style != TaskbarStyle::Default {
                ctrl.apply_current_style(true, false);
            }
            ctrl
        }

        pub fn style(&self) -> TaskbarStyle {
            self.style
        }

        pub fn needs_fast_tick(&self) -> bool {
            self.style == TaskbarStyle::LiquidGlass
        }

        pub fn needs_timer(&self) -> bool {
            self.style != TaskbarStyle::Default
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
            let _ = (TaskbarConfig { style }).save_to_path(&self.config_path);
            self.apply_current_style(true, true);
        }

        pub fn refresh_theme(&mut self, light_glass: bool, accent: Rgb) {
            self.light_glass = light_glass;
            self.glass.set_theme(light_glass, accent);
            if self.style != TaskbarStyle::Default {
                self.last_maintain = Instant::now() - Duration::from_secs(10);
                self.apply_current_style(false, false);
            }
        }

        /// Re-applies style after Explorer restarts; returns `true` if safety circuit breaker reverted to `Default`.
        pub fn on_taskbar_created(&mut self) -> bool {
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

            if self.last_maintain.elapsed() >= Duration::from_millis(120) {
                self.tap.maintain(explorer_pid);
                if self.tap.is_circuit_tripped() {
                    self.fallback_to_default_on_trip(&taskbars);
                    return true;
                }
                if matches!(self.style, TaskbarStyle::Blur | TaskbarStyle::Acrylic) {
                    self.glass.sync(&taskbars);
                }
                self.last_maintain = Instant::now();
            }

            if self.style == TaskbarStyle::LiquidGlass {
                self.glass.sync(&taskbars);
            }
            false
        }

        pub fn restore_default_and_shutdown(&mut self) {
            let taskbars = enumerate_taskbar_windows();
            self.glass.set_style(TaskbarStyle::Default, &taskbars);
            self.tap.shutdown();
        }

        fn fallback_to_default_on_trip(&mut self, taskbars: &[HWND]) {
            self.style = TaskbarStyle::Default;
            self.glass.set_style(TaskbarStyle::Default, taskbars);
        }

        fn apply_current_style(&mut self, update_tap: bool, user_initiated: bool) {
            let taskbars = enumerate_taskbar_windows();
            let explorer_pid = explorer_pid_from_taskbars(&taskbars).unwrap_or(0);

            if self.style == TaskbarStyle::Default {
                self.glass.set_style(TaskbarStyle::Default, &taskbars);
                if update_tap {
                    self.tap
                        .set_hide_background(explorer_pid, false, user_initiated);
                }
                return;
            }

            if update_tap {
                self.tap
                    .set_hide_background(explorer_pid, true, user_initiated);
                if self.tap.is_circuit_tripped() {
                    self.fallback_to_default_on_trip(&taskbars);
                    return;
                }
            }

            self.glass.set_style(self.style, &taskbars);
            self.last_maintain = Instant::now();
        }
    }

    impl Drop for TaskbarController {
        fn drop(&mut self) {
            self.restore_default_and_shutdown();
        }
    }
}
