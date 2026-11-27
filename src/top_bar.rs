//! Opt-in borderless status overlay with native clock and power data.

use crate::{
    desktop, island,
    overlay::{self, Palette},
    windows::{shell_menu, wide},
};
use std::{cell::Cell, ffi::c_void, io, mem::size_of, ptr::null_mut};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, SYSTEMTIME, WPARAM},
    Graphics::Gdi::*,
    System::{
        Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS},
        SystemInformation::{GetLocalTime, GetTickCount64},
        SystemServices::{SS_CENTERIMAGE, SS_RIGHT},
    },
    UI::{
        Accessibility::*,
        Controls::{DRAWITEMSTRUCT, ODS_SELECTED},
        HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi},
        Shell::*,
        WindowsAndMessaging::*,
    },
};

const CLOCK: usize = 1;
const LAYOUT: usize = 2;
const REVEAL: usize = 3;
const FADE: usize = 4;
const FADE_MS: u64 = 120;
const APPBAR_NOTIFICATION: u32 = WM_APP + 20;
/// Posted by the notification surface with the unread count in `wParam`.
pub const UNREAD: u32 = WM_APP + 32;
const CONTROL_BUTTON: usize = 10;
const BELL_BUTTON: usize = 11;
// Segoe Fluent Icons code points for quick-settings sliders and the bell.
const CONTROL_GLYPH: &str = "\u{E9E9}";
const BELL_GLYPH: &str = "\u{EA8F}";

thread_local! {
    static BAR_WINDOW: Cell<HWND> = const { Cell::new(null_mut()) };
    static COLLAPSED: Cell<bool> = const { Cell::new(false) };
    static COLLAPSED_SHOWN: Cell<bool> = const { Cell::new(false) };
    static LAYOUT_PENDING: Cell<bool> = const { Cell::new(false) };
    static SHELL_WINDOW: Cell<HWND> = const { Cell::new(null_mut()) };
}

pub struct TopBar {
    hwnd: HWND,
    hooks: Vec<HWINEVENTHOOK>,
    state: Box<State>,
}

struct State {
    launcher: HWND,
    status: HWND,
    /// Control Center and Notification Centre access, at the right end.
    buttons: [HWND; 2],
    icon_font: HFONT,
    unread: usize,
    palette: Palette,
    portuguese: bool,
    brush: HBRUSH,
    font: HFONT,
    dpi: u32,
    registered: bool,
    reservation: Option<[i32; 4]>,
    taskbar_created: u32,
    shell: HWND,
    region_size: Option<(i32, i32)>,
    recovery_attempts: u8,
    /// The foreground window draws into the strip; the bar waits at the top edge.
    covered: bool,
    /// The pointer revealed the bar over that window.
    revealed: bool,
    /// A reveal or collapse fade in progress: fading in, and when it started.
    fade: Option<(bool, u64)>,
}

/// The bar is collapsed to its edge strip; hosted surfaces such as the Island hide too.
pub fn collapsed() -> bool {
    COLLAPSED.get()
}

impl TopBar {
    pub fn handle(&self) -> HWND {
        self.hwnd
    }

    pub fn new(
        instance: *mut c_void,
        launcher: HWND,
        palette: Palette,
        portuguese: bool,
    ) -> io::Result<Self> {
        // The shell owns the bar on its existing GUI thread.
        unsafe {
            let name = wide("MonolithTopBar");
            let class = WNDCLASSW {
                lpszClassName: name.as_ptr(),
                hInstance: instance,
                lpfnWndProc: Some(window_proc),
                hCursor: LoadCursorW(null_mut(), IDC_ARROW),
                ..Default::default()
            };
            if RegisterClassW(&class) == 0 {
                return Err(io::Error::last_os_error());
            }
            let hwnd = CreateWindowExW(
                overlay::EX_STYLE | WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_LAYERED,
                name.as_ptr(),
                wide("Monolith Top Bar").as_ptr(),
                overlay::STYLE | WS_CLIPCHILDREN,
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
            let mut bar = Self {
                hwnd,
                hooks: Vec::new(),
                state: Box::new(State {
                    launcher,
                    palette,
                    portuguese,
                    brush: CreateSolidBrush(palette.background),
                    status: null_mut(),
                    buttons: [null_mut(); 2],
                    icon_font: null_mut(),
                    unread: 0,
                    font: null_mut(),
                    dpi: 0,
                    registered: false,
                    reservation: None,
                    taskbar_created: RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()),
                    shell: GetShellWindow(),
                    region_size: None,
                    recovery_attempts: 0,
                    covered: false,
                    revealed: false,
                    fade: None,
                }),
            };
            if bar.state.brush.is_null() {
                return Err(io::Error::last_os_error());
            }
            let status = CreateWindowExW(
                0,
                wide("STATIC").as_ptr(),
                std::ptr::null(),
                WS_CHILD | WS_VISIBLE | SS_RIGHT | SS_CENTERIMAGE,
                0,
                0,
                0,
                0,
                hwnd,
                null_mut(),
                instance,
                null_mut(),
            );
            if status.is_null() {
                return Err(io::Error::last_os_error());
            }
            bar.state.status = status;
            let labels = if portuguese {
                ["Central de Controle", "Notificações"]
            } else {
                ["Control Center", "Notifications"]
            };
            for (index, (id, label)) in [CONTROL_BUTTON, BELL_BUTTON]
                .into_iter()
                .zip(labels)
                .enumerate()
            {
                // The caption is the accessible name; the glyph is owner-drawn.
                let button = CreateWindowExW(
                    0,
                    wide("BUTTON").as_ptr(),
                    wide(label).as_ptr(),
                    WS_CHILD | WS_VISIBLE | BS_OWNERDRAW as u32,
                    0,
                    0,
                    0,
                    0,
                    hwnd,
                    id as _,
                    instance,
                    null_mut(),
                );
                if button.is_null() {
                    return Err(io::Error::last_os_error());
                }
                bar.state.buttons[index] = button;
            }
            let state = &raw mut *bar.state;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, state as isize);
            BAR_WINDOW.set(hwnd);
            SHELL_WINDOW.set((*state).shell);
            for event in [
                EVENT_SYSTEM_FOREGROUND,
                EVENT_OBJECT_LOCATIONCHANGE,
                EVENT_OBJECT_SHOW,
                EVENT_OBJECT_DESTROY,
            ] {
                let hook = SetWinEventHook(
                    event,
                    event,
                    null_mut(),
                    Some(foreground_event),
                    0,
                    0,
                    WINEVENT_OUTOFCONTEXT,
                );
                if hook.is_null() {
                    return Err(io::Error::last_os_error());
                }
                bar.hooks.push(hook);
            }
            place(hwnd, state);
            if (*state).reservation.is_none() {
                return Err(io::Error::other(
                    "Windows did not reserve the Top Bar work area",
                ));
            }
            Ok(bar)
        }
    }
}

impl Drop for TopBar {
    fn drop(&mut self) {
        BAR_WINDOW.set(null_mut());
        SHELL_WINDOW.set(null_mut());
        LAYOUT_PENDING.set(false);
        unsafe {
            release_reservation(self.hwnd, &raw mut *self.state);
            for hook in self.hooks.drain(..) {
                UnhookWinEvent(hook);
            }
            DestroyWindow(self.hwnd);
            DeleteObject(self.state.brush);
            DeleteObject(self.state.font);
            DeleteObject(self.state.icon_font);
        }
    }
}

fn bar_data(hwnd: HWND) -> APPBARDATA {
    APPBARDATA {
        cbSize: size_of::<APPBARDATA>() as u32,
        hWnd: hwnd,
        ..Default::default()
    }
}

fn appbar(hwnd: HWND, message: u32) {
    unsafe { SHAppBarMessage(message, &mut bar_data(hwnd)) };
}

unsafe fn release_reservation(hwnd: HWND, state: *mut State) {
    unsafe {
        if (*state).registered {
            (*state).registered = false;
            (*state).reservation = None;
            appbar(hwnd, ABM_REMOVE);
        }
    }
}

unsafe fn reserve(hwnd: HWND, state: *mut State, monitor: RECT, height: i32) -> Option<RECT> {
    unsafe {
        let mut data = bar_data(hwnd);
        if !(*state).registered {
            data.uCallbackMessage = APPBAR_NOTIFICATION;
            if SHAppBarMessage(ABM_NEW, &mut data) == 0 {
                return None;
            }
            (*state).registered = true;
        }
        data.uEdge = ABE_TOP;
        data.rc = RECT {
            bottom: monitor.top + height,
            ..monitor
        };
        if SHAppBarMessage(ABM_QUERYPOS, &mut data) == 0 {
            return None;
        }
        data.rc.bottom = data.rc.top + height;
        let desired = [data.rc.left, data.rc.top, data.rc.right, data.rc.bottom];
        if (*state).reservation != Some(desired) {
            if SHAppBarMessage(ABM_SETPOS, &mut data) == 0 {
                return None;
            }
            (*state).reservation = Some([data.rc.left, data.rc.top, data.rc.right, data.rc.bottom]);
        }
        Some(data.rc)
    }
}

unsafe extern "system" fn foreground_event(
    _: HWINEVENTHOOK,
    event: u32,
    window: HWND,
    object: i32,
    child: i32,
    _: u32,
    _: u32,
) {
    if object == OBJID_WINDOW && child == 0 && !window.is_null() {
        let shell_event = matches!(event, EVENT_OBJECT_SHOW | EVENT_OBJECT_DESTROY)
            && unsafe { GetShellWindow() } != SHELL_WINDOW.get();
        let bar = BAR_WINDOW.get();
        if !bar.is_null() && (shell_event || unsafe { GetForegroundWindow() } == window) {
            schedule_layout(bar);
        }
    }
}

fn schedule_layout(hwnd: HWND) {
    if !LAYOUT_PENDING.get() && unsafe { SetTimer(hwnd, LAYOUT, 100, None) } != 0 {
        LAYOUT_PENDING.set(true);
    }
}

unsafe fn hide(hwnd: HWND) {
    unsafe {
        KillTimer(hwnd, CLOCK);
        if ShowWindow(hwnd, SW_HIDE) != 0 {
            island::host_changed(hwnd);
        }
    }
}

unsafe fn place(hwnd: HWND, state: *mut State) {
    unsafe {
        let shell = GetShellWindow();
        if shell != (*state).shell {
            release_reservation(hwnd, state);
            (*state).shell = shell;
            SHELL_WINDOW.set(shell);
            (*state).recovery_attempts = 25;
        }
        let monitor = MonitorFromPoint(POINT::default(), MONITOR_DEFAULTTOPRIMARY);
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if GetMonitorInfoW(monitor, &mut info) == 0 {
            release_reservation(hwnd, state);
            hide(hwnd);
            return;
        }
        if MonitorFromWindow(hwnd, MONITOR_DEFAULTTONULL) != monitor {
            SetWindowPos(
                hwnd,
                null_mut(),
                info.rcMonitor.left,
                info.rcMonitor.top,
                1,
                1,
                SWP_HIDEWINDOW | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
        let dpi = GetDpiForWindow(hwnd);
        let scale = |value| overlay::scale(value, dpi);
        let strip_height = scale(27);
        let resize_margin = GetSystemMetricsForDpi(SM_CYSIZEFRAME, dpi)
            + GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi);
        let Some(bounds) = reserve(hwnd, state, info.rcMonitor, strip_height + resize_margin)
        else {
            release_reservation(hwnd, state);
            hide(hwnd);
            if (*state).recovery_attempts > 0 {
                (*state).recovery_attempts -= 1;
                schedule_layout(hwnd);
            }
            return;
        };
        (*state).recovery_attempts = 0;
        let strip = RECT {
            bottom: bounds.top + strip_height,
            ..bounds
        };
        // When the foreground window draws into the strip, collapse to an invisible
        // edge strip that the pointer reveals, rather than vanishing.
        let covered = desktop::foreground_covers_top_strip(monitor, &info.rcMonitor, &strip);
        if covered != (*state).covered {
            (*state).covered = covered;
            (*state).revealed = false;
        }
        let collapsed = covered && !(*state).revealed;
        // Hosted surfaces hide as soon as the bar starts collapsing, and appear only
        // once it is fully revealed.
        if collapsed {
            COLLAPSED.set(true);
        }
        let animate = overlay::animations_enabled() && IsWindowVisible(hwnd) != 0;
        if (*state).dpi != dpi {
            let font = CreateFontW(
                -scale(13),
                0,
                0,
                0,
                FW_NORMAL as i32,
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
            let icon_font = CreateFontW(
                -scale(14),
                0,
                0,
                0,
                FW_NORMAL as i32,
                0,
                0,
                0,
                1,
                0,
                0,
                0,
                0,
                wide("Segoe Fluent Icons").as_ptr(),
            );
            if !font.is_null() {
                SendMessageW((*state).status, WM_SETFONT, font as usize, 0);
                DeleteObject((*state).font);
                (*state).font = font;
                (*state).dpi = dpi;
            }
            if !icon_font.is_null() {
                DeleteObject((*state).icon_font);
                (*state).icon_font = icon_font;
            }
        }
        let inset = scale(8);
        let width = (bounds.right - bounds.left).max(1);
        let height = (bounds.bottom - bounds.top).max(1);
        // Keep the reservation beyond the visible strip for maximised custom frames.
        // Keep the full strip while fading out; shrink to the edge once collapsed.
        let shrink = collapsed && (COLLAPSED_SHOWN.get() || !animate);
        let visible_height = if shrink { scale(3) } else { strip_height };
        if (*state).region_size != Some((width, visible_height)) {
            let region = CreateRectRgn(0, 0, width, visible_height);
            if region.is_null() || SetWindowRgn(hwnd, region, 1) == 0 {
                DeleteObject(region);
                release_reservation(hwnd, state);
                hide(hwnd);
                return;
            }
            (*state).region_size = Some((width, visible_height));
        }
        let button_width = scale(30);
        let buttons_left = width - inset - 2 * button_width;
        MoveWindow(
            (*state).status,
            inset,
            0,
            (buttons_left - scale(6) - inset).max(1),
            strip_height,
            0,
        );
        for (index, &button) in (*state).buttons.iter().enumerate() {
            MoveWindow(
                button,
                buttons_left + index as i32 * button_width,
                0,
                button_width,
                strip_height,
                0,
            );
        }
        let was_visible = IsWindowVisible(hwnd) != 0;
        let mut previous = RECT::default();
        let needs_position = !was_visible
            || GetWindowRect(hwnd, &mut previous) == 0
            || (previous.left, previous.top, previous.right, previous.bottom)
                != (bounds.left, bounds.top, bounds.right, bounds.bottom);
        if needs_position
            && SetWindowPos(
                hwnd,
                HWND_TOPMOST,
                bounds.left,
                bounds.top,
                width,
                height,
                SWP_SHOWWINDOW | SWP_NOACTIVATE,
            ) == 0
        {
            release_reservation(hwnd, state);
            hide(hwnd);
            return;
        }
        if !was_visible {
            refresh_status(hwnd, state);
        }
        if needs_position {
            island::host_changed(hwnd);
        }
        if collapsed != COLLAPSED_SHOWN.get() && animate {
            if (*state)
                .fade
                .is_none_or(|(fading_in, _)| fading_in == collapsed)
            {
                if !collapsed {
                    // Paint every part while invisible so the whole bar fades in together.
                    SetLayeredWindowAttributes(hwnd, 0, 1, LWA_ALPHA);
                    RedrawWindow(
                        hwnd,
                        std::ptr::null(),
                        null_mut(),
                        RDW_INVALIDATE | RDW_UPDATENOW | RDW_ALLCHILDREN,
                    );
                }
                (*state).fade = Some((!collapsed, GetTickCount64()));
                SetTimer(hwnd, FADE, 15, None);
            }
        } else if (*state).fade.is_none() {
            SetLayeredWindowAttributes(hwnd, 0, if collapsed { 1 } else { 255 }, LWA_ALPHA);
            COLLAPSED.set(collapsed);
            if collapsed != COLLAPSED_SHOWN.replace(collapsed) {
                island::host_changed(hwnd);
            }
        }
    }
}

fn minute_delay(seconds: u16, milliseconds: u16) -> u32 {
    60_000_u32
        .saturating_sub(u32::from(seconds) * 1_000 + u32::from(milliseconds))
        .max(10)
}

fn status_text(time: SYSTEMTIME, power: Option<SYSTEM_POWER_STATUS>, portuguese: bool) -> String {
    let date = if portuguese {
        format!("{:02}/{:02}/{:04}", time.wDay, time.wMonth, time.wYear)
    } else {
        format!("{:04}-{:02}-{:02}", time.wYear, time.wMonth, time.wDay)
    };
    let time = format!("{date}    {:02}:{:02}", time.wHour, time.wMinute);
    let Some(power) = power.filter(|power| power.BatteryLifePercent <= 100) else {
        return time;
    };
    let charging = power.BatteryFlag != u8::MAX && power.BatteryFlag & 8 != 0;
    let suffix = match (charging, power.ACLineStatus, portuguese) {
        (true, _, true) => " carregando",
        (true, _, false) => " charging",
        (false, 1, _) => " AC",
        _ => "",
    };
    format!("{}%{suffix}    {time}", power.BatteryLifePercent)
}

unsafe fn refresh_status(hwnd: HWND, state: *mut State) {
    unsafe {
        KillTimer(hwnd, CLOCK);
        if IsWindowVisible(hwnd) == 0 {
            return;
        }
        let mut time = SYSTEMTIME::default();
        GetLocalTime(&mut time);
        let mut power = SYSTEM_POWER_STATUS::default();
        let power = (GetSystemPowerStatus(&mut power) != 0).then_some(power);
        let status = wide(&status_text(time, power, (*state).portuguese));
        SetWindowTextW((*state).status, status.as_ptr());
        SetTimer(
            hwnd,
            CLOCK,
            minute_delay(time.wSecond, time.wMilliseconds),
            None,
        );
    }
}

unsafe fn draw_button(state: *mut State, item: &DRAWITEMSTRUCT) {
    unsafe {
        let palette = (*state).palette;
        let pressed = item.itemState & ODS_SELECTED != 0;
        let area = item.rcItem;
        overlay::fill(
            item.hDC,
            &area,
            if pressed {
                palette.pressed
            } else {
                palette.background
            },
        );
        let saved = SaveDC(item.hDC);
        SelectObject(item.hDC, (*state).icon_font);
        SetBkMode(item.hDC, TRANSPARENT as i32);
        SetTextColor(item.hDC, palette.text);
        let bell = item.CtlID as usize == BELL_BUTTON;
        let mut glyph: Vec<u16> = if bell { BELL_GLYPH } else { CONTROL_GLYPH }
            .encode_utf16()
            .collect();
        let mut bounds = area;
        DrawTextW(
            item.hDC,
            glyph.as_mut_ptr(),
            glyph.len() as i32,
            &mut bounds,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
        );
        if bell && (*state).unread > 0 {
            // A small blue dot marks unread notifications.
            let size = overlay::scale(5, (*state).dpi);
            let centre = area.left + (area.right - area.left) / 2;
            let dot = RECT {
                left: centre + size,
                top: area.top + overlay::scale(5, (*state).dpi),
                right: centre + 2 * size,
                bottom: area.top + overlay::scale(5, (*state).dpi) + size,
            };
            overlay::fill(item.hDC, &dot, palette.accent);
        }
        RestoreDC(item.hDC, saved);
    }
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State;
        if state.is_null() {
            return DefWindowProcW(hwnd, msg, wp, lp);
        }
        match msg {
            event if event == (*state).taskbar_created => {
                (*state).recovery_attempts = 25;
                schedule_layout(hwnd);
                0
            }
            APPBAR_NOTIFICATION => {
                if matches!(
                    wp as u32,
                    ABN_POSCHANGED | ABN_FULLSCREENAPP | ABN_STATECHANGE
                ) {
                    schedule_layout(hwnd);
                }
                0
            }
            WM_WINDOWPOSCHANGED | WM_ACTIVATE => {
                if (*state).registered {
                    appbar(
                        hwnd,
                        if msg == WM_ACTIVATE {
                            ABM_ACTIVATE
                        } else {
                            ABM_WINDOWPOSCHANGED
                        },
                    );
                }
                DefWindowProcW(hwnd, msg, wp, lp)
            }
            WM_CLOSE => {
                PostMessageW((*state).launcher, WM_CLOSE, 0, 0);
                0
            }
            WM_CONTEXTMENU => {
                shell_menu(hwnd, (*state).launcher, (*state).portuguese);
                0
            }
            WM_TIMER if wp == CLOCK => {
                refresh_status(hwnd, state);
                0
            }
            WM_TIMER if wp == LAYOUT => {
                KillTimer(hwnd, LAYOUT);
                LAYOUT_PENDING.set(false);
                place(hwnd, state);
                0
            }
            WM_TIMECHANGE => {
                refresh_status(hwnd, state);
                0
            }
            WM_POWERBROADCAST => {
                refresh_status(hwnd, state);
                1
            }
            WM_DPICHANGED | WM_DISPLAYCHANGE | WM_SETTINGCHANGE => {
                schedule_layout(hwnd);
                0
            }
            WM_ERASEBKGND => {
                let mut rect = RECT::default();
                GetClientRect(hwnd, &mut rect);
                FillRect(wp as HDC, &rect, (*state).brush);
                1
            }
            WM_COMMAND if (wp >> 16) as u32 == BN_CLICKED => {
                // The click gives this process the foreground rights the panels need.
                let target = match wp & 0xffff {
                    CONTROL_BUTTON => crate::control_center::OPEN,
                    BELL_BUTTON => crate::notifications::OPEN,
                    _ => return 0,
                };
                PostMessageW((*state).launcher, target, hwnd as usize, 0);
                0
            }
            UNREAD => {
                (*state).unread = wp;
                InvalidateRect((*state).buttons[1], std::ptr::null(), 0);
                0
            }
            WM_DRAWITEM if lp != 0 => {
                draw_button(state, &*(lp as *const DRAWITEMSTRUCT));
                1
            }
            WM_CTLCOLORSTATIC => {
                SetBkColor(wp as HDC, (*state).palette.background);
                SetTextColor(wp as HDC, (*state).palette.text);
                (*state).brush as isize
            }
            WM_MOUSEMOVE if (*state).covered && !(*state).revealed => {
                (*state).revealed = true;
                place(hwnd, state);
                SetTimer(hwnd, REVEAL, 400, None);
                0
            }
            WM_TIMER if wp == FADE => {
                let Some((fading_in, started)) = (*state).fade else {
                    KillTimer(hwnd, FADE);
                    return 0;
                };
                let elapsed = GetTickCount64().saturating_sub(started);
                let (alpha, finished) = overlay::fade_alpha(fading_in, elapsed, FADE_MS);
                SetLayeredWindowAttributes(hwnd, 0, alpha.max(1), LWA_ALPHA);
                if finished {
                    KillTimer(hwnd, FADE);
                    (*state).fade = None;
                    COLLAPSED_SHOWN.set(!fading_in);
                    COLLAPSED.set(!fading_in);
                    island::host_changed(hwnd);
                    if !fading_in {
                        // Shrink to the edge strip now that the bar is invisible.
                        place(hwnd, state);
                    }
                }
                0
            }
            WM_TIMER if wp == REVEAL => {
                let mut cursor = POINT::default();
                let mut bounds = RECT::default();
                let hovered = GetCursorPos(&mut cursor) != 0
                    && GetWindowRect(hwnd, &mut bounds) != 0
                    && cursor.y < bounds.top + overlay::scale(34, (*state).dpi.max(96))
                    && cursor.x >= bounds.left
                    && cursor.x < bounds.right;
                if !hovered {
                    KillTimer(hwnd, REVEAL);
                    (*state).revealed = false;
                    place(hwnd, state);
                }
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
    fn aligns_updates_to_the_next_minute() {
        assert_eq!(minute_delay(0, 0), 60_000);
        assert_eq!(minute_delay(59, 750), 250);
    }

    #[test]
    fn displays_known_battery_status_and_omits_unknown_percentages() {
        let time = SYSTEMTIME {
            wYear: 2026,
            wMonth: 11,
            wDay: 26,
            wHour: 9,
            wMinute: 3,
            ..Default::default()
        };
        let charging = SYSTEM_POWER_STATUS {
            ACLineStatus: 1,
            BatteryFlag: 8,
            BatteryLifePercent: 75,
            ..Default::default()
        };
        assert_eq!(
            status_text(time, Some(charging), false),
            "75% charging    2026-11-26    09:03"
        );
        let unknown = SYSTEM_POWER_STATUS {
            BatteryLifePercent: 255,
            ..charging
        };
        assert_eq!(
            status_text(time, Some(unknown), false),
            "2026-11-26    09:03"
        );
        assert_eq!(status_text(time, None, true), "26/11/2026    09:03");
    }
}
