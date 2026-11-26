//! Per-box Win32 layered window, desktop Z-order attachment, GPU backdrop blur sync, and state.

mod animation;
mod items;
mod menu;
mod placement;
mod render;
mod state;
mod tooltip;
mod wndproc;
mod zorder;

pub use menu::{BoxContextMenu, BoxMenuAction, BoxMenuContext, BoxMenuKind, BOX_MENU_CLOSED_CODE};
pub use state::{BatchAutoFitInfo, PeerBoxEntry, PeerRegistry};
pub use tooltip::BoxTooltip;

use state::{
    AnimBodyCacheKey, BoxDisplayItem, BoxInner, BoxWndUserData, ChromeCacheKey, DirScanPayload,
    DragAction, HighResTimerGuard, HoverTarget, InlineEditMode, DESKBOX_GLASS_REF_H_DIP,
};

use super::config::{BoxKind, BoxViewMode, DeskBoxSpec};
use super::desktop::{
    monitor_info_for_window, resolve_box_window_geometry, DesktopZOrderScan,
};
use super::dnd::{BoxDropTarget, DropTargetShared};
use super::icons::{IconService, RtIconBitmaps};
use super::layout::{
    compute_auto_fit_size_dip, BOX_HEADER_H_DIP, BOX_MIN_HEIGHT_DIP, BOX_MIN_WIDTH_DIP,
};
use super::snap::{
    compute_edge_drawer_window_bounds, detect_dock_edge, BoxRectPx, ChainBoxNode,
};
use super::snap_overlay::SnapGuideOverlay;
use crate::system::exclude_from_peek;
use crate::ui::glass::{
    create_dc_render_target, create_round_cap_stroke, CompositionEngine, LayeredSurface,
    PopupNoiseCache,
};
use crate::ui::{GlassSurfaceStyle, Spring1D, SILK_H, SILK_W};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::mem::size_of;
use std::rc::{Rc, Weak};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use windows::core::{w, PCWSTR};
use windows::UI::Composition::Visual;
use windows::Win32::Foundation::{BOOL, HWND, RECT};
use windows::Win32::Graphics::Direct2D::{ID2D1DCRenderTarget, ID2D1Factory1, ID2D1Layer};
use windows::Win32::Graphics::DirectWrite::IDWriteFactory;
use windows::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_USE_HOSTBACKDROPBRUSH, DWMWA_WINDOW_CORNER_PREFERENCE,
    DWM_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND,
};
use windows::Win32::Graphics::Gdi::HFONT;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Ole::{RegisterDragDrop, IDropTarget};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, LoadCursorW, RegisterClassExW, SetWindowLongPtrW, CS_DBLCLKS,
    GWLP_USERDATA, IDC_ARROW, IDC_HAND, IDC_SIZENESW, IDC_SIZENS, IDC_SIZENWSE, IDC_SIZEWE,
    WM_APP, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_NOREDIRECTIONBITMAP,
    WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT, WS_POPUP,
};

pub const WM_APP_DESKBOX_ICON_READY: u32 = WM_APP + 60;
pub const WM_APP_DESKBOX_DIR_CHANGED: u32 = WM_APP + 61;
pub const WM_APP_DESKBOX_DROP: u32 = WM_APP + 62;
pub const WM_APP_DESKBOX_MENU_ACTION: u32 = WM_APP + 63;
pub const WM_APP_DESKBOX_HOST_CMD: u32 = WM_APP + 64;
pub const WM_APP_DESKBOX_ZORDER_SYNC: u32 = WM_APP + 65;
pub const WM_APP_DESKBOX_DIR_LOADED: u32 = WM_APP + 66;

pub const HOST_CMD_NEW_COLLECTION: usize = 1;
pub const HOST_CMD_NEW_FOLDER_PORTAL: usize = 2;
pub const HOST_CMD_DELETE_BOX: usize = 3;
pub const HOST_CMD_PERSIST_CONFIG: usize = 4;

pub(super) const TIMER_HOVER_COLLAPSE_ID: usize = 2;
pub(super) const TIMER_BOX_ANIM_ID: usize = 3;
pub(super) const TIMER_TOOLTIP_ID: usize = 4;
pub(super) const TIMER_HOVER_MENU_ID: usize = 5;
// 8ms cadence feeds both 60Hz and high-refresh monitors cleanly without aliasing 15.625ms ticks.
const BOX_ANIM_INTERVAL_MS: u32 = 8;
// Second-order near-critically-damped springs (ζ ≈ 0.97) for silky velocity-continuous hover expand/retract.
const COLLAPSE_EXPAND_STIFFNESS: f32 = 780.0;
const COLLAPSE_EXPAND_DAMPING: f32 = 54.0;
const COLLAPSE_RETRACT_STIFFNESS: f32 = 540.0;
const COLLAPSE_RETRACT_DAMPING: f32 = 45.5;
const EDGE_ANIM_DUR_S: f32 = 0.115;

const BOX_CLASS_NAME: PCWSTR = w!("PrettyWindowsDeskBoxWnd");
const BLUR_CLASS_NAME: PCWSTR = w!("PrettyWindowsDeskBoxBlurWnd");

const FILE_ATTR_HIDDEN: u32 = 0x2;
const FILE_ATTR_SYSTEM: u32 = 0x4;

static BOX_CLASSES_READY: std::sync::Once = std::sync::Once::new();


pub struct BoxWnd {
    hwnd: HWND,
    inner: Rc<RefCell<BoxInner>>,
}

impl BoxWnd {
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        host_hwnd: HWND,
        d2d_factory: ID2D1Factory1,
        dwrite_factory: IDWriteFactory,
        comp: Option<Rc<CompositionEngine>>,
        icon_service: Rc<RefCell<IconService>>,
        box_menu: Rc<BoxContextMenu>,
        box_tooltip: Rc<RefCell<BoxTooltip>>,
        snap_overlay: Rc<RefCell<SnapGuideOverlay>>,
        peer_rects: PeerRegistry,
        mut spec: DeskBoxSpec,
        surface_style: GlassSurfaceStyle,
        light_glass: bool,
        transparency_enabled: bool,
    ) -> windows::core::Result<Self> {
        ensure_box_window_classes()?;
        if spec.edge_auto_hide {
            spec.collapsed = false;
        }

        let (cx, cy, cw, ch, mut win_x, mut win_y, mut win_w, mut win_h, mon) =
            resolve_box_window_geometry(
                &spec.monitor_id,
                spec.x_dip,
                spec.y_dip,
                spec.w_dip,
                spec.h_dip,
                spec.collapsed,
            );
        spec.x_dip = cx;
        spec.y_dip = cy;
        spec.w_dip = cw;
        spec.h_dip = ch;
        if spec.monitor_id.trim().is_empty() && !mon.monitor_id.is_empty() {
            spec.monitor_id = mon.monitor_id.clone();
        }
        let s = (mon.dpi.max(96) as f32) / 96.0;
        let work_w_dip = (mon.work.right - mon.work.left).max(260) as f32 / s;
        let edge_dock = detect_dock_edge(spec.x_dip, spec.y_dip, spec.w_dip, work_w_dip);
        if spec.edge_auto_hide {
            let (ex, ey, ew, eh) = compute_edge_drawer_window_bounds(
                edge_dock,
                spec.x_dip,
                spec.y_dip,
                spec.w_dip,
                spec.h_dip,
                mon.work.left,
                mon.work.top,
                mon.work.right,
                mon.work.bottom,
                mon.dpi,
                0.0,
            );
            win_x = ex;
            win_y = ey;
            win_w = ew;
            win_h = eh;
        }
        let init_card_h_px = if spec.collapsed {
            BOX_HEADER_H_DIP * s
        } else {
            spec.h_dip.max(BOX_MIN_HEIGHT_DIP) * s
        };

        unsafe {
            let hinst = GetModuleHandleW(None)?;
            let arrow_cursor = LoadCursorW(None, IDC_ARROW)?;
            let hand_cursor = LoadCursorW(None, IDC_HAND)?;
            let size_we_cursor = LoadCursorW(None, IDC_SIZEWE)?;
            let size_ns_cursor = LoadCursorW(None, IDC_SIZENS)?;
            let size_nwse_cursor = LoadCursorW(None, IDC_SIZENWSE)?;
            let size_nesw_cursor = LoadCursorW(None, IDC_SIZENESW)?;

            let blur_hwnd = CreateWindowExW(
                WS_EX_NOREDIRECTIONBITMAP | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                BLUR_CLASS_NAME,
                w!("PrettyWindowsDeskBoxBlur"),
                WS_POPUP,
                win_x,
                win_y,
                1,
                1,
                None,
                None,
                hinst,
                None,
            )?;
            exclude_from_peek(blur_hwnd);
            let no_round = DWMWCP_DONOTROUND;
            let _ = DwmSetWindowAttribute(
                blur_hwnd,
                DWMWA_WINDOW_CORNER_PREFERENCE,
                (&no_round as *const DWM_WINDOW_CORNER_PREFERENCE).cast(),
                size_of::<DWM_WINDOW_CORNER_PREFERENCE>() as u32,
            );
            let enable_hb = BOOL(1);
            let _ = DwmSetWindowAttribute(
                blur_hwnd,
                DWMWA_USE_HOSTBACKDROPBRUSH,
                (&enable_hb as *const BOOL).cast(),
                size_of::<BOOL>() as u32,
            );

            let (comp_target, comp_visual) = match comp
                .as_ref()
                .and_then(|c| c.create_target_and_visual(blur_hwnd))
            {
                Some((t, v)) => (Some(t), Some(v)),
                None => (None, None),
            };

            let box_hwnd = match CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOOLWINDOW,
                BOX_CLASS_NAME,
                w!("PrettyWindowsDeskBox"),
                WS_POPUP,
                win_x,
                win_y,
                win_w,
                win_h,
                None,
                None,
                hinst,
                None,
            ) {
                Ok(h) => h,
                Err(e) => {
                    if let Some(target) = comp_target {
                        let _ = target.SetRoot(None::<&Visual>);
                        let _ = target.Close();
                    }
                    let _ = DestroyWindow(blur_hwnd);
                    return Err(e);
                }
            };
            exclude_from_peek(box_hwnd);

            let d2d_init = (|| -> windows::core::Result<(
                ID2D1DCRenderTarget,
                ID2D1Layer,
                LayeredSurface,
            )> {
                let rt = create_dc_render_target(&d2d_factory)?;
                let content_layer = rt.CreateLayer(None)?;
                let surface = LayeredSurface::new()?;
                Ok((rt, content_layer, surface))
            })();
            let (rt, content_layer, surface) = match d2d_init {
                Ok(v) => v,
                Err(e) => {
                    let _ = DestroyWindow(box_hwnd);
                    if let Some(target) = comp_target {
                        let _ = target.SetRoot(None::<&Visual>);
                        let _ = target.Close();
                    }
                    let _ = DestroyWindow(blur_hwnd);
                    return Err(e);
                }
            };
            let round_stroke = create_round_cap_stroke(&d2d_factory);

            let drop_shared = Arc::new(DropTargetShared::new(spec.kind == BoxKind::Collection));

            let drop_target: IDropTarget =
                BoxDropTarget::new(box_hwnd, WM_APP_DESKBOX_DROP, Arc::clone(&drop_shared)).into();
            let dnd_registered = RegisterDragDrop(box_hwnd, &drop_target).is_ok();

            let initial_edge_auto_hide = spec.edge_auto_hide;
            let initial_header_opacity = if spec.auto_hide_header && !spec.collapsed {
                0.0
            } else {
                1.0
            };
            let inner = Rc::new(RefCell::new(BoxInner {
                hwnd: box_hwnd,
                host_hwnd,
                self_weak: Weak::new(),
                d2d_factory,
                dwrite_factory,
                comp,
                icon_service,
                box_menu,
                box_tooltip,
                snap_overlay,
                peer_rects,
                resize_chain_snapshot: Vec::new(),
                resize_chain_work: mon.work,
                resize_chain_dpi: mon.dpi,
                spec,
                surface_style,
                light_glass,
                transparency_enabled,
                dpi: mon.dpi,
                win_x,
                win_y,
                win_w,
                win_h,
                allow_zorder_change: false,
                lifted_topmost: initial_edge_auto_hide,
                fullscreen_suppressed: false,
                blur_hwnd,
                comp_target,
                comp_visual,
                comp_clip_geo: None,
                blur_hwnd_bounds: None,
                blur_visible: false,
                has_live_blur: false,
                last_blur_key: None,
                last_clip_size: None,
                shadow_geo_key: None,
                cached_clip_geo: None,
                cached_shadow_rings: [None, None, None],
                noise_cache: PopupNoiseCache::default(),
                silk_bmp: None,
                silk_brush: None,
                chrome_cache: [None, None],
                chrome_cache_next_slot: 0,
                anim_body_bmp: None,
                anim_body_brush: None,
                anim_body_key: None,
                content_rev: 0,
                icon_bitmaps: RtIconBitmaps::default(),
                content_layer,
                round_stroke,
                rt,
                surface,
                cached_title_fmt: None,
                cached_edit_fmt: None,
                cached_grid_fmt: None,
                cached_list_fmt: None,
                cached_hint_fmt: None,
                text_layout_cache: HashMap::new(),
                all_items: Vec::new(),
                items: Vec::new(),
                nav_stack: Vec::new(),
                dir_manual_orders: HashMap::new(),
                selected: HashSet::new(),
                selection_anchor: None,
                hover: None,
                header_hovered: false,
                header_revealed_at: None,
                pending_hover_menu: None,
                suppress_hover_menu_btn: None,
                hover_expanded: false,
                suppress_hover_expand_until_leave: false,
                collapse_height_spring: Spring1D::still(init_card_h_px),
                collapse_anim_to_h: init_card_h_px,
                collapse_animating: false,
                edge_dock,
                edge_drawer_open: false,
                edge_slide_spring: Spring1D::still(if initial_edge_auto_hide { 0.0 } else { 1.0 }),
                edge_anim_from_t: if initial_edge_auto_hide { 0.0 } else { 1.0 },
                edge_anim_to_t: if initial_edge_auto_hide { 0.0 } else { 1.0 },
                edge_anim_elapsed: EDGE_ANIM_DUR_S,
                edge_animating: false,
                suppress_edge_open_until_leave: false,
                last_anim_tick: Instant::now(),
                high_res_timer: None,
                search_active: false,
                search_query: String::new(),
                scroll_y: 0.0,
                scroll_target_y: 0.0,
                scroll_spring: Spring1D::still(0.0),
                scroll_animating: false,
                header_opacity_spring: Spring1D::still(initial_header_opacity),
                header_opacity_animating: false,
                scrollbar_hovered: false,
                drag: None,
                is_drop_hovered: false,
                drop_shared,
                dnd_registered,
                watcher: None,
                watch_gen: 0,
                dir_scan_gen: 0,
                dir_scan_loading: false,
                dir_scan_result: Arc::new(Mutex::new(None)),
                portal_dir_missing: false,
                inline_edit_hwnd: HWND::default(),
                inline_edit_mode: None,
                inline_edit_orig_proc: 0,
                inline_edit_font: HFONT::default(),
                inline_edit_caret_visible: false,
                inline_edit_scroll_x: 0.0,
                last_inline_edit_closed_at: None,
                suppress_next_dblclk_select: false,
                silk_buf: vec![0u8; (SILK_W * SILK_H * 4) as usize],
                flow_phase: crate::ui::shared_liquid_flow_phase(),
                halo_colors: [(0.76, 0.76, 0.78); 2],
                ui_accent: (0.76, 0.76, 0.78),
                last_silk_tick: None,
                tracking_leave: false,
                arrow_cursor,
                hand_cursor,
                size_we_cursor,
                size_ns_cursor,
                size_nwse_cursor,
                size_nesw_cursor,
                cleaned_up: false,
            }));

            let active_shell_menu = Rc::new(RefCell::new(None));
            let pending_shell_menu = Rc::new(RefCell::new(None));
            let user_data = Box::new(BoxWndUserData {
                inner: Rc::clone(&inner),
                active_shell_menu,
                pending_shell_menu,
            });
            SetWindowLongPtrW(box_hwnd, GWLP_USERDATA, Box::into_raw(user_data) as isize);

            if let Ok(mut st) = inner.try_borrow_mut() {
                st.self_weak = Rc::downgrade(&inner);
                st.reload_items();
                st.restart_watcher();
                st.update_halo_colors();
                st.register_peer_rect();
                let _ = st.render();
                st.reattach_desktop_layer_internal();
            }

            Ok(Self {
                hwnd: box_hwnd,
                inner,
            })
        }
    }

    pub fn spec(&self) -> DeskBoxSpec {
        self.inner
            .try_borrow()
            .map(|s| s.spec.clone())
            .unwrap_or_else(|_| DeskBoxSpec::new_collection(0, "", 0.0, 0.0))
    }

    pub fn box_id(&self) -> u64 {
        self.inner.try_borrow().map_or(0, |s| s.spec.id)
    }

    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }

    pub fn set_view_mode(&mut self, view: BoxViewMode) {
        if let Ok(mut st) = self.inner.try_borrow_mut() {
            if st.spec.view_mode != view {
                st.spec.view_mode = view;
                st.bump_content_rev();
                let layout = st.current_layout();
                st.scroll_y = layout.scroll_y;
                st.scroll_target_y = layout.scroll_y;
                st.scroll_spring = Spring1D::still(layout.scroll_y);
                st.scroll_animating = false;
                let _ = st.render();
            }
        }
    }

    pub fn auto_fit_size(&mut self) {
        if let Ok(mut st) = self.inner.try_borrow_mut() {
            st.auto_fit_size();
        }
    }

    pub fn is_edge_auto_hide(&self) -> bool {
        self.inner.try_borrow().is_ok_and(|st| st.spec.edge_auto_hide)
    }

    pub fn set_locked(&mut self, locked: bool) {
        if let Ok(mut st) = self.inner.try_borrow_mut() {
            if st.spec.locked != locked {
                st.spec.locked = locked;
                st.register_peer_rect();
                let _ = st.render();
            }
        }
    }

    /// Computes this box's monitor group, current resting footprint, and target auto-fit size
    /// without mutating position yet so `DeskBoxController` can run monitor-wide batch elastic reflow.
    pub fn prepare_batch_auto_fit(&mut self) -> Option<BatchAutoFitInfo> {
        let mut st = self.inner.try_borrow_mut().ok()?;
        if st.cleaned_up || st.spec.edge_auto_hide {
            return None;
        }
        st.cancel_inline_rename();
        st.settle_collapse_animation_now();
        let mon = monitor_info_for_window(st.hwnd);
        st.dpi = mon.dpi;
        st.spec.monitor_id = mon.monitor_id.clone();
        let rest_rect = st.resting_card_rect_px_inner();
        if st.spec.locked {
            return Some(BatchAutoFitInfo {
                monitor_id: mon.monitor_id,
                work: mon.work,
                dpi: st.dpi,
                node: ChainBoxNode {
                    rect: rest_rect,
                    locked: true,
                },
                target_rest_w: rest_rect.width(),
                target_rest_h: rest_rect.height(),
                fit_w_dip: st.spec.w_dip,
                fit_h_dip: st.spec.h_dip,
            });
        }
        let s = (st.dpi.max(96) as f32) / 96.0;
        let margin = (6.0 * s).round();
        let work_w_dip = ((mon.work.right - mon.work.left).max(260) as f32 - margin * 2.0) / s;
        let work_h_dip = ((mon.work.bottom - mon.work.top).max(220) as f32 - margin * 2.0) / s;
        let min_header_w_dip = st.measure_header_min_width_dip();
        let preferred_list_w_dip = st.measure_compact_list_width_dip();
        let (fit_w_dip, fit_h_dip) = compute_auto_fit_size_dip(
            st.items.len(),
            st.spec.view_mode,
            st.spec.auto_hide_header,
            st.spec.w_dip,
            min_header_w_dip,
            preferred_list_w_dip,
            work_w_dip,
            work_h_dip,
        );
        let target_rest_w = (fit_w_dip.max(BOX_MIN_WIDTH_DIP) * s).round() as i32;
        let target_rest_h = if st.spec.collapsed {
            (BOX_HEADER_H_DIP * s).round() as i32
        } else {
            (fit_h_dip.max(BOX_MIN_HEIGHT_DIP) * s).round() as i32
        };
        Some(BatchAutoFitInfo {
            monitor_id: mon.monitor_id,
            work: mon.work,
            dpi: st.dpi,
            node: ChainBoxNode {
                rect: rest_rect,
                locked: st.spec.locked,
            },
            target_rest_w,
            target_rest_h,
            fit_w_dip,
            fit_h_dip,
        })
    }

    /// Applies the resolved resting rectangle and auto-fit dimensions produced by batch elastic reflow.
    pub fn apply_batch_auto_fit(
        &mut self,
        resolved_rest: BoxRectPx,
        fit_w_dip: f32,
        fit_h_dip: f32,
        work: RECT,
        dpi: u32,
    ) {
        if let Ok(mut st) = self.inner.try_borrow_mut() {
            if st.cleaned_up || st.spec.edge_auto_hide || st.spec.locked {
                return;
            }
            st.apply_resolved_auto_fit_rect(resolved_rest, fit_w_dip, fit_h_dip, work, dpi);
            let _ = st.render();
        }
    }

    pub fn set_appearance(
        &mut self,
        style: GlassSurfaceStyle,
        light_glass: bool,
        transparency_enabled: bool,
    ) {
        if let Ok(mut st) = self.inner.try_borrow_mut() {
            let changed = st.surface_style != style
                || st.light_glass != light_glass
                || st.transparency_enabled != transparency_enabled;
            st.surface_style = style;
            st.light_glass = light_glass;
            st.transparency_enabled = transparency_enabled;
            let accent_changed = st.update_halo_colors();
            if changed || accent_changed {
                if changed {
                    st.last_blur_key = None;
                }
                st.last_silk_tick = None;
                st.sync_blur_layer();
                let _ = st.render();
                if changed {
                    st.reattach_desktop_layer_internal();
                }
            }
        }
    }

    pub fn on_display_change(&mut self) {
        if let Ok(mut st) = self.inner.try_borrow_mut() {
            let (cx, cy, cw, ch, mut win_x, mut win_y, mut win_w, mut win_h, mon) =
                resolve_box_window_geometry(
                    &st.spec.monitor_id,
                    st.spec.x_dip,
                    st.spec.y_dip,
                    st.spec.w_dip,
                    st.spec.h_dip,
                    st.is_visually_collapsed(),
                );
            st.spec.x_dip = cx;
            st.spec.y_dip = cy;
            st.spec.w_dip = cw;
            st.spec.h_dip = ch;
            st.dpi = mon.dpi;
            st.collapse_animating = false;
            st.edge_animating = false;
            if st.spec.edge_auto_hide {
                let s = (mon.dpi.max(96) as f32) / 96.0;
                let work_w_dip = (mon.work.right - mon.work.left).max(260) as f32 / s;
                st.edge_dock = detect_dock_edge(st.spec.x_dip, st.spec.y_dip, st.spec.w_dip, work_w_dip);
                let slide_t = if st.edge_drawer_open { 1.0 } else { 0.0 };
                st.edge_slide_spring = Spring1D::still(slide_t);
                let (ex, ey, ew, eh) = compute_edge_drawer_window_bounds(
                    st.edge_dock,
                    st.spec.x_dip,
                    st.spec.y_dip,
                    st.spec.w_dip,
                    st.spec.h_dip,
                    mon.work.left,
                    mon.work.top,
                    mon.work.right,
                    mon.work.bottom,
                    mon.dpi,
                    slide_t,
                );
                win_x = ex;
                win_y = ey;
                win_w = ew;
                win_h = eh;
            }
            st.win_x = win_x;
            st.win_y = win_y;
            st.win_w = win_w;
            st.win_h = win_h;
            st.register_peer_rect();
            st.sync_blur_layer();
            let _ = st.render();
            st.reattach_desktop_layer_internal();
        }
    }

    pub fn reattach_desktop_layer(&mut self) {
        if let Ok(mut st) = self.inner.try_borrow_mut() {
            st.reattach_desktop_layer_internal();
        }
    }

    pub fn maintain_desktop_zorder_with_scan(
        &mut self,
        desktop_host: Option<HWND>,
        scan: Option<&DesktopZOrderScan>,
    ) {
        if let Ok(mut st) = self.inner.try_borrow_mut() {
            st.sync_desktop_zorder_with_scan(false, desktop_host, scan);
        }
    }

    pub fn is_animating(&self) -> bool {
        self.inner.try_borrow().is_ok_and(|st| {
            st.collapse_animating
                || st.edge_animating
                || st.scroll_animating
                || st.header_opacity_animating
        })
    }

    pub fn tick_flow(&mut self) {
        if let Ok(mut st) = self.inner.try_borrow_mut() {
            if st.surface_style == GlassSurfaceStyle::LiquidGlass && !st.fullscreen_suppressed {
                if st.update_halo_colors() {
                    st.last_silk_tick = None;
                    st.clear_anim_body_cache();
                }
                st.flow_phase = crate::ui::shared_liquid_flow_phase();
                // Render at the steady ~30 FPS flow timer cadence (matching Taskbar & popups),
                // skipping tucked edge tabs (which do not draw silk) and boxes already driven by
                // interactive animations (which regenerate silk inline on their 8ms timer).
                if !st.is_edge_tucked()
                    && !st.collapse_animating
                    && !st.edge_animating
                    && !st.scroll_animating
                    && !st.header_opacity_animating
                {
                    let _ = st.render();
                }
            }
        }
    }
}

impl Drop for BoxWnd {
    fn drop(&mut self) {
        if let Ok(mut st) = self.inner.try_borrow_mut() {
            st.cleanup_resources();
        }
        unsafe {
            let blur_hwnd = self.inner.try_borrow().map(|s| s.blur_hwnd).unwrap_or_default();
            if !self.hwnd.is_invalid() {
                let _ = DestroyWindow(self.hwnd);
            }
            if !blur_hwnd.is_invalid() {
                let _ = DestroyWindow(blur_hwnd);
            }
        }
    }
}

fn ensure_box_window_classes() -> windows::core::Result<()> {
    let mut err: Option<windows::core::Error> = None;
    BOX_CLASSES_READY.call_once(|| unsafe {
        match GetModuleHandleW(None) {
            Ok(hinst) => {
                let wc_box = WNDCLASSEXW {
                    cbSize: size_of::<WNDCLASSEXW>() as u32,
                    style: CS_DBLCLKS,
                    lpfnWndProc: Some(wndproc::deskbox_wnd_proc),
                    hInstance: hinst.into(),
                    lpszClassName: BOX_CLASS_NAME,
                    ..Default::default()
                };
                RegisterClassExW(&wc_box);

                let wc_blur = WNDCLASSEXW {
                    cbSize: size_of::<WNDCLASSEXW>() as u32,
                    lpfnWndProc: Some(wndproc::deskbox_blur_wnd_proc),
                    hInstance: hinst.into(),
                    lpszClassName: BLUR_CLASS_NAME,
                    ..Default::default()
                };
                RegisterClassExW(&wc_blur);
            }
            Err(e) => err = Some(e),
        }
    });
    match err {
        Some(e) => Err(e),
        None => Ok(()),
    }
}
