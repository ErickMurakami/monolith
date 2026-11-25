//! On-demand DWM previews. A registration exists only for the hovered tile.

use crate::{desktop::WindowTarget, overlay, windows::wide};
use std::{io, mem::size_of, ptr::null_mut};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, RECT, SIZE, WPARAM},
    Graphics::{Dwm::*, Gdi::*},
    System::LibraryLoader::GetModuleHandleW,
    UI::{HiDpi::GetDpiForWindow, WindowsAndMessaging::*},
};

const CLASS: &str = "MonolithDockPreview";

pub struct Preview {
    window: HWND,
    thumbnail: isize,
    brush: HBRUSH,
}

pub fn register() -> io::Result<()> {
    let name = wide(CLASS);
    // Dock construction happens once on the GUI thread.
    unsafe {
        let class = WNDCLASSW {
            hInstance: GetModuleHandleW(std::ptr::null()),
            lpfnWndProc: Some(window_proc),
            lpszClassName: name.as_ptr(),
            ..Default::default()
        };
        if RegisterClassW(&class) == 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

impl Preview {
    pub fn open(dock: HWND, button: HWND, target: WindowTarget, colour: u32) -> Option<Self> {
        let source = target.preview_source()?;
        // All destination state remains on the Dock's thread. DWM owns the
        // rendering link and enforces the source window's capture policy.
        unsafe {
            let mut anchor = RECT::default();
            if GetWindowRect(button, &mut anchor) == 0 {
                return None;
            }
            let monitor = MonitorFromWindow(button, MONITOR_DEFAULTTONEAREST);
            let mut info = MONITORINFO {
                cbSize: size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            if GetMonitorInfoW(monitor, &mut info) == 0 {
                return None;
            }
            let window = CreateWindowExW(
                overlay::EX_STYLE | WS_EX_TOPMOST | WS_EX_NOACTIVATE,
                wide(CLASS).as_ptr(),
                wide("Monolith window preview").as_ptr(),
                overlay::STYLE,
                anchor.left,
                anchor.top,
                1,
                1,
                dock,
                null_mut(),
                GetModuleHandleW(std::ptr::null()),
                null_mut(),
            );
            if window.is_null() {
                return None;
            }
            let mut preview = Self {
                window,
                thumbnail: 0,
                brush: CreateSolidBrush(colour),
            };
            if preview.brush.is_null()
                || DwmRegisterThumbnail(window, source, &mut preview.thumbnail) < 0
            {
                return None;
            }
            SetWindowLongPtrW(window, GWLP_USERDATA, preview.brush as isize);
            let mut source_size = SIZE::default();
            if DwmQueryThumbnailSourceSize(preview.thumbnail, &mut source_size) < 0 {
                return None;
            }
            let work = info.rcWork;
            let dpi = GetDpiForWindow(dock);
            let pad = overlay::scale(4, dpi);
            let gap = overlay::scale(8, dpi);
            let max_width = overlay::scale(272, dpi).min(work.right - work.left - 2 * pad);
            let max_height = overlay::scale(170, dpi).min(anchor.top - work.top - 2 * pad - gap);
            let (width, height) = fit_thumbnail(source_size, max_width, max_height)?;
            let frame_width = width + 2 * pad;
            let frame_height = height + 2 * pad;
            let centre = anchor.left + (anchor.right - anchor.left) / 2;
            let left = (centre - frame_width / 2).clamp(work.left, work.right - frame_width);
            let top = anchor.top - frame_height - gap;
            if SetWindowPos(
                window,
                HWND_TOPMOST,
                left,
                top,
                frame_width,
                frame_height,
                SWP_NOACTIVATE,
            ) == 0
            {
                return None;
            }
            let properties = DWM_THUMBNAIL_PROPERTIES {
                dwFlags: DWM_TNP_RECTDESTINATION | DWM_TNP_VISIBLE | DWM_TNP_OPACITY,
                rcDestination: RECT {
                    left: pad,
                    top: pad,
                    right: pad + width,
                    bottom: pad + height,
                },
                opacity: u8::MAX,
                fVisible: 1,
                ..Default::default()
            };
            if DwmUpdateThumbnailProperties(preview.thumbnail, &properties) < 0 {
                return None;
            }
            ShowWindow(window, SW_SHOWNOACTIVATE);
            Some(preview)
        }
    }
}

impl Drop for Preview {
    fn drop(&mut self) {
        unsafe {
            if self.thumbnail != 0 {
                DwmUnregisterThumbnail(self.thumbnail);
            }
            DestroyWindow(self.window);
            DeleteObject(self.brush);
        }
    }
}

fn fit_thumbnail(source: SIZE, width: i32, height: i32) -> Option<(i32, i32)> {
    if source.cx <= 0 || source.cy <= 0 || width <= 0 || height <= 0 {
        return None;
    }
    if i64::from(source.cx) * i64::from(height) >= i64::from(source.cy) * i64::from(width) {
        let scaled = i64::from(source.cy) * i64::from(width) / i64::from(source.cx);
        Some((width, scaled.max(1) as i32))
    } else {
        let scaled = i64::from(source.cx) * i64::from(height) / i64::from(source.cy);
        Some((scaled.max(1) as i32, height))
    }
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_NCHITTEST => HTTRANSPARENT as isize,
            WM_MOUSEACTIVATE => MA_NOACTIVATE as isize,
            WM_ERASEBKGND => {
                let brush = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as HBRUSH;
                if !brush.is_null() {
                    let mut bounds = RECT::default();
                    GetClientRect(hwnd, &mut bounds);
                    FillRect(wp as HDC, &bounds, brush);
                    return 1;
                }
                DefWindowProcW(hwnd, msg, wp, lp)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fits_landscape_and_portrait_sources_without_stretching() {
        assert_eq!(
            fit_thumbnail(SIZE { cx: 1920, cy: 1080 }, 272, 170),
            Some((272, 153))
        );
        assert_eq!(
            fit_thumbnail(SIZE { cx: 900, cy: 1800 }, 272, 170),
            Some((85, 170))
        );
        assert_eq!(fit_thumbnail(SIZE { cx: 0, cy: 1 }, 272, 170), None);
    }
}
