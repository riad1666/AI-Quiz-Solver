use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;

use windows::core::PCWSTR;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, WPARAM};
use windows::Win32::Graphics::Gdi::{GetStockObject, DEFAULT_GUI_FONT};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, GetSystemMetrics, GetWindowTextLengthW, GetWindowTextW, LoadIconW,
    LoadImageW, SendMessageW, HICON, IDI_APPLICATION, IMAGE_ICON, LR_DEFAULTCOLOR, LR_SHARED,
    SM_CXICON, SM_CXSMICON, SM_CYICON, SM_CYSMICON, WINDOW_EX_STYLE, WINDOW_STYLE, WM_SETFONT,
};

pub unsafe fn load_app_icon(small: bool) -> HICON {
    let (cx, cy) = if small {
        (GetSystemMetrics(SM_CXSMICON), GetSystemMetrics(SM_CYSMICON))
    } else {
        (GetSystemMetrics(SM_CXICON), GetSystemMetrics(SM_CYICON))
    };

    if let Ok(hmodule) = GetModuleHandleW(None) {
        let hinst: HINSTANCE = hmodule.into();
        if let Ok(handle) = LoadImageW(
            hinst,
            PCWSTR(1 as *const u16),
            IMAGE_ICON,
            cx,
            cy,
            LR_DEFAULTCOLOR | LR_SHARED,
        ) {
            if !handle.is_invalid() {
                return HICON(handle.0);
            }
        }
    }

    LoadIconW(None, IDI_APPLICATION).unwrap_or_default()
}

pub fn to_wide_null(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
}

pub unsafe fn apply_default_gui_font(hwnd: HWND) {
    let font = GetStockObject(DEFAULT_GUI_FONT);
    let _ = SendMessageW(hwnd, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));
}

pub unsafe fn create_child(
    parent: HWND,
    hinstance: HINSTANCE,
    class: PCWSTR,
    text: &str,
    style: WINDOW_STYLE,
    ex_style: WINDOW_EX_STYLE,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    id: isize,
) -> Option<HWND> {
    let wide_text = to_wide_null(text);
    let hwnd = CreateWindowExW(
        ex_style,
        class,
        PCWSTR(wide_text.as_ptr()),
        style,
        x,
        y,
        w,
        h,
        parent,
        windows::Win32::UI::WindowsAndMessaging::HMENU(id as *mut _),
        hinstance,
        None,
    )
    .ok()?;
    apply_default_gui_font(hwnd);
    Some(hwnd)
}

pub unsafe fn read_control_text(hwnd: HWND) -> String {
    let len = GetWindowTextLengthW(hwnd);
    if len <= 0 {
        return String::new();
    }
    let mut buf = vec![0u16; (len as usize) + 1];
    let copied = GetWindowTextW(hwnd, &mut buf);
    String::from_utf16_lossy(&buf[..copied as usize])
}
