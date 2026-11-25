//! Read-only desktop discovery and explicit, user-initiated window activation.

use std::{collections::HashMap, io, mem::size_of, ptr::null_mut};
use windows_sys::Win32::{
    Foundation::{CloseHandle, ERROR_INSUFFICIENT_BUFFER, GetLastError, HWND, LPARAM, RECT},
    Graphics::{
        Dwm::{DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute},
        Gdi::{
            EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITOR_DEFAULTTONULL,
            MONITORINFO, MonitorFromWindow,
        },
    },
    Storage::Packaging::Appx::GetApplicationUserModelId,
    System::Threading::{
        OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
    },
    UI::WindowsAndMessaging::{
        EnumWindows, GW_OWNER, GWL_EXSTYLE, GWL_STYLE, GetDesktopWindow, GetForegroundWindow,
        GetShellWindow, GetWindow, GetWindowLongW, GetWindowRect, GetWindowTextW,
        GetWindowThreadProcessId, IsIconic, IsWindowVisible, IsZoomed, SW_MINIMIZE, SW_RESTORE,
        SetForegroundWindow, ShowWindowAsync, WS_CAPTION, WS_EX_APPWINDOW, WS_EX_NOACTIVATE,
        WS_EX_TOOLWINDOW,
    },
};

pub struct Window {
    pub target: WindowTarget,
    pub title: String,
    pub monitor: Option<usize>,
    pub minimised: bool,
    pub fullscreen: bool,
    pub active: bool,
}

#[derive(Clone, Copy)]
pub struct WindowTarget {
    handle: HWND,
    process: u32,
    thread: u32,
}

pub struct Snapshot {
    pub windows: Vec<Window>,
    pub monitor_count: usize,
}

#[derive(Clone, PartialEq, Eq, Hash)]
enum AppIdentity {
    Packaged(String),
    Executable(String),
}

pub struct WindowGroup {
    pub windows: Vec<Window>,
}

impl WindowGroup {
    pub fn caption(&self) -> String {
        let Some(first) = self.windows.first() else {
            return String::new();
        };
        if self.windows.len() == 1 {
            first.title.clone()
        } else {
            format!("{} ({})", first.title, self.windows.len())
        }
    }

    pub fn active(&self) -> bool {
        self.windows.iter().any(|window| window.active)
    }
}

pub fn group_windows(mut windows: Vec<Window>) -> Vec<WindowGroup> {
    windows.sort_by_cached_key(|window| window.title.to_lowercase());
    let mut apps = HashMap::new();
    let mut group_indices = HashMap::new();
    let mut groups: Vec<WindowGroup> = Vec::new();
    let mut image = vec![0_u16; 1024];
    for window in windows {
        let app = apps
            .entry(window.target.process)
            .or_insert_with(|| process_identity(window.target.process, &mut image));
        let Some(app) = app else {
            // A shared host or inaccessible process stays independent.
            groups.push(WindowGroup {
                windows: vec![window],
            });
            continue;
        };
        let index = *group_indices.entry(app.clone()).or_insert_with(|| {
            groups.push(WindowGroup {
                windows: Vec::new(),
            });
            groups.len() - 1
        });
        groups[index].windows.push(window);
    }
    groups
}

fn process_identity(pid: u32, image: &mut Vec<u16>) -> Option<AppIdentity> {
    // Only query process metadata and close each handle before leaving the snapshot.
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return None;
        }
        let mut app_id = [0_u16; 512];
        let mut length = app_id.len() as u32;
        let result = if GetApplicationUserModelId(process, &mut length, app_id.as_mut_ptr()) == 0
            && length > 1
        {
            Some(AppIdentity::Packaged(
                String::from_utf16_lossy(&app_id[..length as usize - 1]).to_lowercase(),
            ))
        } else {
            let mut length = image.len() as u32;
            let mut read = QueryFullProcessImageNameW(process, 0, image.as_mut_ptr(), &mut length);
            if read == 0 && GetLastError() == ERROR_INSUFFICIENT_BUFFER && image.len() < 32768 {
                image.resize(32768, 0);
                length = image.len() as u32;
                read = QueryFullProcessImageNameW(process, 0, image.as_mut_ptr(), &mut length);
            }
            if read != 0 && length != 0 {
                let image = String::from_utf16_lossy(&image[..length as usize]).to_lowercase();
                let name = image.rsplit('\\').next();
                // The legacy packaged-app frame process can host unrelated applications.
                if matches!(name, Some("applicationframehost.exe" | "wwahost.exe")) {
                    None
                } else {
                    Some(AppIdentity::Executable(image))
                }
            } else {
                None
            }
        };
        CloseHandle(process);
        result
    }
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
        .filter_map(|hwnd| describe(hwnd, own_process, foreground, &displays))
        .collect();
    // Preserve native enumeration order for the other windows.
    if let Some(index) = windows.iter().position(|window| window.active) {
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

fn describe(
    hwnd: HWND,
    own_process: u32,
    foreground: HWND,
    displays: &[Monitor],
) -> Option<Window> {
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
        target: WindowTarget {
            handle: hwnd,
            process,
            thread,
        },
        title,
        monitor,
        minimised,
        fullscreen,
        active: hwnd == foreground,
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

impl WindowTarget {
    pub fn preview_source(self) -> Option<HWND> {
        (self.available() && unsafe { IsIconic(self.handle) } == 0).then_some(self.handle)
    }

    pub fn toggle(self) -> Result<(), ActivationError> {
        if !self.available() {
            return Err(ActivationError::Unavailable);
        }
        unsafe {
            if GetForegroundWindow() == self.handle && IsIconic(self.handle) == 0 {
                return (ShowWindowAsync(self.handle, SW_MINIMIZE) != 0)
                    .then_some(())
                    .ok_or(ActivationError::Refused);
            }
        }
        self.activate()
    }

    fn available(self) -> bool {
        identity(self.handle) == Some((self.process, self.thread)) && eligible(self.handle)
    }

    pub fn activate(self) -> Result<(), ActivationError> {
        if !self.available() {
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

pub fn foreground_covers_monitor(monitor: HMONITOR, bounds: &RECT) -> bool {
    unsafe {
        let foreground = GetForegroundWindow();
        identity(foreground).is_some_and(|(process, _)| process != std::process::id())
            && eligible(foreground)
            && IsIconic(foreground) == 0
            && MonitorFromWindow(foreground, MONITOR_DEFAULTTONULL) == monitor
            && covers_display(foreground, bounds)
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
