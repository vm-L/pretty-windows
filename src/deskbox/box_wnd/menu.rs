//! Unified Liquid Glass context menu for DeskBox headers, empty space, and `⋯` actions.

use super::super::config::{BoxKind, BoxSortMode, BoxViewMode};
use super::super::desktop::monitor_info_at_point;
use crate::color::{Rgb, NEUTRAL_BACKDROP_FALLBACK};
use crate::system::{
    active_taskbar_surface_style, resolve_unified_accent, visible_transient_overlay_rects,
};
use crate::ui::glass::{
    capture_blurred_desktop_backdrop_excluding, create_dc_render_target, create_text_format,
    d2d_rect, danger_text_color, draw_divider, draw_glass_interior, draw_rounded_card_rim,
    draw_rounded_card_shadows, draw_row_highlight, draw_text_in_rect, identity_matrix,
    primary_text_color, update_silk_bitmap, LayeredSurface, PopupNoiseCache,
};
use crate::ui::{
    compute_halo_pair, ease_out_cubic, ease_retract, popup_glass_recipe, rgb_luma, row_stagger_dy,
    window_master_alpha, PopupAnimPhase, RectF, Spring1D, CLOSE_ANIM_DUR_S, DIVIDER_H_DIP,
    INNER_PAD_DIP, OPEN_SPRING_DAMPING, OPEN_SPRING_STIFFNESS, PANEL_RADIUS_DIP,
    POPUP_ANIM_INTERVAL_MS, POPUP_FLOW_INTERVAL_MS, POPUP_REOPEN_DEBOUNCE_MS, ROW_GAP_DIP,
    ROW_H_DIP, SHADOW_PAD_DIP, SILK_H, SILK_W,
};
use std::cell::RefCell;
use std::mem::{size_of, ManuallyDrop};
use std::rc::Rc;
use std::time::{Duration, Instant};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{D2DERR_RECREATE_TARGET, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;
use windows::Win32::Graphics::Direct2D::{
    ID2D1Bitmap, ID2D1DCRenderTarget, ID2D1Factory1, ID2D1Geometry, ID2D1Layer,
    D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_LAYER_OPTIONS_NONE, D2D1_LAYER_PARAMETERS,
    D2D1_ROUNDED_RECT,
};
use windows::Win32::Graphics::DirectWrite::{IDWriteFactory, IDWriteTextFormat};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT, VK_ESCAPE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetCursorPos, GetWindowLongPtrW, IsWindow,
    IsWindowVisible, KillTimer, LoadCursorW, PostMessageW, RegisterClassExW, SetForegroundWindow,
    SetTimer, SetWindowLongPtrW, SetWindowPos, ShowWindow, GWLP_USERDATA, HCURSOR, HTCLIENT,
    HTTRANSPARENT, HWND_TOPMOST, IDC_ARROW, SWP_NOACTIVATE, SWP_SHOWWINDOW, SW_HIDE, WA_INACTIVE,
    WM_ACTIVATE, WM_CLOSE, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_NCDESTROY,
    WM_NCHITTEST, WM_SETCURSOR, WM_TIMER, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_POPUP,
};

const WM_MOUSELEAVE_MSG: u32 = 0x02A3;
const CLASS_NAME: PCWSTR = w!("PrettyWindowsDeskBoxMenuWnd");
const MENU_WIDTH_DIP: f32 = 178.0;
const TIMER_FLOW_ID: usize = 1;
const TIMER_HOVER_CHECK_ID: usize = 2;
const HOVER_CHECK_INTERVAL_MS: u32 = 50;
const HOVER_LEAVE_CLOSE_MS: u64 = 180;

/// `wparam` code posted to `target_box_hwnd` when `BoxContextMenu` finishes hiding.
pub const BOX_MENU_CLOSED_CODE: usize = 0;

/// Subset of DeskBox context menu actions to display.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoxMenuKind {
    NewBox,
    Sort,
    ViewAndTools,
    Full,
    ConfirmDelete,
}

/// Snapshot of the target DeskBox state used to populate context menu items & active highlights.
#[derive(Clone, Copy, Debug)]
pub struct BoxMenuContext {
    pub kind: BoxKind,
    pub view_mode: BoxViewMode,
    pub edge_auto_hide: bool,
    pub has_missing: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoxMenuAction {
    NewCollection,
    NewFolderPortal,
    SetView(BoxViewMode),
    SetSort(BoxSortMode),
    AutoFitSize,
    ToggleEdgeAutoHide,
    OpenInExplorer,
    CleanMissing,
    ConfirmDeleteBox,
    CancelDeleteBox,
}

impl BoxMenuAction {
    pub const fn to_usize(self) -> usize {
        match self {
            Self::NewCollection => 1,
            Self::NewFolderPortal => 2,
            Self::SetView(BoxViewMode::LargeIcons) => 10,
            Self::SetView(BoxViewMode::MediumIcons) => 11,
            Self::SetView(BoxViewMode::SmallIcons) => 12,
            Self::SetView(BoxViewMode::CompactList) => 13,
            Self::OpenInExplorer => 20,
            Self::CleanMissing => 22,
            Self::SetSort(BoxSortMode::TypeAsc) => 30,
            Self::SetSort(BoxSortMode::NameAsc) => 31,
            Self::SetSort(BoxSortMode::NameDesc) => 32,
            Self::SetSort(BoxSortMode::ModifiedDesc) => 33,
            Self::ToggleEdgeAutoHide => 40,
            Self::AutoFitSize => 41,
            Self::ConfirmDeleteBox => 50,
            Self::CancelDeleteBox => 51,
        }
    }

    pub const fn from_usize(v: usize) -> Option<Self> {
        match v {
            1 => Some(Self::NewCollection),
            2 => Some(Self::NewFolderPortal),
            10 => Some(Self::SetView(BoxViewMode::LargeIcons)),
            11 => Some(Self::SetView(BoxViewMode::MediumIcons)),
            12 => Some(Self::SetView(BoxViewMode::SmallIcons)),
            13 => Some(Self::SetView(BoxViewMode::CompactList)),
            20 => Some(Self::OpenInExplorer),
            22 => Some(Self::CleanMissing),
            30 => Some(Self::SetSort(BoxSortMode::TypeAsc)),
            31 => Some(Self::SetSort(BoxSortMode::NameAsc)),
            32 => Some(Self::SetSort(BoxSortMode::NameDesc)),
            33 => Some(Self::SetSort(BoxSortMode::ModifiedDesc)),
            40 => Some(Self::ToggleEdgeAutoHide),
            41 => Some(Self::AutoFitSize),
            50 => Some(Self::ConfirmDeleteBox),
            51 => Some(Self::CancelDeleteBox),
            _ => None,
        }
    }
}

#[derive(Clone, Copy)]
enum MenuEntry {
    Action {
        action: BoxMenuAction,
        label: &'static str,
        selected: bool,
        danger: bool,
    },
    Divider,
}

fn build_entries(menu_kind: BoxMenuKind, ctx: BoxMenuContext) -> Vec<MenuEntry> {
    match menu_kind {
        BoxMenuKind::NewBox => vec![
            MenuEntry::Action {
                action: BoxMenuAction::NewCollection,
                label: "新建自由收纳格",
                selected: false,
                danger: false,
            },
            MenuEntry::Action {
                action: BoxMenuAction::NewFolderPortal,
                label: "新建关联文件夹格",
                selected: false,
                danger: false,
            },
        ],
        BoxMenuKind::Sort => BoxSortMode::ALL
            .iter()
            .map(|&sort| MenuEntry::Action {
                action: BoxMenuAction::SetSort(sort),
                label: sort.label(),
                selected: false,
                danger: false,
            })
            .collect(),
        BoxMenuKind::ViewAndTools => {
            let mut entries: Vec<MenuEntry> = BoxViewMode::ALL
                .iter()
                .map(|&mode| MenuEntry::Action {
                    action: BoxMenuAction::SetView(mode),
                    label: mode.label(),
                    selected: mode == ctx.view_mode,
                    danger: false,
                })
                .collect();
            entries.push(MenuEntry::Divider);
            entries.push(MenuEntry::Action {
                action: BoxMenuAction::AutoFitSize,
                label: "自适应尺寸",
                selected: false,
                danger: false,
            });
            entries.push(MenuEntry::Action {
                action: BoxMenuAction::ToggleEdgeAutoHide,
                label: "贴边隐藏",
                selected: ctx.edge_auto_hide,
                danger: false,
            });
            entries
        }
        BoxMenuKind::Full => {
            let mut entries = vec![MenuEntry::Action {
                action: BoxMenuAction::AutoFitSize,
                label: "自适应尺寸",
                selected: false,
                danger: false,
            }];
            if ctx.kind == BoxKind::FolderPortal {
                entries.push(MenuEntry::Divider);
                entries.push(MenuEntry::Action {
                    action: BoxMenuAction::OpenInExplorer,
                    label: "打开所在文件夹",
                    selected: false,
                    danger: false,
                });
            } else if ctx.has_missing {
                entries.push(MenuEntry::Divider);
                entries.push(MenuEntry::Action {
                    action: BoxMenuAction::CleanMissing,
                    label: "清理失效项目",
                    selected: false,
                    danger: false,
                });
            }
            entries
        }
        BoxMenuKind::ConfirmDelete => vec![
            MenuEntry::Action {
                action: BoxMenuAction::ConfirmDeleteBox,
                label: "确认删除收纳格",
                selected: false,
                danger: true,
            },
            MenuEntry::Action {
                action: BoxMenuAction::CancelDeleteBox,
                label: "取消",
                selected: false,
                danger: false,
            },
        ],
    }
}

struct MenuLayout {
    scale: f32,
    win_w: i32,
    win_h: i32,
    card_rect: RectF,
    card_radius: f32,
    rows: Vec<(usize, RectF)>,
    dividers: Vec<RectF>,
}

impl MenuLayout {
    fn compute(dpi: u32, entries: &[MenuEntry]) -> Self {
        let s = (dpi.max(96) as f32) / 96.0;
        let pad = SHADOW_PAD_DIP * s;
        let inner = INNER_PAD_DIP * s;
        let row_h = ROW_H_DIP * s;
        let gap = ROW_GAP_DIP * s;
        let div_h = DIVIDER_H_DIP * s;
        let card_w = (MENU_WIDTH_DIP * s).round();

        let mut content_h = 0.0f32;
        for (i, entry) in entries.iter().enumerate() {
            let next_is_row = matches!(entries.get(i + 1), Some(MenuEntry::Action { .. }));
            match entry {
                MenuEntry::Action { .. } => {
                    content_h += row_h;
                    if next_is_row {
                        content_h += gap;
                    }
                }
                MenuEntry::Divider => {
                    content_h += div_h;
                }
            }
        }
        let card_h = (inner * 2.0 + content_h).round();
        let win_w = (card_w + pad * 2.0).round() as i32;
        let win_h = (card_h + pad * 2.0).round() as i32;
        let card_rect = RectF::new(pad, pad, pad + card_w, pad + card_h);

        let rx0 = card_rect.left + inner;
        let rx1 = card_rect.right - inner;
        let mut y = card_rect.top + inner;
        let mut rows = Vec::new();
        let mut dividers = Vec::new();
        for (i, entry) in entries.iter().enumerate() {
            let next_is_row = matches!(entries.get(i + 1), Some(MenuEntry::Action { .. }));
            match entry {
                MenuEntry::Action { .. } => {
                    let r = RectF::new(rx0, y, rx1, y + row_h);
                    rows.push((i, r));
                    y = r.bottom + if next_is_row { gap } else { 0.0 };
                }
                MenuEntry::Divider => {
                    dividers.push(RectF::new(rx0, y - div_h, rx1, y));
                    y += div_h;
                }
            }
        }

        Self {
            scale: s,
            win_w,
            win_h,
            card_rect,
            card_radius: PANEL_RADIUS_DIP * s,
            rows,
            dividers,
        }
    }

    fn hit_test(&self, x: f32, y: f32) -> Option<usize> {
        if !self.card_rect.contains(x, y) {
            return None;
        }
        self.rows
            .iter()
            .find(|(_, r)| r.contains(x, y))
            .map(|(idx, _)| *idx)
    }
}

struct MenuInner {
    hwnd: HWND,
    target_box_hwnd: HWND,
    notify_msg: u32,
    d2d_factory: ID2D1Factory1,
    dwrite_factory: IDWriteFactory,
    backdrop_bmp: Option<ID2D1Bitmap>,
    noise_cache: PopupNoiseCache,
    silk_bmp: Option<ID2D1Bitmap>,
    layer: ID2D1Layer,
    rt: ID2D1DCRenderTarget,
    surface: LayeredSurface,
    arrow_cursor: HCURSOR,

    dpi: u32,
    light_glass: bool,
    backdrop_rgb: Rgb,
    win_x: i32,
    win_y: i32,
    win_w: i32,
    win_h: i32,

    menu_kind: BoxMenuKind,
    hover_mode: bool,
    trigger_rect_screen: RECT,
    card_rect_screen: RECT,
    leave_since: Option<Instant>,
    closed_info: Option<(HWND, BoxMenuKind, Instant)>,
    entries: Vec<MenuEntry>,
    hover_idx: Option<usize>,
    tracking_leave: bool,

    silk_buf: Vec<u8>,
    flow_phase: f64,
    halo_colors: [(f32, f32, f32); 2],
    ui_accent: (f32, f32, f32),
    halo_colors_ready: bool,
    last_flow_tick: Instant,
    last_silk_tick: Option<Instant>,
    last_color_tick: Instant,

    anim_phase: PopupAnimPhase,
    open_spring: Spring1D,
    open_elapsed: f32,
    close_progress: f32,
    timer_interval_ms: u32,
    cached_fmt: Option<(u32, IDWriteTextFormat)>,
}

/// Liquid Glass popup menu shared across all DeskBox windows (shown one at a time).
pub struct BoxContextMenu {
    hwnd: HWND,
    inner: Rc<RefCell<MenuInner>>,
}

impl BoxContextMenu {
    pub fn new(
        d2d_factory: ID2D1Factory1,
        dwrite_factory: IDWriteFactory,
        notify_msg: u32,
    ) -> windows::core::Result<Self> {
        unsafe {
            let hinst = GetModuleHandleW(None)?;
            let arrow_cursor = LoadCursorW(None, IDC_ARROW)?;
            let wc = WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(menu_wnd_proc),
                hInstance: hinst.into(),
                hCursor: arrow_cursor,
                lpszClassName: CLASS_NAME,
                ..Default::default()
            };
            RegisterClassExW(&wc);

            let hwnd = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
                CLASS_NAME,
                w!("DeskBox Menu"),
                WS_POPUP,
                0,
                0,
                1,
                1,
                None,
                None,
                hinst,
                None,
            )?;

            let init_res = (|| -> windows::core::Result<(
                ID2D1DCRenderTarget,
                ID2D1Layer,
                LayeredSurface,
            )> {
                let rt = create_dc_render_target(&d2d_factory)?;
                let layer = rt.CreateLayer(None)?;
                let surface = LayeredSurface::new()?;
                Ok((rt, layer, surface))
            })();
            let (rt, layer, surface) = match init_res {
                Ok(v) => v,
                Err(e) => {
                    let _ = DestroyWindow(hwnd);
                    return Err(e);
                }
            };

            let inner = Rc::new(RefCell::new(MenuInner {
                hwnd,
                target_box_hwnd: HWND::default(),
                notify_msg,
                d2d_factory,
                dwrite_factory,
                backdrop_bmp: None,
                noise_cache: PopupNoiseCache::default(),
                silk_bmp: None,
                layer,
                rt,
                surface,
                arrow_cursor,
                dpi: 96,
                light_glass: false,
                backdrop_rgb: NEUTRAL_BACKDROP_FALLBACK,
                win_x: 0,
                win_y: 0,
                win_w: 1,
                win_h: 1,
                menu_kind: BoxMenuKind::Full,
                hover_mode: false,
                trigger_rect_screen: RECT::default(),
                card_rect_screen: RECT::default(),
                leave_since: None,
                closed_info: None,
                entries: Vec::new(),
                hover_idx: None,
                tracking_leave: false,
                silk_buf: vec![0u8; (SILK_W * SILK_H * 4) as usize],
                flow_phase: 0.0,
                halo_colors: [(0.76, 0.76, 0.78); 2],
                ui_accent: (0.76, 0.76, 0.78),
                halo_colors_ready: false,
                last_flow_tick: Instant::now(),
                last_silk_tick: None,
                last_color_tick: Instant::now(),
                anim_phase: PopupAnimPhase::Open,
                open_spring: Spring1D::still(1.0),
                open_elapsed: 1.0,
                close_progress: 0.0,
                timer_interval_ms: 0,
                cached_fmt: None,
            }));

            SetWindowLongPtrW(
                hwnd,
                GWLP_USERDATA,
                Rc::into_raw(Rc::clone(&inner)) as isize,
            );
            Ok(Self { hwnd, inner })
        }
    }

    pub fn open_kind_for(&self, box_hwnd: HWND) -> Option<BoxMenuKind> {
        let vis = unsafe { IsWindowVisible(self.hwnd).as_bool() };
        if !vis {
            return None;
        }
        self.inner.try_borrow().ok().and_then(|s| {
            (s.anim_phase != PopupAnimPhase::Closing && s.target_box_hwnd == box_hwnd)
                .then_some(s.menu_kind)
        })
    }

    pub fn was_just_closed_for(&self, box_hwnd: HWND, kind: BoxMenuKind) -> bool {
        self.inner.try_borrow().ok().is_some_and(|s| {
            s.closed_info.is_some_and(|(h, k, t)| {
                h == box_hwnd
                    && k == kind
                    && t.elapsed() < Duration::from_millis(POPUP_REOPEN_DEBOUNCE_MS)
            })
        })
    }

    pub fn hide(&self) {
        let was_vis = unsafe { IsWindowVisible(self.hwnd).as_bool() };
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
        let notify = if let Ok(mut s) = self.inner.try_borrow_mut() {
            let info = (was_vis && !s.target_box_hwnd.is_invalid())
                .then_some((s.target_box_hwnd, s.notify_msg));
            if was_vis {
                s.closed_info = Some((s.target_box_hwnd, s.menu_kind, Instant::now()));
            }
            s.reset_transient();
            info
        } else {
            None
        };
        if let Some((target_hwnd, msg_id)) = notify {
            unsafe {
                if IsWindow(target_hwnd).as_bool() {
                    let _ = PostMessageW(target_hwnd, msg_id, WPARAM(BOX_MENU_CLOSED_CODE), LPARAM(0));
                }
            }
        }
    }

    pub fn is_cursor_inside_menu_or_trigger(&self, box_hwnd: HWND) -> bool {
        let vis = unsafe { IsWindowVisible(self.hwnd).as_bool() };
        if !vis {
            return false;
        }
        let mut pt = POINT::default();
        if unsafe { GetCursorPos(&mut pt).is_err() } {
            return false;
        }
        self.inner.try_borrow().ok().is_some_and(|s| {
            s.target_box_hwnd == box_hwnd
                && s.anim_phase != PopupAnimPhase::Closing
                && pt_in_hover_corridor(pt, s.trigger_rect_screen, s.card_rect_screen)
        })
    }

    pub fn promote_hover_menu_focus(&self, box_hwnd: HWND, kind: BoxMenuKind) {
        if self.open_kind_for(box_hwnd) == Some(kind) {
            if let Ok(mut s) = self.inner.try_borrow_mut() {
                s.hover_mode = false;
                s.leave_since = None;
            }
            unsafe {
                let _ = KillTimer(self.hwnd, TIMER_HOVER_CHECK_ID);
                let _ = SetForegroundWindow(self.hwnd);
            }
        }
    }

    pub fn show(
        &self,
        target_box_hwnd: HWND,
        screen_pt: POINT,
        light_glass: bool,
        menu_kind: BoxMenuKind,
        ctx: BoxMenuContext,
    ) {
        let anchor_rect = RECT {
            left: screen_pt.x,
            top: screen_pt.y,
            right: screen_pt.x,
            bottom: screen_pt.y,
        };
        self.show_for_trigger(
            target_box_hwnd,
            anchor_rect,
            light_glass,
            menu_kind,
            ctx,
            false,
        );
    }

    pub fn show_for_trigger(
        &self,
        target_box_hwnd: HWND,
        trigger_rect_screen: RECT,
        light_glass: bool,
        menu_kind: BoxMenuKind,
        ctx: BoxMenuContext,
        hover_mode: bool,
    ) {
        self.hide();
        let anchor_pt = POINT {
            x: trigger_rect_screen.left,
            y: trigger_rect_screen.bottom,
        };
        let mon = monitor_info_at_point(anchor_pt);
        let entries = build_entries(menu_kind, ctx);
        if entries.is_empty() {
            return;
        }
        let layout = MenuLayout::compute(mon.dpi, &entries);
        let s = layout.scale;
        let pad = (SHADOW_PAD_DIP * s).round() as i32;
        let margin = (4.0 * s).round() as i32;
        let gap = if trigger_rect_screen.bottom > trigger_rect_screen.top {
            (3.0 * s).round() as i32
        } else {
            0
        };
        let card_w = layout.win_w - pad * 2;
        let card_h = layout.win_h - pad * 2;

        let card_left = if trigger_rect_screen.left + card_w <= mon.work.right - margin {
            trigger_rect_screen.left
        } else {
            (trigger_rect_screen.right - card_w).max(mon.work.left + margin)
        };
        let card_top = if trigger_rect_screen.bottom + gap + card_h <= mon.work.bottom - margin {
            trigger_rect_screen.bottom + gap
        } else {
            (trigger_rect_screen.top - gap - card_h).max(mon.work.top + margin)
        };
        let card_rect_screen = RECT {
            left: card_left,
            top: card_top,
            right: card_left + card_w,
            bottom: card_top + card_h,
        };
        let win_x = card_left - pad;
        let win_y = card_top - pad;
        let capture_rect = RECT {
            left: win_x,
            top: win_y,
            right: win_x + layout.win_w,
            bottom: win_y + layout.win_h,
        };
        let occluders = visible_transient_overlay_rects(capture_rect);

        if let Ok(mut st) = self.inner.try_borrow_mut() {
            st.target_box_hwnd = target_box_hwnd;
            st.menu_kind = menu_kind;
            st.hover_mode = hover_mode;
            st.trigger_rect_screen = trigger_rect_screen;
            st.card_rect_screen = card_rect_screen;
            st.leave_since = None;
            st.closed_info = None;
            st.dpi = mon.dpi;
            st.light_glass = light_glass;
            st.entries = entries;
            st.win_x = win_x;
            st.win_y = win_y;
            st.win_w = layout.win_w;
            st.win_h = layout.win_h;
            st.hover_idx = None;
            st.backdrop_rgb = NEUTRAL_BACKDROP_FALLBACK;

            let initial_recipe =
                popup_glass_recipe(active_taskbar_surface_style(), light_glass, 0.5);
            let (bmp, sampled_bg) = capture_blurred_desktop_backdrop_excluding(
                &st.rt,
                win_x,
                win_y,
                layout.win_w,
                layout.win_h,
                initial_recipe.blur_radius_4x,
                &occluders,
            );
            st.backdrop_bmp = bmp;
            if let Some(bg) = sampled_bg {
                st.backdrop_rgb = bg;
            }
            st.halo_colors_ready = false;
            st.last_silk_tick = None;
            st.last_color_tick = Instant::now();
            st.last_flow_tick = st.last_color_tick;
            st.anim_phase = PopupAnimPhase::Opening;
            st.open_spring = Spring1D::new(0.14, 2.8);
            st.open_elapsed = 0.0;
            st.close_progress = 0.0;

            let _ = st.render();
            st.sync_timer_rate();
        }

        unsafe {
            let mut flags = SWP_SHOWWINDOW;
            if hover_mode {
                flags |= SWP_NOACTIVATE;
            }
            let _ = SetWindowPos(
                self.hwnd,
                HWND_TOPMOST,
                win_x,
                win_y,
                layout.win_w,
                layout.win_h,
                flags,
            );
            if hover_mode {
                SetTimer(self.hwnd, TIMER_HOVER_CHECK_ID, HOVER_CHECK_INTERVAL_MS, None);
            } else {
                let _ = SetForegroundWindow(self.hwnd);
            }
        }
    }
}

impl Drop for BoxContextMenu {
    fn drop(&mut self) {
        unsafe {
            let _ = KillTimer(self.hwnd, TIMER_FLOW_ID);
            let _ = KillTimer(self.hwnd, TIMER_HOVER_CHECK_ID);
            let raw = SetWindowLongPtrW(self.hwnd, GWLP_USERDATA, 0) as *const RefCell<MenuInner>;
            if !raw.is_null() {
                drop(Rc::from_raw(raw));
            }
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

impl MenuInner {
    fn current_layout(&self) -> MenuLayout {
        MenuLayout::compute(self.dpi, &self.entries)
    }

    fn reset_transient(&mut self) {
        unsafe {
            let _ = KillTimer(self.hwnd, TIMER_FLOW_ID);
            let _ = KillTimer(self.hwnd, TIMER_HOVER_CHECK_ID);
        }
        self.target_box_hwnd = HWND::default();
        self.hover_mode = false;
        self.trigger_rect_screen = RECT::default();
        self.card_rect_screen = RECT::default();
        self.leave_since = None;
        self.timer_interval_ms = 0;
        self.backdrop_bmp = None;
        self.hover_idx = None;
        self.tracking_leave = false;
        self.anim_phase = PopupAnimPhase::Open;
        self.open_spring = Spring1D::still(1.0);
        self.open_elapsed = 1.0;
        self.close_progress = 0.0;
    }

    fn begin_close(&mut self) {
        if self.anim_phase == PopupAnimPhase::Closing
            || !unsafe { IsWindowVisible(self.hwnd).as_bool() }
        {
            return;
        }
        unsafe {
            let _ = KillTimer(self.hwnd, TIMER_HOVER_CHECK_ID);
        }
        self.hover_mode = false;
        self.leave_since = None;
        if !self.target_box_hwnd.is_invalid() {
            self.closed_info = Some((self.target_box_hwnd, self.menu_kind, Instant::now()));
        }
        self.hover_idx = None;
        self.anim_phase = PopupAnimPhase::Closing;
        self.close_progress = 0.0;
        self.last_flow_tick = Instant::now();
        self.sync_timer_rate();
        let _ = self.render();
    }

    fn sync_timer_rate(&mut self) {
        let want = if matches!(
            self.anim_phase,
            PopupAnimPhase::Opening | PopupAnimPhase::Closing
        ) {
            POPUP_ANIM_INTERVAL_MS
        } else {
            let rec = popup_glass_recipe(active_taskbar_surface_style(), self.light_glass, 0.5);
            if rec.silk_alpha > 0.002 {
                POPUP_FLOW_INTERVAL_MS
            } else {
                0
            }
        };
        if want == POPUP_ANIM_INTERVAL_MS {
            crate::ui::notify_interactive_anim_frame();
        }
        if self.timer_interval_ms != want {
            unsafe {
                if want == 0 {
                    let _ = KillTimer(self.hwnd, TIMER_FLOW_ID);
                } else {
                    if self.timer_interval_ms == 0 {
                        self.last_flow_tick = Instant::now();
                    }
                    SetTimer(self.hwnd, TIMER_FLOW_ID, want, None);
                }
            }
            self.timer_interval_ms = want;
        }
    }

    fn ensure_fmt(&mut self, s: f32) -> windows::core::Result<IDWriteTextFormat> {
        if let Some((dpi, ref fmt)) = self.cached_fmt {
            if dpi == self.dpi {
                return Ok(fmt.clone());
            }
        }
        let fmt = create_text_format(&self.dwrite_factory, 13.0 * s, false, false, true)?;
        self.cached_fmt = Some((self.dpi, fmt.clone()));
        Ok(fmt)
    }

    fn render(&mut self) -> windows::core::Result<()> {
        match self.render_once() {
            Err(e) if e.code() == D2DERR_RECREATE_TARGET => {
                self.backdrop_bmp = None;
                self.silk_bmp = None;
                self.noise_cache.clear();
                self.last_silk_tick = None;
                let new_rt = create_dc_render_target(&self.d2d_factory)?;
                self.layer = unsafe { new_rt.CreateLayer(None)? };
                self.rt = new_rt;
                self.render_once()
            }
            res => res,
        }
    }

    fn render_once(&mut self) -> windows::core::Result<()> {
        let layout = self.current_layout();
        let s = layout.scale;
        self.surface.ensure_size(layout.win_w, layout.win_h)?;
        let fmt = self.ensure_fmt(s)?;

        let recipe = popup_glass_recipe(
            active_taskbar_surface_style(),
            self.light_glass,
            rgb_luma(self.backdrop_rgb),
        );
        let now = Instant::now();
        let dt_c = now
            .duration_since(self.last_color_tick)
            .as_secs_f32()
            .clamp(0.001, 0.05);
        self.last_color_tick = now;

        let (ui_rgb, base_dark) = resolve_unified_accent(self.backdrop_rgb, self.light_glass);
        let pair = compute_halo_pair(base_dark, self.light_glass);
        let tgt_halo = [pair[0].to_normalized(), pair[1].to_normalized()];
        let tgt_ui = ui_rgb.to_normalized();
        let was_ready = self.halo_colors_ready;
        if !was_ready {
            self.halo_colors = tgt_halo;
            self.ui_accent = tgt_ui;
            self.halo_colors_ready = true;
        } else {
            let k = (dt_c * 5.0).clamp(0.02, 1.0);
            for (c, t) in self.halo_colors.iter_mut().zip(tgt_halo.iter()) {
                c.0 += (t.0 - c.0) * k;
                c.1 += (t.1 - c.1) * k;
                c.2 += (t.2 - c.2) * k;
            }
            self.ui_accent.0 += (tgt_ui.0 - self.ui_accent.0) * k;
            self.ui_accent.1 += (tgt_ui.1 - self.ui_accent.1) * k;
            self.ui_accent.2 += (tgt_ui.2 - self.ui_accent.2) * k;
        }

        let accent = Rgb::new(
            (self.ui_accent.0 * 255.0).round().clamp(0.0, 255.0) as u8,
            (self.ui_accent.1 * 255.0).round().clamp(0.0, 255.0) as u8,
            (self.ui_accent.2 * 255.0).round().clamp(0.0, 255.0) as u8,
        );

        let card_rect = match self.anim_phase {
            PopupAnimPhase::Open => layout.card_rect,
            PopupAnimPhase::Opening => {
                let p = self.open_spring.pos.clamp(0.0, 1.06);
                let h = (layout.card_rect.height() * (0.38 + 0.62 * p)).max(36.0 * s);
                RectF::new(
                    layout.card_rect.left,
                    layout.card_rect.top,
                    layout.card_rect.right,
                    (layout.card_rect.top + h).min(layout.win_h as f32 - 4.0 * s),
                )
            }
            PopupAnimPhase::Closing => {
                let r = ease_retract(self.close_progress);
                let h = (layout.card_rect.height() * (1.0 - 0.40 * r)).max(32.0 * s);
                RectF::new(
                    layout.card_rect.left,
                    layout.card_rect.top,
                    layout.card_rect.right,
                    layout.card_rect.top + h,
                )
            }
        };

        if recipe.silk_alpha > 0.002 {
            update_silk_bitmap(
                &self.rt,
                &mut self.silk_bmp,
                &mut self.silk_buf,
                &mut self.last_silk_tick,
                !was_ready,
                layout.win_w,
                layout.win_h,
                card_rect.left,
                card_rect.bottom,
                s,
                self.flow_phase,
                &self.halo_colors,
                self.light_glass,
            );
        }

        let content_alpha = match self.anim_phase {
            PopupAnimPhase::Opening => {
                ease_out_cubic(((self.open_elapsed - 0.008) / 0.080).clamp(0.0, 1.0))
            }
            PopupAnimPhase::Open => 1.0,
            PopupAnimPhase::Closing => (1.0 - 0.30 * ease_retract(self.close_progress)).clamp(0.0, 1.0),
        };
        let row_count = layout.rows.len();

        let bind_rect = RECT {
            left: 0,
            top: 0,
            right: self.surface.width(),
            bottom: self.surface.height(),
        };

        unsafe {
            self.rt.BindDC(self.surface.dc(), &bind_rect)?;
            self.rt.BeginDraw();
            self.rt.Clear(Some(&D2D1_COLOR_F {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.0,
            }));

            draw_rounded_card_shadows(
                &self.rt,
                card_rect,
                layout.card_radius,
                layout.win_w as f32,
                layout.win_h as f32,
                s,
                self.light_glass,
            );

            let card_rr = D2D1_ROUNDED_RECT {
                rect: d2d_rect(card_rect),
                radiusX: layout.card_radius,
                radiusY: layout.card_radius,
            };
            if let Ok(geo) = self.d2d_factory.CreateRoundedRectangleGeometry(&card_rr) {
                let geom: ID2D1Geometry = geo.into();
                let params = D2D1_LAYER_PARAMETERS {
                    contentBounds: d2d_rect(card_rect),
                    geometricMask: ManuallyDrop::new(Some(geom)),
                    maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                    maskTransform: identity_matrix(),
                    opacity: 1.0,
                    opacityBrush: ManuallyDrop::new(None),
                    layerOptions: D2D1_LAYER_OPTIONS_NONE,
                };
                self.rt.PushLayer(&params, &self.layer);

                draw_glass_interior(
                    &self.rt,
                    card_rect,
                    layout.win_w,
                    layout.win_h,
                    self.backdrop_bmp.as_ref(),
                    self.silk_bmp.as_ref(),
                    self.light_glass,
                    &recipe,
                    &mut self.noise_cache,
                );

                if content_alpha > 0.005 {
                    for (row_idx, (entry_idx, mut r)) in layout.rows.iter().copied().enumerate() {
                        let dy = match self.anim_phase {
                            PopupAnimPhase::Opening => -row_stagger_dy(
                                row_count.saturating_sub(1 + row_idx),
                                row_count.max(1),
                                self.open_elapsed,
                                self.open_spring.pos,
                                s,
                            ),
                            PopupAnimPhase::Open => 0.0,
                            PopupAnimPhase::Closing => -5.0 * s * ease_retract(self.close_progress),
                        };
                        r.top += dy;
                        r.bottom += dy;

                        if let MenuEntry::Action {
                            label,
                            selected,
                            danger,
                            ..
                        } = self.entries[entry_idx]
                        {
                            let hovered = self.hover_idx == Some(entry_idx);
                            draw_row_highlight(
                                &self.rt,
                                r,
                                hovered,
                                selected,
                                accent,
                                s,
                                self.light_glass,
                            );
                            let mut fg = if danger {
                                danger_text_color(self.light_glass)
                            } else {
                                let mut c = primary_text_color(self.light_glass);
                                if !selected && !hovered {
                                    c.a *= 0.90;
                                }
                                c
                            };
                            fg.a *= content_alpha;
                            let text_r = RectF::new(r.left + 12.0 * s, r.top, r.right - 10.0 * s, r.bottom);
                            draw_text_in_rect(
                                &self.dwrite_factory,
                                &self.rt,
                                label,
                                text_r,
                                &fmt,
                                fg,
                            );
                        }
                    }

                    for upper in &layout.dividers {
                        draw_divider(&self.rt, *upper, s, self.light_glass);
                    }
                }

                self.rt.PopLayer();
                let _ = ManuallyDrop::into_inner(params.geometricMask);
                let _ = ManuallyDrop::into_inner(params.opacityBrush);
            }

            draw_rounded_card_rim(&self.rt, card_rect, layout.card_radius, s, &recipe);
            self.rt.EndDraw(None, None)?;
        }

        let win_alpha =
            window_master_alpha(self.anim_phase, self.open_elapsed, self.close_progress);
        let alpha_u8 = (win_alpha * 255.0).round().clamp(0.0, 255.0) as u8;
        self.surface
            .commit_with_alpha(self.hwnd, self.win_x, self.win_y, alpha_u8);
        Ok(())
    }
}

unsafe extern "system" fn menu_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_NCDESTROY {
        let raw = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) as *const RefCell<MenuInner>;
        if !raw.is_null() {
            drop(Rc::from_raw(raw));
        }
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }

    let raw_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const RefCell<MenuInner>;
    if raw_ptr.is_null() {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    Rc::increment_strong_count(raw_ptr);
    let rc = Rc::from_raw(raw_ptr);

    match msg {
        WM_CLOSE => {
            if let Ok(mut st) = rc.try_borrow_mut() {
                st.begin_close();
            }
            LRESULT(0)
        }
        WM_ACTIVATE => {
            if (wparam.0 & 0xFFFF) as u32 == WA_INACTIVE {
                if let Ok(mut st) = rc.try_borrow_mut() {
                    st.begin_close();
                }
            }
            LRESULT(0)
        }
        WM_KEYDOWN => {
            if wparam.0 as u16 == VK_ESCAPE.0 {
                if let Ok(mut st) = rc.try_borrow_mut() {
                    st.begin_close();
                }
            }
            LRESULT(0)
        }
        WM_TIMER => {
            if wparam.0 == TIMER_HOVER_CHECK_ID && IsWindowVisible(hwnd).as_bool() {
                if let Ok(mut st) = rc.try_borrow_mut() {
                    if st.hover_mode && st.anim_phase != PopupAnimPhase::Closing {
                        let mut pt = POINT::default();
                        if GetCursorPos(&mut pt).is_ok()
                            && pt_in_hover_corridor(pt, st.trigger_rect_screen, st.card_rect_screen)
                        {
                            st.leave_since = None;
                        } else {
                            let started = *st.leave_since.get_or_insert_with(Instant::now);
                            if started.elapsed() >= Duration::from_millis(HOVER_LEAVE_CLOSE_MS) {
                                st.begin_close();
                            }
                        }
                    }
                }
                return LRESULT(0);
            }
            if wparam.0 == TIMER_FLOW_ID && IsWindowVisible(hwnd).as_bool() {
                let finished = if let Ok(mut st) = rc.try_borrow_mut() {
                    let now = Instant::now();
                    let raw_dt = now.duration_since(st.last_flow_tick).as_secs_f32();
                    st.last_flow_tick = now;
                    st.flow_phase += f64::from(raw_dt.clamp(0.001, 0.1));
                    let dt = raw_dt.clamp(0.001, 0.028);
                    let close_done = match st.anim_phase {
                        PopupAnimPhase::Opening => {
                            st.open_elapsed += dt;
                            st.open_spring
                                .step(1.0, OPEN_SPRING_STIFFNESS, OPEN_SPRING_DAMPING, dt);
                            if (st.open_spring.is_settled(1.0, 0.0015, 0.015)
                                && st.open_elapsed >= 0.145)
                                || st.open_elapsed >= 0.38
                            {
                                st.open_spring = Spring1D::still(1.0);
                                st.anim_phase = PopupAnimPhase::Open;
                            }
                            false
                        }
                        PopupAnimPhase::Open => false,
                        PopupAnimPhase::Closing => {
                            st.close_progress = (st.close_progress + dt / CLOSE_ANIM_DUR_S).min(1.0);
                            st.close_progress >= 1.0
                        }
                    };
                    if close_done {
                        true
                    } else {
                        st.sync_timer_rate();
                        let _ = st.render();
                        false
                    }
                } else {
                    false
                };
                if finished {
                    let _ = ShowWindow(hwnd, SW_HIDE);
                    let notify = if let Ok(mut st) = rc.try_borrow_mut() {
                        let target = st.target_box_hwnd;
                        let msg_id = st.notify_msg;
                        st.reset_transient();
                        (!target.is_invalid()).then_some((target, msg_id))
                    } else {
                        None
                    };
                    if let Some((target_hwnd, msg_id)) = notify {
                        if IsWindow(target_hwnd).as_bool() {
                            let _ = PostMessageW(
                                target_hwnd,
                                msg_id,
                                WPARAM(BOX_MENU_CLOSED_CODE),
                                LPARAM(0),
                            );
                        }
                    }
                }
            }
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            if let Ok(mut st) = rc.try_borrow_mut() {
                if st.anim_phase == PopupAnimPhase::Closing {
                    return LRESULT(0);
                }
                if !st.tracking_leave {
                    let mut tme = TRACKMOUSEEVENT {
                        cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    if TrackMouseEvent(&mut tme).is_ok() {
                        st.tracking_leave = true;
                    }
                }
                let x = ((lparam.0 & 0xFFFF) as i16) as f32;
                let y = (((lparam.0 >> 16) & 0xFFFF) as i16) as f32;
                let next_hover = st.current_layout().hit_test(x, y);
                if st.hover_idx != next_hover {
                    st.hover_idx = next_hover;
                    let _ = st.render();
                }
            }
            LRESULT(0)
        }
        WM_MOUSELEAVE_MSG => {
            if let Ok(mut st) = rc.try_borrow_mut() {
                st.tracking_leave = false;
                if st.anim_phase != PopupAnimPhase::Closing && st.hover_idx.is_some() {
                    st.hover_idx = None;
                    let _ = st.render();
                }
            }
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            if let Ok(mut st) = rc.try_borrow_mut() {
                let x = ((lparam.0 & 0xFFFF) as i16) as f32;
                let y = (((lparam.0 >> 16) & 0xFFFF) as i16) as f32;
                if !st.current_layout().card_rect.contains(x, y) {
                    st.begin_close();
                }
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let post_cmd = if let Ok(mut st) = rc.try_borrow_mut() {
                if st.anim_phase == PopupAnimPhase::Closing {
                    return LRESULT(0);
                }
                let x = ((lparam.0 & 0xFFFF) as i16) as f32;
                let y = (((lparam.0 >> 16) & 0xFFFF) as i16) as f32;
                let cmd = st.current_layout().hit_test(x, y).and_then(|idx| {
                    match st.entries.get(idx) {
                        Some(MenuEntry::Action { action, .. }) => {
                            Some((st.target_box_hwnd, st.notify_msg, action.to_usize()))
                        }
                        _ => None,
                    }
                });
                st.begin_close();
                cmd
            } else {
                None
            };
            if let Some((target_hwnd, msg_id, code)) = post_cmd {
                if !target_hwnd.is_invalid() && IsWindow(target_hwnd).as_bool() {
                    let _ = PostMessageW(target_hwnd, msg_id, WPARAM(code), LPARAM(0));
                }
            }
            LRESULT(0)
        }
        WM_SETCURSOR => {
            if (lparam.0 & 0xFFFF) as u32 == HTCLIENT {
                if let Ok(st) = rc.try_borrow() {
                    windows::Win32::UI::WindowsAndMessaging::SetCursor(st.arrow_cursor);
                    return LRESULT(1);
                }
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_NCHITTEST => {
            let mut pt = POINT {
                x: i32::from((lparam.0 & 0xFFFF) as i16),
                y: i32::from(((lparam.0 >> 16) & 0xFFFF) as i16),
            };
            let _ = windows::Win32::Graphics::Gdi::ScreenToClient(hwnd, &mut pt);
            let inside = rc.try_borrow().is_ok_and(|st| {
                st.anim_phase != PopupAnimPhase::Closing
                    && st.current_layout().card_rect.contains(pt.x as f32, pt.y as f32)
            });
            if inside {
                DefWindowProcW(hwnd, msg, wparam, lparam)
            } else {
                LRESULT(HTTRANSPARENT as isize)
            }
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn pt_in_rect(pt: POINT, r: RECT) -> bool {
    pt.x >= r.left && pt.x <= r.right && pt.y >= r.top && pt.y <= r.bottom
}

fn pt_in_hover_corridor(pt: POINT, trigger: RECT, card: RECT) -> bool {
    if pt_in_rect(pt, card) || pt_in_rect(pt, trigger) {
        return true;
    }
    let bridge_left = trigger.left.min(card.left);
    let bridge_right = trigger.right.max(card.right);
    if card.top >= trigger.bottom {
        let bridge = RECT {
            left: bridge_left,
            top: trigger.bottom,
            right: bridge_right,
            bottom: card.top,
        };
        pt_in_rect(pt, bridge)
    } else if card.bottom <= trigger.top {
        let bridge = RECT {
            left: bridge_left,
            top: card.bottom,
            right: bridge_right,
            bottom: trigger.top,
        };
        pt_in_rect(pt, bridge)
    } else {
        false
    }
}
