//! Read-only desktop discovery and explicit, user-initiated window activation.

use std::{io, mem::size_of, ptr::null_mut};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, RECT},
    Graphics::{
        Dwm::{DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute},
        Gdi::{
            EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITOR_DEFAULTTONULL,
            MONITORINFO, MonitorFromWindow,
        },
    },
    UI::WindowsAndMessaging::{
        EnumWindows, GW_OWNER, GWL_EXSTYLE, GWL_STYLE, GetDesktopWindow, GetForegroundWindow,
        GetShellWindow, GetWindow, GetWindowLongW, GetWindowRect, GetWindowTextW,
        GetWindowThreadProcessId, IsIconic, IsWindowVisible, IsZoomed, SW_RESTORE,
        SetForegroundWindow, ShowWindowAsync, WS_CAPTION, WS_EX_APPWINDOW, WS_EX_NOACTIVATE,
        WS_EX_TOOLWINDOW,
    },
};

pub struct Window {
    handle: HWND,
    process: u32,
    thread: u32,
    pub title: String,
    pub monitor: Option<usize>,
    pub minimised: bool,
    pub fullscreen: bool,
}

pub struct Snapshot {
    pub windows: Vec<Window>,
    pub monitor_count: usize,
}

struct Monitor {
    handle: HMONITOR,
    bounds: RECT,
}

// Win32 calls each enumeration callback synchronously with the stack-owned
// vector pointer supplied by its caller. The callbacks only collect handles.
unsafe extern "system" fn collect_window(hwnd: HWND, data: LPARAM) -> i32 {
    unsafe { (*(data as *mut Vec<HWND>)).push(hwnd) };
    1
}

unsafe extern "system" fn collect_monitor(
    monitor: HMONITOR,
    _: HDC,
    _: *mut RECT,
    data: LPARAM,
) -> i32 {
    unsafe { (*(data as *mut Vec<HMONITOR>)).push(monitor) };
    1
}

fn monitors() -> Vec<Monitor> {
    let mut handles: Vec<HMONITOR> = Vec::new();
    // Null HDC and clip enumerate the whole desktop.
    let ok = unsafe {
        EnumDisplayMonitors(
            null_mut(),
            std::ptr::null(),
            Some(collect_monitor),
            (&raw mut handles) as LPARAM,
        )
    };
    if ok == 0 {
        return Vec::new();
    }
    let mut displays: Vec<_> = handles
        .into_iter()
        .filter_map(|handle| {
            let mut info = MONITORINFO {
                cbSize: size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            // The handle came from EnumDisplayMonitors. It may become invalid
            // during a topology change, in which case the entry is skipped.
            (unsafe { GetMonitorInfoW(handle, &mut info) } != 0).then_some(Monitor {
                handle,
                bounds: info.rcMonitor,
            })
        })
        .collect();
    displays.sort_by_key(|display| (display.bounds.left, display.bounds.top));
    displays
}

pub fn snapshot() -> io::Result<Snapshot> {
    let mut handles: Vec<HWND> = Vec::new();
    // No mutable state is borrowed across the call, other than the callback's
    // owned output vector. A disappearing window is handled during inspection.
    if unsafe { EnumWindows(Some(collect_window), (&raw mut handles) as LPARAM) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let displays = monitors();
    let (shell, desktop, foreground) =
        unsafe { (GetShellWindow(), GetDesktopWindow(), GetForegroundWindow()) };
    let own_process = std::process::id();
    let mut windows: Vec<_> = handles
        .into_iter()
        .filter(|&hwnd| hwnd != shell && hwnd != desktop)
        .filter_map(|hwnd| describe(hwnd, own_process, &displays))
        .collect();
    // Preserve native enumeration order for the other windows.
    if let Some(index) = windows
        .iter()
        .position(|window| window.handle == foreground)
    {
        windows[..=index].rotate_right(1);
    }
    Ok(Snapshot {
        windows,
        monitor_count: displays.len(),
    })
}

fn identity(hwnd: HWND) -> Option<(u32, u32)> {
    let mut process = 0;
    let thread = unsafe { GetWindowThreadProcessId(hwnd, &mut process) };
    (thread != 0 && process != 0).then_some((process, thread))
}

fn eligible(hwnd: HWND) -> bool {
    // These read-only queries tolerate a window closing mid-enumeration.
    unsafe {
        if IsWindowVisible(hwnd) == 0 {
            return false;
        }
        let style = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
        if style & (WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE) != 0
            || (style & WS_EX_APPWINDOW == 0 && !GetWindow(hwnd, GW_OWNER).is_null())
        {
            return false;
        }
        let mut cloaked = 0_u32;
        let status = DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED as u32,
            (&raw mut cloaked).cast(),
            size_of::<u32>() as u32,
        );
        status < 0 || cloaked == 0
    }
}

fn describe(hwnd: HWND, own_process: u32, displays: &[Monitor]) -> Option<Window> {
    let (process, thread) = identity(hwnd)?;
    if process == own_process || !eligible(hwnd) {
        return None;
    }
    // For other processes this reads the system-managed caption without
    // synchronously messaging the target application.
    let mut buffer = [0_u16; 513];
    let length = unsafe { GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32) };
    let title = String::from_utf16_lossy(&buffer[..length as usize]);
    if title.trim().is_empty() {
        return None;
    }
    let minimised = unsafe { IsIconic(hwnd) != 0 };
    let handle = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONULL) };
    let monitor = displays.iter().position(|display| display.handle == handle);
    let fullscreen =
        !minimised && monitor.is_some_and(|index| covers_display(hwnd, &displays[index].bounds));
    Some(Window {
        handle: hwnd,
        process,
        thread,
        title,
        monitor,
        minimised,
        fullscreen,
    })
}

fn covers_display(hwnd: HWND, display: &RECT) -> bool {
    let mut frame = RECT::default();
    // Both rectangles use screen pixels on the per-monitor-aware GUI thread.
    unsafe {
        if IsZoomed(hwnd) != 0 && (GetWindowLongW(hwnd, GWL_STYLE) as u32 & WS_CAPTION) != 0 {
            return false;
        }
        if DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS as u32,
            (&raw mut frame).cast(),
            size_of::<RECT>() as u32,
        ) < 0
            && GetWindowRect(hwnd, &mut frame) == 0
        {
            return false;
        }
    }
    matches_display(&frame, display)
}

fn matches_display(frame: &RECT, display: &RECT) -> bool {
    frame.right > frame.left
        && frame.bottom > frame.top
        && frame.left.abs_diff(display.left) <= 2
        && frame.top.abs_diff(display.top) <= 2
        && frame.right.abs_diff(display.right) <= 2
        && frame.bottom.abs_diff(display.bottom) <= 2
}

#[derive(Debug, PartialEq, Eq)]
pub enum ActivationError {
    Unavailable,
    Refused,
}

impl Window {
    pub fn activate(&self) -> Result<(), ActivationError> {
        if identity(self.handle) != Some((self.process, self.thread)) || !eligible(self.handle) {
            return Err(ActivationError::Unavailable);
        }
        // Recheck minimisation at selection time. The async variant avoids
        // blocking on a window whose message loop is suspended.
        unsafe {
            if IsIconic(self.handle) != 0 && ShowWindowAsync(self.handle, SW_RESTORE) == 0 {
                return Err(ActivationError::Refused);
            }
            if SetForegroundWindow(self.handle) == 0 {
                return Err(ActivationError::Refused);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_fullscreen_in_screen_coordinates() {
        let display = RECT {
            left: -1920,
            top: -100,
            right: 0,
            bottom: 980,
        };
        assert!(matches_display(&display, &display));
        let border = RECT {
            left: -1921,
            top: -101,
            right: 1,
            bottom: 981,
        };
        assert!(matches_display(&border, &display));
        let working_area = RECT {
            bottom: 940,
            ..display
        };
        assert!(!matches_display(&working_area, &display));
        let spanning = RECT {
            right: 1920,
            ..display
        };
        assert!(!matches_display(&spanning, &display));
        assert!(!matches_display(&RECT::default(), &display));
    }
}
