//! Opt-in presentation of incoming Windows notifications below the Island.
//!
//! Windows keeps delivering and storing every notification; Monolith only reads
//! them. `UserNotificationListener` lists notifications for this unpackaged process,
//! but its change event requires package identity. Instead, an out-of-context
//! WinEvent hook notices when the Shell uncloaks its toast host window, and a worker
//! re-reads the list and presents identifiers it has not seen. Nothing is stored,
//! and Do Not Disturb suppresses the Shell toast and therefore this banner too.

use crate::{
    artwork::Artwork,
    desktop,
    overlay::{self, Palette},
    windows::wide,
};
use std::{
    cell::RefCell,
    collections::{HashSet, VecDeque},
    ffi::c_void,
    io,
    mem::size_of,
    ptr::null_mut,
    rc::Rc,
    sync::mpsc::{self, Receiver, SyncSender},
    thread::{self, JoinHandle},
    time::Duration,
};
use windows::Foundation::Size;
use windows::Storage::Streams::IRandomAccessStreamReference;
use windows::UI::Notifications::{
    KnownNotificationBindings,
    Management::{UserNotificationListener, UserNotificationListenerAccessStatus},
    NotificationKinds, UserNotification,
};
use windows::core::Interface;
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM},
    Graphics::{
        Dwm::{DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmSetWindowAttribute},
        Gdi::*,
    },
    System::SystemInformation::GetTickCount64,
    UI::{
        Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent},
        HiDpi::GetDpiForWindow,
        WindowsAndMessaging::*,
    },
};

const READY: u32 = WM_APP + 30;
/// Posted to the launcher to open the Notification Centre.
pub const OPEN: u32 = WM_APP + 31;
const HIDE: usize = 1;
const FADE: usize = 2;
const FADE_MS: u64 = 150;
const VISIBLE_MS: u32 = 6000;
const QUEUE: usize = 4;
const TOAST_HOST: &str = "Windows.UI.Core.CoreWindow";

#[derive(Debug, PartialEq, Eq)]
pub struct Incoming {
    pub app: String,
    pub title: String,
    pub body: String,
    /// The source app's own logo, when Windows provides one.
    pub icon: Option<Artwork>,
    /// The application user model ID, used to find the source app's window.
    pub app_id: String,
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
    current: Option<Rc<Incoming>>,
    /// Fading in (true) or out, and when it started.
    fade: Option<(bool, u64)>,
    /// Session-only history, newest first. Never written to disk.
    history: VecDeque<Entry>,
    centre: HWND,
    scroll: i32,
    palette: Palette,
    fonts: [HFONT; 3],
    dpi: u32,
    portuguese: bool,
}

impl Notifications {
    /// With `hide_content`, only the source app is shown and kept; never the text.
    pub fn new(
        instance: *mut c_void,
        palette: Palette,
        portuguese: bool,
        hide_content: bool,
    ) -> io::Result<Self> {
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
                overlay::EX_STYLE | WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_LAYERED,
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
            // A layered window stays invisible until it has an opacity.
            SetLayeredWindowAttributes(hwnd, 0, 255, LWA_ALPHA);
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
                .spawn(move || {
                    watch(pending, deliver, target, hide_content.then_some(portuguese))
                })?;
            let mut notifications = Self {
                hwnd,
                hook: null_mut(),
                state: Box::new(State {
                    incoming,
                    current: None,
                    fade: None,
                    history: VecDeque::new(),
                    centre: null_mut(),
                    scroll: 0,
                    palette,
                    fonts: [null_mut(); 3],
                    dpi: 0,
                    portuguese,
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
            if !self.state.centre.is_null() {
                SetWindowLongPtrW(self.state.centre, GWLP_USERDATA, 0);
                DestroyWindow(self.state.centre);
            }
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

/// `private` carries the label language when content must be withheld.
fn watch(
    checks: Receiver<()>,
    deliver: SyncSender<Incoming>,
    target: usize,
    private: Option<bool>,
) {
    use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};
    // Shell items for app icons expect a single-threaded apartment.
    if unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_err() {
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
                    && let Some(incoming) =
                        describe(&notification).map(|incoming| withhold(incoming, private))
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
    let mut incoming = compose(app, texts)?;
    incoming.app_id = notification
        .AppInfo()
        .and_then(|info| info.AppUserModelId())
        .map(|id| id.to_string())
        .unwrap_or_default();
    incoming.icon = notification
        .AppInfo()
        .and_then(|info| info.DisplayInfo())
        .and_then(|display| {
            display.GetLogo(Size {
                Width: 64.0,
                Height: 64.0,
            })
        })
        .and_then(|logo| logo.cast::<IRandomAccessStreamReference>())
        .ok()
        .and_then(|logo| Artwork::decode(&logo))
        .or_else(|| {
            let id = notification
                .AppInfo()
                .and_then(|info| info.AppUserModelId())
                .ok()?;
            shell_icon(&id.to_string())
        });
    Some(incoming)
}

/// The Shell's icon for an application identity, for desktop apps without a logo.
fn shell_icon(app_id: &str) -> Option<Artwork> {
    use windows::{
        Win32::{
            Foundation::SIZE,
            UI::Shell::{IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_ICONONLY},
        },
        core::HSTRING,
    };
    use windows_sys::Win32::Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC,
        DeleteObject, GetDIBits, HBITMAP,
    };
    const SIDE: i32 = 64;
    if app_id.is_empty() || app_id.contains(['\0', '/']) {
        return None;
    }
    let name = HSTRING::from(format!(r"shell:AppsFolder\{app_id}"));
    let factory: IShellItemImageFactory =
        unsafe { SHCreateItemFromParsingName(&name, None) }.ok()?;
    let bitmap = unsafe { factory.GetImage(SIZE { cx: SIDE, cy: SIDE }, SIIGBF_ICONONLY) }.ok()?;
    let handle = bitmap.0 as HBITMAP;
    let mut info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: SIDE,
            biHeight: -SIDE,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut pixels = vec![0_u8; (SIDE * SIDE * 4) as usize];
    let rows = unsafe {
        let dc = CreateCompatibleDC(null_mut());
        let rows = GetDIBits(
            dc,
            handle,
            0,
            SIDE as u32,
            pixels.as_mut_ptr().cast(),
            &mut info,
            DIB_RGB_COLORS,
        );
        DeleteDC(dc);
        DeleteObject(handle);
        rows
    };
    (rows == SIDE).then(|| Artwork::from_bgra(SIDE as u32, SIDE as u32, pixels))?
}

/// Replace the text with a neutral label so private content is never shown or kept.
fn withhold(incoming: Incoming, private: Option<bool>) -> Incoming {
    match private {
        None => incoming,
        Some(portuguese) => Incoming {
            title: if portuguese {
                "Nova notificação"
            } else {
                "New notification"
            }
            .to_owned(),
            body: String::new(),
            ..incoming
        },
    }
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
        icon: None,
        app_id: String::new(),
    })
}

unsafe fn present(hwnd: HWND, state: *mut State) {
    unsafe {
        let mut latest = None;
        while let Ok(incoming) = (*state).incoming.try_recv() {
            let incoming = Rc::new(incoming);
            record(state, Rc::clone(&incoming));
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
        start_fade(hwnd, state, true);
    }
}

unsafe fn start_fade(hwnd: HWND, state: *mut State, fading_in: bool) {
    unsafe {
        if !overlay::animations_enabled() {
            finish_fade(hwnd, state, fading_in);
            return;
        }
        (*state).fade = Some((fading_in, GetTickCount64()));
        if fading_in {
            SetLayeredWindowAttributes(hwnd, 0, 0, LWA_ALPHA);
        }
        SetTimer(hwnd, FADE, 15, None);
    }
}

unsafe fn step_fade(hwnd: HWND, state: *mut State) {
    unsafe {
        let Some((fading_in, started)) = (*state).fade else {
            KillTimer(hwnd, FADE);
            return;
        };
        let (alpha, finished) =
            overlay::fade_alpha(fading_in, GetTickCount64().saturating_sub(started), FADE_MS);
        if finished {
            finish_fade(hwnd, state, fading_in);
        } else {
            SetLayeredWindowAttributes(hwnd, 0, alpha, LWA_ALPHA);
        }
    }
}

/// End any fade: fully opaque when showing, hidden (and reset to opaque) otherwise.
unsafe fn finish_fade(hwnd: HWND, state: *mut State, visible: bool) {
    unsafe {
        KillTimer(hwnd, FADE);
        (*state).fade = None;
        if !visible {
            ShowWindow(hwnd, SW_HIDE);
            (*state).current = None;
        }
        SetLayeredWindowAttributes(hwnd, 0, 255, LWA_ALPHA);
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
            let mut left = scale(14);
            if let Some(icon) = &incoming.icon {
                let size = scale(28);
                let top = (area.bottom - size) / 2;
                let bounds = RECT {
                    left: scale(12),
                    top,
                    right: scale(12) + size,
                    bottom: top + size,
                };
                icon.draw(hdc, bounds, palette.background);
                left = bounds.right + scale(10);
            }
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
                start_fade(hwnd, state, false);
                0
            }
            WM_TIMER if wp == FADE => {
                step_fade(hwnd, state);
                0
            }
            WM_LBUTTONUP => {
                // Bring up the source app when one of its windows is open. The click
                // gives this process foreground rights. Windows keeps the notification.
                if let Some(target) = (*state)
                    .current
                    .as_ref()
                    .and_then(|incoming| desktop::app_window(&incoming.app_id))
                {
                    let _ = target.activate();
                }
                KillTimer(hwnd, HIDE);
                // A click hides at once; any fade in progress is cancelled.
                finish_fade(hwnd, state, false);
                0
            }
            WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

const HISTORY_LIMIT: usize = 50;
const CENTRE_CLASS: &str = "MonolithNotificationCentre";
const HEADER: i32 = 44;
const ROW: i32 = 68;

struct Entry {
    incoming: Rc<Incoming>,
    /// Local hour and minute of arrival.
    time: (u16, u16),
    unread: bool,
}

unsafe fn record(state: *mut State, incoming: Rc<Incoming>) {
    unsafe {
        let mut now = windows_sys::Win32::Foundation::SYSTEMTIME::default();
        windows_sys::Win32::System::SystemInformation::GetLocalTime(&mut now);
        let history = &mut (*state).history;
        history.push_front(Entry {
            incoming,
            time: (now.wHour, now.wMinute),
            unread: true,
        });
        history.truncate(HISTORY_LIMIT);
        publish_unread(state);
        if !(*state).centre.is_null() && IsWindowVisible((*state).centre) != 0 {
            InvalidateRect((*state).centre, std::ptr::null(), 1);
        }
    }
}

impl Notifications {
    /// Open the right-side Notification Centre at the primary display's upper right.
    pub fn open_centre(&self) {
        unsafe {
            let state = (&raw const *self.state).cast_mut();
            if (*state).centre.is_null() {
                let instance = GetWindowLongPtrW(self.hwnd, GWLP_HINSTANCE) as *mut c_void;
                let class_name = wide(CENTRE_CLASS);
                let class = WNDCLASSW {
                    hInstance: instance,
                    lpfnWndProc: Some(centre_proc),
                    hCursor: LoadCursorW(null_mut(), IDC_ARROW),
                    lpszClassName: class_name.as_ptr(),
                    ..Default::default()
                };
                RegisterClassW(&class);
                let centre = CreateWindowExW(
                    overlay::EX_STYLE | WS_EX_TOPMOST,
                    class_name.as_ptr(),
                    wide("Monolith Notification Centre").as_ptr(),
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
                if centre.is_null() {
                    return;
                }
                let corner = DWMWCP_ROUND;
                DwmSetWindowAttribute(
                    centre,
                    DWMWA_WINDOW_CORNER_PREFERENCE as u32,
                    (&raw const corner).cast(),
                    size_of::<i32>() as u32,
                );
                SetWindowLongPtrW(centre, GWLP_USERDATA, state as isize);
                (*state).centre = centre;
            }
            let centre = (*state).centre;
            let monitor = MonitorFromWindow(
                FindWindowW(wide("MonolithTopBar").as_ptr(), null_mut()),
                MONITOR_DEFAULTTOPRIMARY,
            );
            let mut info = MONITORINFO {
                cbSize: size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            if GetMonitorInfoW(monitor, &mut info) == 0 {
                return;
            }
            // Move first so the DPI reflects the target display.
            SetWindowPos(
                centre,
                null_mut(),
                info.rcWork.left,
                info.rcWork.top,
                1,
                1,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            let dpi = GetDpiForWindow(centre).max(96);
            ensure_fonts(state, dpi);
            let scale = |value| overlay::scale(value, dpi);
            let work = info.rcWork;
            let width = scale(360).min((work.right - work.left - scale(16)).max(1));
            let height = (work.bottom - work.top - scale(16)).max(1);
            (*state).scroll = 0;
            SetWindowPos(
                centre,
                HWND_TOPMOST,
                work.right - width - scale(8),
                work.top + scale(8),
                width,
                height,
                SWP_SHOWWINDOW,
            );
            SetForegroundWindow(centre);
            InvalidateRect(centre, std::ptr::null(), 1);
        }
    }
}

/// What a click selects in a list scrolled by `scroll` pixels.
#[derive(Debug, PartialEq, Eq)]
enum Target {
    ClearAll,
    Entry(usize),
    Nothing,
}

fn target(x: i32, y: i32, width: i32, scroll: i32, entries: usize, dpi: u32) -> Target {
    let scale = |value| overlay::scale(value, dpi);
    if y < scale(HEADER) {
        // "Clear all" occupies the right part of the header.
        return if x > width - scale(110) && entries > 0 {
            Target::ClearAll
        } else {
            Target::Nothing
        };
    }
    let index = ((y - scale(HEADER) + scroll) / scale(ROW)) as usize;
    if index < entries {
        Target::Entry(index)
    } else {
        Target::Nothing
    }
}

/// Tell the Top Bar how many notifications are unread.
unsafe fn publish_unread(state: *mut State) {
    unsafe {
        let count = (*state).history.iter().filter(|entry| entry.unread).count();
        let bar = FindWindowW(wide("MonolithTopBar").as_ptr(), null_mut());
        if !bar.is_null() {
            PostMessageW(bar, crate::top_bar::UNREAD, count, 0);
        }
    }
}

unsafe fn close_centre(hwnd: HWND, state: *mut State) {
    unsafe {
        ShowWindow(hwnd, SW_HIDE);
        for entry in &mut (*state).history {
            entry.unread = false;
        }
        publish_unread(state);
    }
}

unsafe fn text(hdc: HDC, font: HFONT, colour: u32, value: &str, mut bounds: RECT, flags: u32) {
    unsafe {
        if value.is_empty() {
            return;
        }
        SelectObject(hdc, font);
        SetTextColor(hdc, colour);
        let mut wide: Vec<u16> = value.encode_utf16().collect();
        DrawTextW(
            hdc,
            wide.as_mut_ptr(),
            wide.len() as i32,
            &mut bounds,
            flags | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX | DT_VCENTER,
        );
    }
}

unsafe fn paint_centre(hwnd: HWND, state: *mut State) {
    unsafe {
        let mut paint = PAINTSTRUCT::default();
        let hdc = BeginPaint(hwnd, &mut paint);
        if hdc.is_null() {
            return;
        }
        let palette = (*state).palette;
        let secondary = dim(palette.text, palette.background);
        let scale = |value| overlay::scale(value, (*state).dpi);
        let fonts = (*state).fonts;
        let portuguese = (*state).portuguese;
        let mut area = RECT::default();
        GetClientRect(hwnd, &mut area);
        overlay::fill(hdc, &area, palette.background);
        SetBkMode(hdc, TRANSPARENT as i32);
        let pad = scale(14);
        let header = RECT {
            left: pad,
            top: 0,
            right: area.right - pad,
            bottom: scale(HEADER),
        };
        let heading = if portuguese {
            "Notificações"
        } else {
            "Notifications"
        };
        text(hdc, fonts[1], palette.text, heading, header, DT_LEFT);
        let history = &(*state).history;
        if history.is_empty() {
            let empty = RECT {
                top: scale(HEADER),
                bottom: scale(HEADER + ROW),
                ..header
            };
            let label = if portuguese {
                "Nenhuma notificação"
            } else {
                "No notifications"
            };
            text(hdc, fonts[2], secondary, label, empty, DT_LEFT);
        } else {
            let label = if portuguese {
                "Limpar tudo"
            } else {
                "Clear all"
            };
            text(hdc, fonts[2], palette.accent, label, header, DT_RIGHT);
        }
        // Rows scroll beneath the header; clip so they never draw over it.
        IntersectClipRect(hdc, 0, scale(HEADER), area.right, area.bottom);
        for (index, entry) in history.iter().enumerate() {
            let top = scale(HEADER) + index as i32 * scale(ROW) - (*state).scroll;
            if top > area.bottom {
                break;
            }
            if top + scale(ROW) < scale(HEADER) {
                continue;
            }
            let row = RECT {
                left: scale(6),
                top: top + scale(3),
                right: area.right - scale(6),
                bottom: top + scale(ROW) - scale(3),
            };
            overlay::fill(hdc, &row, palette.surface);
            if entry.unread {
                let marker = RECT {
                    left: row.left,
                    top: row.top + scale(10),
                    right: row.left + scale(3),
                    bottom: row.bottom - scale(10),
                };
                overlay::fill(hdc, &marker, palette.accent);
            }
            let mut left = row.left + pad;
            if let Some(icon) = &entry.incoming.icon {
                let size = scale(28);
                let icon_top = row.top + (row.bottom - row.top - size) / 2;
                let bounds = RECT {
                    left: row.left + scale(12),
                    top: icon_top,
                    right: row.left + scale(12) + size,
                    bottom: icon_top + size,
                };
                icon.draw(hdc, bounds, palette.surface);
                left = bounds.right + scale(10);
            }
            let right = row.right - scale(10);
            let line = |offset: i32, height: i32| RECT {
                left,
                top: row.top + scale(offset),
                right,
                bottom: row.top + scale(offset + height),
            };
            let (hour, minute) = entry.time;
            let time = format!("{hour:02}:{minute:02}");
            text(hdc, fonts[0], secondary, &time, line(5, 15), DT_RIGHT);
            let mut app = line(5, 15);
            app.right -= scale(44);
            text(hdc, fonts[0], secondary, &entry.incoming.app, app, DT_LEFT);
            let title = &entry.incoming.title;
            text(hdc, fonts[1], palette.text, title, line(20, 19), DT_LEFT);
            let body = &entry.incoming.body;
            text(hdc, fonts[2], palette.text, body, line(39, 17), DT_LEFT);
        }
        EndPaint(hwnd, &paint);
    }
}

unsafe fn clamp_scroll(hwnd: HWND, state: *mut State) {
    unsafe {
        let scale = |value| overlay::scale(value, (*state).dpi);
        let mut area = RECT::default();
        GetClientRect(hwnd, &mut area);
        let content = (*state).history.len() as i32 * scale(ROW);
        let visible = (area.bottom - scale(HEADER)).max(1);
        (*state).scroll = (*state).scroll.clamp(0, (content - visible).max(0));
    }
}

unsafe extern "system" fn centre_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State;
        if state.is_null() {
            return DefWindowProcW(hwnd, msg, wp, lp);
        }
        match msg {
            WM_PAINT => {
                paint_centre(hwnd, state);
                0
            }
            WM_ERASEBKGND => 1,
            WM_LBUTTONUP => {
                let mut area = RECT::default();
                GetClientRect(hwnd, &mut area);
                let x = (lp & 0xffff) as i16 as i32;
                let y = ((lp >> 16) & 0xffff) as i16 as i32;
                let (scroll, dpi) = ((*state).scroll, (*state).dpi);
                let history = &mut (*state).history;
                match target(x, y, area.right, scroll, history.len(), dpi) {
                    Target::ClearAll => history.clear(),
                    // Dismissing removes the entry from Monolith; Windows keeps its copy.
                    Target::Entry(index) => drop(history.remove(index)),
                    Target::Nothing => return 0,
                }
                publish_unread(state);
                clamp_scroll(hwnd, state);
                InvalidateRect(hwnd, std::ptr::null(), 1);
                0
            }
            WM_MOUSEWHEEL => {
                let delta = ((wp >> 16) & 0xffff) as i16 as i32;
                (*state).scroll -= delta * overlay::scale(ROW, (*state).dpi) / 120;
                clamp_scroll(hwnd, state);
                InvalidateRect(hwnd, std::ptr::null(), 1);
                0
            }
            WM_KEYDOWN
                if wp as u16 == windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE =>
            {
                close_centre(hwnd, state);
                0
            }
            WM_CLOSE => {
                // Hide rather than destroy: the window is reused and owned by `Notifications`.
                close_centre(hwnd, state);
                0
            }
            WM_ACTIVATE if wp as u16 == WA_INACTIVE as u16 => {
                close_centre(hwnd, state);
                0
            }
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

    #[test]
    #[ignore = "resolves an icon through the interactive Shell"]
    fn resolves_a_desktop_app_icon_through_the_shell() {
        use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok().unwrap() };
        let id = r"{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\WindowsPowerShell\v1.0\powershell.exe";
        assert!(shell_icon(id).is_some());
    }

    #[test]
    fn targets_clear_all_and_scrolled_rows() {
        // 96 DPI: header 44 px, rows 68 px, panel 360 px wide.
        assert_eq!(target(300, 20, 360, 0, 3, 96), Target::ClearAll);
        assert_eq!(target(300, 20, 360, 0, 0, 96), Target::Nothing);
        assert_eq!(target(40, 20, 360, 0, 3, 96), Target::Nothing);
        assert_eq!(target(40, 50, 360, 0, 3, 96), Target::Entry(0));
        assert_eq!(target(40, 50, 360, 68, 3, 96), Target::Entry(1));
        assert_eq!(target(40, 44 + 3 * 68, 360, 0, 3, 96), Target::Nothing);
    }

    #[test]
    fn withholds_text_but_keeps_the_source_app() {
        let incoming = compose("Signal".into(), vec!["Ana".into(), "See you at 8".into()]).unwrap();
        let hidden = withhold(incoming, Some(false));
        assert_eq!(hidden.app, "Signal");
        assert_eq!(hidden.title, "New notification");
        assert!(hidden.body.is_empty());
        let shown = withhold(compose("A".into(), vec!["T".into()]).unwrap(), None);
        assert_eq!(shown.title, "T");
    }

    #[test]
    fn fades_linearly_in_and_out() {
        assert_eq!(overlay::fade_alpha(true, 0, FADE_MS), (0, false));
        assert_eq!(overlay::fade_alpha(true, FADE_MS / 2, FADE_MS).0, 127);
        assert_eq!(overlay::fade_alpha(true, FADE_MS, FADE_MS), (255, true));
        assert_eq!(overlay::fade_alpha(false, 0, FADE_MS), (255, false));
        assert_eq!(overlay::fade_alpha(false, FADE_MS * 3, FADE_MS), (0, true));
    }
}
