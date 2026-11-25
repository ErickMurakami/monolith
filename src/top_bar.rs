//! Opt-in borderless status overlay with native clock and power data.

use crate::{
    desktop,
    overlay::{self, Palette},
    windows::{shell_menu, wide},
};
use std::{cell::Cell, ffi::c_void, io, mem::size_of, ptr::null_mut};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, SYSTEMTIME, WPARAM},
    Graphics::Gdi::*,
    System::{
        Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS},
        SystemInformation::GetLocalTime,
        SystemServices::{SS_CENTERIMAGE, SS_RIGHT},
    },
    UI::{Accessibility::*, HiDpi::GetDpiForWindow, WindowsAndMessaging::*},
};

const CLOCK: usize = 1;
const LAYOUT: usize = 2;

thread_local! {
    static BAR_WINDOW: Cell<HWND> = const { Cell::new(null_mut()) };
    static LAYOUT_PENDING: Cell<bool> = const { Cell::new(false) };
}

pub struct TopBar {
    hwnd: HWND,
    hooks: Vec<HWINEVENTHOOK>,
    state: Box<State>,
}

struct State {
    launcher: HWND,
    status: HWND,
    palette: Palette,
    portuguese: bool,
    brush: HBRUSH,
    font: HFONT,
    dpi: u32,
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
                overlay::EX_STYLE | WS_EX_TOPMOST | WS_EX_NOACTIVATE,
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
                    font: null_mut(),
                    dpi: 0,
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
            let state = &raw mut *bar.state;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, state as isize);
            BAR_WINDOW.set(hwnd);
            for event in [EVENT_SYSTEM_FOREGROUND, EVENT_OBJECT_LOCATIONCHANGE] {
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
            Ok(bar)
        }
    }
}

impl Drop for TopBar {
    fn drop(&mut self) {
        BAR_WINDOW.set(null_mut());
        LAYOUT_PENDING.set(false);
        unsafe {
            for hook in self.hooks.drain(..) {
                UnhookWinEvent(hook);
            }
            DestroyWindow(self.hwnd);
            DeleteObject(self.state.brush);
            DeleteObject(self.state.font);
        }
    }
}

unsafe extern "system" fn foreground_event(
    _: HWINEVENTHOOK,
    _: u32,
    window: HWND,
    object: i32,
    child: i32,
    _: u32,
    _: u32,
) {
    if object == OBJID_WINDOW
        && child == 0
        && !window.is_null()
        && unsafe { GetForegroundWindow() } == window
    {
        let bar = BAR_WINDOW.get();
        if !bar.is_null() {
            schedule_layout(bar);
        }
    }
}

fn schedule_layout(hwnd: HWND) {
    if !LAYOUT_PENDING.get() && unsafe { SetTimer(hwnd, LAYOUT, 100, None) } != 0 {
        LAYOUT_PENDING.set(true);
    }
}

unsafe fn place(hwnd: HWND, state: *mut State) {
    unsafe {
        let monitor = MonitorFromPoint(POINT::default(), MONITOR_DEFAULTTOPRIMARY);
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if GetMonitorInfoW(monitor, &mut info) == 0
            || desktop::foreground_covers_monitor(monitor, &info.rcMonitor)
        {
            KillTimer(hwnd, CLOCK);
            ShowWindow(hwnd, SW_HIDE);
            return;
        }
        let dpi = GetDpiForWindow(hwnd);
        let scale = |value| overlay::scale(value, dpi);
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
            if !font.is_null() {
                SendMessageW((*state).status, WM_SETFONT, font as usize, 0);
                DeleteObject((*state).font);
                (*state).font = font;
                (*state).dpi = dpi;
            }
        }
        let work = info.rcWork;
        let inset = scale(8);
        let width = (work.right - work.left - 2 * inset).max(1);
        let height = scale(27);
        MoveWindow(
            (*state).status,
            inset,
            0,
            (width - 2 * inset).max(1),
            height,
            0,
        );
        let was_visible = IsWindowVisible(hwnd) != 0;
        SetWindowPos(
            hwnd,
            HWND_TOPMOST,
            work.left + inset,
            work.top + scale(4),
            width,
            height,
            SWP_SHOWWINDOW | SWP_NOACTIVATE,
        );
        if !was_visible {
            refresh_status(hwnd, state);
        }
    }
}

fn minute_delay(seconds: u16, milliseconds: u16) -> u32 {
    60_000_u32
        .saturating_sub(u32::from(seconds) * 1_000 + u32::from(milliseconds))
        .max(10)
}

fn status_text(time: SYSTEMTIME, power: Option<SYSTEM_POWER_STATUS>, portuguese: bool) -> String {
    let time = format!("{:02}:{:02}", time.wHour, time.wMinute);
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

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State;
        if state.is_null() {
            return DefWindowProcW(hwnd, msg, wp, lp);
        }
        match msg {
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
            WM_CTLCOLORSTATIC => {
                SetBkColor(wp as HDC, (*state).palette.background);
                SetTextColor(wp as HDC, (*state).palette.text);
                (*state).brush as isize
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
            "75% charging    09:03"
        );
        let unknown = SYSTEM_POWER_STATUS {
            BatteryLifePercent: 255,
            ..charging
        };
        assert_eq!(status_text(time, Some(unknown), false), "09:03");
    }
}
