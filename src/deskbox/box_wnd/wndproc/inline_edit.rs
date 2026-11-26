//! Hidden Win32 EDIT proxy subclassing for inline title rename and inline search, plus cursor hit-test.

use super::super::{BoxInner, InlineEditMode};
use super::{
    schedule_hover_collapse_check, DLGC_WANTALLKEYS_VAL, INLINE_EDIT_CARET_BLINK_MS,
    INLINE_EDIT_CARET_TIMER_ID,
};
use crate::deskbox::config::BoxKind;
use crate::deskbox::layout::{BoxHitZone, DeskBoxLayout, DeskBoxLayoutParams, ResizeEdge};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    ClientToScreen, CreateFontW, DeleteObject, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS,
    DEFAULT_CHARSET, DEFAULT_PITCH, FF_DONTCARE, FW_SEMIBOLD, HFONT, OUT_DEFAULT_PRECIS,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::{EM_SETMARGINS, EM_SETSEL};
use windows::Win32::UI::Input::KeyboardAndMouse::{SetFocus, VK_ESCAPE, VK_RETURN};
use windows::Win32::UI::WindowsAndMessaging::{
    CallWindowProcW, CreateWindowExW, DefWindowProcW, DestroyWindow, GetWindowLongPtrW,
    GetWindowTextLengthW, GetWindowTextW, IsWindow, KillTimer, SendMessageW,
    SetLayeredWindowAttributes, SetTimer, SetWindowLongPtrW, SetWindowPos, ShowWindow,
    EC_LEFTMARGIN, EC_RIGHTMARGIN, ES_AUTOHSCROLL, ES_LEFT, GWLP_USERDATA, GWLP_WNDPROC,
    HWND_TOPMOST, LWA_ALPHA, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SWP_SHOWWINDOW, SW_SHOW,
    WM_CHAR, WM_CLEAR, WM_CUT, WM_GETDLGCODE, WM_IME_COMPOSITION, WM_IME_ENDCOMPOSITION,
    WM_KEYDOWN, WM_KEYUP, WM_KILLFOCUS, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_MOUSEMOVE,
    WM_NCDESTROY, WM_PASTE, WM_SETFONT, WM_TIMER, WM_UNDO, WS_EX_LAYERED, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_POPUP,
};

impl BoxInner {
    pub(super) fn cursor_for_point(&self, x: f32, y: f32) -> windows::Win32::UI::WindowsAndMessaging::HCURSOR {
        if self.is_edge_tucked() {
            return self.hand_cursor;
        }
        let layout = self.current_layout();
        match layout.hit_test(x, y) {
            BoxHitZone::Resize(ResizeEdge::Left | ResizeEdge::Right) => self.size_we_cursor,
            BoxHitZone::Resize(ResizeEdge::Top | ResizeEdge::Bottom) => self.size_ns_cursor,
            BoxHitZone::Resize(ResizeEdge::TopLeft | ResizeEdge::BottomRight) => {
                self.size_nwse_cursor
            }
            BoxHitZone::Resize(ResizeEdge::TopRight | ResizeEdge::BottomLeft) => {
                self.size_nesw_cursor
            }
            BoxHitZone::BackButton
            | BoxHitZone::SearchClearButton
            | BoxHitZone::NewButton
            | BoxHitZone::SortButton
            | BoxHitZone::ViewButton
            | BoxHitZone::LockButton
            | BoxHitZone::CollapseButton
            | BoxHitZone::DeleteButton
            | BoxHitZone::Item(_)
            | BoxHitZone::ItemRemove(_) => self.hand_cursor,
            _ => self.arrow_cursor,
        }
    }

    pub(in super::super) fn cancel_inline_rename(&mut self) {
        self.inline_edit_caret_visible = false;
        self.inline_edit_scroll_x = 0.0;
        self.inline_edit_mode = None;
        if !self.inline_edit_hwnd.is_invalid() {
            self.last_inline_edit_closed_at = Some(Instant::now());
            let eh = self.inline_edit_hwnd;
            self.inline_edit_hwnd = HWND::default();
            unsafe {
                let _ = KillTimer(eh, INLINE_EDIT_CARET_TIMER_ID);
                if self.inline_edit_orig_proc != 0 {
                    let _ = SetWindowLongPtrW(eh, GWLP_WNDPROC, self.inline_edit_orig_proc);
                    self.inline_edit_orig_proc = 0;
                }
                let raw_data = SetWindowLongPtrW(eh, GWLP_USERDATA, 0) as *mut EditSubclassData;
                if !raw_data.is_null() {
                    drop(Box::from_raw(raw_data));
                }
                let _ = DestroyWindow(eh);
            }
        }
        if !self.inline_edit_font.is_invalid() {
            unsafe {
                let _ = DeleteObject(self.inline_edit_font);
            }
            self.inline_edit_font = HFONT::default();
        }
    }
}

pub(super) unsafe fn read_edit_text(eh: HWND) -> String {
    if eh.is_invalid() {
        return String::new();
    }
    let len = GetWindowTextLengthW(eh);
    if len <= 0 {
        return String::new();
    }
    let mut buf = vec![0u16; (len as usize) + 1];
    let copied = GetWindowTextW(eh, &mut buf);
    if copied > 0 {
        String::from_utf16_lossy(&buf[..copied as usize])
    } else {
        String::new()
    }
}

struct EditSubclassData {
    box_inner: Rc<RefCell<BoxInner>>,
    orig_proc: isize,
}

unsafe extern "system" fn inline_edit_subclass_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_NCDESTROY {
        let _ = KillTimer(hwnd, INLINE_EDIT_CARET_TIMER_ID);
        let raw = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) as *mut EditSubclassData;
        if !raw.is_null() {
            let data = Box::from_raw(raw);
            let _ = SetWindowLongPtrW(hwnd, GWLP_WNDPROC, data.orig_proc);
            let orig: unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT =
                std::mem::transmute(data.orig_proc);
            return CallWindowProcW(Some(orig), hwnd, msg, wparam, lparam);
        }
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }

    let raw = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const EditSubclassData;
    if raw.is_null() {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    let rc = Rc::clone(&(*raw).box_inner);
    let orig: unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT =
        std::mem::transmute((*raw).orig_proc);

    match msg {
        WM_GETDLGCODE => return LRESULT(DLGC_WANTALLKEYS_VAL),
        WM_CHAR if wparam.0 == VK_RETURN.0 as usize || wparam.0 == VK_ESCAPE.0 as usize => {
            return LRESULT(0);
        }
        WM_KEYDOWN => {
            let vk = wparam.0 as u16;
            if vk == VK_RETURN.0 {
                finish_inline_edit(&rc, true);
                return LRESULT(0);
            } else if vk == VK_ESCAPE.0 {
                finish_inline_edit(&rc, false);
                return LRESULT(0);
            }
        }
        WM_KILLFOCUS => {
            finish_inline_edit(&rc, true);
            return LRESULT(0);
        }
        WM_TIMER if wparam.0 == INLINE_EDIT_CARET_TIMER_ID => {
            if let Ok(mut st) = rc.try_borrow_mut() {
                if !st.inline_edit_hwnd.is_invalid() {
                    st.inline_edit_caret_visible = !st.inline_edit_caret_visible;
                    let _ = st.render();
                }
            }
            return LRESULT(0);
        }
        _ => {}
    }

    let res = CallWindowProcW(Some(orig), hwnd, msg, wparam, lparam);

    let should_sync = match msg {
        WM_KEYDOWN
        | WM_KEYUP
        | WM_LBUTTONDOWN
        | WM_LBUTTONUP
        | WM_LBUTTONDBLCLK
        | WM_IME_COMPOSITION
        | WM_IME_ENDCOMPOSITION
        | EM_SETSEL => true,
        WM_MOUSEMOVE => (wparam.0 & 1) != 0,
        WM_CHAR | WM_PASTE | WM_CUT | WM_CLEAR | WM_UNDO => {
            SetTimer(hwnd, INLINE_EDIT_CARET_TIMER_ID, INLINE_EDIT_CARET_BLINK_MS, None);
            false
        }
        _ => false,
    };

    if should_sync {
        SetTimer(hwnd, INLINE_EDIT_CARET_TIMER_ID, INLINE_EDIT_CARET_BLINK_MS, None);
        if let Ok(mut st) = rc.try_borrow_mut() {
            if !st.inline_edit_hwnd.is_invalid() {
                st.inline_edit_caret_visible = true;
                let _ = st.render();
            }
        }
    }

    res
}

pub(super) fn start_inline_edit(rc: &Rc<RefCell<BoxInner>>, mode: InlineEditMode, seed_char: Option<char>) {
    let setup = {
        let Ok(mut st) = rc.try_borrow_mut() else {
            return;
        };
        if mode == InlineEditMode::Rename && st.spec.kind == BoxKind::FolderPortal {
            return;
        }
        st.cancel_inline_rename();
        if mode == InlineEditMode::Search {
            if st.spec.collapsed && !st.hover_expanded && !st.spec.edge_auto_hide {
                st.set_hover_expanded(true);
            }
            st.search_active = true;
            if let Some(ch) = seed_char {
                st.search_query.push(ch);
                st.apply_search_filter();
            }
        }
        let menu_rc = Rc::clone(&st.box_menu);
        drop(st);
        menu_rc.hide();
        let Ok(st) = rc.try_borrow() else {
            return;
        };
        let layout = DeskBoxLayout::compute(DeskBoxLayoutParams {
            width_dip: st.spec.w_dip,
            height_dip: st.spec.h_dip,
            dpi: st.dpi,
            item_count: st.items.len(),
            view_mode: st.spec.view_mode,
            scroll_y: st.scroll_y,
            can_go_back: st.can_go_back(),
            show_item_remove: st.spec.kind == BoxKind::Collection,
            collapsed: st.is_visually_collapsed(),
            show_header_actions: false,
            locked: st.spec.locked,
            auto_hide_header: st.spec.auto_hide_header,
            header_visible: true,
            search_active: mode == InlineEditMode::Search,
            card_height_override_px: st
                .collapse_animating
                .then_some(st.collapse_height_spring.pos),
        });
        let s = layout.scale;
        let pill = BoxInner::inline_edit_pill_rect(&layout, mode);
        let (edit_left, edit_right) = match mode {
            InlineEditMode::Rename => (pill.left, pill.right),
            InlineEditMode::Search => {
                let left = pill.left + 21.0 * s;
                let right = if layout.search_clear_btn_rect.width() > 1.0 {
                    (layout.search_clear_btn_rect.left - 2.0 * s).max(left + 24.0 * s)
                } else {
                    (pill.right - 6.0 * s).max(left + 24.0 * s)
                };
                (left, right)
            }
        };
        let mut pt0 = POINT {
            x: edit_left.round() as i32,
            y: pill.top.round() as i32,
        };
        let _ = unsafe { ClientToScreen(st.hwnd, &mut pt0) };
        let w = ((edit_right - edit_left).round() as i32).max(24);
        let h = (pill.height().round() as i32).max(20);
        let initial_text = match mode {
            InlineEditMode::Rename => st.spec.title.clone(),
            InlineEditMode::Search => st.search_query.clone(),
        };
        (
            st.hwnd,
            initial_text,
            pt0,
            w,
            h,
            s,
            st.lifted_topmost,
            seed_char.is_some(),
        )
    };

    let (box_hwnd, initial_text, pt0, w, h, s, lifted_topmost, caret_at_end) = setup;
    unsafe {
        let Ok(hinst) = GetModuleHandleW(None) else {
            return;
        };
        let wide_text: Vec<u16> = initial_text.encode_utf16().chain(Some(0)).collect();
        let mut ex_style = WS_EX_LAYERED | WS_EX_TOOLWINDOW;
        if lifted_topmost {
            ex_style |= WS_EX_TOPMOST;
        }
        let Ok(edit_hwnd) = CreateWindowExW(
            ex_style,
            w!("EDIT"),
            PCWSTR(wide_text.as_ptr()),
            WS_POPUP
                | windows::Win32::UI::WindowsAndMessaging::WINDOW_STYLE(
                    (ES_AUTOHSCROLL | ES_LEFT) as u32,
                ),
            pt0.x,
            pt0.y,
            w,
            h,
            box_hwnd,
            None,
            hinst,
            None,
        ) else {
            return;
        };

        // Invisible native proxy (alpha = 1/255 keeps mouse hit-testing & IME active while D2D renders the UI).
        let _ = SetLayeredWindowAttributes(edit_hwnd, COLORREF(0), 1, LWA_ALPHA);

        let orig_proc = SetWindowLongPtrW(
            edit_hwnd,
            GWLP_WNDPROC,
            inline_edit_subclass_proc as *const () as isize,
        );
        let data = Box::new(EditSubclassData {
            box_inner: Rc::clone(rc),
            orig_proc,
        });
        SetWindowLongPtrW(edit_hwnd, GWLP_USERDATA, Box::into_raw(data) as isize);

        let font_h = -(13.5 * s).round() as i32;
        let edit_font = CreateFontW(
            font_h,
            0,
            0,
            0,
            FW_SEMIBOLD.0 as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET.0 as u32,
            OUT_DEFAULT_PRECIS.0 as u32,
            CLIP_DEFAULT_PRECIS.0 as u32,
            CLEARTYPE_QUALITY.0 as u32,
            (DEFAULT_PITCH.0 as u32) | (FF_DONTCARE.0 as u32),
            w!("Segoe UI Variable Text"),
        );
        if !edit_font.is_invalid() {
            let _ = SendMessageW(edit_hwnd, WM_SETFONT, WPARAM(edit_font.0 as usize), LPARAM(0));
        }

        let pad_px = if mode == InlineEditMode::Rename {
            (7.0 * s).round().clamp(1.0, 24.0) as usize
        } else {
            1usize
        };
        let _ = SendMessageW(
            edit_hwnd,
            EM_SETMARGINS,
            WPARAM((EC_LEFTMARGIN | EC_RIGHTMARGIN) as usize),
            LPARAM(((pad_px << 16) | pad_px) as isize),
        );

        // Attach `inline_edit_hwnd` into `BoxInner` BEFORE `ShowWindow`/`SetFocus` so synchronous
        // `WM_ACTIVATE(WA_INACTIVE)` / `WM_KILLFOCUS` recognizes the child edit window and any
        // immediate focus loss cleanly tears down `edit_hwnd` via `finish_inline_edit`.
        let mut attached = false;
        if let Ok(mut st) = rc.try_borrow_mut() {
            if !st.cleaned_up && IsWindow(edit_hwnd).as_bool() {
                st.inline_edit_hwnd = edit_hwnd;
                st.inline_edit_mode = Some(mode);
                st.inline_edit_orig_proc = orig_proc;
                st.inline_edit_font = edit_font;
                st.inline_edit_caret_visible = true;
                st.inline_edit_scroll_x = 0.0;
                st.sync_header_opacity_target();
                attached = true;
            }
        }
        if !attached {
            if IsWindow(edit_hwnd).as_bool() {
                if orig_proc != 0 {
                    let _ = SetWindowLongPtrW(edit_hwnd, GWLP_WNDPROC, orig_proc);
                }
                let raw_data =
                    SetWindowLongPtrW(edit_hwnd, GWLP_USERDATA, 0) as *mut EditSubclassData;
                if !raw_data.is_null() {
                    drop(Box::from_raw(raw_data));
                }
                let _ = DestroyWindow(edit_hwnd);
            }
            if !edit_font.is_invalid() {
                let _ = DeleteObject(edit_font);
            }
            return;
        }

        if lifted_topmost {
            let _ = SetWindowPos(
                edit_hwnd,
                HWND_TOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW,
            );
        } else {
            let _ = SetWindowPos(
                edit_hwnd,
                None,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_SHOWWINDOW,
            );
        }
        let _ = ShowWindow(edit_hwnd, SW_SHOW);
        let _ = SetFocus(edit_hwnd);
        let still_attached = rc
            .try_borrow()
            .is_ok_and(|st| st.inline_edit_hwnd == edit_hwnd && IsWindow(edit_hwnd).as_bool());
        if still_attached {
            if caret_at_end {
                let utf16_len = wide_text.len().saturating_sub(1);
                let _ = SendMessageW(
                    edit_hwnd,
                    EM_SETSEL,
                    WPARAM(utf16_len),
                    LPARAM(utf16_len as isize),
                );
            } else {
                let _ = SendMessageW(edit_hwnd, EM_SETSEL, WPARAM(0), LPARAM(-1));
            }
            SetTimer(
                edit_hwnd,
                INLINE_EDIT_CARET_TIMER_ID,
                INLINE_EDIT_CARET_BLINK_MS,
                None,
            );
            if let Ok(mut st) = rc.try_borrow_mut() {
                if st.inline_edit_hwnd == edit_hwnd {
                    let _ = st.render();
                }
            }
        }
    }
}

pub(super) fn finish_inline_edit(rc: &Rc<RefCell<BoxInner>>, commit: bool) {
    let snapshot = {
        let Ok(st) = rc.try_borrow() else {
            return;
        };
        let eh = st.inline_edit_hwnd;
        let Some(mode) = st.inline_edit_mode else {
            return;
        };
        if eh.is_invalid() {
            return;
        }
        let raw_text = unsafe { read_edit_text(eh) };
        (mode, raw_text)
    };

    let (mode, raw_text) = snapshot;
    if let Ok(mut st) = rc.try_borrow_mut() {
        st.cancel_inline_rename();
        match mode {
            InlineEditMode::Rename => {
                if commit && st.spec.kind != BoxKind::FolderPortal {
                    let trimmed = raw_text.trim().to_string();
                    if !trimmed.is_empty() && st.spec.title != trimmed {
                        st.spec.title = trimmed;
                        st.persist_to_host();
                    }
                }
            }
            InlineEditMode::Search => {
                if !commit || raw_text.trim().is_empty() {
                    st.search_active = false;
                    st.search_query.clear();
                    st.apply_search_filter();
                } else {
                    st.search_active = true;
                    st.search_query = raw_text;
                    st.apply_search_filter();
                }
            }
        }
        st.sync_header_opacity_target();
        if (st.spec.collapsed && st.hover_expanded)
            || (st.spec.edge_auto_hide && st.edge_drawer_open)
        {
            unsafe {
                schedule_hover_collapse_check(st.hwnd);
            }
        }
        let _ = st.render();
    }
}

