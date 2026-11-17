use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::{Arc, Mutex, Once};

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{COLOR_BTNFACE, HBRUSH};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, LoadLibraryW};
use windows::Win32::UI::Controls::RichEdit::{
    CFE_BOLD, CFM_BOLD, CFM_FACE, CFM_SIZE, CHARFORMAT2W, CHARFORMATW, EM_SETCHARFORMAT,
    SCF_SELECTION,
};
use windows::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetWindowLongPtrW, IsWindow, RegisterClassW,
    SendMessageW, SetForegroundWindow, SetWindowLongPtrW, SetWindowTextW, ShowWindow,
    CREATESTRUCTW, CW_USEDEFAULT, ES_AUTOVSCROLL, ES_MULTILINE, ES_READONLY, GWLP_USERDATA,
    SW_SHOW, WINDOW_STYLE, WM_COMMAND, WM_DESTROY, WM_KEYDOWN, WM_NCCREATE, WNDCLASSW, WS_BORDER,
    WS_CAPTION, WS_CHILD, WS_EX_TOPMOST, WS_OVERLAPPED, WS_SYSMENU, WS_VISIBLE, WS_VSCROLL,
};

use crate::interaction::uia::set_clipboard_unicode;
use crate::ui::dialogs::{create_child, load_app_icon, to_wide_null};
use crate::ui::tray::{LastQaSummary, QaDisplayItem};

const IDC_QA_QUESTION: isize = 201;
const IDC_QA_ANSWER: isize = 202;
const IDC_QA_STATUS: isize = 203;
const IDC_QA_COPY_BTN: isize = 204;
const IDC_QA_CLOSE_BTN: isize = 205;

const EM_SETSEL: u32 = 0x00B1;
const EM_REPLACESEL: u32 = 0x00C2;

pub const BODY_FONT_TWIPS: i32 = 200; // 10pt
pub const ANSWER_FONT_TWIPS: i32 = 240; // 12pt

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RichSpan {
    pub text: String,
    pub bold: bool,
    pub height_twips: i32,
}

/// Builds styled RichEdit spans for a question's detail block (`Explanation`, `Fill`, verification
/// note, and confidence), rendering the explanation body and any `PythonExecuted (stdout: ...)`
/// output in bold at `BODY_FONT_TWIPS` (200 twips / 10pt) while keeping labels normal weight.
pub fn build_detail_spans(
    explanation: &str,
    fill_status: &str,
    ver_note: &str,
    confidence_pct: f32,
) -> Vec<RichSpan> {
    let mut spans = Vec::with_capacity(8);
    spans.push(RichSpan {
        text: "Explanation: ".to_string(),
        bold: false,
        height_twips: BODY_FONT_TWIPS,
    });

    let norm_expl = explanation.replace("\r\n", "\n").replace('\n', "\r\n");
    if !norm_expl.is_empty() {
        spans.push(RichSpan {
            text: norm_expl,
            bold: true,
            height_twips: BODY_FONT_TWIPS,
        });
    }

    spans.push(RichSpan {
        text: format!("\r\nFill: {fill_status} | "),
        bold: false,
        height_twips: BODY_FONT_TWIPS,
    });

    if let Some(stdout) = ver_note
        .strip_prefix("PythonExecuted (stdout: ")
        .and_then(|s| s.strip_suffix(')'))
        .filter(|s| !s.is_empty())
    {
        let norm_stdout = stdout.replace("\r\n", "\n").replace('\n', "\r\n");
        spans.push(RichSpan {
            text: "PythonExecuted (stdout: ".to_string(),
            bold: false,
            height_twips: BODY_FONT_TWIPS,
        });
        spans.push(RichSpan {
            text: norm_stdout,
            bold: true,
            height_twips: BODY_FONT_TWIPS,
        });
        spans.push(RichSpan {
            text: ")".to_string(),
            bold: false,
            height_twips: BODY_FONT_TWIPS,
        });
    } else {
        spans.push(RichSpan {
            text: ver_note.replace("\r\n", "\n").replace('\n', "\r\n"),
            bold: false,
            height_twips: BODY_FONT_TWIPS,
        });
    }

    spans.push(RichSpan {
        text: format!(" ({confidence_pct:.0}%)"),
        bold: false,
        height_twips: BODY_FONT_TWIPS,
    });

    spans
}

/// Builds all styled RichEdit spans for a single `QaDisplayItem` (label, larger bold answer,
/// detail spans, and trailing newline).
pub fn build_qa_item_spans(item: &QaDisplayItem) -> Vec<RichSpan> {
    let mut spans = Vec::with_capacity(10);
    spans.push(RichSpan {
        text: format!("{}  ", item.label),
        bold: false,
        height_twips: BODY_FONT_TWIPS,
    });
    spans.push(RichSpan {
        text: format!(
            "{}\r\n",
            item.correct_answer.replace("\r\n", "\n").replace('\n', "\r\n")
        ),
        bold: true,
        height_twips: ANSWER_FONT_TWIPS,
    });

    if !item.explanation.is_empty() || !item.fill_status.is_empty() || !item.ver_note.is_empty() {
        spans.extend(build_detail_spans(
            &item.explanation,
            &item.fill_status,
            &item.ver_note,
            item.confidence * 100.0,
        ));
        spans.push(RichSpan {
            text: "\r\n".to_string(),
            bold: false,
            height_twips: BODY_FONT_TWIPS,
        });
    } else if !item.details.is_empty() {
        spans.push(RichSpan {
            text: format!(
                "{}\r\n",
                item.details.replace("\r\n", "\n").replace('\n', "\r\n")
            ),
            bold: false,
            height_twips: BODY_FONT_TWIPS,
        });
    }

    spans
}

struct QaViewerState {
    answer_text: String,
    hwnd_question: HWND,
    hwnd_answer: HWND,
    hwnd_status: HWND,
}

static QA_CLASS_REGISTERED: Once = Once::new();
static RICHEDIT_LOADED: Once = Once::new();
static OPEN_QA_HWND: AtomicIsize = AtomicIsize::new(0);

/// Toggles the Last Q&A viewer dialog: closes it if currently open, or opens & displays it.
pub unsafe fn toggle_qa_viewer_window(
    parent: HWND,
    last_qa: &Arc<Mutex<Option<LastQaSummary>>>,
    capture_hotkey: &str,
) {
    let existing = OPEN_QA_HWND.load(Ordering::SeqCst);
    if existing != 0 {
        let hwnd = HWND(existing as *mut _);
        if IsWindow(hwnd).as_bool() {
            let _ = DestroyWindow(hwnd);
            OPEN_QA_HWND.store(0, Ordering::SeqCst);
            return;
        }
        OPEN_QA_HWND.store(0, Ordering::SeqCst);
    }

    show_qa_viewer_window(parent, last_qa, capture_hotkey);
}

pub unsafe fn show_qa_viewer_window(
    parent: HWND,
    last_qa: &Arc<Mutex<Option<LastQaSummary>>>,
    capture_hotkey: &str,
) {
    let existing = OPEN_QA_HWND.load(Ordering::SeqCst);
    if existing != 0 {
        let hwnd = HWND(existing as *mut _);
        if IsWindow(hwnd).as_bool() {
            let _ = DestroyWindow(hwnd);
        }
        OPEN_QA_HWND.store(0, Ordering::SeqCst);
    }

    let Ok(hinstance) = GetModuleHandleW(None) else {
        return;
    };
    let class_name = w!("KnuDiagnosisQaViewerClass");

    QA_CLASS_REGISTERED.call_once(|| {
        let wc = WNDCLASSW {
            lpfnWndProc: Some(qa_wnd_proc),
            hInstance: hinstance.into(),
            lpszClassName: class_name,
            hIcon: load_app_icon(false),
            hbrBackground: HBRUSH((COLOR_BTNFACE.0 + 1) as *mut _),
            ..Default::default()
        };
        let _ = RegisterClassW(&wc);
    });

    RICHEDIT_LOADED.call_once(|| {
        let _ = LoadLibraryW(w!("msftedit.dll"));
    });

    let summary = last_qa
        .lock()
        .expect("last_qa mutex poisoned")
        .clone()
        .unwrap_or_else(|| LastQaSummary {
            question: format!(
                "No question captured yet. Press {capture_hotkey} and drag across questions."
            ),
            answer: String::new(),
            raw_answer: String::new(),
            raw_explanation: String::new(),
            status: "Idle".to_string(),
            items: Vec::new(),
        });

    let state = Box::new(QaViewerState {
        answer_text: summary.raw_answer.clone(),
        hwnd_question: HWND::default(),
        hwnd_answer: HWND::default(),
        hwnd_status: HWND::default(),
    });
    let state_ptr = Box::into_raw(state);

    let Ok(hwnd) = CreateWindowExW(
        WS_EX_TOPMOST,
        class_name,
        w!("KNU Diagnosis — Last Q&A"),
        WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU,
        CW_USEDEFAULT,
        CW_USEDEFAULT,
        560,
        450,
        parent,
        None,
        hinstance,
        Some(state_ptr as *const _),
    ) else {
        let _ = Box::from_raw(state_ptr);
        return;
    };

    OPEN_QA_HWND.store(hwnd.0 as isize, Ordering::SeqCst);

    let hinst: windows::Win32::Foundation::HINSTANCE = hinstance.into();
    let edit_style = WINDOW_STYLE(
        WS_CHILD.0
            | WS_VISIBLE.0
            | WS_BORDER.0
            | WS_VSCROLL.0
            | ES_MULTILINE as u32
            | ES_AUTOVSCROLL as u32
            | ES_READONLY as u32,
    );

    let _ = create_child(
        hwnd,
        hinst,
        w!("STATIC"),
        "Question(s):",
        WS_CHILD | WS_VISIBLE,
        Default::default(),
        14,
        10,
        140,
        18,
        0,
    );
    let q_hwnd = create_child(
        hwnd,
        hinst,
        w!("EDIT"),
        &summary.question.replace('\n', "\r\n"),
        edit_style,
        Default::default(),
        14,
        30,
        516,
        96,
        IDC_QA_QUESTION,
    )
    .unwrap_or_default();

    let _ = create_child(
        hwnd,
        hinst,
        w!("STATIC"),
        "Correct Answer & Explanation:",
        WS_CHILD | WS_VISIBLE,
        Default::default(),
        14,
        134,
        240,
        18,
        0,
    );

    // Use RICHEDIT50W so every Correct Answer renders Bold and +2pt larger than surrounding text.
    let a_hwnd = if let Some(rich) = create_child(
        hwnd,
        hinst,
        w!("RICHEDIT50W"),
        "",
        edit_style,
        Default::default(),
        14,
        154,
        516,
        175,
        IDC_QA_ANSWER,
    ) {
        populate_rich_answers(rich, &summary);
        rich
    } else {
        create_child(
            hwnd,
            hinst,
            w!("EDIT"),
            &summary.answer.replace('\n', "\r\n"),
            edit_style,
            Default::default(),
            14,
            154,
            516,
            175,
            IDC_QA_ANSWER,
        )
        .unwrap_or_default()
    };

    let s_hwnd = create_child(
        hwnd,
        hinst,
        w!("STATIC"),
        &format!("Status: {}", summary.status),
        WS_CHILD | WS_VISIBLE,
        Default::default(),
        14,
        338,
        516,
        22,
        IDC_QA_STATUS,
    )
    .unwrap_or_default();

    let _ = create_child(
        hwnd,
        hinst,
        w!("BUTTON"),
        "Copy Answer(s)",
        WS_CHILD | WS_VISIBLE,
        Default::default(),
        310,
        368,
        112,
        28,
        IDC_QA_COPY_BTN,
    );
    let _ = create_child(
        hwnd,
        hinst,
        w!("BUTTON"),
        "Close",
        WS_CHILD | WS_VISIBLE,
        Default::default(),
        430,
        368,
        100,
        28,
        IDC_QA_CLOSE_BTN,
    );

    if let Some(st) = (state_ptr as *mut QaViewerState).as_mut() {
        st.hwnd_question = q_hwnd;
        st.hwnd_answer = a_hwnd;
        st.hwnd_status = s_hwnd;
    }

    let _ = ShowWindow(hwnd, SW_SHOW);
    let _ = SetForegroundWindow(hwnd);
}

unsafe fn populate_rich_answers(rich: HWND, summary: &LastQaSummary) {
    if summary.items.is_empty() {
        if !summary.answer.is_empty() {
            append_formatted_span(
                rich,
                &summary.answer.replace("\r\n", "\n").replace('\n', "\r\n"),
                true,
                ANSWER_FONT_TWIPS,
            );
        }
        return;
    }

    for (idx, item) in summary.items.iter().enumerate() {
        if idx > 0 {
            append_formatted_span(
                rich,
                "\r\n----------------------------------------\r\n",
                false,
                BODY_FONT_TWIPS,
            );
        }
        for span in build_qa_item_spans(item) {
            append_formatted_span(rich, &span.text, span.bold, span.height_twips);
        }
    }
}

unsafe fn append_formatted_span(rich: HWND, text: &str, bold: bool, height_twips: i32) {
    // Move selection to end of text
    let _ = SendMessageW(rich, EM_SETSEL, WPARAM(usize::MAX), LPARAM(-1));

    let mut cf = CHARFORMAT2W {
        Base: CHARFORMATW {
            cbSize: std::mem::size_of::<CHARFORMAT2W>() as u32,
            dwMask: CFM_BOLD | CFM_SIZE | CFM_FACE,
            dwEffects: if bold { CFE_BOLD } else { Default::default() },
            yHeight: height_twips,
            ..Default::default()
        },
        ..Default::default()
    };
    let face = to_wide_null("Segoe UI");
    let face_len = face.len().min(cf.Base.szFaceName.len());
    cf.Base.szFaceName[..face_len].copy_from_slice(&face[..face_len]);

    let _ = SendMessageW(
        rich,
        EM_SETCHARFORMAT,
        WPARAM(SCF_SELECTION as usize),
        LPARAM(&cf as *const _ as isize),
    );

    let wide = to_wide_null(text);
    let _ = SendMessageW(
        rich,
        EM_REPLACESEL,
        WPARAM(0),
        LPARAM(wide.as_ptr() as isize),
    );
}

unsafe extern "system" fn qa_wnd_proc(
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

    let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut QaViewerState;

    match msg {
        WM_KEYDOWN => {
            if wparam.0 as u16 == VK_ESCAPE.0 {
                let _ = DestroyWindow(hwnd);
            }
            LRESULT(0)
        }
        WM_COMMAND => {
            if !state_ptr.is_null() {
                let st = &*state_ptr;
                let id = (wparam.0 & 0xFFFF) as isize;
                if id == IDC_QA_COPY_BTN {
                    if !st.answer_text.is_empty() && set_clipboard_unicode(&st.answer_text).is_ok()
                    {
                        let w = to_wide_null("Status: Answer(s) copied to clipboard.");
                        let _ = SetWindowTextW(st.hwnd_status, PCWSTR(w.as_ptr()));
                    }
                } else if id == IDC_QA_CLOSE_BTN {
                    let _ = DestroyWindow(hwnd);
                }
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            if OPEN_QA_HWND.load(Ordering::SeqCst) == hwnd.0 as isize {
                OPEN_QA_HWND.store(0, Ordering::SeqCst);
            }
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

    #[test]
    fn detail_spans_bold_multiline_explanation_and_keep_labels_normal() {
        let spans = build_detail_spans(
            "Line 1: lists are mutable.\nLine 2: tuples are immutable.",
            "Filled 'Option B' (SelectionItem)",
            "AiReasoned",
            96.0,
        );

        assert_eq!(
            spans,
            vec![
                RichSpan {
                    text: "Explanation: ".to_string(),
                    bold: false,
                    height_twips: BODY_FONT_TWIPS,
                },
                RichSpan {
                    text: "Line 1: lists are mutable.\r\nLine 2: tuples are immutable.".to_string(),
                    bold: true,
                    height_twips: BODY_FONT_TWIPS,
                },
                RichSpan {
                    text: "\r\nFill: Filled 'Option B' (SelectionItem) | ".to_string(),
                    bold: false,
                    height_twips: BODY_FONT_TWIPS,
                },
                RichSpan {
                    text: "AiReasoned".to_string(),
                    bold: false,
                    height_twips: BODY_FONT_TWIPS,
                },
                RichSpan {
                    text: " (96%)".to_string(),
                    bold: false,
                    height_twips: BODY_FONT_TWIPS,
                },
            ]
        );
    }

    #[test]
    fn detail_spans_bold_python_stdout_with_nested_parens_and_newlines() {
        let spans = build_detail_spans(
            "Evaluates tuple and function output.",
            "Filled 'Answer' (ValuePattern)",
            "PythonExecuted (stdout: (1, 2)\nfn(3))",
            98.0,
        );

        assert_eq!(
            spans,
            vec![
                RichSpan {
                    text: "Explanation: ".to_string(),
                    bold: false,
                    height_twips: BODY_FONT_TWIPS,
                },
                RichSpan {
                    text: "Evaluates tuple and function output.".to_string(),
                    bold: true,
                    height_twips: BODY_FONT_TWIPS,
                },
                RichSpan {
                    text: "\r\nFill: Filled 'Answer' (ValuePattern) | ".to_string(),
                    bold: false,
                    height_twips: BODY_FONT_TWIPS,
                },
                RichSpan {
                    text: "PythonExecuted (stdout: ".to_string(),
                    bold: false,
                    height_twips: BODY_FONT_TWIPS,
                },
                RichSpan {
                    text: "(1, 2)\r\nfn(3)".to_string(),
                    bold: true,
                    height_twips: BODY_FONT_TWIPS,
                },
                RichSpan {
                    text: ")".to_string(),
                    bold: false,
                    height_twips: BODY_FONT_TWIPS,
                },
                RichSpan {
                    text: " (98%)".to_string(),
                    bold: false,
                    height_twips: BODY_FONT_TWIPS,
                },
            ]
        );
    }

    #[test]
    fn detail_spans_keep_syntax_ok_and_python_error_normal_weight() {
        let ok_spans = build_detail_spans(
            "Valid syntax.",
            "Not filled",
            "PythonExecuted (syntax OK)",
            88.0,
        );
        assert_eq!(ok_spans[3].text, "PythonExecuted (syntax OK)");
        assert!(!ok_spans[3].bold);
        assert_eq!(ok_spans[3].height_twips, BODY_FONT_TWIPS);

        let err_spans = build_detail_spans(
            "Check syntax.",
            "Skipped: low confidence",
            "PythonCheckError: SyntaxError: invalid syntax",
            60.0,
        );
        assert_eq!(
            err_spans[3].text,
            "PythonCheckError: SyntaxError: invalid syntax"
        );
        assert!(!err_spans[3].bold);
        assert_eq!(err_spans[3].height_twips, BODY_FONT_TWIPS);
    }

    #[test]
    fn qa_item_spans_render_larger_bold_answer_and_bold_body_details() {
        let item = QaDisplayItem {
            label: "[Q1] Answer:".to_string(),
            correct_answer: "42".to_string(),
            details: String::new(),
            explanation: "Because 6 * 7 = 42.".to_string(),
            fill_status: "Filled 'Input'".to_string(),
            ver_note: "PythonExecuted (stdout: 42)".to_string(),
            confidence: 0.99,
        };
        let spans = build_qa_item_spans(&item);
        assert_eq!(
            spans[0],
            RichSpan {
                text: "[Q1] Answer:  ".to_string(),
                bold: false,
                height_twips: BODY_FONT_TWIPS,
            }
        );
        assert_eq!(
            spans[1],
            RichSpan {
                text: "42\r\n".to_string(),
                bold: true,
                height_twips: ANSWER_FONT_TWIPS,
            }
        );
        assert!(spans.iter().any(|s| s.text == "Because 6 * 7 = 42." && s.bold && s.height_twips == BODY_FONT_TWIPS));
        assert!(spans.iter().any(|s| s.text == "42" && s.bold && s.height_twips == BODY_FONT_TWIPS));
    }
}
