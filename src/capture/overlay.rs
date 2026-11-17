use std::sync::Once;

use tracing::{error, info};
use windows::core::w;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, ClientToScreen, EndPaint, FrameRect, GetStockObject, InvalidateRect, BLACK_BRUSH,
    HBRUSH, PAINTSTRUCT, WHITE_BRUSH,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture, VK_ESCAPE};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetForegroundWindow, GetSystemMetrics,
    GetWindowLongPtrW, LoadCursorW, PostMessageW, RegisterClassW, SetForegroundWindow,
    SetLayeredWindowAttributes, SetWindowLongPtrW, CREATESTRUCTW, GWLP_USERDATA, IDC_ARROW,
    LWA_ALPHA, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
    WM_DESTROY, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_NCCREATE, WM_PAINT,
    WM_RBUTTONUP, WNDCLASSW, WS_EX_LAYERED, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP, WS_VISIBLE,
};

use crate::capture::screen::CaptureRect;

static OVERLAY_CLASS_REGISTERED: Once = Once::new();

struct OverlayState {
    tray_hwnd: HWND,
    notify_msg: u32,
    foreground_hwnd: isize,
    hairline: bool,
    dragging: bool,
    start_client: POINT,
    current_client: POINT,
    completed: bool,
}

/// Opens a transparent full-virtual-screen overlay for drag-selecting a region.
/// When finished or cancelled, posts `notify_msg` to `tray_hwnd`:
/// - `WPARAM(1)`, `LPARAM(Box::into_raw(Box::new(CaptureRect)))` on selection
/// - `WPARAM(0)`, `LPARAM(0)` on cancel
pub unsafe fn start_region_selection(
    tray_hwnd: HWND,
    notify_msg: u32,
    selection_visual: &str,
) -> bool {
    let foreground_hwnd = GetForegroundWindow().0 as isize;
    let Ok(hinstance) = GetModuleHandleW(None) else {
        return false;
    };

    let class_name = w!("KnuDiagnosisTransparentOverlayClass");
    OVERLAY_CLASS_REGISTERED.call_once(|| {
        let wc = WNDCLASSW {
            lpfnWndProc: Some(overlay_wnd_proc),
            hInstance: hinstance.into(),
            lpszClassName: class_name,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hbrBackground: HBRUSH(GetStockObject(WHITE_BRUSH).0),
            ..Default::default()
        };
        let _ = RegisterClassW(&wc);
    });

    let vx = GetSystemMetrics(SM_XVIRTUALSCREEN);
    let vy = GetSystemMetrics(SM_YVIRTUALSCREEN);
    let vw = GetSystemMetrics(SM_CXVIRTUALSCREEN).max(1);
    let vh = GetSystemMetrics(SM_CYVIRTUALSCREEN).max(1);

    let hairline = selection_visual.eq_ignore_ascii_case("hairline");
    let state = Box::new(OverlayState {
        tray_hwnd,
        notify_msg,
        foreground_hwnd,
        hairline,
        dragging: false,
        start_client: POINT::default(),
        current_client: POINT::default(),
        completed: false,
    });
    let state_ptr = Box::into_raw(state);

    let Ok(hwnd) = CreateWindowExW(
        WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
        class_name,
        w!(""),
        WS_POPUP | WS_VISIBLE,
        vx,
        vy,
        vw,
        vh,
        None,
        None,
        hinstance,
        Some(state_ptr as *const _),
    ) else {
        let _ = Box::from_raw(state_ptr);
        error!("Failed to create transparent selection overlay");
        return false;
    };

    // alpha = 1 keeps the overlay practically invisible (0.4% opacity) while
    // capturing mouse clicks across the entire virtual screen.
    let alpha = if hairline { 28 } else { 1 };
    let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), alpha, LWA_ALPHA);
    let _ = SetForegroundWindow(hwnd);
    info!(hairline, "Transparent region selection overlay active");
    true
}

fn client_rect_from_points(a: POINT, b: POINT) -> RECT {
    RECT {
        left: a.x.min(b.x),
        top: a.y.min(b.y),
        right: a.x.max(b.x),
        bottom: a.y.max(b.y),
    }
}

fn point_from_lparam(lparam: LPARAM) -> POINT {
    let raw = lparam.0 as u32;
    POINT {
        x: (raw & 0xFFFF) as i16 as i32,
        y: ((raw >> 16) & 0xFFFF) as i16 as i32,
    }
}

unsafe fn cancel_overlay(hwnd: HWND, st: &mut OverlayState) {
    if st.completed {
        return;
    }
    st.completed = true;
    let _ = ReleaseCapture();
    let tray = st.tray_hwnd;
    let msg = st.notify_msg;
    let _ = DestroyWindow(hwnd);
    let _ = PostMessageW(tray, msg, WPARAM(0), LPARAM(0));
}

unsafe extern "system" fn overlay_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_NCCREATE {
        let cs = &*(lparam.0 as *const CREATESTRUCTW);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, cs.lpCreateParams as isize);
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }

    let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut OverlayState;

    match msg {
        WM_LBUTTONDOWN => {
            if !state_ptr.is_null() {
                let st = &mut *state_ptr;
                let pt = point_from_lparam(lparam);
                st.dragging = true;
                st.start_client = pt;
                st.current_client = pt;
                let _ = SetCapture(hwnd);
                if st.hairline {
                    let _ = InvalidateRect(hwnd, None, true);
                }
            }
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            if !state_ptr.is_null() {
                let st = &mut *state_ptr;
                if st.dragging {
                    st.current_client = point_from_lparam(lparam);
                    if st.hairline {
                        let _ = InvalidateRect(hwnd, None, true);
                    }
                }
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            if !state_ptr.is_null() {
                let st = &mut *state_ptr;
                if st.dragging && !st.completed {
                    st.dragging = false;
                    let _ = ReleaseCapture();
                    st.current_client = point_from_lparam(lparam);

                    let mut p1 = st.start_client;
                    let mut p2 = st.current_client;
                    let _ = ClientToScreen(hwnd, &mut p1);
                    let _ = ClientToScreen(hwnd, &mut p2);

                    let left = p1.x.min(p2.x);
                    let top = p1.y.min(p2.y);
                    let width = (p1.x - p2.x).abs();
                    let height = (p1.y - p2.y).abs();

                    if width >= 10 && height >= 10 {
                        st.completed = true;
                        let tray = st.tray_hwnd;
                        let notify_msg = st.notify_msg;
                        let rect = Box::new(CaptureRect {
                            left,
                            top,
                            width,
                            height,
                            foreground_hwnd: st.foreground_hwnd,
                        });
                        let rect_ptr = Box::into_raw(rect);
                        let _ = DestroyWindow(hwnd);
                        let _ = PostMessageW(tray, notify_msg, WPARAM(1), LPARAM(rect_ptr as isize));
                    } else {
                        cancel_overlay(hwnd, st);
                    }
                }
            }
            LRESULT(0)
        }
        WM_RBUTTONUP => {
            if !state_ptr.is_null() {
                cancel_overlay(hwnd, &mut *state_ptr);
            }
            LRESULT(0)
        }
        WM_KEYDOWN => {
            if wparam.0 as u16 == VK_ESCAPE.0 && !state_ptr.is_null() {
                cancel_overlay(hwnd, &mut *state_ptr);
            }
            LRESULT(0)
        }
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);
            if !state_ptr.is_null() {
                let st = &*state_ptr;
                if st.hairline && st.dragging {
                    let r = client_rect_from_points(st.start_client, st.current_client);
                    let brush = HBRUSH(GetStockObject(BLACK_BRUSH).0);
                    let _ = FrameRect(hdc, &r, brush);
                }
            }
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_DESTROY => {
            if !state_ptr.is_null() {
                let st = Box::from_raw(state_ptr);
                if !st.completed {
                    let _ = PostMessageW(st.tray_hwnd, st.notify_msg, WPARAM(0), LPARAM(0));
                }
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
