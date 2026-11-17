use std::sync::{Arc, Mutex};

use thiserror::Error;
use tracing::{error, info};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{RegisterHotKey, UnregisterHotKey};
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIIF_INFO, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE,
    NIM_MODIFY, NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow,
    DispatchMessageW, GetCursorPos, GetMessageW, PostMessageW, PostQuitMessage, RegisterClassW,
    SetForegroundWindow, TrackPopupMenu, TranslateMessage, CW_USEDEFAULT, MF_SEPARATOR, MF_STRING,
    MSG, TPM_BOTTOMALIGN, TPM_LEFTALIGN, WM_APP, WM_COMMAND, WM_DESTROY, WM_HOTKEY, WM_LBUTTONUP,
    WM_RBUTTONUP, WNDCLASSW, WS_OVERLAPPEDWINDOW,
};

use crate::ai::openrouter::OpenRouterClient;
use crate::capture::hotkey::parse_hotkey;
use crate::capture::overlay::start_region_selection;
use crate::capture::screen::{capture_region, CaptureRect};
use crate::config::secrets::load_openrouter_api_key;
use crate::config::settings::{AppSettings, DEFAULT_COPY_EXPLANATIONS_HOTKEY};
use crate::core::state::{WorkflowCoordinator, WorkflowState};
use crate::interaction::controls::ScreenBounds;
use crate::interaction::uia::{detect_and_fill_multi, set_clipboard_unicode, FillOutcome};
use crate::storage::db::{HistoryDb, NewHistoryEntry};
use crate::ui::dialogs::{load_app_icon, to_wide_null};
use crate::ui::history_window::show_history_window;
use crate::ui::qa_viewer::toggle_qa_viewer_window;
use crate::ui::settings_window::show_settings_window;
use crate::verification::sandbox::{verify_python_isolated, VerificationOutcome};

const WM_TRAY_ICON: u32 = WM_APP + 1;
const WM_SETTINGS_UPDATED: u32 = WM_APP + 2;
const WM_SHOW_BALLOON: u32 = WM_APP + 3;
const WM_REGION_SELECTED: u32 = WM_APP + 4;
const TRAY_ICON_ID: u32 = 1;
const HOTKEY_ID: i32 = 1;
const HOTKEY_TOGGLE_QA_ID: i32 = 2;
const HOTKEY_COPY_ANSWERS_ID: i32 = 3;
const HOTKEY_COPY_EXPLANATIONS_ID: i32 = 4;

const IDM_VIEW_LAST_QA: usize = 1001;
const IDM_COPY_ANSWERS: usize = 1002;
const IDM_SETTINGS: usize = 1003;
const IDM_HISTORY: usize = 1004;
const IDM_CANCEL_WORKFLOW: usize = 1005;
const IDM_EXIT: usize = 1006;
const IDM_COPY_EXPLANATIONS: usize = 1007;

#[derive(Debug, Clone, Default)]
pub struct QaDisplayItem {
    pub label: String,
    pub correct_answer: String,
    pub details: String,
    pub explanation: String,
    pub fill_status: String,
    pub ver_note: String,
    pub confidence: f32,
}

#[derive(Debug, Clone, Default)]
pub struct LastQaSummary {
    pub question: String,
    pub answer: String,
    pub raw_answer: String,
    pub raw_explanation: String,
    pub status: String,
    pub items: Vec<QaDisplayItem>,
}

/// Formats `(question_label, explanation)` pairs into a clipboard payload matching `raw_answer`
/// conventions: empty if none are non-empty, plain trimmed text for 1 item, or
/// `"Q{n}: <explanation>"` lines joined with `"\r\n"` for multiple items.
pub fn format_explanations_for_clipboard(items: &[(String, String)]) -> String {
    let non_empty: Vec<(&str, &str)> = items
        .iter()
        .filter_map(|(label, exp)| {
            let trimmed = exp.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some((label.trim(), trimmed))
            }
        })
        .collect();

    match non_empty.len() {
        0 => String::new(),
        1 => non_empty[0].1.to_string(),
        _ => non_empty
            .into_iter()
            .map(|(label, exp)| format!("{label}: {exp}"))
            .collect::<Vec<_>>()
            .join("\r\n"),
    }
}

#[derive(Debug, Error)]
pub enum UiError {
    #[error("Win32 API error: {0}")]
    Windows(#[from] windows::core::Error),
    #[error("failed to create Win32 window")]
    WindowCreationFailed,
}

struct AppContext {
    coordinator: WorkflowCoordinator,
    settings: Arc<Mutex<AppSettings>>,
    last_qa: Arc<Mutex<Option<LastQaSummary>>>,
    history_db: Option<Arc<Mutex<HistoryDb>>>,
    balloon_text: Arc<Mutex<String>>,
    pending_api_key: Arc<Mutex<Option<String>>>,
}

/// Runs the native Win32 message loop with a System Tray icon.
pub fn run_tray_app(
    coordinator: WorkflowCoordinator,
    settings: AppSettings,
    last_qa: Arc<Mutex<Option<LastQaSummary>>>,
    history_db: Option<Arc<Mutex<HistoryDb>>>,
) -> Result<(), UiError> {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);

        let hinstance = GetModuleHandleW(None)?;
        let class_name = w!("KnuDiagnosisTrayWindowClass");

        let wc = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: hinstance.into(),
            lpszClassName: class_name,
            hIcon: load_app_icon(false),
            ..Default::default()
        };

        let atom = RegisterClassW(&wc);
        if atom == 0 {
            return Err(UiError::WindowCreationFailed);
        }

        let shared_settings = Arc::new(Mutex::new(settings));
        let ctx = Box::new(AppContext {
            coordinator,
            settings: Arc::clone(&shared_settings),
            last_qa,
            history_db,
            balloon_text: Arc::new(Mutex::new(String::new())),
            pending_api_key: Arc::new(Mutex::new(None)),
        });
        let ctx_ptr = Box::into_raw(ctx);

        let hwnd = CreateWindowExW(
            Default::default(),
            class_name,
            w!("KNU Diagnosis"),
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            400,
            240,
            None,
            None,
            hinstance,
            Some(ctx_ptr as *const _),
        )?;

        if hwnd.0.is_null() {
            let _ = Box::from_raw(ctx_ptr);
            return Err(UiError::WindowCreationFailed);
        }

        if let Err(err) = add_tray_icon(hwnd, &shared_settings.lock().unwrap().hotkey) {
            let _ = DestroyWindow(hwnd);
            return Err(err);
        }

        {
            let s = shared_settings.lock().unwrap();
            register_configured_hotkeys(
                hwnd,
                &s.hotkey,
                &s.qa_toggle_hotkey,
                &s.copy_answers_hotkey,
                &s.copy_explanations_hotkey,
            );
        }

        info!("KNU Diagnosis tray icon initialized; entering Win32 message loop");

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        let _ = UnregisterHotKey(hwnd, HOTKEY_ID);
        let _ = UnregisterHotKey(hwnd, HOTKEY_TOGGLE_QA_ID);
        let _ = UnregisterHotKey(hwnd, HOTKEY_COPY_ANSWERS_ID);
        let _ = UnregisterHotKey(hwnd, HOTKEY_COPY_EXPLANATIONS_ID);
        remove_tray_icon(hwnd);
    }

    Ok(())
}

unsafe fn register_configured_hotkeys(
    hwnd: HWND,
    capture_hotkey: &str,
    qa_hotkey: &str,
    copy_hotkey: &str,
    copy_expl_hotkey: &str,
) {
    let _ = UnregisterHotKey(hwnd, HOTKEY_ID);
    let _ = UnregisterHotKey(hwnd, HOTKEY_TOGGLE_QA_ID);
    let _ = UnregisterHotKey(hwnd, HOTKEY_COPY_ANSWERS_ID);
    let _ = UnregisterHotKey(hwnd, HOTKEY_COPY_EXPLANATIONS_ID);

    let parsed = parse_hotkey(capture_hotkey).unwrap_or_else(|err| {
        error!("Invalid capture hotkey '{capture_hotkey}' ({err}); falling back to Alt+Z");
        parse_hotkey("Alt+Z").expect("Alt+Z is a valid hotkey")
    });
    if let Err(err) = RegisterHotKey(hwnd, HOTKEY_ID, parsed.modifiers, parsed.vk) {
        error!("Failed to register capture hotkey '{capture_hotkey}': {err}");
    } else {
        info!("Capture hotkey '{capture_hotkey}' registered");
    }

    let parsed_qa = parse_hotkey(qa_hotkey).unwrap_or_else(|err| {
        error!("Invalid Q&A toggle hotkey '{qa_hotkey}' ({err}); falling back to Alt+X");
        parse_hotkey("Alt+X").expect("Alt+X is a valid hotkey")
    });
    if let Err(err) = RegisterHotKey(hwnd, HOTKEY_TOGGLE_QA_ID, parsed_qa.modifiers, parsed_qa.vk) {
        error!("Failed to register Q&A toggle hotkey '{qa_hotkey}': {err}");
    } else {
        info!("Q&A toggle hotkey '{qa_hotkey}' registered");
    }

    let parsed_copy = parse_hotkey(copy_hotkey).unwrap_or_else(|err| {
        error!("Invalid copy answers hotkey '{copy_hotkey}' ({err}); falling back to Alt+C");
        parse_hotkey("Alt+C").expect("Alt+C is a valid hotkey")
    });
    if let Err(err) = RegisterHotKey(
        hwnd,
        HOTKEY_COPY_ANSWERS_ID,
        parsed_copy.modifiers,
        parsed_copy.vk,
    ) {
        error!("Failed to register copy answers hotkey '{copy_hotkey}': {err}");
    } else {
        info!("Copy answers hotkey '{copy_hotkey}' registered");
    }

    let parsed_copy_expl = parse_hotkey(copy_expl_hotkey).unwrap_or_else(|err| {
        error!(
            "Invalid copy explanations hotkey '{copy_expl_hotkey}' ({err}); falling back to {DEFAULT_COPY_EXPLANATIONS_HOTKEY}"
        );
        parse_hotkey(DEFAULT_COPY_EXPLANATIONS_HOTKEY)
            .expect("DEFAULT_COPY_EXPLANATIONS_HOTKEY is a valid hotkey")
    });
    if let Err(err) = RegisterHotKey(
        hwnd,
        HOTKEY_COPY_EXPLANATIONS_ID,
        parsed_copy_expl.modifiers,
        parsed_copy_expl.vk,
    ) {
        error!("Failed to register copy explanations hotkey '{copy_expl_hotkey}': {err}");
    } else {
        info!("Copy explanations hotkey '{copy_expl_hotkey}' registered");
    }
}

unsafe fn update_tray_tooltip(hwnd: HWND, hotkey: &str) {
    let mut nid = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: TRAY_ICON_ID,
        uFlags: NIF_TIP,
        ..Default::default()
    };
    let tip = format!("KNU Diagnosis (Kangwon National University) — Ready ({hotkey})");
    let wide_tip = to_wide_null(&tip);
    let copy_len = wide_tip.len().min(nid.szTip.len());
    nid.szTip[..copy_len].copy_from_slice(&wide_tip[..copy_len]);
    let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
}

unsafe fn add_tray_icon(hwnd: HWND, hotkey: &str) -> Result<(), UiError> {
    let mut nid = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: TRAY_ICON_ID,
        uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
        uCallbackMessage: WM_TRAY_ICON,
        hIcon: load_app_icon(true),
        ..Default::default()
    };

    let tip = format!("KNU Diagnosis (Kangwon National University) — Ready ({hotkey})");
    let wide_tip = to_wide_null(&tip);
    let copy_len = wide_tip.len().min(nid.szTip.len());
    nid.szTip[..copy_len].copy_from_slice(&wide_tip[..copy_len]);

    if !Shell_NotifyIconW(NIM_ADD, &nid).as_bool() {
        error!("Shell_NotifyIconW(NIM_ADD) returned false");
    }
    Ok(())
}

unsafe fn show_tray_balloon(hwnd: HWND, message: &str) {
    let mut nid = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: TRAY_ICON_ID,
        uFlags: NIF_INFO,
        dwInfoFlags: NIIF_INFO,
        ..Default::default()
    };
    let wide_title = to_wide_null("KNU Diagnosis");
    let t_len = wide_title.len().min(nid.szInfoTitle.len());
    nid.szInfoTitle[..t_len].copy_from_slice(&wide_title[..t_len]);

    let wide_info = to_wide_null(message);
    let i_len = wide_info.len().min(nid.szInfo.len());
    nid.szInfo[..i_len].copy_from_slice(&wide_info[..i_len]);

    let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
}

unsafe fn remove_tray_icon(hwnd: HWND) {
    let nid = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: TRAY_ICON_ID,
        ..Default::default()
    };
    let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
}

unsafe fn show_context_menu(hwnd: HWND, ctx: &AppContext) {
    let Ok(menu) = CreatePopupMenu() else {
        return;
    };

    let (hotkey, qa_hotkey, copy_hotkey, copy_expl_hotkey) = {
        let s = ctx.settings.lock().expect("settings mutex poisoned");
        (
            s.hotkey.clone(),
            s.qa_toggle_hotkey.clone(),
            s.copy_answers_hotkey.clone(),
            s.copy_explanations_hotkey.clone(),
        )
    };
    let status_label = to_wide_null(&format!(
        "Status: {} ({}, Q&A: {}, Ans: {}, Expl: {})",
        ctx.coordinator.state().label(),
        hotkey,
        qa_hotkey,
        copy_hotkey,
        copy_expl_hotkey
    ));
    let _ = AppendMenuW(menu, MF_STRING, 0, PCWSTR(status_label.as_ptr()));
    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
    let _ = AppendMenuW(menu, MF_STRING, IDM_VIEW_LAST_QA, w!("Show / Hide Last Q&&A"));
    let _ = AppendMenuW(menu, MF_STRING, IDM_COPY_ANSWERS, w!("Copy Last Answer(s)"));
    let _ = AppendMenuW(
        menu,
        MF_STRING,
        IDM_COPY_EXPLANATIONS,
        w!("Copy Last Explanation(s)"),
    );
    let _ = AppendMenuW(menu, MF_STRING, IDM_HISTORY, w!("History..."));
    let _ = AppendMenuW(menu, MF_STRING, IDM_SETTINGS, w!("Settings..."));
    let _ = AppendMenuW(menu, MF_STRING, IDM_CANCEL_WORKFLOW, w!("Cancel Current Run"));
    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
    let _ = AppendMenuW(menu, MF_STRING, IDM_EXIT, w!("Exit"));

    let mut pt = POINT::default();
    let _ = GetCursorPos(&mut pt);
    let _ = SetForegroundWindow(hwnd);
    let _ = TrackPopupMenu(menu, TPM_LEFTALIGN | TPM_BOTTOMALIGN, pt.x, pt.y, 0, hwnd, None);
    let _ = DestroyMenu(menu);
}

unsafe fn handle_hotkey_trigger(hwnd: HWND, ctx: &AppContext) {
    if !ctx.coordinator.try_begin() {
        info!("Hotkey ignored: workflow already in progress");
        return;
    }

    let api_key = match load_openrouter_api_key() {
        Ok(Some(k)) if !k.trim().is_empty() => k,
        Ok(_) => {
            error!("OpenRouter API key is not configured");
            let mut qa = ctx.last_qa.lock().expect("last_qa mutex poisoned");
            *qa = Some(LastQaSummary {
                question: "Configuration required".to_string(),
                answer: "Open Settings from the tray icon to enter your OpenRouter API key."
                    .to_string(),
                raw_answer: String::new(),
                raw_explanation: String::new(),
                status: "Failed (Missing API Key)".to_string(),
                items: Vec::new(),
            });
            ctx.coordinator.set_state(WorkflowState::Failed);
            show_tray_balloon(hwnd, "Missing OpenRouter API key. Open Tray -> Settings.");
            return;
        }
        Err(err) => {
            error!("Failed to read OpenRouter API key: {err}");
            ctx.coordinator.set_state(WorkflowState::Failed);
            return;
        }
    };

    *ctx.pending_api_key.lock().expect("pending_api_key poisoned") = Some(api_key);
    let selection_visual = ctx
        .settings
        .lock()
        .expect("settings mutex poisoned")
        .selection_visual
        .clone();

    if !start_region_selection(hwnd, WM_REGION_SELECTED, &selection_visual) {
        *ctx.pending_api_key.lock().expect("pending_api_key poisoned") = None;
        ctx.coordinator.set_state(WorkflowState::Failed);
    }
}

unsafe fn handle_region_selected(hwnd: HWND, ctx: &AppContext, wparam: WPARAM, lparam: LPARAM) {
    let api_key = ctx
        .pending_api_key
        .lock()
        .expect("pending_api_key poisoned")
        .take();

    if wparam.0 == 0 || lparam.0 == 0 {
        info!("Region selection cancelled by user");
        ctx.coordinator.set_state(WorkflowState::Idle);
        return;
    }

    let rect = Box::from_raw(lparam.0 as *mut CaptureRect);
    let Some(api_key) = api_key else {
        ctx.coordinator.set_state(WorkflowState::Failed);
        return;
    };

    // Allow DWM to finalize overlay removal before BitBlt capture.
    std::thread::sleep(std::time::Duration::from_millis(25));

    let capture = match capture_region(&rect) {
        Ok(cap) => {
            info!(
                left = rect.left,
                top = rect.top,
                width = cap.width,
                height = cap.height,
                bytes = cap.image_bytes.len(),
                "Selected region captured; sending to Vision AI"
            );
            cap
        }
        Err(err) => {
            error!("Region screen capture failed: {err}");
            ctx.coordinator.set_state(WorkflowState::Failed);
            return;
        }
    };

    ctx.coordinator.set_state(WorkflowState::Analyzing);
    let coordinator = ctx.coordinator.clone();
    let settings = ctx.settings.lock().expect("settings mutex poisoned").clone();
    let last_qa = Arc::clone(&ctx.last_qa);
    let history_db = ctx.history_db.clone();
    let balloon_text = Arc::clone(&ctx.balloon_text);
    let hwnd_raw = hwnd.0 as usize;
    let bounds = ScreenBounds {
        left: rect.left,
        top: rect.top,
        width: rect.width,
        height: rect.height,
    };

    tokio::spawn(async move {
        let client = match OpenRouterClient::new(api_key, settings.request_timeout_secs) {
            Ok(c) => c,
            Err(err) => {
                error!("Failed to initialize OpenRouter client: {err}");
                coordinator.set_state(WorkflowState::Failed);
                return;
            }
        };

        let fg_hwnd = capture.foreground_hwnd;
        let result = client.solve_from_capture(&capture, &settings).await;
        drop(capture);

        if coordinator.is_cancelled() {
            info!("Workflow cancelled before applying AI result");
            return;
        }

        match result {
            Ok(outcome) => {
                let mut verification_notes = Vec::with_capacity(outcome.response.questions.len());
                for q in &outcome.response.questions {
                    let mut note = format!("{:?}", q.verification);
                    if let Some(code) = q.code.as_deref() {
                        if !code.trim().is_empty() {
                            match verify_python_isolated(code, 5).await {
                                VerificationOutcome::SyntaxValid { stdout } => {
                                    note = if stdout.is_empty() {
                                        "PythonExecuted (syntax OK)".to_string()
                                    } else {
                                        format!("PythonExecuted (stdout: {stdout})")
                                    };
                                }
                                VerificationOutcome::SyntaxOrRuntimeError(err) => {
                                    note = format!("PythonCheckError: {err}");
                                }
                                VerificationOutcome::Unavailable(_) => {}
                            }
                        }
                    }
                    verification_notes.push(note);
                }

                coordinator.set_state(WorkflowState::LocatingControl);
                let questions_for_uia = outcome.response.questions.clone();
                let settings_clone = settings.clone();
                let fill_results = tokio::task::spawn_blocking(move || {
                    detect_and_fill_multi(fg_hwnd, bounds, &questions_for_uia, &settings_clone)
                })
                .await;

                let per_q_fill: Vec<String> = match fill_results {
                    Ok(Ok(outcomes)) => outcomes
                        .into_iter()
                        .map(|o| match o {
                            FillOutcome::Filled {
                                control_name,
                                method,
                            } => format!("Filled '{control_name}' ({method})"),
                            FillOutcome::NeedsConfirmation(r) => format!("Confirm: {r}"),
                            FillOutcome::Skipped(r) => format!("Skipped: {r}"),
                        })
                        .collect(),
                    Ok(Err(err)) => vec![format!("UIA error: {err}")],
                    Err(err) => vec![format!("UIA task error: {err}")],
                };

                let mut q_blocks = Vec::new();
                let mut a_blocks = Vec::new();
                let mut raw_answers = Vec::new();
                let mut raw_explanations = Vec::new();
                let mut display_items = Vec::new();
                let mut filled_count = 0usize;

                for (idx, q) in outcome.response.questions.iter().enumerate() {
                    let q_label = q
                        .question_number
                        .as_deref()
                        .map(|n| format!("Q{n}"))
                        .unwrap_or_else(|| format!("Q{}", idx + 1));
                    let fill_status = per_q_fill
                        .get(idx)
                        .cloned()
                        .unwrap_or_else(|| "Not filled".to_string());
                    if fill_status.starts_with("Filled ") {
                        filled_count += 1;
                    }
                    let ver_note = verification_notes
                        .get(idx)
                        .cloned()
                        .unwrap_or_else(|| "AiReasoned".to_string());

                    let raw_ans = q
                        .code
                        .clone()
                        .filter(|c| !c.trim().is_empty())
                        .unwrap_or_else(|| q.answer.clone());
                    let ans_display = q
                        .answer_text
                        .as_deref()
                        .map(|t| format!("{} ({})", q.answer, t))
                        .unwrap_or_else(|| raw_ans.clone());

                    q_blocks.push(format!("[{q_label} - {:?}] {}", q.question_type, q.question_text));
                    a_blocks.push(format!(
                        "[{q_label}] {ans_display}\nExplanation: {}\nFill: {fill_status} | {ver_note} ({:.0}%)",
                        q.explanation,
                        q.confidence * 100.0
                    ));
                    raw_answers.push(format!("{q_label}: {raw_ans}"));
                    raw_explanations.push((q_label.clone(), q.explanation.clone()));
                    display_items.push(QaDisplayItem {
                        label: format!("[{q_label}] Answer:"),
                        correct_answer: ans_display.clone(),
                        details: format!(
                            "Explanation: {}\nFill: {fill_status} | {ver_note} ({:.0}%)",
                            q.explanation,
                            q.confidence * 100.0
                        ),
                        explanation: q.explanation.clone(),
                        fill_status: fill_status.clone(),
                        ver_note: ver_note.clone(),
                        confidence: q.confidence,
                    });

                    if let Some(db) = &history_db {
                        let guard = db.lock().expect("history db mutex poisoned");
                        let _ = guard.insert(&NewHistoryEntry {
                            question_number: q.question_number.clone(),
                            question_type: format!("{:?}", q.question_type),
                            question_text: q.question_text.clone(),
                            answer: ans_display,
                            explanation: q.explanation.clone(),
                            confidence: q.confidence,
                            verification_status: ver_note,
                            model_name: outcome.model_used.clone(),
                            filled_status: fill_status,
                            input_tokens: outcome.usage.prompt_tokens,
                            output_tokens: outcome.usage.completion_tokens,
                        });
                    }
                }

                let total_q = outcome.response.questions.len();
                let summary_status = format!(
                    "Solved {total_q} item(s), auto-filled {filled_count}/{total_q} | {}",
                    outcome.model_used
                );

                {
                    let raw_text = if raw_answers.len() == 1 {
                        outcome
                            .response
                            .questions
                            .first()
                            .map(|q| {
                                q.code
                                    .clone()
                                    .filter(|c| !c.trim().is_empty())
                                    .unwrap_or_else(|| q.answer.clone())
                            })
                            .unwrap_or_default()
                    } else {
                        raw_answers.join("\r\n")
                    };
                    let raw_explanation = format_explanations_for_clipboard(&raw_explanations);
                    let mut qa = last_qa.lock().expect("last_qa mutex poisoned");
                    *qa = Some(LastQaSummary {
                        question: q_blocks.join("\n\n"),
                        answer: a_blocks.join("\n\n---\n\n"),
                        raw_answer: raw_text,
                        raw_explanation,
                        status: summary_status.clone(),
                        items: display_items,
                    });
                }

                coordinator.set_state(WorkflowState::Completed);
                info!("Multi-question solve & fill completed: {summary_status}");

                if settings.show_tray_notifications {
                    let mut b = balloon_text.lock().expect("balloon mutex poisoned");
                    *b = format!("{summary_status}. Click tray icon to view Q&A.");
                    let _ = PostMessageW(
                        HWND(hwnd_raw as *mut _),
                        WM_SHOW_BALLOON,
                        WPARAM(0),
                        LPARAM(0),
                    );
                }
            }
            Err(err) => {
                error!("Vision AI request failed: {err}");
                let mut qa = last_qa.lock().expect("last_qa mutex poisoned");
                *qa = Some(LastQaSummary {
                    question: "Vision AI Error".to_string(),
                    answer: err.to_string(),
                    raw_answer: String::new(),
                    raw_explanation: String::new(),
                    status: "Failed".to_string(),
                    items: Vec::new(),
                });
                coordinator.set_state(WorkflowState::Failed);
            }
        }
    });
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowLongPtrW, SetWindowLongPtrW, CREATESTRUCTW, GWLP_USERDATA, WM_NCCREATE,
    };

    if msg == WM_NCCREATE {
        let cs = &*(lparam.0 as *const CREATESTRUCTW);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, cs.lpCreateParams as isize);
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }

    let ctx_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut AppContext;

    match msg {
        WM_HOTKEY => {
            if !ctx_ptr.is_null() {
                let ctx = &*ctx_ptr;
                let id = wparam.0 as i32;
                if id == HOTKEY_ID {
                    handle_hotkey_trigger(hwnd, ctx);
                } else if id == HOTKEY_TOGGLE_QA_ID {
                    let hotkey = ctx.settings.lock().expect("settings mutex poisoned").hotkey.clone();
                    toggle_qa_viewer_window(hwnd, &ctx.last_qa, &hotkey);
                } else if id == HOTKEY_COPY_ANSWERS_ID {
                    copy_last_answers_to_clipboard(hwnd, ctx);
                } else if id == HOTKEY_COPY_EXPLANATIONS_ID {
                    copy_last_explanations_to_clipboard(hwnd, ctx);
                }
            }
            LRESULT(0)
        }
        WM_REGION_SELECTED => {
            if !ctx_ptr.is_null() {
                let ctx = &*ctx_ptr;
                handle_region_selected(hwnd, ctx, wparam, lparam);
            }
            LRESULT(0)
        }
        WM_SETTINGS_UPDATED => {
            if !ctx_ptr.is_null() {
                let ctx = &*ctx_ptr;
                let (hotkey, qa_hotkey, copy_hotkey, copy_expl_hotkey) = {
                    let s = ctx.settings.lock().expect("settings mutex poisoned");
                    (
                        s.hotkey.clone(),
                        s.qa_toggle_hotkey.clone(),
                        s.copy_answers_hotkey.clone(),
                        s.copy_explanations_hotkey.clone(),
                    )
                };
                register_configured_hotkeys(
                    hwnd,
                    &hotkey,
                    &qa_hotkey,
                    &copy_hotkey,
                    &copy_expl_hotkey,
                );
                update_tray_tooltip(hwnd, &hotkey);
            }
            LRESULT(0)
        }
        WM_SHOW_BALLOON => {
            if !ctx_ptr.is_null() {
                let ctx = &*ctx_ptr;
                let msg_text = ctx.balloon_text.lock().expect("balloon mutex poisoned").clone();
                if !msg_text.is_empty() {
                    show_tray_balloon(hwnd, &msg_text);
                }
            }
            LRESULT(0)
        }
        WM_TRAY_ICON => {
            if !ctx_ptr.is_null() {
                let ctx = &*ctx_ptr;
                let event = lparam.0 as u32;
                if event == WM_RBUTTONUP {
                    show_context_menu(hwnd, ctx);
                } else if event == WM_LBUTTONUP {
                    let hotkey = ctx.settings.lock().expect("settings mutex poisoned").hotkey.clone();
                    toggle_qa_viewer_window(hwnd, &ctx.last_qa, &hotkey);
                }
            }
            LRESULT(0)
        }
        WM_COMMAND => {
            if !ctx_ptr.is_null() {
                let ctx = &*ctx_ptr;
                let cmd = wparam.0 & 0xFFFF;
                match cmd {
                    IDM_VIEW_LAST_QA => {
                        let hotkey = ctx.settings.lock().expect("settings mutex poisoned").hotkey.clone();
                        toggle_qa_viewer_window(hwnd, &ctx.last_qa, &hotkey);
                    }
                    IDM_COPY_ANSWERS => {
                        copy_last_answers_to_clipboard(hwnd, ctx);
                    }
                    IDM_COPY_EXPLANATIONS => {
                        copy_last_explanations_to_clipboard(hwnd, ctx);
                    }
                    IDM_SETTINGS => {
                        show_settings_window(
                            hwnd,
                            WM_SETTINGS_UPDATED,
                            Arc::clone(&ctx.settings),
                        );
                    }
                    IDM_HISTORY => {
                        if let Some(db) = &ctx.history_db {
                            show_history_window(hwnd, Arc::clone(db));
                        }
                    }
                    IDM_CANCEL_WORKFLOW => {
                        ctx.coordinator.request_cancel();
                        info!("Workflow cancellation requested from tray menu");
                    }
                    IDM_EXIT => {
                        let _ = DestroyWindow(hwnd);
                    }
                    _ => {}
                }
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            if !ctx_ptr.is_null() {
                let _ = Box::from_raw(ctx_ptr);
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            }
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe fn copy_last_answers_to_clipboard(hwnd: HWND, ctx: &AppContext) {
    let raw = ctx
        .last_qa
        .lock()
        .expect("last_qa mutex poisoned")
        .as_ref()
        .map(|qa| qa.raw_answer.clone())
        .unwrap_or_default();

    if raw.trim().is_empty() {
        info!("Copy answers hotkey pressed, but no solved answers are available yet");
        return;
    }

    match set_clipboard_unicode(&raw) {
        Ok(()) => {
            info!("Copied last answer(s) to clipboard via hotkey");
            if ctx
                .settings
                .lock()
                .expect("settings mutex poisoned")
                .show_tray_notifications
            {
                show_tray_balloon(hwnd, "Copied answer(s) to clipboard.");
            }
        }
        Err(err) => {
            error!("Failed to copy answer(s) to clipboard: {err}");
        }
    }
}

unsafe fn copy_last_explanations_to_clipboard(hwnd: HWND, ctx: &AppContext) {
    let raw = ctx
        .last_qa
        .lock()
        .expect("last_qa mutex poisoned")
        .as_ref()
        .map(|qa| qa.raw_explanation.clone())
        .unwrap_or_default();

    if raw.trim().is_empty() {
        info!("Copy explanations hotkey pressed, but no solved explanations are available yet");
        return;
    }

    match set_clipboard_unicode(&raw) {
        Ok(()) => {
            info!("Copied last explanation(s) to clipboard via hotkey");
            if ctx
                .settings
                .lock()
                .expect("settings mutex poisoned")
                .show_tray_notifications
            {
                show_tray_balloon(hwnd, "Copied explanation(s) to clipboard.");
            }
        }
        Err(err) => {
            error!("Failed to copy explanation(s) to clipboard: {err}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_explanations_for_clipboard_empty_single_and_multi() {
        assert_eq!(format_explanations_for_clipboard(&[]), "");
        assert_eq!(
            format_explanations_for_clipboard(&[("Q1".to_string(), "   \n ".to_string())]),
            ""
        );

        assert_eq!(
            format_explanations_for_clipboard(&[(
                "Q1".to_string(),
                "  Lists are mutable in Python.  ".to_string()
            )]),
            "Lists are mutable in Python."
        );

        assert_eq!(
            format_explanations_for_clipboard(&[
                ("Q1".to_string(), "  First explanation. ".to_string()),
                ("Q2".to_string(), "   ".to_string()),
                ("Q3".to_string(), "Second explanation.\nDetails.".to_string()),
            ]),
            "Q1: First explanation.\r\nQ3: Second explanation.\nDetails."
        );
    }
}
