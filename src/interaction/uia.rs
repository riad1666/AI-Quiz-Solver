use std::collections::HashSet;
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;

use thiserror::Error;
use tracing::{info, warn};
use windows::core::{BSTR, Interface};
use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
    COINIT_APARTMENTTHREADED,
};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationElement, IUIAutomationInvokePattern,
    IUIAutomationSelectionItemPattern, IUIAutomationTogglePattern, IUIAutomationValuePattern,
    TreeScope_Descendants, UIA_ButtonControlTypeId, UIA_CheckBoxControlTypeId,
    UIA_ComboBoxControlTypeId, UIA_DocumentControlTypeId, UIA_EditControlTypeId,
    UIA_InvokePatternId, UIA_ListItemControlTypeId, UIA_RadioButtonControlTypeId,
    UIA_SelectionItemPatternId, UIA_TogglePatternId, UIA_ValuePatternId,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYEVENTF_KEYUP,
    MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MOVE,
    MOUSEEVENTF_VIRTUALDESK, MOUSEINPUT, VIRTUAL_KEY, VK_CONTROL,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, SetForegroundWindow, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN,
    SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
};

use crate::ai::response::{AiAnswerResponse, FillActionKind};
use crate::config::settings::AppSettings;
use crate::interaction::controls::{
    filter_candidates_in_region, match_target_control_excluding, ControlCandidate, ControlKind,
    MatchResult, ScreenBounds,
};
use crate::interaction::safety::{
    evaluate_fill_safety, is_forbidden_submit_label, SafetyDecision,
};

const CF_UNICODETEXT: u32 = 13;

#[derive(Debug, Error)]
pub enum UiaError {
    #[error("Win32/COM error: {0}")]
    Windows(#[from] windows::core::Error),
    #[error("invalid foreground window handle")]
    InvalidWindow,
    #[error("clipboard operation failed")]
    ClipboardFailed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FillOutcome {
    Filled { control_name: String, method: &'static str },
    NeedsConfirmation(String),
    Skipped(String),
}

struct ComGuard;
impl Drop for ComGuard {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

pub fn is_point_in_bounds(x: i32, y: i32, bounds: ScreenBounds) -> bool {
    x >= bounds.left
        && x <= bounds.left + bounds.width
        && y >= bounds.top
        && y <= bounds.top + bounds.height
}

/// Inspects the foreground window via UI Automation and Vision target coordinates,
/// controlling mouse and keyboard to fill all MCQs (`o`/`0` circles) and fill-in-the-gap boxes
/// inside `bounds`, without ever auto-submitting.
pub fn detect_and_fill_multi(
    foreground_hwnd: isize,
    bounds: ScreenBounds,
    questions: &[AiAnswerResponse],
    settings: &AppSettings,
) -> Result<Vec<FillOutcome>, UiaError> {
    if foreground_hwnd == 0 {
        return Err(UiaError::InvalidWindow);
    }

    unsafe {
        let hr = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let _com_guard = if hr.is_ok() { Some(ComGuard) } else { None };

        let _ = SetForegroundWindow(HWND(foreground_hwnd as *mut _));
        std::thread::sleep(std::time::Duration::from_millis(40));

        let automation: IUIAutomation =
            CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)?;
        let root = automation.ElementFromHandle(HWND(foreground_hwnd as *mut _))?;
        let true_cond = automation.CreateTrueCondition()?;
        let elements = root.FindAll(TreeScope_Descendants, &true_cond)?;

        let count = elements.Length()?;
        let mut raw_elements: Vec<IUIAutomationElement> = Vec::new();
        let mut all_candidates: Vec<ControlCandidate> = Vec::new();

        for i in 0..count.min(600) {
            let Ok(el) = elements.GetElement(i) else {
                continue;
            };
            let Ok(ctrl_type) = el.CurrentControlType() else {
                continue;
            };
            let Some(kind) = map_control_kind(ctrl_type.0) else {
                continue;
            };

            let name = el.CurrentName().map(|b| b.to_string()).unwrap_or_default();
            let automation_id = el
                .CurrentAutomationId()
                .map(|b| b.to_string())
                .unwrap_or_default();
            let is_enabled = el.CurrentIsEnabled().map(|b| b.as_bool()).unwrap_or(false);
            let is_offscreen = el.CurrentIsOffscreen().map(|b| b.as_bool()).unwrap_or(true);
            let rect = el.CurrentBoundingRectangle().unwrap_or_default();
            let center_x = (rect.left + rect.right) / 2;
            let center_y = (rect.top + rect.bottom) / 2;

            let idx = raw_elements.len();
            raw_elements.push(el);
            all_candidates.push(ControlCandidate {
                index: idx,
                kind,
                name,
                automation_id,
                center_x,
                center_y,
                is_enabled,
                is_offscreen,
            });
        }

        let scoped_candidates = filter_candidates_in_region(&all_candidates, bounds, 60);
        let candidates_to_use = if scoped_candidates.is_empty() {
            &all_candidates
        } else {
            &scoped_candidates
        };

        let mut used_indices: HashSet<usize> = HashSet::new();
        let mut outcomes = Vec::with_capacity(questions.len());

        for q in questions {
            if !settings.auto_fill_enabled {
                outcomes.push(FillOutcome::Skipped(
                    "Auto-fill is disabled in settings".to_string(),
                ));
                continue;
            }
            if q.confidence < settings.confidence_threshold {
                outcomes.push(FillOutcome::NeedsConfirmation(format!(
                    "AI confidence ({:.0}%) is below threshold ({:.0}%)",
                    q.confidence * 100.0,
                    settings.confidence_threshold * 100.0
                )));
                continue;
            }

            let text_to_fill = q
                .code
                .as_deref()
                .filter(|c| !c.trim().is_empty())
                .unwrap_or(&q.answer);

            // Strategy A: Use exact Vision AI `target_point` with UIA ElementFromPoint safety guard.
            if let Some((tx, ty)) = q.target_screen_point(
                bounds.left,
                bounds.top,
                bounds.width.max(1) as u32,
                bounds.height.max(1) as u32,
            ) {
                if is_point_in_bounds(tx, ty, bounds) {
                    let mut element_label = q
                        .target_hint
                        .clone()
                        .unwrap_or_else(|| format!("({tx}, {ty})"));
                    let mut blocked = false;

                    if let Ok(el) = automation.ElementFromPoint(POINT { x: tx, y: ty }) {
                        let name = el.CurrentName().map(|b| b.to_string()).unwrap_or_default();
                        let aid = el
                            .CurrentAutomationId()
                            .map(|b| b.to_string())
                            .unwrap_or_default();
                        if is_forbidden_submit_label(&name) || is_forbidden_submit_label(&aid) {
                            blocked = true;
                            outcomes.push(FillOutcome::Skipped(format!(
                                "Refused potential submission control '{name}'"
                            )));
                        } else if !name.trim().is_empty() {
                            element_label = name;
                        }
                    }

                    if blocked {
                        continue;
                    }

                    match q.effective_action() {
                        FillActionKind::ClickOption => {
                            click_screen_point(tx, ty);
                            std::thread::sleep(std::time::Duration::from_millis(65));
                            info!(target = %element_label, x = tx, y = ty, "Clicked MCQ/choice option");
                            outcomes.push(FillOutcome::Filled {
                                control_name: element_label,
                                method: "Mouse Click (Vision Target)",
                            });
                            continue;
                        }
                        FillActionKind::FillText => {
                            click_screen_point(tx, ty);
                            std::thread::sleep(std::time::Duration::from_millis(55));
                            if set_clipboard_unicode(text_to_fill).is_ok() {
                                send_ctrl_key(b'A');
                                std::thread::sleep(std::time::Duration::from_millis(25));
                                send_ctrl_key(b'V');
                                std::thread::sleep(std::time::Duration::from_millis(75));
                                info!(target = %element_label, x = tx, y = ty, "Filled gap/input box");
                                outcomes.push(FillOutcome::Filled {
                                    control_name: element_label,
                                    method: "Mouse + Paste (Vision Target)",
                                });
                                continue;
                            }
                        }
                    }
                }
            }

            // Strategy B: Fallback to scoped UIA element matching when `target_point` is absent.
            let (anchor_x, anchor_y) = q.anchor_screen_coords(
                bounds.left,
                bounds.top,
                bounds.width.max(1) as u32,
                bounds.height.max(1) as u32,
            );
            let match_result = match_target_control_excluding(
                candidates_to_use,
                q,
                anchor_x,
                anchor_y,
                &used_indices,
            );

            match evaluate_fill_safety(q, &match_result, settings) {
                SafetyDecision::Allow => {}
                SafetyDecision::RequireConfirmation(reason) => {
                    warn!("Auto-fill requires confirmation: {reason}");
                    outcomes.push(FillOutcome::NeedsConfirmation(reason));
                    continue;
                }
                SafetyDecision::Deny(reason) => {
                    info!("Auto-fill skipped: {reason}");
                    outcomes.push(FillOutcome::Skipped(reason));
                    continue;
                }
            }

            let MatchResult::Unambiguous(target) = match_result else {
                outcomes.push(FillOutcome::Skipped("No unambiguous target".to_string()));
                continue;
            };

            used_indices.insert(target.index);
            let element = &raw_elements[target.index];

            match apply_fill_to_element(element, &target, text_to_fill) {
                Ok(method) => {
                    info!(control = %target.name, method, "Answer safely filled via UIA");
                    outcomes.push(FillOutcome::Filled {
                        control_name: target.name,
                        method,
                    });
                    std::thread::sleep(std::time::Duration::from_millis(60));
                }
                Err(err) => {
                    outcomes.push(FillOutcome::Skipped(format!("Fill failed: {err}")));
                }
            }
        }

        Ok(outcomes)
    }
}

/// Convenience wrapper for single-question UIA filling.
pub fn detect_and_fill_answer(
    foreground_hwnd: isize,
    cursor_x: i32,
    cursor_y: i32,
    ai: &AiAnswerResponse,
    settings: &AppSettings,
) -> Result<FillOutcome, UiaError> {
    let bounds = ScreenBounds {
        left: cursor_x - 400,
        top: cursor_y - 400,
        width: 800,
        height: 800,
    };
    let mut results =
        detect_and_fill_multi(foreground_hwnd, bounds, std::slice::from_ref(ai), settings)?;
    Ok(results
        .pop()
        .unwrap_or_else(|| FillOutcome::Skipped("No result".to_string())))
}

fn map_control_kind(id: i32) -> Option<ControlKind> {
    if id == UIA_EditControlTypeId.0 {
        Some(ControlKind::Edit)
    } else if id == UIA_DocumentControlTypeId.0 {
        Some(ControlKind::Document)
    } else if id == UIA_RadioButtonControlTypeId.0 {
        Some(ControlKind::RadioButton)
    } else if id == UIA_CheckBoxControlTypeId.0 {
        Some(ControlKind::CheckBox)
    } else if id == UIA_ComboBoxControlTypeId.0 {
        Some(ControlKind::ComboBox)
    } else if id == UIA_ListItemControlTypeId.0 {
        Some(ControlKind::ListItem)
    } else if id == UIA_ButtonControlTypeId.0 {
        Some(ControlKind::ChoiceButton)
    } else {
        None
    }
}

unsafe fn apply_fill_to_element(
    element: &IUIAutomationElement,
    target: &ControlCandidate,
    text_to_fill: &str,
) -> Result<&'static str, UiaError> {
    match target.kind {
        ControlKind::RadioButton | ControlKind::ListItem => {
            if let Ok(unk) = element.GetCurrentPattern(UIA_SelectionItemPatternId) {
                if let Ok(sel) = unk.cast::<IUIAutomationSelectionItemPattern>() {
                    if sel.Select().is_ok() {
                        return Ok("UIA SelectionItemPattern");
                    }
                }
            }
            click_screen_point(target.center_x, target.center_y);
            Ok("Verified UIA Click")
        }
        ControlKind::CheckBox => {
            if let Ok(unk) = element.GetCurrentPattern(UIA_TogglePatternId) {
                if let Ok(toggle) = unk.cast::<IUIAutomationTogglePattern>() {
                    if toggle.Toggle().is_ok() {
                        return Ok("UIA TogglePattern");
                    }
                }
            }
            click_screen_point(target.center_x, target.center_y);
            Ok("Verified UIA Click")
        }
        ControlKind::ChoiceButton => {
            if let Ok(unk) = element.GetCurrentPattern(UIA_InvokePatternId) {
                if let Ok(inv) = unk.cast::<IUIAutomationInvokePattern>() {
                    if inv.Invoke().is_ok() {
                        return Ok("UIA InvokePattern");
                    }
                }
            }
            click_screen_point(target.center_x, target.center_y);
            Ok("Verified UIA Click")
        }
        ControlKind::Edit | ControlKind::Document | ControlKind::ComboBox => {
            if target.kind == ControlKind::Edit {
                if let Ok(unk) = element.GetCurrentPattern(UIA_ValuePatternId) {
                    if let Ok(val) = unk.cast::<IUIAutomationValuePattern>() {
                        let bstr = BSTR::from(text_to_fill);
                        if val.SetValue(&bstr).is_ok() {
                            return Ok("UIA ValuePattern");
                        }
                    }
                }
            }
            let _ = element.SetFocus();
            click_screen_point(target.center_x, target.center_y);
            std::thread::sleep(std::time::Duration::from_millis(45));
            set_clipboard_unicode(text_to_fill)?;
            send_ctrl_key(b'A');
            std::thread::sleep(std::time::Duration::from_millis(25));
            send_ctrl_key(b'V');
            Ok("Focused Clipboard Paste")
        }
    }
}

pub unsafe fn set_clipboard_unicode(text: &str) -> Result<(), UiaError> {
    let wide: Vec<u16> = OsStr::new(text)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let byte_len = wide.len() * std::mem::size_of::<u16>();

    if OpenClipboard(None).is_err() {
        return Err(UiaError::ClipboardFailed);
    }
    let _ = EmptyClipboard();

    let hmem = GlobalAlloc(GMEM_MOVEABLE, byte_len).map_err(|_| {
        let _ = CloseClipboard();
        UiaError::ClipboardFailed
    })?;
    let ptr = GlobalLock(hmem) as *mut u16;
    if ptr.is_null() {
        let _ = CloseClipboard();
        return Err(UiaError::ClipboardFailed);
    }
    std::ptr::copy_nonoverlapping(wide.as_ptr(), ptr, wide.len());
    let _ = GlobalUnlock(hmem);

    let set_res = SetClipboardData(CF_UNICODETEXT, windows::Win32::Foundation::HANDLE(hmem.0));
    let _ = CloseClipboard();
    if set_res.is_err() {
        return Err(UiaError::ClipboardFailed);
    }
    Ok(())
}

unsafe fn click_screen_point(x: i32, y: i32) {
    let vx = GetSystemMetrics(SM_XVIRTUALSCREEN);
    let vy = GetSystemMetrics(SM_YVIRTUALSCREEN);
    let vw = GetSystemMetrics(SM_CXVIRTUALSCREEN).max(1);
    let vh = GetSystemMetrics(SM_CYVIRTUALSCREEN).max(1);

    let norm_x = (((x - vx) as f64 / vw as f64) * 65535.0).round() as i32;
    let norm_y = (((y - vy) as f64 / vh as f64) * 65535.0).round() as i32;

    let flags_move = MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK;
    let inputs = [
        INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dx: norm_x,
                    dy: norm_y,
                    dwFlags: flags_move,
                    ..Default::default()
                },
            },
        },
        INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dwFlags: MOUSEEVENTF_LEFTDOWN,
                    ..Default::default()
                },
            },
        },
        INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dwFlags: MOUSEEVENTF_LEFTUP,
                    ..Default::default()
                },
            },
        },
    ];
    let _ = SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
}

unsafe fn send_ctrl_key(key: u8) {
    let inputs = [
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VK_CONTROL,
                    ..Default::default()
                },
            },
        },
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(key as u16),
                    ..Default::default()
                },
            },
        },
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(key as u16),
                    dwFlags: KEYEVENTF_KEYUP,
                    ..Default::default()
                },
            },
        },
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VK_CONTROL,
                    dwFlags: KEYEVENTF_KEYUP,
                    ..Default::default()
                },
            },
        },
    ];
    let _ = SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
}
