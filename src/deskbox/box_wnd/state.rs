//! Core DeskBox window state, UI metadata helpers, layout query, and DirectWrite format cache.

use super::{
    BoxContextMenu, BoxMenuContext, BoxMenuKind, BoxTooltip, TIMER_BOX_ANIM_ID,
    TIMER_HOVER_COLLAPSE_ID, TIMER_HOVER_MENU_ID, TIMER_TOOLTIP_ID,
};
use crate::deskbox::config::{folder_portal_display_name, BoxKind, BoxViewMode, DeskBoxSpec};
use crate::deskbox::dnd::DropTargetShared;
use crate::deskbox::icons::{IconService, RtIconBitmaps};
use crate::deskbox::layout::{
    compute_max_scroll_px, DeskBoxLayout, DeskBoxLayoutParams, ResizeEdge, BOX_MIN_HEIGHT_DIP,
};
use crate::deskbox::shell_menu::{ActiveShellMenu, PreparedShellMenu};
use crate::deskbox::snap::{BoxRectPx, ChainBoxNode, DockEdge};
use crate::deskbox::snap_overlay::SnapGuideOverlay;
use crate::deskbox::watcher::FolderWatcher;
use crate::ui::glass::{
    create_dc_render_target, create_text_format, CompositionEngine, LayeredSurface, PopupNoiseCache,
};
use crate::ui::{GlassSurfaceStyle, RectF, Spring1D};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use windows::core::w;

/// Fixed top-anchored vertical reference height (DIP) shared across every DeskBox's glass
/// gradients and liquid-silk origin so boxes of varying heights maintain identical appearance.
pub(super) const DESKBOX_GLASS_REF_H_DIP: f32 = 220.0;
use windows::UI::Composition::Desktop::DesktopWindowTarget;
use windows::UI::Composition::{
    CompositionBrush, CompositionRoundedRectangleGeometry, SpriteVisual, Visual,
};
use windows::Win32::Foundation::{D2DERR_RECREATE_TARGET, HWND, POINT, RECT};
use windows::Win32::Graphics::Direct2D::{
    ID2D1Bitmap, ID2D1BitmapBrush, ID2D1DCRenderTarget, ID2D1Factory1, ID2D1Geometry, ID2D1Layer,
    ID2D1StrokeStyle,
};
use windows::Win32::Graphics::DirectWrite::{
    IDWriteFactory, IDWriteTextFormat, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_PARAGRAPH_ALIGNMENT_NEAR,
    DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_TRIMMING, DWRITE_TRIMMING_GRANULARITY_CHARACTER,
    DWRITE_WORD_WRAPPING_WRAP,
};
use windows::Win32::Graphics::Gdi::HFONT;
use windows::Win32::System::Ole::RevokeDragDrop;
use windows::Win32::UI::WindowsAndMessaging::{KillTimer, ShowWindow, HCURSOR, SW_HIDE};

#[derive(Clone, Debug)]
pub(super) struct BoxDisplayItem {
    pub path: PathBuf,
    pub name: String,
    pub name_lower: String,
    pub icon_key_l: String,
    pub icon_key_j: String,
    pub is_dir: bool,
    pub exists: bool,
    pub modified_secs: u64,
}

#[link(name = "winmm")]
extern "system" {
    fn timeBeginPeriod(uperiod: u32) -> u32;
    fn timeEndPeriod(uperiod: u32) -> u32;
}

static HIGH_RES_TIMER_REFS: AtomicUsize = AtomicUsize::new(0);

/// Scoped RAII guard that elevates Windows timer resolution to 1ms only while an interactive
/// DeskBox animation is actively running, ensuring smooth ~120 FPS `WM_TIMER` ticks with zero
/// idle battery impact.
pub(super) struct HighResTimerGuard;

impl HighResTimerGuard {
    pub(super) fn acquire() -> Self {
        if HIGH_RES_TIMER_REFS.fetch_add(1, Ordering::AcqRel) == 0 {
            unsafe {
                let _ = timeBeginPeriod(1);
            }
        }
        Self
    }
}

impl Drop for HighResTimerGuard {
    fn drop(&mut self) {
        if HIGH_RES_TIMER_REFS.fetch_sub(1, Ordering::AcqRel) == 1 {
            unsafe {
                let _ = timeEndPeriod(1);
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct ChromeCacheKey {
    pub surf_w: i32,
    pub surf_h: i32,
    pub card_h_px: i32,
    pub grad_h_px: i32,
    pub dpi: u32,
    pub style: GlassSurfaceStyle,
    pub light_glass: bool,
    pub has_live_blur: bool,
    pub transparency_enabled: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct AnimBodyCacheKey {
    pub card_w_px: i32,
    pub full_card_h_px: i32,
    pub grad_h_px: i32,
    pub dpi: u32,
    pub style: GlassSurfaceStyle,
    pub light_glass: bool,
    pub has_live_blur: bool,
    pub transparency_enabled: bool,
    pub view_mode: BoxViewMode,
    pub auto_hide_header: bool,
    pub scroll_y_px: i32,
    pub selected_count: usize,
    pub item_count: usize,
    pub content_rev: u32,
}

pub(super) struct DirScanPayload {
    pub gen_id: usize,
    pub items: Vec<BoxDisplayItem>,
    pub dir_missing: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum HoverTarget {
    BackButton,
    SearchClearButton,
    NewButton,
    SortButton,
    ViewButton,
    LockButton,
    CollapseButton,
    DeleteButton,
    Item(usize),
    ItemRemove(usize),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum InlineEditMode {
    Rename,
    Search,
}

#[derive(Clone, Copy)]
pub(super) enum DragAction {
    MoveBox {
        start_screen: POINT,
        orig_win_x: i32,
        orig_win_y: i32,
    },
    Resize {
        edge: ResizeEdge,
        start_screen: POINT,
        orig_card_left: i32,
        orig_card_top: i32,
        orig_card_right: i32,
        orig_card_bottom: i32,
    },
    ScrollbarThumb {
        grab_offset_y: f32,
    },
    PendingItemDrag {
        item_idx: usize,
        start_client: POINT,
    },
    ReorderItem {
        from_idx: usize,
        target_idx: usize,
    },
    PendingRemoveClick {
        item_idx: usize,
    },
    Marquee {
        anchor_client: POINT,
        current_client: POINT,
    },
}

pub(super) struct BoxWndUserData {
    pub inner: Rc<RefCell<BoxInner>>,
    pub active_shell_menu: Rc<RefCell<Option<ActiveShellMenu>>>,
    pub pending_shell_menu: Rc<RefCell<Option<PreparedShellMenu>>>,
}

/// Shared peer entry stored in `PeerRegistry` so resizing or auto-fitting one DeskBox can
/// synchronously relocate neighboring unlocked boxes in the exact same frame without reference cycles.
#[derive(Clone)]
pub struct PeerBoxEntry {
    pub rect: BoxRectPx,
    pub locked: bool,
    pub monitor_id: String,
    pub(super) inner: Weak<RefCell<BoxInner>>,
}

pub type PeerRegistry = Rc<RefCell<HashMap<u64, PeerBoxEntry>>>;

/// Per-box target metrics prepared for monitor-wide batch elastic `AutoFitAll`.
pub struct BatchAutoFitInfo {
    pub monitor_id: String,
    pub work: RECT,
    pub dpi: u32,
    pub node: ChainBoxNode,
    pub target_rest_w: i32,
    pub target_rest_h: i32,
    pub fit_w_dip: f32,
    pub fit_h_dip: f32,
}

pub(super) struct BoxInner {
    pub hwnd: HWND,
    pub host_hwnd: HWND,
    pub self_weak: Weak<RefCell<BoxInner>>,
    pub d2d_factory: ID2D1Factory1,
    pub dwrite_factory: IDWriteFactory,
    pub comp: Option<Rc<CompositionEngine>>,
    pub icon_service: Rc<RefCell<IconService>>,
    pub box_menu: Rc<BoxContextMenu>,
    pub box_tooltip: Rc<RefCell<BoxTooltip>>,
    pub snap_overlay: Rc<RefCell<SnapGuideOverlay>>,
    pub peer_rects: PeerRegistry,
    pub resize_chain_snapshot: Vec<ChainBoxNode>,
    pub resize_chain_work: RECT,
    pub resize_chain_dpi: u32,
    pub spec: DeskBoxSpec,
    pub surface_style: GlassSurfaceStyle,
    pub light_glass: bool,
    pub transparency_enabled: bool,
    pub dpi: u32,
    pub win_x: i32,
    pub win_y: i32,
    pub win_w: i32,
    pub win_h: i32,
    pub allow_zorder_change: bool,
    pub lifted_topmost: bool,
    pub fullscreen_suppressed: bool,

    // Live desktop blur window behind card_rect (released cleanly on drop before DestroyWindow)
    pub blur_hwnd: HWND,
    pub comp_target: Option<DesktopWindowTarget>,
    pub comp_visual: Option<SpriteVisual>,
    pub comp_clip_geo: Option<CompositionRoundedRectangleGeometry>,
    pub blur_hwnd_bounds: Option<(i32, i32, i32, i32)>,
    pub blur_visible: bool,
    pub has_live_blur: bool,
    pub last_blur_key: Option<(GlassSurfaceStyle, bool, u32)>,
    pub last_clip_size: Option<(i32, i32, i32)>,
    pub shadow_geo_key: Option<(i32, i32, i32, u32)>,
    pub cached_clip_geo: Option<ID2D1Geometry>,
    pub cached_shadow_rings: [Option<ID2D1Geometry>; 3],

    // Declare D2D RT-owned resources BEFORE `surface: LayeredSurface` so RT releases before DeleteDC.
    pub noise_cache: PopupNoiseCache,
    pub silk_bmp: Option<ID2D1Bitmap>,
    pub silk_brush: Option<ID2D1BitmapBrush>,
    pub chrome_cache: [Option<(ChromeCacheKey, ID2D1Bitmap)>; 2],
    pub chrome_cache_next_slot: usize,
    pub anim_body_bmp: Option<ID2D1Bitmap>,
    pub anim_body_brush: Option<ID2D1BitmapBrush>,
    pub anim_body_key: Option<AnimBodyCacheKey>,
    pub content_rev: u32,
    pub icon_bitmaps: RtIconBitmaps,
    pub content_layer: ID2D1Layer,
    pub round_stroke: Option<ID2D1StrokeStyle>,
    pub rt: ID2D1DCRenderTarget,
    pub surface: LayeredSurface,

    pub cached_title_fmt: Option<(u32, IDWriteTextFormat)>,
    pub cached_edit_fmt: Option<(u32, IDWriteTextFormat)>,
    pub cached_grid_fmt: Option<(u32, IDWriteTextFormat)>,
    pub cached_list_fmt: Option<(u32, IDWriteTextFormat)>,
    pub cached_hint_fmt: Option<(u32, IDWriteTextFormat)>,
    pub text_layout_cache: HashMap<
        (String, i32, i32, u8, u32),
        windows::Win32::Graphics::DirectWrite::IDWriteTextLayout,
    >,

    pub all_items: Vec<BoxDisplayItem>,
    pub items: Vec<BoxDisplayItem>,
    pub nav_stack: Vec<PathBuf>,
    pub dir_manual_orders: HashMap<String, Vec<String>>,
    pub selected: HashSet<usize>,
    pub selection_anchor: Option<usize>,
    pub hover: Option<HoverTarget>,
    pub header_hovered: bool,
    pub header_revealed_at: Option<Instant>,
    pub pending_hover_menu: Option<HoverTarget>,
    pub suppress_hover_menu_btn: Option<HoverTarget>,
    pub hover_expanded: bool,
    pub suppress_hover_expand_until_leave: bool,
    pub collapse_height_spring: Spring1D,
    pub collapse_anim_to_h: f32,
    pub collapse_animating: bool,
    pub edge_dock: DockEdge,
    pub edge_drawer_open: bool,
    pub edge_slide_spring: Spring1D,
    pub edge_anim_from_t: f32,
    pub edge_anim_to_t: f32,
    pub edge_anim_elapsed: f32,
    pub edge_animating: bool,
    pub suppress_edge_open_until_leave: bool,
    pub last_anim_tick: Instant,
    pub high_res_timer: Option<HighResTimerGuard>,
    pub search_active: bool,
    pub search_query: String,
    pub scroll_y: f32,
    pub scroll_target_y: f32,
    pub scroll_spring: Spring1D,
    pub scroll_animating: bool,
    pub header_opacity_spring: Spring1D,
    pub header_opacity_animating: bool,
    pub scrollbar_hovered: bool,
    pub drag: Option<DragAction>,
    pub is_drop_hovered: bool,
    pub drop_shared: Arc<DropTargetShared>,
    pub dnd_registered: bool,
    pub watcher: Option<FolderWatcher>,
    pub watch_gen: usize,
    pub dir_scan_gen: usize,
    pub dir_scan_loading: bool,
    pub dir_scan_result: Arc<Mutex<Option<DirScanPayload>>>,
    pub portal_dir_missing: bool,

    // Transparent native EDIT proxy rendered directly as a Liquid Glass Direct2D pill
    pub inline_edit_hwnd: HWND,
    pub inline_edit_mode: Option<InlineEditMode>,
    pub inline_edit_orig_proc: isize,
    pub inline_edit_font: HFONT,
    pub inline_edit_caret_visible: bool,
    pub inline_edit_scroll_x: f32,
    pub last_inline_edit_closed_at: Option<Instant>,
    pub suppress_next_dblclk_select: bool,

    pub silk_buf: Vec<u8>,
    pub flow_phase: f64,
    pub halo_colors: [(f32, f32, f32); 2],
    pub ui_accent: (f32, f32, f32),
    pub last_silk_tick: Option<Instant>,
    pub tracking_leave: bool,

    pub arrow_cursor: HCURSOR,
    pub hand_cursor: HCURSOR,
    pub size_we_cursor: HCURSOR,
    pub size_ns_cursor: HCURSOR,
    pub size_nwse_cursor: HCURSOR,
    pub size_nesw_cursor: HCURSOR,
    pub cleaned_up: bool,
}

impl BoxInner {
    pub(super) fn cleanup_resources(&mut self) {
        if self.cleaned_up {
            return;
        }
        self.cleaned_up = true;
        self.watcher = None;
        self.cancel_inline_rename();
        self.unregister_peer_rect();
        if let Ok(mut tip) = self.box_tooltip.try_borrow_mut() {
            tip.hide_if_owner(self.hwnd);
        }
        if let Ok(mut overlay) = self.snap_overlay.try_borrow_mut() {
            overlay.hide();
        }
        self.pending_hover_menu = None;
        self.suppress_hover_menu_btn = None;
        self.header_revealed_at = None;
        if !self.hwnd.is_invalid() {
            unsafe {
                let _ = KillTimer(self.hwnd, TIMER_HOVER_COLLAPSE_ID);
                let _ = KillTimer(self.hwnd, TIMER_BOX_ANIM_ID);
                let _ = KillTimer(self.hwnd, TIMER_TOOLTIP_ID);
                let _ = KillTimer(self.hwnd, TIMER_HOVER_MENU_ID);
            }
        }
        if self.dnd_registered && !self.hwnd.is_invalid() {
            unsafe {
                let _ = RevokeDragDrop(self.hwnd);
            }
            self.dnd_registered = false;
        }
        self.hide_blur_layer();
        unsafe {
            if !self.hwnd.is_invalid() {
                let _ = ShowWindow(self.hwnd, SW_HIDE);
            }
        }
        if let Some(ref visual) = self.comp_visual {
            let _ = visual.SetBrush(None::<&CompositionBrush>);
            let _ = visual.SetClip(None);
        }
        self.comp_clip_geo = None;
        self.blur_hwnd_bounds = None;
        if let Some(target) = self.comp_target.take() {
            let _ = target.SetRoot(None::<&Visual>);
            let _ = target.Close();
        }
        self.comp_visual = None;
        self.icon_bitmaps.clear();
        self.silk_bmp = None;
        self.silk_brush = None;
        self.clear_chrome_cache();
        self.clear_anim_body_cache();
        self.high_res_timer = None;
        self.text_layout_cache.clear();
    }

    #[inline]
    pub(super) fn clear_chrome_cache(&mut self) {
        self.chrome_cache = [None, None];
        self.chrome_cache_next_slot = 0;
    }

    #[inline]
    pub(super) fn clear_anim_body_cache(&mut self) {
        self.anim_body_brush = None;
        self.anim_body_bmp = None;
        self.anim_body_key = None;
    }

    #[inline]
    pub(super) fn bump_content_rev(&mut self) {
        self.content_rev = self.content_rev.wrapping_add(1);
        self.clear_anim_body_cache();
    }

    #[inline]
    pub(super) fn full_expanded_card_height_px(&self, s: f32) -> f32 {
        (self.spec.h_dip.max(BOX_MIN_HEIGHT_DIP) * s).round().max(1.0)
    }

    pub(super) fn current_portal_dir(&self) -> PathBuf {
        self.nav_stack
            .last()
            .cloned()
            .unwrap_or_else(|| PathBuf::from(&self.spec.folder_path))
    }

    pub(super) fn can_go_back(&self) -> bool {
        self.spec.kind == BoxKind::FolderPortal && !self.nav_stack.is_empty()
    }

    pub(super) fn header_display_title(&self) -> String {
        if self.spec.kind == BoxKind::FolderPortal {
            let mut out = folder_portal_display_name(&self.spec.folder_path);
            for sub in &self.nav_stack {
                let name = sub
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or_default();
                if !name.is_empty() {
                    out.push_str(" / ");
                    out.push_str(name);
                }
            }
            out
        } else {
            self.spec.title.clone()
        }
    }

    pub(super) fn open_menu_kind_for_self(&self) -> Option<BoxMenuKind> {
        self.box_menu.open_kind_for(self.hwnd)
    }

    pub(super) fn menu_context(&self) -> BoxMenuContext {
        BoxMenuContext {
            kind: self.spec.kind,
            view_mode: self.spec.view_mode,
            edge_auto_hide: self.spec.edge_auto_hide,
            has_missing: self.has_missing_items(),
        }
    }

    /// Returns `(client_rect, tooltip_text)` for hoverable buttons that do not open a dropdown menu.
    pub(super) fn tooltip_spec_for_hover(
        &self,
        target: HoverTarget,
        layout: &DeskBoxLayout,
    ) -> Option<(RectF, &'static str)> {
        match target {
            HoverTarget::BackButton if layout.back_btn_rect.width() > 1.0 => {
                Some((layout.back_btn_rect, "返回上一级"))
            }
            HoverTarget::LockButton if layout.lock_btn_rect.width() > 1.0 => Some((
                layout.lock_btn_rect,
                if self.spec.locked {
                    "解锁位置"
                } else {
                    "锁定位置"
                },
            )),
            HoverTarget::CollapseButton if layout.collapse_btn_rect.width() > 1.0 => Some((
                layout.collapse_btn_rect,
                if self.spec.collapsed {
                    "展开收纳格"
                } else {
                    "收起收纳格"
                },
            )),
            HoverTarget::DeleteButton if layout.delete_btn_rect.width() > 1.0 => {
                Some((layout.delete_btn_rect, "删除收纳格"))
            }
            HoverTarget::SearchClearButton if layout.search_clear_btn_rect.width() > 1.0 => {
                Some((layout.search_clear_btn_rect, "清除搜索"))
            }
            HoverTarget::ItemRemove(idx) => layout
                .visible_cells
                .iter()
                .find(|c| c.index == idx && c.remove_btn_rect.width() > 1.0)
                .map(|c| (c.remove_btn_rect, "移出收纳格")),
            _ => None,
        }
    }

    pub(super) fn hover_menu_spec(
        target: HoverTarget,
        layout: &DeskBoxLayout,
    ) -> Option<(BoxMenuKind, RectF)> {
        match target {
            HoverTarget::NewButton if layout.new_btn_rect.width() > 1.0 => {
                Some((BoxMenuKind::NewBox, layout.new_btn_rect))
            }
            HoverTarget::SortButton if layout.sort_btn_rect.width() > 1.0 => {
                Some((BoxMenuKind::Sort, layout.sort_btn_rect))
            }
            HoverTarget::ViewButton if layout.view_btn_rect.width() > 1.0 => {
                Some((BoxMenuKind::ViewAndTools, layout.view_btn_rect))
            }
            _ => None,
        }
    }

    pub(super) fn cancel_pending_hover_menu(&mut self) {
        self.pending_hover_menu = None;
        if !self.hwnd.is_invalid() {
            unsafe {
                let _ = KillTimer(self.hwnd, TIMER_HOVER_MENU_ID);
            }
        }
    }

    pub(super) fn hide_tooltip(&self) {
        if !self.hwnd.is_invalid() {
            unsafe {
                let _ = KillTimer(self.hwnd, TIMER_TOOLTIP_ID);
            }
        }
        if let Ok(mut tip) = self.box_tooltip.try_borrow_mut() {
            tip.hide_if_owner(self.hwnd);
        }
    }

    pub(super) fn is_edge_tucked(&self) -> bool {
        self.spec.edge_auto_hide && self.edge_slide_spring.pos <= 0.015
    }

    pub(super) fn should_show_header_actions(&self) -> bool {
        self.inline_edit_hwnd.is_invalid()
            && !self.search_active
            && (self.header_hovered || self.open_menu_kind_for_self().is_some())
    }

    pub(super) fn is_header_visible(&self) -> bool {
        !self.spec.auto_hide_header
            || self.is_visually_collapsed()
            || self.header_hovered
            || self.search_active
            || !self.inline_edit_hwnd.is_invalid()
            || self.open_menu_kind_for_self().is_some()
    }

    pub(super) fn inline_edit_pill_rect(layout: &DeskBoxLayout, mode: InlineEditMode) -> RectF {
        let s = layout.scale;
        let cy = (layout.header_rect.top + layout.header_rect.bottom) * 0.5;
        let left = match mode {
            InlineEditMode::Rename => {
                (layout.title_rect.left - 4.0 * s).max(layout.card_rect.left + 6.0 * s)
            }
            InlineEditMode::Search => layout.card_rect.left + 8.0 * s,
        };
        let right = (layout.card_rect.right - 8.0 * s).max(left + 60.0 * s);
        RectF::new(left, cy - 11.5 * s, right, cy + 11.5 * s)
    }

    pub(super) fn is_visually_collapsed(&self) -> bool {
        self.spec.collapsed && !self.hover_expanded
    }

    pub(super) fn current_layout(&self) -> DeskBoxLayout {
        DeskBoxLayout::compute(DeskBoxLayoutParams {
            width_dip: self.spec.w_dip,
            height_dip: self.spec.h_dip,
            dpi: self.dpi,
            item_count: self.items.len(),
            view_mode: self.spec.view_mode,
            scroll_y: self.scroll_y,
            can_go_back: self.can_go_back(),
            show_item_remove: self.spec.kind == BoxKind::Collection,
            collapsed: self.is_visually_collapsed() && !self.collapse_animating,
            show_header_actions: self.should_show_header_actions(),
            locked: self.spec.locked,
            auto_hide_header: self.spec.auto_hide_header,
            header_visible: self.is_header_visible(),
            search_active: self.search_active,
            card_height_override_px: self
                .collapse_animating
                .then_some(self.collapse_height_spring.pos),
        })
    }

    pub(super) fn max_scroll_px(&self) -> f32 {
        compute_max_scroll_px(
            self.spec.w_dip,
            self.spec.h_dip,
            self.dpi,
            self.items.len(),
            self.spec.view_mode,
            self.spec.auto_hide_header,
            self.is_visually_collapsed(),
        )
    }

    pub(super) fn set_scroll_target(&mut self, desired_y: f32, animate: bool) {
        let max_scroll = self.max_scroll_px();
        let clamped = desired_y.clamp(0.0, max_scroll);
        if animate {
            if (clamped - self.scroll_target_y).abs() > 0.1
                || (clamped - self.scroll_y).abs() > 0.5
            {
                self.scroll_target_y = clamped;
                if !self.scroll_animating {
                    self.scroll_spring = Spring1D::still(self.scroll_y);
                }
                self.scroll_animating = true;
                self.start_anim_timer();
            }
        } else {
            self.scroll_y = clamped;
            self.scroll_target_y = clamped;
            self.scroll_spring = Spring1D::still(clamped);
            self.scroll_animating = false;
        }
    }


    pub(super) fn ensure_formats(
        &mut self,
        s: f32,
    ) -> windows::core::Result<(
        IDWriteTextFormat,
        IDWriteTextFormat,
        IDWriteTextFormat,
        IDWriteTextFormat,
        IDWriteTextFormat,
    )> {
        if let (Some((d0, f0)), Some((d1, f1)), Some((d2, f2)), Some((d3, f3)), Some((d4, f4))) = (
            &self.cached_title_fmt,
            &self.cached_edit_fmt,
            &self.cached_grid_fmt,
            &self.cached_list_fmt,
            &self.cached_hint_fmt,
        ) {
            if *d0 == self.dpi
                && *d1 == self.dpi
                && *d2 == self.dpi
                && *d3 == self.dpi
                && *d4 == self.dpi
            {
                return Ok((f0.clone(), f1.clone(), f2.clone(), f3.clone(), f4.clone()));
            }
        }

        let title_fmt = create_text_format(&self.dwrite_factory, 13.5 * s, true, false, true)?;
        let edit_fmt = create_text_format(&self.dwrite_factory, 13.5 * s, true, false, false)?;
        let grid_fmt = create_grid_label_format(&self.dwrite_factory, 11.5 * s)?;
        let list_fmt = create_text_format(&self.dwrite_factory, 12.0 * s, false, false, true)?;
        let hint_fmt = create_hint_format(&self.dwrite_factory, 12.5 * s)?;

        self.cached_title_fmt = Some((self.dpi, title_fmt.clone()));
        self.cached_edit_fmt = Some((self.dpi, edit_fmt.clone()));
        self.cached_grid_fmt = Some((self.dpi, grid_fmt.clone()));
        self.cached_list_fmt = Some((self.dpi, list_fmt.clone()));
        self.cached_hint_fmt = Some((self.dpi, hint_fmt.clone()));
        Ok((title_fmt, edit_fmt, grid_fmt, list_fmt, hint_fmt))
    }

    pub(super) fn render(&mut self) -> windows::core::Result<()> {
        match self.render_once() {
            Err(e) if e.code() == D2DERR_RECREATE_TARGET => {
                self.silk_bmp = None;
                self.silk_brush = None;
                self.clear_chrome_cache();
                self.clear_anim_body_cache();
                self.text_layout_cache.clear();
                self.noise_cache.clear();
                self.icon_bitmaps.clear();
                self.shadow_geo_key = None;
                self.cached_clip_geo = None;
                self.cached_shadow_rings = [None, None, None];
                self.last_silk_tick = None;
                let new_rt = create_dc_render_target(&self.d2d_factory)?;
                self.content_layer = unsafe { new_rt.CreateLayer(None)? };
                self.rt = new_rt;
                self.render_once()
            }
            res => res,
        }
    }

    pub(super) fn marquee_rect_in_content(&self, content_rect: RectF) -> Option<RectF> {
        let Some(DragAction::Marquee {
            anchor_client,
            current_client,
        }) = self.drag
        else {
            return None;
        };
        let max_r = content_rect.right.max(content_rect.left);
        let max_b = content_rect.bottom.max(content_rect.top);
        let x0 = (anchor_client.x as f32).clamp(content_rect.left, max_r);
        let y0 = (anchor_client.y as f32).clamp(content_rect.top, max_b);
        let x1 = (current_client.x as f32).clamp(content_rect.left, max_r);
        let y1 = (current_client.y as f32).clamp(content_rect.top, max_b);
        let left = x0.min(x1);
        let right = x0.max(x1);
        let top = y0.min(y1);
        let bottom = y0.max(y1);
        if (right - left) < 2.0 && (bottom - top) < 2.0 {
            None
        } else {
            Some(RectF::new(left, top, right, bottom))
        }
    }
}

fn create_grid_label_format(
    factory: &IDWriteFactory,
    size_px: f32,
) -> windows::core::Result<IDWriteTextFormat> {
    unsafe {
        let fmt = factory.CreateTextFormat(
            w!("Segoe UI Variable Text"),
            None,
            DWRITE_FONT_WEIGHT_NORMAL,
            windows::Win32::Graphics::DirectWrite::DWRITE_FONT_STYLE_NORMAL,
            windows::Win32::Graphics::DirectWrite::DWRITE_FONT_STRETCH_NORMAL,
            size_px.max(9.0),
            w!("zh-CN"),
        )?;
        let _ = fmt.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER);
        let _ = fmt.SetParagraphAlignment(
            windows::Win32::Graphics::DirectWrite::DWRITE_PARAGRAPH_ALIGNMENT_CENTER,
        );
        let _ = fmt.SetWordWrapping(
            windows::Win32::Graphics::DirectWrite::DWRITE_WORD_WRAPPING_NO_WRAP,
        );
        if let Ok(ellipsis) = factory.CreateEllipsisTrimmingSign(&fmt) {
            let trimming = DWRITE_TRIMMING {
                granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER,
                delimiter: 0,
                delimiterCount: 0,
            };
            let _ = fmt.SetTrimming(&trimming, Some(&ellipsis));
        }
        Ok(fmt)
    }
}

fn create_hint_format(
    factory: &IDWriteFactory,
    size_px: f32,
) -> windows::core::Result<IDWriteTextFormat> {
    unsafe {
        let fmt = factory.CreateTextFormat(
            w!("Segoe UI Variable Text"),
            None,
            DWRITE_FONT_WEIGHT_NORMAL,
            windows::Win32::Graphics::DirectWrite::DWRITE_FONT_STYLE_NORMAL,
            windows::Win32::Graphics::DirectWrite::DWRITE_FONT_STRETCH_NORMAL,
            size_px.max(10.0),
            w!("zh-CN"),
        )?;
        let _ = fmt.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER);
        let _ = fmt.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_NEAR);
        let _ = fmt.SetWordWrapping(DWRITE_WORD_WRAPPING_WRAP);
        Ok(fmt)
    }
}
