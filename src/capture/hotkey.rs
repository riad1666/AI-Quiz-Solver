use thiserror::Error;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN, VK_CONTROL,
    VK_DELETE, VK_DOWN, VK_END, VK_F1, VK_F10, VK_F11, VK_F12, VK_F2, VK_F3, VK_F4, VK_F5, VK_F6,
    VK_F7, VK_F8, VK_F9, VK_HOME, VK_INSERT, VK_LCONTROL, VK_LEFT, VK_LMENU, VK_LSHIFT, VK_LWIN,
    VK_MENU, VK_NEXT, VK_PRIOR, VK_RCONTROL, VK_RIGHT, VK_RMENU, VK_RSHIFT, VK_RWIN, VK_SHIFT,
    VK_SPACE, VK_UP,
};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum HotkeyParseError {
    #[error("hotkey string is empty")]
    Empty,
    #[error("unknown key or modifier in hotkey: '{0}'")]
    UnknownToken(String),
    #[error("hotkey must specify a non-modifier key")]
    MissingKey,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParsedHotkey {
    pub modifiers: HOT_KEY_MODIFIERS,
    pub vk: u32,
}

/// Parses a human-readable hotkey string such as `"Alt+Z"` or `"Ctrl+Shift+P"`.
pub fn parse_hotkey(spec: &str) -> Result<ParsedHotkey, HotkeyParseError> {
    let trimmed = spec.trim();
    if trimmed.is_empty() {
        return Err(HotkeyParseError::Empty);
    }

    let mut modifiers = MOD_NOREPEAT;
    let mut vk: Option<u32> = None;

    for raw_part in trimmed.split('+') {
        let token = raw_part.trim().to_ascii_uppercase();
        if token.is_empty() {
            return Err(HotkeyParseError::UnknownToken(raw_part.to_string()));
        }

        match token.as_str() {
            "CTRL" | "CONTROL" => modifiers |= MOD_CONTROL,
            "ALT" => modifiers |= MOD_ALT,
            "SHIFT" => modifiers |= MOD_SHIFT,
            "WIN" | "SUPER" => modifiers |= MOD_WIN,
            _ => {
                if vk.is_some() {
                    return Err(HotkeyParseError::UnknownToken(raw_part.to_string()));
                }
                vk = Some(parse_virtual_key(&token)?);
            }
        }
    }

    let vk = vk.ok_or(HotkeyParseError::MissingKey)?;
    Ok(ParsedHotkey { modifiers, vk })
}

/// Returns `true` if `vk` is purely a modifier key (Ctrl, Alt, Shift, Win).
pub fn is_modifier_vk(vk: u32) -> bool {
    let u = vk as u16;
    u == VK_CONTROL.0
        || u == VK_LCONTROL.0
        || u == VK_RCONTROL.0
        || u == VK_MENU.0
        || u == VK_LMENU.0
        || u == VK_RMENU.0
        || u == VK_SHIFT.0
        || u == VK_LSHIFT.0
        || u == VK_RSHIFT.0
        || u == VK_LWIN.0
        || u == VK_RWIN.0
}

/// Formats modifier flags and a virtual key code into a canonical hotkey string like `"Alt+Z"`.
/// Returns `None` if `vk` is a modifier or unsupported key.
pub fn format_hotkey(ctrl: bool, alt: bool, shift: bool, win: bool, vk: u32) -> Option<String> {
    if is_modifier_vk(vk) {
        return None;
    }
    let key_name = virtual_key_to_name(vk)?;
    let mut parts: Vec<String> = Vec::new();
    if ctrl {
        parts.push("Ctrl".to_string());
    }
    if alt {
        parts.push("Alt".to_string());
    }
    if shift {
        parts.push("Shift".to_string());
    }
    if win {
        parts.push("Win".to_string());
    }
    parts.push(key_name);
    Some(parts.join("+"))
}

fn virtual_key_to_name(vk: u32) -> Option<String> {
    let u = vk as u16;
    if u == VK_F1.0 {
        Some("F1".to_string())
    } else if u == VK_F2.0 {
        Some("F2".to_string())
    } else if u == VK_F3.0 {
        Some("F3".to_string())
    } else if u == VK_F4.0 {
        Some("F4".to_string())
    } else if u == VK_F5.0 {
        Some("F5".to_string())
    } else if u == VK_F6.0 {
        Some("F6".to_string())
    } else if u == VK_F7.0 {
        Some("F7".to_string())
    } else if u == VK_F8.0 {
        Some("F8".to_string())
    } else if u == VK_F9.0 {
        Some("F9".to_string())
    } else if u == VK_F10.0 {
        Some("F10".to_string())
    } else if u == VK_F11.0 {
        Some("F11".to_string())
    } else if u == VK_F12.0 {
        Some("F12".to_string())
    } else if u == VK_SPACE.0 {
        Some("Space".to_string())
    } else if u == VK_INSERT.0 {
        Some("Insert".to_string())
    } else if u == VK_DELETE.0 {
        Some("Delete".to_string())
    } else if u == VK_HOME.0 {
        Some("Home".to_string())
    } else if u == VK_END.0 {
        Some("End".to_string())
    } else if u == VK_PRIOR.0 {
        Some("PageUp".to_string())
    } else if u == VK_NEXT.0 {
        Some("PageDown".to_string())
    } else if u == VK_UP.0 {
        Some("Up".to_string())
    } else if u == VK_DOWN.0 {
        Some("Down".to_string())
    } else if u == VK_LEFT.0 {
        Some("Left".to_string())
    } else if u == VK_RIGHT.0 {
        Some("Right".to_string())
    } else if (b'0' as u32..=b'9' as u32).contains(&vk)
        || (b'A' as u32..=b'Z' as u32).contains(&vk)
    {
        Some((vk as u8 as char).to_string())
    } else {
        None
    }
}

fn parse_virtual_key(token: &str) -> Result<u32, HotkeyParseError> {
    let code = match token {
        "F1" => VK_F1.0 as u32,
        "F2" => VK_F2.0 as u32,
        "F3" => VK_F3.0 as u32,
        "F4" => VK_F4.0 as u32,
        "F5" => VK_F5.0 as u32,
        "F6" => VK_F6.0 as u32,
        "F7" => VK_F7.0 as u32,
        "F8" => VK_F8.0 as u32,
        "F9" => VK_F9.0 as u32,
        "F10" => VK_F10.0 as u32,
        "F11" => VK_F11.0 as u32,
        "F12" => VK_F12.0 as u32,
        "SPACE" => VK_SPACE.0 as u32,
        "INSERT" => VK_INSERT.0 as u32,
        "DELETE" => VK_DELETE.0 as u32,
        "HOME" => VK_HOME.0 as u32,
        "END" => VK_END.0 as u32,
        "PAGEUP" => VK_PRIOR.0 as u32,
        "PAGEDOWN" => VK_NEXT.0 as u32,
        "UP" => VK_UP.0 as u32,
        "DOWN" => VK_DOWN.0 as u32,
        "LEFT" => VK_LEFT.0 as u32,
        "RIGHT" => VK_RIGHT.0 as u32,
        s if s.len() == 1 => {
            let ch = s.as_bytes()[0];
            if ch.is_ascii_alphanumeric() {
                ch as u32
            } else {
                return Err(HotkeyParseError::UnknownToken(token.to_string()));
            }
        }
        _ => return Err(HotkeyParseError::UnknownToken(token.to_string())),
    };
    Ok(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_alt_defaults_and_roundtrips_formatted_hotkeys() {
        let z = parse_hotkey("Alt+Z").expect("parse Alt+Z");
        assert_eq!(z.vk, b'Z' as u32);
        assert_eq!(z.modifiers, MOD_NOREPEAT | MOD_ALT);

        let x = parse_hotkey("Alt+x").expect("parse Alt+x");
        assert_eq!(x.vk, b'X' as u32);
        assert_eq!(x.modifiers, MOD_NOREPEAT | MOD_ALT);

        let c = parse_hotkey("Alt+c").expect("parse Alt+c");
        assert_eq!(c.vk, b'C' as u32);
        assert_eq!(c.modifiers, MOD_NOREPEAT | MOD_ALT);

        let formatted = format_hotkey(false, true, false, false, b'Z' as u32);
        assert_eq!(formatted.as_deref(), Some("Alt+Z"));
        assert_eq!(format_hotkey(false, true, false, false, VK_MENU.0 as u32), None);

        let ctrl_alt_c = parse_hotkey("Ctrl+Alt+C").expect("parse Ctrl+Alt+C");
        assert_eq!(ctrl_alt_c.vk, b'C' as u32);
        assert_eq!(ctrl_alt_c.modifiers, MOD_NOREPEAT | MOD_CONTROL | MOD_ALT);
        assert_eq!(
            format_hotkey(true, true, false, false, b'C' as u32).as_deref(),
            Some("Ctrl+Alt+C")
        );

        assert!(parse_hotkey("").is_err());
        assert!(parse_hotkey("Ctrl+Shift").is_err());
        assert!(parse_hotkey("Ctrl+InvalidKey").is_err());
    }
}
