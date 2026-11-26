//! Opt-in presentation of incoming Windows notifications below the Island.
//!
//! Windows keeps delivering and storing every notification; Monolith only reads
//! them. `UserNotificationListener` lists notifications for this unpackaged process,
//! but its change event requires package identity. Instead, an out-of-context
//! WinEvent hook notices when the Shell uncloaks its toast host window, and a worker
//! re-reads the list and presents identifiers it has not seen. Nothing is stored,
//! and Do Not Disturb suppresses the Shell toast and therefore this banner too.

use crate::{
    desktop,
    overlay::{self, Palette},
    windows::wide,
};
use std::{
    cell::RefCell,
    collections::HashSet,
    ffi::c_void,
    io,
    mem::size_of,
    ptr::null_mut,
    sync::mpsc::{self, Receiver, SyncSender},
    thread::{self, JoinHandle},
    time::Duration,
};
use windows::UI::Notifications::{
    KnownNotificationBindings,
    Management::{UserNotificationListener, UserNotificationListenerAccessStatus},
    NotificationKinds, UserNotification,
};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM},
    Graphics::{
        Dwm::{DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmSetWindowAttribute},
        Gdi::*,
    },
    UI::{
        Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent},
        HiDpi::GetDpiForWindow,
        WindowsAndMessaging::*,
    },
};

const READY: u32 = WM_APP + 30;
const HIDE: usize = 1;
const VISIBLE_MS: u32 = 6000;
const QUEUE: usize = 4;
const TOAST_HOST: &str = "Windows.UI.Core.CoreWindow";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Incoming {
    pub app: String,
    pub title: String,
    pub body: String,
}

thread_local! {
    static CHECKS: RefCell<Option<SyncSender<()>>> = const { RefCell::new(None) };
}

pub struct Notifications {
    hwnd: HWND,
    hook: HWINEVENTHOOK,
    state: Box<State>,
    worker: Option<JoinHandle<()>>,
}

struct State {
    incoming: Receiver<Incoming>,
    current: Option<Incoming>,
    palette: Palette,
    fonts: [HFONT; 3],
    dpi: u32,
}

impl Notifications {
    pub fn new(instance: *mut c_void, palette: Palette) -> io::Result<Self> {
        unsafe {
            let class_name = wide("MonolithNotification");
            let class = WNDCLASSW {
                hInstance: instance,
                lpfnWndProc: Some(window_proc),
                hCursor: LoadCursorW(null_mut(), IDC_ARROW),
                lpszClassName: class_name.as_ptr(),
                ..Default::default()
            };
            if RegisterClassW(&class) == 0 {
                return Err(io::Error::last_os_error());
            }
            let hwnd = CreateWindowExW(
                overlay::EX_STYLE | WS_EX_TOPMOST | WS_EX_NOACTIVATE,
                class_name.as_ptr(),
                wide("Monolith notification").as_ptr(),
                overlay::STYLE,
                0,
                0,
                0,
                0,
                null_mut(),
                null_mut(),
                instance,
                null_mut(),
            );
            if hwnd.is_null() {
                return Err(io::Error::last_os_error());
            }
            let corner = DWMWCP_ROUND;
            DwmSetWindowAttribute(
                hwnd,
                DWMWA_WINDOW_CORNER_PREFERENCE as u32,
                (&raw const corner).cast(),
                size_of::<i32>() as u32,
            );
            let (checks, pending) = mpsc::sync_channel(1);
            let (deliver, incoming) = mpsc::sync_channel(QUEUE);
            let target = hwnd as usize;
            let worker = thread::Builder::new()
                .name("monolith-notifications".into())
                .spawn(move || watch(pending, deliver, target))?;
            let mut notifications = Self {
                hwnd,
                hook: null_mut(),
                state: Box::new(State {
                    incoming,
                    current: None,
                    palette,
                    fonts: [null_mut(); 3],
                    dpi: 0,
                }),
                worker: Some(worker),
            };
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, &raw mut *notifications.state as isize);
            CHECKS.set(Some(checks));
            notifications.hook = SetWinEventHook(
                EVENT_OBJECT_UNCLOAKED,
                EVENT_OBJECT_UNCLOAKED,
                null_mut(),
                Some(shell_event),
                0,
                0,
                WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
            );
            if notifications.hook.is_null() {
                return Err(io::Error::last_os_error());
            }
            Ok(notifications)
        }
    }
}

impl Drop for Notifications {
    fn drop(&mut self) {
        unsafe {
            if !self.hook.is_null() {
                UnhookWinEvent(self.hook);
            }
            // Closing the channel ends the worker after any query in progress.
            CHECKS.set(None);
            SetWindowLongPtrW(self.hwnd, GWLP_USERDATA, 0);
            KillTimer(self.hwnd, HIDE);
            DestroyWindow(self.hwnd);
            for font in self.state.fonts {
                if !font.is_null() {
                    DeleteObject(font);
                }
            }
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

unsafe extern "system" fn shell_event(
    _: HWINEVENTHOOK,
    _: u32,
    hwnd: HWND,
    object: i32,
    _: i32,
    _: u32,
    _: u32,
) {
    if object != OBJID_WINDOW || hwnd.is_null() {
        return;
    }
    let mut class = [0_u16; 64];
    let length = unsafe { GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32) };
    if String::from_utf16_lossy(&class[..length.max(0) as usize]) != TOAST_HOST {
        return;
    }
    CHECKS.with_borrow(|checks| {
        // A full channel already has a pending check; coalesce.
        if let Some(checks) = checks {
            let _ = checks.try_send(());
        }
    });
}

fn watch(checks: Receiver<()>, deliver: SyncSender<Incoming>, target: usize) {
    use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize};
    if unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_err() {
        return;
    }
    let listener = UserNotificationListener::Current().ok().filter(|listener| {
        listener.GetAccessStatus().ok() == Some(UserNotificationListenerAccessStatus::Allowed)
    });
    if let Some(listener) = listener {
        // Notifications present at start-up were already delivered; never re-present them.
        let mut known: HashSet<u32> = toasts(&listener)
            .map(|list| list.iter().filter_map(|n| n.Id().ok()).collect())
            .unwrap_or_default();
        while checks.recv().is_ok() {
            // The Shell can uncloak its host before the listener lists the new item.
            thread::sleep(Duration::from_millis(150));
            let Some(list) = toasts(&listener) else {
                continue;
            };
            let mut current = HashSet::with_capacity(list.len());
            for notification in list {
                let Ok(id) = notification.Id() else { continue };
                current.insert(id);
                if !known.contains(&id)
                    && let Some(incoming) = describe(&notification)
                    && deliver.try_send(incoming).is_ok()
                {
                    unsafe { PostMessageW(target as HWND, READY, 0, 0) };
                }
            }
            known = current;
        }
    }
    unsafe { CoUninitialize() };
}

fn toasts(listener: &UserNotificationListener) -> Option<Vec<UserNotification>> {
    let list = listener
        .GetNotificationsAsync(NotificationKinds::Toast)
        .ok()?
        .join()
        .ok()?;
    Some(list.into_iter().collect())
}

fn describe(notification: &UserNotification) -> Option<Incoming> {
    let app = notification
        .AppInfo()
        .and_then(|info| info.DisplayInfo())
        .and_then(|display| display.DisplayName())
        .map(|name| name.to_string())
        .unwrap_or_default();
    let texts: Vec<String> = notification
        .Notification()
        .ok()?
        .Visual()
        .ok()?
        .GetBinding(&KnownNotificationBindings::ToastGeneric().ok()?)
        .ok()?
        .GetTextElements()
        .ok()?
        .into_iter()
        .filter_map(|text| text.Text().ok().map(|text| text.to_string()))
        .collect();
    compose(app, texts)
}

/// The first text element is the title; the remaining ones form the body.
fn compose(app: String, texts: Vec<String>) -> Option<Incoming> {
    let mut texts = texts
        .into_iter()
        .map(|text| text.trim().to_owned())
        .filter(|text| !text.is_empty());
    let title = texts.next()?;
    let body = texts.collect::<Vec<_>>().join(" · ");
    Some(Incoming {
        app: app.trim().to_owned(),
        title,
        body,
    })
}

unsafe fn present(hwnd: HWND, state: *mut State) {
    unsafe {
        let mut latest = None;
        while let Ok(incoming) = (*state).incoming.try_recv() {
            latest = Some(incoming);
        }
        let Some(incoming) = latest else { return };
        let island = FindWindowW(wide("MonolithIsland").as_ptr(), null_mut());
        let anchor = if !island.is_null() && IsWindowVisible(island) != 0 {
            island
        } else {
            FindWindowW(wide("MonolithTopBar").as_ptr(), null_mut())
        };
        let monitor = MonitorFromWindow(
            if anchor.is_null() { hwnd } else { anchor },
            MONITOR_DEFAULTTOPRIMARY,
        );
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if GetMonitorInfoW(monitor, &mut info) == 0
            || desktop::foreground_covers_monitor(monitor, &info.rcMonitor)
        {
            // Never interrupt fullscreen work; Windows still holds the notification.
            return;
        }
        // Move first so the DPI reflects the target display.
        SetWindowPos(
            hwnd,
            null_mut(),
            info.rcWork.left,
            info.rcWork.top,
            1,
            1,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
        let dpi = GetDpiForWindow(hwnd).max(96);
        ensure_fonts(state, dpi);
        let scale = |value| overlay::scale(value, dpi);
        let work = info.rcWork;
        let width = scale(346).min((work.right - work.left - scale(16)).max(1));
        let height = scale(if incoming.body.is_empty() { 50 } else { 68 });
        let left = work.left + (work.right - work.left - width) / 2;
        let mut top = work.top + scale(6);
        let mut bounds = RECT::default();
        if anchor == island && !island.is_null() && GetWindowRect(island, &mut bounds) != 0 {
            // Below the player, which keeps its position.
            top = bounds.bottom + scale(6);
        }
        (*state).current = Some(incoming);
        SetWindowPos(
            hwnd,
            HWND_TOPMOST,
            left,
            top,
            width,
            height,
            SWP_SHOWWINDOW | SWP_NOACTIVATE,
        );
        InvalidateRect(hwnd, std::ptr::null(), 1);
        SetTimer(hwnd, HIDE, VISIBLE_MS, None);
    }
}

unsafe fn ensure_fonts(state: *mut State, dpi: u32) {
    unsafe {
        if (*state).dpi == dpi {
            return;
        }
        for (font, (size, weight)) in
            (*state)
                .fonts
                .iter_mut()
                .zip([(11, FW_NORMAL), (13, FW_SEMIBOLD), (12, FW_NORMAL)])
        {
            if !font.is_null() {
                DeleteObject(*font);
            }
            *font = CreateFontW(
                -overlay::scale(size, dpi),
                0,
                0,
                0,
                weight as i32,
                0,
                0,
                0,
                1,
                0,
                0,
                0,
                0,
                wide("Segoe UI").as_ptr(),
            );
        }
        (*state).dpi = dpi;
    }
}

/// Midpoint between text and background for secondary labels.
fn dim(text: u32, background: u32) -> u32 {
    let channel = |shift: u32| (((text >> shift) & 0xff) + ((background >> shift) & 0xff)) / 2;
    channel(0) | (channel(8) << 8) | (channel(16) << 16)
}

unsafe fn paint(hwnd: HWND, state: *mut State) {
    unsafe {
        let mut paint = PAINTSTRUCT::default();
        let hdc = BeginPaint(hwnd, &mut paint);
        if hdc.is_null() {
            return;
        }
        let palette = (*state).palette;
        let mut area = RECT::default();
        GetClientRect(hwnd, &mut area);
        overlay::fill(hdc, &area, palette.background);
        if let Some(incoming) = &(*state).current {
            let scale = |value| overlay::scale(value, (*state).dpi);
            SetBkMode(hdc, TRANSPARENT as i32);
            let left = scale(14);
            let right = area.right - scale(14);
            let mut row = scale(8);
            let lines: [(usize, u32, &str, i32, u32); 3] = [
                (
                    0,
                    dim(palette.text, palette.background),
                    &incoming.app,
                    15,
                    0,
                ),
                (1, palette.text, &incoming.title, 19, 0),
                (2, palette.text, &incoming.body, 17, 0),
            ];
            for (font, colour, text, height, _) in lines {
                if text.is_empty() {
                    continue;
                }
                SelectObject(hdc, (*state).fonts[font]);
                SetTextColor(hdc, colour);
                let mut bounds = RECT {
                    left,
                    top: row,
                    right,
                    bottom: row + scale(height),
                };
                let mut text: Vec<u16> = text.encode_utf16().collect();
                DrawTextW(
                    hdc,
                    text.as_mut_ptr(),
                    text.len() as i32,
                    &mut bounds,
                    DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX | DT_VCENTER,
                );
                row += scale(height);
            }
        }
        EndPaint(hwnd, &paint);
    }
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State;
        if state.is_null() {
            return DefWindowProcW(hwnd, msg, wp, lp);
        }
        match msg {
            READY => {
                present(hwnd, state);
                0
            }
            WM_PAINT => {
                paint(hwnd, state);
                0
            }
            WM_ERASEBKGND => 1,
            WM_TIMER if wp == HIDE => {
                KillTimer(hwnd, HIDE);
                ShowWindow(hwnd, SW_HIDE);
                (*state).current = None;
                0
            }
            WM_LBUTTONUP => {
                // Dismiss only the Monolith banner; the notification stays in Windows.
                KillTimer(hwnd, HIDE);
                ShowWindow(hwnd, SW_HIDE);
                (*state).current = None;
                0
            }
            WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uses_the_first_text_as_title_and_joins_the_rest() {
        let incoming = compose(
            " GitHub ".into(),
            vec![
                "Pull request merged".into(),
                " ".into(),
                "#482 was merged".into(),
                "into main".into(),
            ],
        )
        .unwrap();
        assert_eq!(incoming.app, "GitHub");
        assert_eq!(incoming.title, "Pull request merged");
        assert_eq!(incoming.body, "#482 was merged · into main");
        assert!(compose("App".into(), vec![" ".into()]).is_none());
    }

    #[test]
    fn dims_secondary_text_towards_the_background() {
        assert_eq!(dim(0x00ff_ffff, 0), 0x007f_7f7f);
    }
}
