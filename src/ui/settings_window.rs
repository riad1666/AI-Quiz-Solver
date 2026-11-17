use std::sync::{Arc, Mutex};

use tracing::error;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{COLOR_BTNFACE, HBRUSH};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::{BST_CHECKED, BST_UNCHECKED};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, SetFocus, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallWindowProcW, CreateWindowExW, DefWindowProcW, DestroyWindow, GetWindowLongPtrW,
    IsWindow, MessageBoxW, PostMessageW, RegisterClassW, SendMessageW, SetForegroundWindow,
    SetWindowLongPtrW, SetWindowTextW, ShowWindow, BM_GETCHECK, BM_SETCHECK, BS_AUTOCHECKBOX,
    CB_SETCURSEL, CB_SHOWDROPDOWN, CREATESTRUCTW, CW_USEDEFAULT, ES_AUTOHSCROLL, ES_PASSWORD,
    ES_READONLY, GWLP_USERDATA, GWLP_WNDPROC, MB_ICONERROR, MB_ICONINFORMATION, MB_OK, SW_SHOW,
    WINDOW_STYLE, WM_APP, WM_CHAR, WM_COMMAND, WM_DESTROY, WM_KEYDOWN, WM_NCCREATE, WM_NCDESTROY,
    WM_SYSCHAR, WM_SYSKEYDOWN, WNDCLASSW, WNDPROC, WS_BORDER, WS_CAPTION, WS_CHILD, WS_OVERLAPPED,
    WS_SYSMENU, WS_TABSTOP, WS_VISIBLE, WS_VSCROLL,
};

use crate::capture::hotkey::{format_hotkey, parse_hotkey};
use crate::config::secrets::{load_openrouter_api_key, save_openrouter_api_key};
use crate::config::settings::{
    AppSettings, CUSTOM_MODEL_OPTION, REASONING_MODEL_PRESETS, VISION_MODEL_PRESETS,
};
use crate::ui::dialogs::{create_child, load_app_icon, read_control_text, to_wide_null};

const IDC_SET_API_KEY: isize = 301;
const IDC_SET_VISION_MODEL: isize = 302;
const IDC_SET_REASON_MODEL: isize = 303;
const IDC_SET_HOTKEY: isize = 304;
const IDC_SET_QA_HOTKEY: isize = 305;
const IDC_SET_COPY_HOTKEY: isize = 306;
const IDC_SET_CONFIDENCE: isize = 307;
const IDC_SET_AUTOFILL: isize = 308;
const IDC_SET_HAIRLINE: isize = 309;
const IDC_SET_SAVE: isize = 310;
const IDC_SET_CANCEL: isize = 311;
const IDC_SET_COPY_EXPL_HOTKEY: isize = 312;

const EM_SETSEL: u32 = 0x00B1;
const CBS_DROPDOWN: u32 = 0x0002;
const CBS_AUTOHSCROLL: u32 = 0x0040;
const CB_ADDSTRING: u32 = 0x0143;
const CB_GETCURSEL: u32 = 0x0147;
const CBN_SELCHANGE: usize = 1;
const CBN_SELENDOK: usize = 9;
const WM_CLEAR_CUSTOM_COMBO: u32 = WM_APP + 10;

struct SettingsDialogState {
    tray_hwnd: HWND,
    notify_msg: u32,
    shared_settings: Arc<Mutex<AppSettings>>,
    hwnd_api_key: HWND,
    hwnd_vision: HWND,
    hwnd_reason: HWND,
    hwnd_hotkey: HWND,
    hwnd_qa_hotkey: HWND,
    hwnd_copy_hotkey: HWND,
    hwnd_copy_expl_hotkey: HWND,
    hwnd_conf: HWND,
    hwnd_autofill: HWND,
    hwnd_hairline: HWND,
}

static SETTINGS_CLASS_REGISTERED: std::sync::Once = std::sync::Once::new();

pub unsafe fn show_settings_window(
    tray_hwnd: HWND,
    notify_msg: u32,
    shared_settings: Arc<Mutex<AppSettings>>,
) {
    let Ok(hinstance) = GetModuleHandleW(None) else {
        return;
    };
    let class_name = w!("KnuDiagnosisSettingsClass");

    SETTINGS_CLASS_REGISTERED.call_once(|| {
        let wc = WNDCLASSW {
            lpfnWndProc: Some(settings_wnd_proc),
            hInstance: hinstance.into(),
            lpszClassName: class_name,
            hIcon: load_app_icon(false),
            hbrBackground: HBRUSH((COLOR_BTNFACE.0 + 1) as *mut _),
            ..Default::default()
        };
        let _ = RegisterClassW(&wc);
    });

    let current = shared_settings.lock().expect("settings mutex poisoned").clone();
    let existing_key = load_openrouter_api_key().ok().flatten().unwrap_or_default();

    let state = Box::new(SettingsDialogState {
        tray_hwnd,
        notify_msg,
        shared_settings,
        hwnd_api_key: HWND::default(),
        hwnd_vision: HWND::default(),
        hwnd_reason: HWND::default(),
        hwnd_hotkey: HWND::default(),
        hwnd_qa_hotkey: HWND::default(),
        hwnd_copy_hotkey: HWND::default(),
        hwnd_copy_expl_hotkey: HWND::default(),
        hwnd_conf: HWND::default(),
        hwnd_autofill: HWND::default(),
        hwnd_hairline: HWND::default(),
    });
    let state_ptr = Box::into_raw(state);

    let Ok(hwnd) = CreateWindowExW(
        Default::default(),
        class_name,
        w!("KNU Diagnosis — Settings"),
        WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU,
        CW_USEDEFAULT,
        CW_USEDEFAULT,
        490,
        504,
        tray_hwnd,
        None,
        hinstance,
        Some(state_ptr as *const _),
    ) else {
        let _ = Box::from_raw(state_ptr);
        return;
    };

    let hinst: windows::Win32::Foundation::HINSTANCE = hinstance.into();
    let edit_style =
        WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | WS_BORDER.0 | WS_TABSTOP.0 | ES_AUTOHSCROLL as u32);
    let hotkey_style = WINDOW_STYLE(edit_style.0 | ES_READONLY as u32);
    let pass_style = WINDOW_STYLE(edit_style.0 | ES_PASSWORD as u32);

    let combo_style = WINDOW_STYLE(CBS_DROPDOWN | CBS_AUTOHSCROLL)
        | WS_CHILD
        | WS_VISIBLE
        | WS_TABSTOP
        | WS_VSCROLL;

    let rows: [(&str, &str, isize, WINDOW_STYLE, i32, bool); 8] = [
        ("OpenRouter API Key:", &existing_key, IDC_SET_API_KEY, pass_style, 18, false),
        ("Vision Model:", &current.vision_model, IDC_SET_VISION_MODEL, combo_style, 56, false),
        ("Reasoning Model:", &current.reasoning_model, IDC_SET_REASON_MODEL, combo_style, 94, false),
        ("Capture Hotkey:", &current.hotkey, IDC_SET_HOTKEY, hotkey_style, 132, true),
        ("Show/Hide Q&A Hotkey:", &current.qa_toggle_hotkey, IDC_SET_QA_HOTKEY, hotkey_style, 170, true),
        ("Copy Answers Hotkey:", &current.copy_answers_hotkey, IDC_SET_COPY_HOTKEY, hotkey_style, 208, true),
        ("Copy Explanation Hotkey:", &current.copy_explanations_hotkey, IDC_SET_COPY_EXPL_HOTKEY, hotkey_style, 246, true),
        (
            "Confidence (0.0-1.0):",
            &format!("{:.2}", current.confidence_threshold),
            IDC_SET_CONFIDENCE,
            edit_style,
            300,
            false,
        ),
    ];

    let mut handles = Vec::with_capacity(8);
    for (label, value, id, style, y, is_hotkey_recorder) in rows {
        let _ = create_child(
            hwnd,
            hinst,
            w!("STATIC"),
            label,
            WS_CHILD | WS_VISIBLE,
            Default::default(),
            16,
            y + 3,
            150,
            20,
            0,
        );
        let presets: Option<&[&str]> = match id {
            IDC_SET_VISION_MODEL => Some(VISION_MODEL_PRESETS),
            IDC_SET_REASON_MODEL => Some(REASONING_MODEL_PRESETS),
            _ => None,
        };
        let is_combo = presets.is_some();
        let class_name = if is_combo {
            w!("COMBOBOX")
        } else {
            w!("EDIT")
        };
        let control_h = if is_combo { 220 } else { 24 };
        let h = create_child(
            hwnd,
            hinst,
            class_name,
            value,
            style,
            Default::default(),
            172,
            y,
            286,
            control_h,
            id,
        )
        .unwrap_or_default();
        if let Some(model_presets) = presets {
            if !h.0.is_null() {
                for preset in model_presets {
                    let wide = to_wide_null(preset);
                    SendMessageW(h, CB_ADDSTRING, WPARAM(0), LPARAM(wide.as_ptr() as isize));
                }
                let wide_val = to_wide_null(value);
                let _ = SetWindowTextW(h, PCWSTR(wide_val.as_ptr()));
            }
        }
        if is_hotkey_recorder && !h.0.is_null() {
            install_hotkey_recorder(h);
        }
        handles.push(h);
    }

    let _ = create_child(
        hwnd,
        hinst,
        w!("STATIC"),
        "Tip: Click any Hotkey box above and press a key combo (e.g. Alt+Z).",
        WS_CHILD | WS_VISIBLE,
        Default::default(),
        172,
        274,
        296,
        18,
        0,
    );

    let hwnd_autofill = create_child(
        hwnd,
        hinst,
        w!("BUTTON"),
        "Enable safe automatic answer filling (never auto-submits)",
        WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | BS_AUTOCHECKBOX as u32),
        Default::default(),
        16,
        338,
        430,
        22,
        IDC_SET_AUTOFILL,
    )
    .unwrap_or_default();

    let check_state = if current.auto_fill_enabled {
        BST_CHECKED.0 as usize
    } else {
        BST_UNCHECKED.0 as usize
    };
    let _ = SendMessageW(hwnd_autofill, BM_SETCHECK, WPARAM(check_state), LPARAM(0));

    let hwnd_hairline = create_child(
        hwnd,
        hinst,
        w!("BUTTON"),
        "Show faint outline during region drag (unchecked = completely invisible)",
        WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | BS_AUTOCHECKBOX as u32),
        Default::default(),
        16,
        366,
        442,
        22,
        IDC_SET_HAIRLINE,
    )
    .unwrap_or_default();

    let hairline_state = if current.selection_visual.eq_ignore_ascii_case("hairline") {
        BST_CHECKED.0 as usize
    } else {
        BST_UNCHECKED.0 as usize
    };
    let _ = SendMessageW(hwnd_hairline, BM_SETCHECK, WPARAM(hairline_state), LPARAM(0));

    let _ = create_child(
        hwnd,
        hinst,
        w!("STATIC"),
        "Provider: Kangwon National University",
        WS_CHILD | WS_VISIBLE,
        Default::default(),
        16,
        416,
        244,
        20,
        0,
    );
    let _ = create_child(
        hwnd,
        hinst,
        w!("BUTTON"),
        "Save",
        WS_CHILD | WS_VISIBLE | WS_TABSTOP,
        Default::default(),
        268,
        412,
        90,
        28,
        IDC_SET_SAVE,
    );
    let _ = create_child(
        hwnd,
        hinst,
        w!("BUTTON"),
        "Cancel",
        WS_CHILD | WS_VISIBLE | WS_TABSTOP,
        Default::default(),
        368,
        412,
        90,
        28,
        IDC_SET_CANCEL,
    );

    if let Some(st) = (state_ptr as *mut SettingsDialogState).as_mut() {
        st.hwnd_api_key = handles[0];
        st.hwnd_vision = handles[1];
        st.hwnd_reason = handles[2];
        st.hwnd_hotkey = handles[3];
        st.hwnd_qa_hotkey = handles[4];
        st.hwnd_copy_hotkey = handles[5];
        st.hwnd_copy_expl_hotkey = handles[6];
        st.hwnd_conf = handles[7];
        st.hwnd_autofill = hwnd_autofill;
        st.hwnd_hairline = hwnd_hairline;
    }

    let _ = ShowWindow(hwnd, SW_SHOW);
    let _ = SetForegroundWindow(hwnd);
}

unsafe fn install_hotkey_recorder(edit_hwnd: HWND) {
    let prev = SetWindowLongPtrW(
        edit_hwnd,
        GWLP_WNDPROC,
        hotkey_edit_proc as *const () as usize as isize,
    );
    SetWindowLongPtrW(edit_hwnd, GWLP_USERDATA, prev);
}

unsafe extern "system" fn hotkey_edit_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let prev_raw = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
    let prev_proc: WNDPROC = if prev_raw != 0 {
        Some(std::mem::transmute::<isize, unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT>(prev_raw))
    } else {
        None
    };

    match msg {
        WM_KEYDOWN | WM_SYSKEYDOWN => {
            let vk = (wparam.0 & 0xFFFF) as u32;
            let ctrl = GetKeyState(VK_CONTROL.0 as i32) < 0;
            let alt = GetKeyState(VK_MENU.0 as i32) < 0;
            let shift = GetKeyState(VK_SHIFT.0 as i32) < 0;
            let win = GetKeyState(VK_LWIN.0 as i32) < 0 || GetKeyState(VK_RWIN.0 as i32) < 0;

            if let Some(combo) = format_hotkey(ctrl, alt, shift, win, vk) {
                let wide = to_wide_null(&combo);
                let _ = SetWindowTextW(hwnd, PCWSTR(wide.as_ptr()));
                let _ = SendMessageW(hwnd, EM_SETSEL, WPARAM(combo.len()), LPARAM(combo.len() as isize));
            }
            LRESULT(0)
        }
        WM_CHAR | WM_SYSCHAR => LRESULT(0),
        WM_NCDESTROY => {
            if prev_raw != 0 {
                SetWindowLongPtrW(hwnd, GWLP_WNDPROC, prev_raw);
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            }
            CallWindowProcW(prev_proc, hwnd, msg, wparam, lparam)
        }
        _ => CallWindowProcW(prev_proc, hwnd, msg, wparam, lparam),
    }
}

unsafe extern "system" fn settings_wnd_proc(
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

    let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut SettingsDialogState;

    match msg {
        WM_COMMAND => {
            if !state_ptr.is_null() {
                let st = &*state_ptr;
                let id = (wparam.0 & 0xFFFF) as isize;
                let notify = (wparam.0 >> 16) & 0xFFFF;
                if id == IDC_SET_VISION_MODEL
                    && (notify == CBN_SELCHANGE || notify == CBN_SELENDOK)
                {
                    let sel = SendMessageW(st.hwnd_vision, CB_GETCURSEL, WPARAM(0), LPARAM(0)).0;
                    if sel == (VISION_MODEL_PRESETS.len() as isize - 1) {
                        let _ = PostMessageW(
                            hwnd,
                            WM_CLEAR_CUSTOM_COMBO,
                            WPARAM(id as usize),
                            LPARAM(st.hwnd_vision.0 as isize),
                        );
                    }
                    return LRESULT(0);
                }
                if id == IDC_SET_REASON_MODEL
                    && (notify == CBN_SELCHANGE || notify == CBN_SELENDOK)
                {
                    let sel = SendMessageW(st.hwnd_reason, CB_GETCURSEL, WPARAM(0), LPARAM(0)).0;
                    if sel == (REASONING_MODEL_PRESETS.len() as isize - 1) {
                        let _ = PostMessageW(
                            hwnd,
                            WM_CLEAR_CUSTOM_COMBO,
                            WPARAM(id as usize),
                            LPARAM(st.hwnd_reason.0 as isize),
                        );
                    }
                    return LRESULT(0);
                }
                if id == IDC_SET_SAVE {
                    let api_key = read_control_text(st.hwnd_api_key);
                    let vision = read_control_text(st.hwnd_vision);
                    let trimmed_vision = vision.trim();
                    if trimmed_vision.is_empty() || trimmed_vision == CUSTOM_MODEL_OPTION {
                        let _ = MessageBoxW(
                            hwnd,
                            w!("Please select a preset Vision Model or enter a custom OpenRouter model ID."),
                            w!("Invalid Vision Model"),
                            MB_OK | MB_ICONERROR,
                        );
                        return LRESULT(0);
                    }
                    let reason = read_control_text(st.hwnd_reason);
                    let trimmed_reason = reason.trim();
                    if trimmed_reason.is_empty() || trimmed_reason == CUSTOM_MODEL_OPTION {
                        let _ = MessageBoxW(
                            hwnd,
                            w!("Please select a preset Reasoning Model or enter a custom OpenRouter model ID."),
                            w!("Invalid Reasoning Model"),
                            MB_OK | MB_ICONERROR,
                        );
                        return LRESULT(0);
                    }
                    let hotkey = read_control_text(st.hwnd_hotkey);
                    let qa_hotkey = read_control_text(st.hwnd_qa_hotkey);
                    let copy_hotkey = read_control_text(st.hwnd_copy_hotkey);
                    let copy_expl_hotkey = read_control_text(st.hwnd_copy_expl_hotkey);
                    let conf_str = read_control_text(st.hwnd_conf);

                    for (label, spec) in [
                        ("capture hotkey", &hotkey),
                        ("show/hide Q&A hotkey", &qa_hotkey),
                        ("copy answers hotkey", &copy_hotkey),
                        ("copy explanation hotkey", &copy_expl_hotkey),
                    ] {
                        if let Err(err) = parse_hotkey(spec) {
                            let msg = to_wide_null(&format!("Invalid {label} '{spec}': {err}"));
                            let _ = MessageBoxW(
                                hwnd,
                                PCWSTR(msg.as_ptr()),
                                w!("Invalid Hotkey"),
                                MB_OK | MB_ICONERROR,
                            );
                            return LRESULT(0);
                        }
                    }

                    let conf: f32 = match conf_str.trim().parse::<f32>() {
                        Ok(v) if (0.0..=1.0).contains(&v) => v,
                        _ => {
                            let _ = MessageBoxW(
                                hwnd,
                                w!("Confidence threshold must be between 0.0 and 1.0."),
                                w!("Invalid Confidence"),
                                MB_OK | MB_ICONERROR,
                            );
                            return LRESULT(0);
                        }
                    };

                    if !api_key.trim().is_empty() {
                        if let Err(err) = save_openrouter_api_key(api_key.trim()) {
                            error!("Failed to save OpenRouter API key: {err}");
                        }
                    }

                    let autofill =
                        SendMessageW(st.hwnd_autofill, BM_GETCHECK, WPARAM(0), LPARAM(0)).0
                            == BST_CHECKED.0 as isize;
                    let hairline =
                        SendMessageW(st.hwnd_hairline, BM_GETCHECK, WPARAM(0), LPARAM(0)).0
                            == BST_CHECKED.0 as isize;

                    {
                        let mut guard = st
                            .shared_settings
                            .lock()
                            .expect("settings mutex poisoned");
                        guard.vision_model = trimmed_vision.to_string();
                        guard.reasoning_model = trimmed_reason.to_string();
                        guard.hotkey = hotkey.trim().to_string();
                        guard.qa_toggle_hotkey = qa_hotkey.trim().to_string();
                        guard.copy_answers_hotkey = copy_hotkey.trim().to_string();
                        guard.copy_explanations_hotkey = copy_expl_hotkey.trim().to_string();
                        guard.confidence_threshold = conf;
                        guard.auto_fill_enabled = autofill;
                        guard.selection_visual = if hairline {
                            "hairline".to_string()
                        } else {
                            "none".to_string()
                        };

                        if let Ok(path) = AppSettings::default_path() {
                            if let Err(err) = guard.save(&path) {
                                error!("Failed to persist settings: {err}");
                            }
                        }
                    }

                    let _ = PostMessageW(st.tray_hwnd, st.notify_msg, WPARAM(0), LPARAM(0));
                    let _ = MessageBoxW(
                        hwnd,
                        w!("Settings saved."),
                        w!("KNU Diagnosis"),
                        MB_OK | MB_ICONINFORMATION,
                    );
                    let _ = DestroyWindow(hwnd);
                } else if id == IDC_SET_CANCEL {
                    let _ = DestroyWindow(hwnd);
                }
            }
            LRESULT(0)
        }
        WM_CLEAR_CUSTOM_COMBO => {
            if !state_ptr.is_null() {
                let st = &*state_ptr;
                let id = wparam.0 as isize;
                let expected_combo = match id {
                    IDC_SET_VISION_MODEL => st.hwnd_vision,
                    IDC_SET_REASON_MODEL => st.hwnd_reason,
                    _ => HWND::default(),
                };
                if !expected_combo.0.is_null()
                    && expected_combo.0 as isize == lparam.0
                    && IsWindow(expected_combo).as_bool()
                {
                    let _ = SendMessageW(expected_combo, CB_SHOWDROPDOWN, WPARAM(0), LPARAM(0));
                    let _ = SendMessageW(
                        expected_combo,
                        CB_SETCURSEL,
                        WPARAM(usize::MAX),
                        LPARAM(0),
                    );
                    let _ = SetWindowTextW(expected_combo, w!(""));
                    let _ = SetFocus(expected_combo);
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

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::UI::Input::KeyboardAndMouse::{VK_DOWN, VK_RETURN};
    use windows::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE,
    };

    struct TempWindowGuard(HWND);

    impl Drop for TempWindowGuard {
        fn drop(&mut self) {
            unsafe {
                if !self.0 .0.is_null() && IsWindow(self.0).as_bool() {
                    let _ = DestroyWindow(self.0);
                }
            }
        }
    }

    unsafe fn pump_thread_messages() {
        let mut msg = MSG::default();
        while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    unsafe fn create_test_settings_window(
        class_name: PCWSTR,
        hinstance: windows::Win32::Foundation::HINSTANCE,
    ) -> (TempWindowGuard, HWND, HWND) {
        let wc = WNDCLASSW {
            lpfnWndProc: Some(settings_wnd_proc),
            hInstance: hinstance,
            lpszClassName: class_name,
            ..Default::default()
        };
        let _ = RegisterClassW(&wc);

        let state = Box::new(SettingsDialogState {
            tray_hwnd: HWND::default(),
            notify_msg: 0,
            shared_settings: Arc::new(Mutex::new(AppSettings::default())),
            hwnd_api_key: HWND::default(),
            hwnd_vision: HWND::default(),
            hwnd_reason: HWND::default(),
            hwnd_hotkey: HWND::default(),
            hwnd_qa_hotkey: HWND::default(),
            hwnd_copy_hotkey: HWND::default(),
            hwnd_copy_expl_hotkey: HWND::default(),
            hwnd_conf: HWND::default(),
            hwnd_autofill: HWND::default(),
            hwnd_hairline: HWND::default(),
        });
        let state_ptr = Box::into_raw(state);

        let hwnd = match CreateWindowExW(
            Default::default(),
            class_name,
            w!("Test Settings Combo"),
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU,
            -32000,
            -32000,
            490,
            504,
            None,
            None,
            hinstance,
            Some(state_ptr as *const _),
        ) {
            Ok(h) => h,
            Err(err) => {
                let _ = Box::from_raw(state_ptr);
                panic!("create test settings window failed: {err}");
            }
        };

        let guard = TempWindowGuard(hwnd);
        let combo_style = WINDOW_STYLE(CBS_DROPDOWN | CBS_AUTOHSCROLL)
            | WS_CHILD
            | WS_VISIBLE
            | WS_TABSTOP
            | WS_VSCROLL;

        let hwnd_vision = create_child(
            hwnd,
            hinstance,
            w!("COMBOBOX"),
            VISION_MODEL_PRESETS[0],
            combo_style,
            Default::default(),
            172,
            56,
            286,
            220,
            IDC_SET_VISION_MODEL,
        )
        .expect("create vision combobox");
        for preset in VISION_MODEL_PRESETS {
            let wide = to_wide_null(preset);
            SendMessageW(
                hwnd_vision,
                CB_ADDSTRING,
                WPARAM(0),
                LPARAM(wide.as_ptr() as isize),
            );
        }

        let hwnd_reason = create_child(
            hwnd,
            hinstance,
            w!("COMBOBOX"),
            REASONING_MODEL_PRESETS[0],
            combo_style,
            Default::default(),
            172,
            94,
            286,
            220,
            IDC_SET_REASON_MODEL,
        )
        .expect("create reason combobox");
        for preset in REASONING_MODEL_PRESETS {
            let wide = to_wide_null(preset);
            SendMessageW(
                hwnd_reason,
                CB_ADDSTRING,
                WPARAM(0),
                LPARAM(wide.as_ptr() as isize),
            );
        }

        if let Some(st) = state_ptr.as_mut() {
            st.hwnd_vision = hwnd_vision;
            st.hwnd_reason = hwnd_reason;
        }

        (guard, hwnd_vision, hwnd_reason)
    }

    #[test]
    fn custom_combo_selection_clears_edit_text_in_closed_and_open_dropdowns() {
        unsafe {
            let hmodule = GetModuleHandleW(None).expect("GetModuleHandleW");
            let hinst: windows::Win32::Foundation::HINSTANCE = hmodule.into();
            let class_name = w!("KnuDiagnosisSettingsComboTestClass");
            let (guard, hwnd_vision, hwnd_reason) =
                create_test_settings_window(class_name, hinst);

            for (combo, presets) in [
                (hwnd_vision, VISION_MODEL_PRESETS),
                (hwnd_reason, REASONING_MODEL_PRESETS),
            ] {
                let prev_idx = presets.len() - 2;
                assert_ne!(presets[prev_idx], CUSTOM_MODEL_OPTION);
                assert_eq!(presets[presets.len() - 1], CUSTOM_MODEL_OPTION);

                // Case A: Closed combobox VK_DOWN onto "Custom..."
                SendMessageW(combo, CB_SETCURSEL, WPARAM(prev_idx), LPARAM(0));
                SendMessageW(combo, WM_KEYDOWN, WPARAM(VK_DOWN.0 as usize), LPARAM(0));
                pump_thread_messages();
                assert_eq!(
                    read_control_text(combo),
                    "",
                    "closed combobox VK_DOWN to Custom... must clear edit text"
                );
                assert_eq!(
                    SendMessageW(combo, CB_GETCURSEL, WPARAM(0), LPARAM(0)).0,
                    -1,
                    "closed combobox selection must reset to CB_ERR (-1)"
                );

                // Case B1: Open dropdown CB_SHOWDROPDOWN(1) + VK_DOWN + VK_RETURN in one batch
                SendMessageW(combo, CB_SETCURSEL, WPARAM(prev_idx), LPARAM(0));
                SendMessageW(combo, CB_SHOWDROPDOWN, WPARAM(1), LPARAM(0));
                SendMessageW(combo, WM_KEYDOWN, WPARAM(VK_DOWN.0 as usize), LPARAM(0));
                SendMessageW(combo, WM_KEYDOWN, WPARAM(VK_RETURN.0 as usize), LPARAM(0));
                pump_thread_messages();
                assert_eq!(
                    read_control_text(combo),
                    "",
                    "open dropdown VK_DOWN + VK_RETURN to Custom... must clear edit text"
                );
                assert_eq!(
                    SendMessageW(combo, CB_GETCURSEL, WPARAM(0), LPARAM(0)).0,
                    -1,
                    "open dropdown selection must reset to CB_ERR (-1)"
                );

                // Case B2: Open dropdown CB_SHOWDROPDOWN(1) + VK_DOWN, pump, then VK_RETURN, pump
                SendMessageW(combo, CB_SETCURSEL, WPARAM(prev_idx), LPARAM(0));
                SendMessageW(combo, CB_SHOWDROPDOWN, WPARAM(1), LPARAM(0));
                SendMessageW(combo, WM_KEYDOWN, WPARAM(VK_DOWN.0 as usize), LPARAM(0));
                pump_thread_messages();
                SendMessageW(combo, WM_KEYDOWN, WPARAM(VK_RETURN.0 as usize), LPARAM(0));
                pump_thread_messages();
                assert_eq!(
                    read_control_text(combo),
                    "",
                    "open dropdown stepped VK_DOWN then VK_RETURN must keep edit text empty"
                );
                assert_eq!(
                    SendMessageW(combo, CB_GETCURSEL, WPARAM(0), LPARAM(0)).0,
                    -1
                );

                // Case C: Normal preset selection must NOT be cleared
                SendMessageW(combo, CB_SETCURSEL, WPARAM(0), LPARAM(0));
                SendMessageW(combo, WM_KEYDOWN, WPARAM(VK_DOWN.0 as usize), LPARAM(0));
                pump_thread_messages();
                assert_eq!(read_control_text(combo), presets[1]);
                assert_eq!(
                    SendMessageW(combo, CB_GETCURSEL, WPARAM(0), LPARAM(0)).0,
                    1
                );
            }

            // Case D: Stale WM_CLEAR_CUSTOM_COMBO posted right before window destruction is safe
            let hwnd_parent = guard.0;
            let _ = PostMessageW(
                hwnd_parent,
                WM_CLEAR_CUSTOM_COMBO,
                WPARAM(IDC_SET_VISION_MODEL as usize),
                LPARAM(hwnd_vision.0 as isize),
            );
            drop(guard);
            pump_thread_messages();
        }
    }
}
