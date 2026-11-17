use std::io::Cursor;

use image::codecs::jpeg::JpegEncoder;
use image::{ColorType, ImageEncoder};
use thiserror::Error;
use windows::Win32::Foundation::{POINT, RECT};
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits,
    GetMonitorInfoW, MonitorFromPoint, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER,
    BI_RGB, DIB_RGB_COLORS, MONITORINFO, MONITOR_DEFAULTTONEAREST, SRCCOPY,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::{GetCursorPos, GetForegroundWindow};

#[derive(Debug, Error)]
pub enum CaptureError {
    #[error("Win32 capture error: {0}")]
    Windows(#[from] windows::core::Error),
    #[error("invalid monitor dimensions: {width}x{height}")]
    InvalidDimensions { width: i32, height: i32 },
    #[error("BitBlt screen capture failed ({width}x{height} at {left},{top}): {os_error}")]
    BitBltFailed {
        width: i32,
        height: i32,
        left: i32,
        top: i32,
        os_error: std::io::Error,
    },
    #[error("GetDIBits failed to read pixel buffer")]
    GetDiBitsFailed,
    #[error("failed to encode screenshot: {0}")]
    Encode(#[from] image::ImageError),
}

#[derive(Debug, Clone, Copy)]
pub struct CaptureRect {
    pub left: i32,
    pub top: i32,
    pub width: i32,
    pub height: i32,
    pub foreground_hwnd: isize,
}

#[derive(Debug, Clone)]
pub struct ScreenCapture {
    pub image_bytes: Vec<u8>,
    pub mime_type: &'static str,
    pub width: u32,
    pub height: u32,
    pub monitor_left: i32,
    pub monitor_top: i32,
    pub cursor_x: i32,
    pub cursor_y: i32,
    pub cursor_rel_x: i32,
    pub cursor_rel_y: i32,
    pub dpi_scale: f32,
    pub foreground_hwnd: isize,
}

/// Captures only the selected physical screen rectangle in memory as a JPEG image.
pub fn capture_region(region: &CaptureRect) -> Result<ScreenCapture, CaptureError> {
    if region.width <= 0 || region.height <= 0 {
        return Err(CaptureError::InvalidDimensions {
            width: region.width,
            height: region.height,
        });
    }

    unsafe {
        let mut cursor = POINT::default();
        let _ = GetCursorPos(&mut cursor);

        let center = POINT {
            x: region.left + region.width / 2,
            y: region.top + region.height / 2,
        };
        let hmonitor = MonitorFromPoint(center, MONITOR_DEFAULTTONEAREST);
        let mut dpi_x: u32 = 96;
        let mut dpi_y: u32 = 96;
        let _ = GetDpiForMonitor(hmonitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y);
        let dpi_scale = dpi_x as f32 / 96.0;

        capture_rect_internal(
            region.left,
            region.top,
            region.width,
            region.height,
            cursor,
            dpi_scale,
            region.foreground_hwnd,
        )
    }
}

/// Captures the active monitor containing the mouse cursor in memory as a JPEG image.
pub fn capture_active_monitor() -> Result<ScreenCapture, CaptureError> {
    unsafe {
        let mut cursor = POINT::default();
        let _ = GetCursorPos(&mut cursor);

        let foreground_hwnd = GetForegroundWindow().0 as isize;
        let hmonitor = MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST);

        let mut mi = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !GetMonitorInfoW(hmonitor, &mut mi).as_bool() {
            return Err(CaptureError::InvalidDimensions { width: 0, height: 0 });
        }

        let rect: RECT = mi.rcMonitor;
        let width = rect.right - rect.left;
        let height = rect.bottom - rect.top;
        if width <= 0 || height <= 0 {
            return Err(CaptureError::InvalidDimensions { width, height });
        }

        let mut dpi_x: u32 = 96;
        let mut dpi_y: u32 = 96;
        let _ = GetDpiForMonitor(hmonitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y);
        let dpi_scale = dpi_x as f32 / 96.0;

        capture_rect_internal(
            rect.left,
            rect.top,
            width,
            height,
            cursor,
            dpi_scale,
            foreground_hwnd,
        )
    }
}

unsafe fn capture_rect_internal(
    left: i32,
    top: i32,
    width: i32,
    height: i32,
    cursor: POINT,
    dpi_scale: f32,
    foreground_hwnd: isize,
) -> Result<ScreenCapture, CaptureError> {
    let screen_dc = GetDC(None);
    if screen_dc.is_invalid() {
        return Err(CaptureError::BitBltFailed {
            width,
            height,
            left,
            top,
            os_error: std::io::Error::last_os_error(),
        });
    }
    let mem_dc = CreateCompatibleDC(screen_dc);
    let bitmap = CreateCompatibleBitmap(screen_dc, width, height);
    if mem_dc.is_invalid() || bitmap.is_invalid() {
        let os_error = std::io::Error::last_os_error();
        if !bitmap.is_invalid() {
            let _ = DeleteObject(bitmap);
        }
        if !mem_dc.is_invalid() {
            let _ = DeleteDC(mem_dc);
        }
        let _ = ReleaseDC(None, screen_dc);
        return Err(CaptureError::BitBltFailed {
            width,
            height,
            left,
            top,
            os_error,
        });
    }
    let old_obj = SelectObject(mem_dc, bitmap);

    let blt_ok = BitBlt(
        mem_dc,
        0,
        0,
        width,
        height,
        screen_dc,
        left,
        top,
        SRCCOPY,
    )
    .is_ok();

    if !blt_ok {
        let os_error = std::io::Error::last_os_error();
        SelectObject(mem_dc, old_obj);
        let _ = DeleteObject(bitmap);
        let _ = DeleteDC(mem_dc);
        let _ = ReleaseDC(None, screen_dc);
        return Err(CaptureError::BitBltFailed {
            width,
            height,
            left,
            top,
            os_error,
        });
    }

    let mut bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: -height, // top-down DIB
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };

    let mut bgra = vec![0u8; (width as usize) * (height as usize) * 4];
    let lines = GetDIBits(
        mem_dc,
        bitmap,
        0,
        height as u32,
        Some(bgra.as_mut_ptr() as *mut _),
        &mut bmi,
        DIB_RGB_COLORS,
    );

    SelectObject(mem_dc, old_obj);
    let _ = DeleteObject(bitmap);
    let _ = DeleteDC(mem_dc);
    let _ = ReleaseDC(None, screen_dc);

    if lines == 0 {
        return Err(CaptureError::GetDiBitsFailed);
    }

    // Convert BGRA -> RGB for compact, fast Vision AI JPEG upload.
    let mut rgb = Vec::with_capacity((width as usize) * (height as usize) * 3);
    for px in bgra.chunks_exact(4) {
        rgb.push(px[2]); // R
        rgb.push(px[1]); // G
        rgb.push(px[0]); // B
    }
    drop(bgra);

    let image_bytes = encode_rgb_jpeg(&rgb, width as u32, height as u32)?;

    Ok(ScreenCapture {
        image_bytes,
        mime_type: "image/jpeg",
        width: width as u32,
        height: height as u32,
        monitor_left: left,
        monitor_top: top,
        cursor_x: cursor.x,
        cursor_y: cursor.y,
        cursor_rel_x: cursor.x - left,
        cursor_rel_y: cursor.y - top,
        dpi_scale,
        foreground_hwnd,
    })
}

pub fn encode_rgb_jpeg(rgb: &[u8], width: u32, height: u32) -> Result<Vec<u8>, CaptureError> {
    let mut out = Cursor::new(Vec::new());
    let encoder = JpegEncoder::new_with_quality(&mut out, 88);
    encoder.write_image(rgb, width, height, ColorType::Rgb8.into())?;
    Ok(out.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_rgb_buffer_to_valid_jpeg() {
        let width = 16;
        let height = 16;
        let rgb = vec![128u8; (width * height * 3) as usize];
        let jpeg = encode_rgb_jpeg(&rgb, width, height).expect("jpeg encoding should succeed");
        assert!(jpeg.starts_with(&[0xFF, 0xD8, 0xFF]));
    }

    #[test]
    fn captures_active_monitor_when_desktop_available() {
        match capture_active_monitor() {
            Ok(cap) => {
                assert!(cap.width > 0);
                assert!(cap.height > 0);
                assert!(cap.dpi_scale >= 0.5);
                assert!(!cap.image_bytes.is_empty());
                assert_eq!(cap.mime_type, "image/jpeg");
            }
            Err(CaptureError::BitBltFailed { os_error, .. })
                if os_error.raw_os_error() == Some(6) =>
            {
                // Headless/non-interactive test runner has no interactive desktop DC.
            }
            Err(err) => panic!("unexpected screen capture failure: {err}"),
        }
    }
}
