use crate::ui::GlassSurfaceStyle;
use std::sync::atomic::{AtomicU8, Ordering};

static ACTIVE_TASKBAR_SURFACE_STYLE: AtomicU8 =
    AtomicU8::new(GlassSurfaceStyle::LiquidGlass.to_u8());

/// Publishes the active taskbar visual style so all popup surfaces harmonize with the taskbar.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn set_active_taskbar_surface_style(style: GlassSurfaceStyle) {
    ACTIVE_TASKBAR_SURFACE_STYLE.store(style.to_u8(), Ordering::Relaxed);
}

/// Returns the active taskbar visual style published by `TaskbarController`.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn active_taskbar_surface_style() -> GlassSurfaceStyle {
    GlassSurfaceStyle::from_u8(ACTIVE_TASKBAR_SURFACE_STYLE.load(Ordering::Relaxed))
}

/// Extracts the executable filename from a Windows/POSIX path.
pub(crate) fn extract_exe_basename(path: &str) -> &str {
    path.rsplit(['\\', '/']).next().unwrap_or(path).trim()
}

/// Returns `true` if `class_name` belongs to Windows desktop surfaces or modern shell flyouts
/// (Start, Quick Settings, Search, System Tray overflow, Input switcher, etc.).
pub(crate) fn is_shell_or_flyout_class_name(class_name: &str) -> bool {
    matches!(
        class_name,
        "Progman"
            | "WorkerW"
            | "Shell_TrayWnd"
            | "Shell_SecondaryTrayWnd"
            | "ControlCenterWindow"
            | "TopLevelWindowForOverflowXamlIsland"
            | "XamlExplorerHostIslandWindow"
            | "NotifyIconOverflowWindow"
            | "Shell_InputSwitchTopLevelWindow"
            | "Windows.UI.Core.CoreWindow"
            | "Windows.UI.Input.InputSite.WindowClass"
    )
}

/// Returns `true` if `exe_name` is a known Windows shell flyout host process.
///
/// Intentionally excludes `explorer.exe` because fullscreen File Explorer (`CabinetWClass`)
/// should still count as fullscreen occlusion.
pub(crate) fn is_shell_flyout_process_name(exe_name: &str) -> bool {
    let name = exe_name.trim();
    name.eq_ignore_ascii_case("shellhost.exe")
        || name.eq_ignore_ascii_case("shellexperiencehost.exe")
        || name.eq_ignore_ascii_case("startmenuexperiencehost.exe")
        || name.eq_ignore_ascii_case("searchhost.exe")
        || name.eq_ignore_ascii_case("searchapp.exe")
        || name.eq_ignore_ascii_case("textinputhost.exe")
        || name.eq_ignore_ascii_case("widgets.exe")
}

/// Returns `true` if `class_name` represents a transient desktop shell flyout or tooltip window
/// that may vanish right after a popup activates and should be excluded from backdrop captures.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn is_transient_backdrop_occluder_class(class_name: &str) -> bool {
    matches!(
        class_name,
        "TopLevelWindowForOverflowXamlIsland"
            | "NotifyIconOverflowWindow"
            | "XamlExplorerHostIslandWindow"
            | "Shell_InputSwitchTopLevelWindow"
            | "ControlCenterWindow"
            | "tooltips_class32"
            | "Xaml_WindowedPopupClass"
            | "TaskListThumbnailWnd"
    )
}

/// Picks the lowest (largest `z_rank`) same-band window among the taskbar and visible taskbar
/// overlay widgets on the same monitor so a behind-taskbar backdrop never jumps above widgets
/// even when Windows temporarily elevates `Shell_TrayWnd` to a higher Z-order band (e.g. Start Menu).
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn pick_lowest_backdrop_anchor(
    taskbar_id: usize,
    taskbar_band: Option<u32>,
    backdrop_band: Option<u32>,
    taskbar_z_rank: Option<usize>,
    same_band_overlays_with_rank: &[(usize, usize)],
) -> usize {
    let mut best: Option<(usize, usize)> = None;
    let tb_same_band = match (taskbar_band, backdrop_band) {
        (Some(tb), Some(bg)) => tb == bg,
        _ => true,
    };
    if tb_same_band {
        if let Some(rank) = taskbar_z_rank {
            best = Some((taskbar_id, rank));
        }
    }
    for &(id, rank) in same_band_overlays_with_rank {
        if best.is_none_or(|(_, best_rank)| rank > best_rank) {
            best = Some((id, rank));
        }
    }
    best.map(|(id, _)| id).unwrap_or(taskbar_id)
}

/// Returns `true` when two axis-aligned screen rectangles `[left, top, right, bottom]` have positive intersection area.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn rects_intersect(a: [i32; 4], b: [i32; 4]) -> bool {
    a[0].max(b[0]) < a[2].min(b[2]) && a[1].max(b[1]) < a[3].min(b[3])
}

/// Horizontal gap (in DIPs) enforced between MediaBar and MonitorBar when adjacent on the taskbar.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) const TASKBAR_BAR_GAP_DIP: f32 = 2.0;

/// Deadband (in DIPs) past the peer's horizontal midpoint before dragging swaps to the opposite side.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) const TASKBAR_BAR_SWAP_HYSTERESIS_DIP: f32 = 8.0;

/// Identifies which taskbar widget occupies a registered horizontal collision slot.
#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TaskbarBarSlot {
    MediaBar = 0,
    MonitorBar = 1,
}

impl TaskbarBarSlot {
    #[cfg(windows)]
    const fn index(self) -> usize {
        self as usize
    }

    #[cfg(windows)]
    const fn peer(self) -> Self {
        match self {
            Self::MediaBar => Self::MonitorBar,
            Self::MonitorBar => Self::MediaBar,
        }
    }
}

/// Strategy used when resolving a bar's horizontal position against an active peer bar.
#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BarCollisionMode {
    /// Passive repositioning (startup, peer resize, display/taskbar change, Reset Position).
    Passive,
    /// Interactive horizontal drag: holds flush at `gap_px` and swaps across peer midpoint
    /// only after exceeding `hysteresis_px`.
    Drag,
}

#[cfg_attr(not(windows), allow(dead_code))]
fn horizontal_overlap_px(x: i32, width: i32, peer_left: i32, peer_right: i32) -> i64 {
    let left = i64::from(x).max(i64::from(peer_left));
    let right = (i64::from(x) + i64::from(width)).min(i64::from(peer_right));
    (right - left).max(0)
}

/// Resolves a bar's horizontal screen X coordinate inside `[min_x, max_x]` so it avoids
/// overlapping `peer_span: Option<(peer_left, peer_right)>` with at least `gap_px` spacing.
#[allow(clippy::too_many_arguments)]
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn resolve_taskbar_bar_x(
    desired_x: i32,
    bar_width_px: i32,
    min_x: i32,
    max_x: i32,
    current_x: Option<i32>,
    peer_span: Option<(i32, i32)>,
    gap_px: i32,
    hysteresis_px: i32,
    mode: BarCollisionMode,
) -> i32 {
    let width = bar_width_px.max(1);
    let effective_max = max_x.max(min_x);
    let base_x = desired_x.clamp(min_x, effective_max);

    let Some((peer_left, peer_right)) = peer_span else {
        return base_x;
    };
    if peer_right <= peer_left {
        return base_x;
    }

    let gap = i64::from(gap_px.max(0));
    let left_candidate64 = i64::from(peer_left) - gap - i64::from(width);
    let right_candidate64 = i64::from(peer_right) + gap;

    if i64::from(base_x) <= left_candidate64 || i64::from(base_x) >= right_candidate64 {
        return base_x;
    }

    let left_fits = left_candidate64 >= i64::from(min_x);
    let right_fits = right_candidate64 <= i64::from(effective_max);

    if left_fits || right_fits {
        let peer_mid2 = i64::from(peer_left) + i64::from(peer_right);
        let desired_mid2 = 2 * i64::from(desired_x) + i64::from(width);
        let choose_left = match (left_fits, right_fits) {
            (true, false) => true,
            (false, true) => false,
            (true, true) => match mode {
                BarCollisionMode::Passive => {
                    if desired_mid2 != peer_mid2 {
                        desired_mid2 < peer_mid2
                    } else if let Some(cur) = current_x {
                        2 * i64::from(cur) + i64::from(width) <= peer_mid2
                    } else {
                        true
                    }
                }
                BarCollisionMode::Drag => {
                    let hyst2 = 2 * i64::from(hysteresis_px.max(0));
                    let cur_left = current_x
                        .map(|cur| 2 * i64::from(cur) + i64::from(width) <= peer_mid2)
                        .unwrap_or(desired_mid2 <= peer_mid2);
                    if cur_left {
                        desired_mid2 <= peer_mid2 + hyst2
                    } else {
                        desired_mid2 < peer_mid2 - hyst2
                    }
                }
            },
            (false, false) => unreachable!(),
        };

        let target64 = if choose_left {
            left_candidate64
        } else {
            right_candidate64
        };
        return target64.clamp(i64::from(min_x), i64::from(effective_max)) as i32;
    }

    // Narrow taskbar fallback: neither clear side fits inside [min_x, effective_max];
    // pick the boundary that yields strictly smaller overlap with the peer.
    let left_overlap = horizontal_overlap_px(min_x, width, peer_left, peer_right);
    let right_overlap = horizontal_overlap_px(effective_max, width, peer_left, peer_right);
    if left_overlap < right_overlap {
        min_x
    } else if right_overlap < left_overlap {
        effective_max
    } else {
        let cur = current_x.unwrap_or(base_x);
        if (i64::from(cur) - i64::from(min_x)).abs()
            <= (i64::from(cur) - i64::from(effective_max)).abs()
        {
            min_x
        } else {
            effective_max
        }
    }
}

/// Writes UTF-8 text atomically via a same-directory temporary file and rename.
pub fn atomic_write_string(path: &std::path::Path, content: &str) -> std::io::Result<()> {
    static TMP_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let parent = path.parent().filter(|p| !p.as_os_str().is_empty());
    if let Some(dir) = parent {
        std::fs::create_dir_all(dir)?;
    }
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|n| !n.is_empty())
        .unwrap_or("config");
    let seq = TMP_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp_name = format!("{file_name}.{}.{seq}.tmp", std::process::id());
    let tmp_path = match parent {
        Some(dir) => dir.join(tmp_name),
        None => std::path::PathBuf::from(tmp_name),
    };
    if let Err(err) = std::fs::write(&tmp_path, content.as_bytes()) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(err);
    }
    if let Err(err) = std::fs::rename(&tmp_path, path) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(err);
    }
    Ok(())
}

/// Checks whether a HKCU Run registry string value points to `exe_path`.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn autostart_value_matches_exe(reg_val: &str, exe_path: &str) -> bool {
    let normalize = |raw: &str| -> String {
        let trimmed = raw.trim();
        let unquoted = trimmed
            .strip_prefix('"')
            .and_then(|s| s.strip_suffix('"'))
            .unwrap_or(trimmed);
        unquoted.trim().to_ascii_lowercase()
    };
    let left = normalize(reg_val);
    let right = normalize(exe_path);
    !left.is_empty() && left == right
}


#[cfg(windows)]
mod bar_registry;
#[cfg(windows)]
mod overlay_guard;
#[cfg(windows)]
mod theme;
#[cfg(windows)]
mod window_state;

#[cfg(windows)]
pub use bar_registry::*;
#[cfg(windows)]
pub use overlay_guard::*;
#[cfg(windows)]
pub(crate) use theme::read_reg_dword;
#[cfg(windows)]
pub use theme::*;
#[cfg(windows)]
pub use window_state::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_exe_basename_handles_windows_and_posix_paths() {
        assert_eq!(
            extract_exe_basename(r"C:\Windows\SystemApps\ShellHost.exe"),
            "ShellHost.exe"
        );
        assert_eq!(
            extract_exe_basename("C:/Windows/System32/explorer.exe"),
            "explorer.exe"
        );
        assert_eq!(extract_exe_basename("SearchHost.exe"), "SearchHost.exe");
        assert_eq!(extract_exe_basename(""), "");
    }

    #[test]
    fn shell_and_flyout_class_filter_matches_expected_windows_only() {
        assert!(is_shell_or_flyout_class_name("Progman"));
        assert!(is_shell_or_flyout_class_name("WorkerW"));
        assert!(is_shell_or_flyout_class_name("Shell_TrayWnd"));
        assert!(is_shell_or_flyout_class_name("Shell_SecondaryTrayWnd"));
        assert!(is_shell_or_flyout_class_name("ControlCenterWindow"));
        assert!(is_shell_or_flyout_class_name(
            "TopLevelWindowForOverflowXamlIsland"
        ));
        assert!(is_shell_or_flyout_class_name("XamlExplorerHostIslandWindow"));
        assert!(is_shell_or_flyout_class_name("Windows.UI.Core.CoreWindow"));

        assert!(!is_shell_or_flyout_class_name("Chrome_WidgetWin_1"));
        assert!(!is_shell_or_flyout_class_name("ApplicationFrameWindow"));
        assert!(!is_shell_or_flyout_class_name("CabinetWClass"));
    }

    #[test]
    fn shell_flyout_process_filter_is_case_insensitive_and_preserves_explorer() {
        assert!(is_shell_flyout_process_name("ShellHost.exe"));
        assert!(is_shell_flyout_process_name("SHELLEXPERIENCEHOST.EXE"));
        assert!(is_shell_flyout_process_name("StartMenuExperienceHost.exe"));
        assert!(is_shell_flyout_process_name("SearchHost.exe"));
        assert!(is_shell_flyout_process_name("TextInputHost.exe"));
        assert!(is_shell_flyout_process_name("Widgets.exe"));

        assert!(!is_shell_flyout_process_name("explorer.exe"));
        assert!(!is_shell_flyout_process_name("chrome.exe"));
    }

    #[test]
    fn backdrop_anchor_stays_below_widgets_when_taskbar_elevates_band() {
        const TB: usize = 0x1001;
        const MEDIA: usize = 0x2001;
        const MONITOR: usize = 0x2002;
        const SEC_TB: usize = 0x1002;

        // Normal same-band state: MediaBar (5) & MonitorBar (6) above Shell_TrayWnd (7) -> anchor = Shell_TrayWnd.
        assert_eq!(
            pick_lowest_backdrop_anchor(TB, Some(1), Some(1), Some(7), &[(MEDIA, 5), (MONITOR, 6)]),
            TB
        );

        // Start Menu open: Shell_TrayWnd elevated to band 16 while widgets & backdrop remain in band 1
        // -> must anchor below the lowest visible widget (MonitorBar), NEVER Shell_TrayWnd.
        assert_eq!(
            pick_lowest_backdrop_anchor(TB, Some(16), Some(1), Some(1), &[(MEDIA, 5), (MONITOR, 6)]),
            MONITOR
        );

        // Transient same-band reorder where Shell_TrayWnd (4) sits above widgets (5, 6)
        // -> still picks lowest widget (MonitorBar) so backdrop stays beneath all widgets.
        assert_eq!(
            pick_lowest_backdrop_anchor(TB, Some(1), Some(1), Some(4), &[(MEDIA, 5), (MONITOR, 6)]),
            MONITOR
        );

        // Secondary monitor with no widgets falls back to its own Shell_SecondaryTrayWnd.
        assert_eq!(
            pick_lowest_backdrop_anchor(SEC_TB, Some(16), Some(1), Some(2), &[]),
            SEC_TB
        );
    }

    #[test]
    fn transient_backdrop_occluder_class_filter_matches_expected_windows() {
        for cls in [
            "TopLevelWindowForOverflowXamlIsland",
            "NotifyIconOverflowWindow",
            "XamlExplorerHostIslandWindow",
            "ControlCenterWindow",
            "tooltips_class32",
            "Xaml_WindowedPopupClass",
            "TaskListThumbnailWnd",
        ] {
            assert!(
                is_transient_backdrop_occluder_class(cls),
                "expected transient occluder match for {cls}"
            );
        }

        for keep in [
            "Progman",
            "WorkerW",
            "Shell_TrayWnd",
            "Chrome_WidgetWin_1",
            "CabinetWClass",
            "PrettyWindowsTrayMenuWnd",
        ] {
            assert!(
                !is_transient_backdrop_occluder_class(keep),
                "should NOT mark {keep} as transient occluder"
            );
        }
    }

    #[test]
    fn rects_intersect_checks_positive_overlap_strictly() {
        let osd = [875, 971, 1044, 1032];
        assert!(rects_intersect(osd, [800, 950, 920, 1000]));
        assert!(rects_intersect(osd, [0, 0, 1920, 1080]));
        // Edge-touching only must not count as overlapping
        assert!(!rects_intersect(osd, [0, 0, 875, 1080]));
        assert!(!rects_intersect(osd, [1044, 971, 1200, 1032]));
        assert!(!rects_intersect(osd, [875, 1032, 1044, 1080]));
        // Degenerate / empty rect must not intersect
        assert!(!rects_intersect(osd, [900, 980, 900, 1020]));
    }

    #[test]
    fn resolve_bar_x_returns_clamped_desired_without_valid_peer() {
        assert_eq!(
            resolve_taskbar_bar_x(100, 256, 4, 1200, None, None, 2, 8, BarCollisionMode::Passive),
            100
        );
        assert_eq!(
            resolve_taskbar_bar_x(-50, 256, 4, 1200, None, Some((500, 500)), 2, 8, BarCollisionMode::Passive),
            4
        );
        assert_eq!(
            resolve_taskbar_bar_x(1500, 256, 4, 1200, None, Some((700, 600)), 2, 8, BarCollisionMode::Drag),
            1200
        );
    }

    #[test]
    fn resolve_bar_x_passive_yields_to_peer_without_moving_peer() {
        // Right-anchored MonitorBar collides on startup/enable with MediaBar parked at right edge:
        // right side does not fit, so MonitorBar yields flush to the left of MediaBar.
        assert_eq!(
            resolve_taskbar_bar_x(
                1516,
                200,
                4,
                1516,
                None,
                Some((1400, 1656)),
                2,
                8,
                BarCollisionMode::Passive,
            ),
            1198
        );

        // MediaBar Reset Position when MonitorBar occupies the left anchor:
        // left side does not fit (< min_x), so MediaBar yields flush to the right of MonitorBar.
        assert_eq!(
            resolve_taskbar_bar_x(
                164,
                256,
                4,
                1460,
                Some(600),
                Some((150, 350)),
                2,
                8,
                BarCollisionMode::Passive,
            ),
            352
        );

        // When both sides fit, Passive mode picks the nearer side cleanly.
        assert_eq!(
            resolve_taskbar_bar_x(
                410,
                256,
                4,
                1200,
                Some(200),
                Some((500, 700)),
                2,
                8,
                BarCollisionMode::Passive,
            ),
            242
        );
        assert_eq!(
            resolve_taskbar_bar_x(
                520,
                256,
                4,
                1200,
                Some(800),
                Some((500, 700)),
                2,
                8,
                BarCollisionMode::Passive,
            ),
            702
        );
    }

    #[test]
    fn resolve_bar_x_passive_narrow_taskbar_minimizes_overlap() {
        // Peer [180, 420], range [4, 400], bar_w = 256: neither side clears peer.
        // Overlap at min_x = 4 ([4, 260]) is 80px; overlap at max_x = 400 ([400, 656]) is 20px.
        assert_eq!(
            resolve_taskbar_bar_x(
                120,
                256,
                4,
                400,
                None,
                Some((180, 420)),
                2,
                8,
                BarCollisionMode::Passive,
            ),
            400
        );

        // Peer [200, 440], range [4, 400], bar_w = 256:
        // Overlap at min_x = 4 ([4, 260]) is 60px; overlap at max_x = 400 ([400, 656]) is 40px -> 400.
        // Symmetric left-win case: Peer [340, 620], range [4, 400], bar_w = 256:
        // Overlap at min_x = 4 ([4, 260]) is 0 -> already clear, or Peer [240, 620]:
        // min_x overlap = 20px vs max_x overlap = 220px -> picks min_x = 4.
        assert_eq!(
            resolve_taskbar_bar_x(
                180,
                256,
                4,
                400,
                None,
                Some((240, 620)),
                2,
                8,
                BarCollisionMode::Passive,
            ),
            4
        );
    }

    #[test]
    fn resolve_bar_x_drag_stops_flush_and_swaps_with_symmetric_hysteresis() {
        let peer = Some((500, 700)); // peer_mid = 600, bar_w = 256 => mid at desired_x = 472
        // Left candidate = 500 - 2 - 256 = 242, right candidate = 700 + 2 = 702.

        // Dragging from left to right: holds flush at 242 until desired center > 600 + 8.
        assert_eq!(
            resolve_taskbar_bar_x(260, 256, 4, 1200, Some(200), peer, 2, 8, BarCollisionMode::Drag),
            242
        );
        assert_eq!(
            resolve_taskbar_bar_x(472, 256, 4, 1200, Some(242), peer, 2, 8, BarCollisionMode::Drag),
            242
        );
        assert_eq!(
            resolve_taskbar_bar_x(480, 256, 4, 1200, Some(242), peer, 2, 8, BarCollisionMode::Drag),
            242
        );
        assert_eq!(
            resolve_taskbar_bar_x(481, 256, 4, 1200, Some(242), peer, 2, 8, BarCollisionMode::Drag),
            702
        );

        // Dragging back from right to left: holds flush at 702 until desired center < 600 - 8.
        assert_eq!(
            resolve_taskbar_bar_x(472, 256, 4, 1200, Some(702), peer, 2, 8, BarCollisionMode::Drag),
            702
        );
        assert_eq!(
            resolve_taskbar_bar_x(464, 256, 4, 1200, Some(702), peer, 2, 8, BarCollisionMode::Drag),
            702
        );
        assert_eq!(
            resolve_taskbar_bar_x(463, 256, 4, 1200, Some(702), peer, 2, 8, BarCollisionMode::Drag),
            242
        );
    }

    #[test]
    fn resolve_bar_x_drag_stays_flush_when_opposite_side_cannot_fit() {
        // Peer parked on far right [1200, 1400], dragged bar max_x = 1184:
        // right_candidate = 1402 > max_x, so dragging past peer midpoint stays flush at 942.
        assert_eq!(
            resolve_taskbar_bar_x(
                1184,
                256,
                4,
                1184,
                Some(942),
                Some((1200, 1400)),
                2,
                8,
                BarCollisionMode::Drag,
            ),
            942
        );
    }

    #[test]
    fn atomic_write_overwrites_cleanly_without_leftover_tmp_files() {
        let dir = std::env::temp_dir().join(format!(
            "pretty_windows_atomic_test_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let file = dir.join("nested").join("Sample.toml");

        atomic_write_string(&file, "first = 1\n").expect("initial atomic write");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "first = 1\n");

        atomic_write_string(&file, "second = 2\n").expect("overwrite atomic write");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "second = 2\n");

        let has_tmp = std::fs::read_dir(file.parent().unwrap())
            .unwrap()
            .filter_map(Result::ok)
            .any(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"));
        assert!(!has_tmp, "no .tmp staging files should remain");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn autostart_value_matches_exe_ignores_quotes_and_case() {
        assert!(autostart_value_matches_exe(
            r#" "D:\Tools\PrettyWindows.exe" "#,
            r#"d:\tools\prettywindows.exe"#
        ));
        assert!(autostart_value_matches_exe(
            r#"D:\Tools\PrettyWindows.exe"#,
            r#"D:\Tools\PrettyWindows.exe"#
        ));
        assert!(!autostart_value_matches_exe(
            r#""C:\Other\PrettyWindows.exe""#,
            r#"D:\Tools\PrettyWindows.exe"#
        ));
        assert!(!autostart_value_matches_exe("", r#"D:\Tools\PrettyWindows.exe"#));
    }
}
