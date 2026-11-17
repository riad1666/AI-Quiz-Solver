use std::sync::{Arc, Mutex};

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{COLOR_BTNFACE, HBRUSH};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetWindowLongPtrW, RegisterClassW,
    SetForegroundWindow, SetWindowLongPtrW, SetWindowTextW, ShowWindow, CREATESTRUCTW, CW_USEDEFAULT,
    ES_AUTOHSCROLL, ES_AUTOVSCROLL, ES_MULTILINE, ES_READONLY, GWLP_USERDATA, SW_SHOW,
    WINDOW_STYLE, WM_COMMAND, WM_DESTROY, WM_NCCREATE, WNDCLASSW, WS_BORDER, WS_CAPTION, WS_CHILD,
    WS_OVERLAPPED, WS_SYSMENU, WS_TABSTOP, WS_VISIBLE, WS_VSCROLL,
};

use crate::storage::db::HistoryDb;
use crate::ui::dialogs::{create_child, load_app_icon, read_control_text, to_wide_null};

const IDC_HIST_QUERY: isize = 401;
const IDC_HIST_SEARCH_BTN: isize = 402;
const IDC_HIST_BODY: isize = 403;
const IDC_HIST_CLOSE_BTN: isize = 404;

struct HistoryWindowState {
    db: Arc<Mutex<HistoryDb>>,
    hwnd_query: HWND,
    hwnd_body: HWND,
}

static HIST_CLASS_REGISTERED: std::sync::Once = std::sync::Once::new();

pub unsafe fn show_history_window(parent: HWND, db: Arc<Mutex<HistoryDb>>) {
    let Ok(hinstance) = GetModuleHandleW(None) else {
        return;
    };
    let class_name = w!("KnuDiagnosisHistoryClass");

    HIST_CLASS_REGISTERED.call_once(|| {
        let wc = WNDCLASSW {
            lpfnWndProc: Some(history_wnd_proc),
            hInstance: hinstance.into(),
            lpszClassName: class_name,
            hIcon: load_app_icon(false),
            hbrBackground: HBRUSH((COLOR_BTNFACE.0 + 1) as *mut _),
            ..Default::default()
        };
        let _ = RegisterClassW(&wc);
    });

    let state = Box::new(HistoryWindowState {
        db,
        hwnd_query: HWND::default(),
        hwnd_body: HWND::default(),
    });
    let state_ptr = Box::into_raw(state);

    let Ok(hwnd) = CreateWindowExW(
        Default::default(),
        class_name,
        w!("KNU Diagnosis — History"),
        WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU,
        CW_USEDEFAULT,
        CW_USEDEFAULT,
        580,
        440,
        parent,
        None,
        hinstance,
        Some(state_ptr as *const _),
    ) else {
        let _ = Box::from_raw(state_ptr);
        return;
    };

    let hinst: windows::Win32::Foundation::HINSTANCE = hinstance.into();
    let q_hwnd = create_child(
        hwnd,
        hinst,
        w!("EDIT"),
        "",
        WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | WS_BORDER.0 | WS_TABSTOP.0 | ES_AUTOHSCROLL as u32),
        Default::default(),
        14,
        14,
        440,
        24,
        IDC_HIST_QUERY,
    )
    .unwrap_or_default();

    let _ = create_child(
        hwnd,
        hinst,
        w!("BUTTON"),
        "Search",
        WS_CHILD | WS_VISIBLE | WS_TABSTOP,
        Default::default(),
        464,
        13,
        86,
        26,
        IDC_HIST_SEARCH_BTN,
    );

    let body_hwnd = create_child(
        hwnd,
        hinst,
        w!("EDIT"),
        "",
        WINDOW_STYLE(
            WS_CHILD.0
                | WS_VISIBLE.0
                | WS_BORDER.0
                | WS_VSCROLL.0
                | ES_MULTILINE as u32
                | ES_AUTOVSCROLL as u32
                | ES_READONLY as u32,
        ),
        Default::default(),
        14,
        48,
        536,
        304,
        IDC_HIST_BODY,
    )
    .unwrap_or_default();

    let _ = create_child(
        hwnd,
        hinst,
        w!("BUTTON"),
        "Close",
        WS_CHILD | WS_VISIBLE | WS_TABSTOP,
        Default::default(),
        460,
        362,
        90,
        28,
        IDC_HIST_CLOSE_BTN,
    );

    if let Some(st) = (state_ptr as *mut HistoryWindowState).as_mut() {
        st.hwnd_query = q_hwnd;
        st.hwnd_body = body_hwnd;
        refresh_history(st, "");
    }

    let _ = ShowWindow(hwnd, SW_SHOW);
    let _ = SetForegroundWindow(hwnd);
}

unsafe fn refresh_history(st: &HistoryWindowState, query: &str) {
    let text = {
        let guard = st.db.lock().expect("history db mutex poisoned");
        match guard.list_recent(query, 25) {
            Ok(rows) if rows.is_empty() => "No saved Q&A history yet.".to_string(),
            Ok(rows) => rows
                .into_iter()
                .map(|r| {
                    format!(
                        "[{}] ({}) Q: {}\r\nA: {}\r\nStatus: {} | Model: {}\r\n----------------------------------------",
                        r.created_at,
                        r.question_type,
                        r.question_text,
                        r.answer,
                        r.filled_status,
                        r.model_name
                    )
                })
                .collect::<Vec<_>>()
                .join("\r\n"),
            Err(err) => format!("Failed to load history: {err}"),
        }
    };
    let wide = to_wide_null(&text);
    let _ = SetWindowTextW(st.hwnd_body, PCWSTR(wide.as_ptr()));
}

unsafe extern "system" fn history_wnd_proc(
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

    let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut HistoryWindowState;

    match msg {
        WM_COMMAND => {
            if !state_ptr.is_null() {
                let st = &*state_ptr;
                let id = (wparam.0 & 0xFFFF) as isize;
                if id == IDC_HIST_SEARCH_BTN {
                    let q = read_control_text(st.hwnd_query);
                    refresh_history(st, &q);
                } else if id == IDC_HIST_CLOSE_BTN {
                    let _ = DestroyWindow(hwnd);
                }
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            if !state_ptr.is_null() {
                let _ = Box::from_raw(state_ptr);
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
