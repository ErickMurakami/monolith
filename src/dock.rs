//! Opt-in native Dock. All handles, hooks and callbacks belong to the GUI thread.

use crate::{
    desktop::{self, Window},
    overlay::{self, Palette},
    search::App,
    windows::{launch_shortcut, wide},
};
use std::{cell::Cell, ffi::c_void, io, mem::size_of, ptr::null_mut};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
    Graphics::Gdi::*,
    UI::{Accessibility::*, Controls::*, HiDpi::GetDpiForWindow, WindowsAndMessaging::*},
};

pub const SHOW_SPOTLIGHT: u32 = WM_APP + 1;
const REFRESH: usize = 1;
const SPOTLIGHT: u32 = 1;
const EXIT: u32 = 2;
const FIRST_WINDOW: u32 = 10;
const CAPACITY: usize = 8;

thread_local! {
    static EVENT_TARGET: Cell<HWND> = const { Cell::new(null_mut()) };
    static REFRESH_PENDING: Cell<bool> = const { Cell::new(false) };
}

pub struct Dock {
    handle: HWND,
    hooks: Vec<HWINEVENTHOOK>,
    state: Box<State>,
}

struct State {
    launcher: HWND,
    buttons: Vec<HWND>,
    windows: Vec<Window>,
    pins: Vec<App>,
    visible_pins: usize,
    brush: HBRUSH,
    font: HFONT,
    dpi: u32,
    palette: Palette,
    portuguese: bool,
}

impl Dock {
    pub fn new(
        instance: *mut c_void,
        launcher: HWND,
        palette: Palette,
        portuguese: bool,
        pins: Vec<App>,
    ) -> io::Result<Self> {
        // This constructor is called once, on the thread that pumps both windows.
        unsafe {
            let class_name = wide("MonolithDock");
            let class = WNDCLASSW {
                lpfnWndProc: Some(window_proc),
                hInstance: instance,
                hCursor: LoadCursorW(null_mut(), IDC_ARROW),
                lpszClassName: class_name.as_ptr(),
                ..Default::default()
            };
            if RegisterClassW(&class) == 0 {
                return Err(io::Error::last_os_error());
            }
            let handle = CreateWindowExW(
                overlay::EX_STYLE | WS_EX_TOPMOST | WS_EX_NOACTIVATE,
                class_name.as_ptr(),
                wide("Monolith Dock").as_ptr(),
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
            if handle.is_null() {
                return Err(io::Error::last_os_error());
            }
            let mut dock = Self {
                handle,
                hooks: Vec::new(),
                state: Box::new(State {
                    launcher,
                    buttons: Vec::new(),
                    windows: Vec::new(),
                    pins,
                    visible_pins: 0,
                    brush: CreateSolidBrush(palette.background),
                    font: null_mut(),
                    dpi: 0,
                    palette,
                    portuguese,
                }),
            };
            if dock.state.brush.is_null() {
                return Err(io::Error::last_os_error());
            }
            let state = &raw mut *dock.state;
            SetWindowLongPtrW(handle, GWLP_USERDATA, state as isize);
            for id in [SPOTLIGHT, EXIT]
                .into_iter()
                .chain(FIRST_WINDOW..FIRST_WINDOW + CAPACITY as u32)
            {
                let button = CreateWindowExW(
                    0,
                    wide("BUTTON").as_ptr(),
                    std::ptr::null(),
                    WS_CHILD | BS_OWNERDRAW as u32,
                    0,
                    0,
                    0,
                    0,
                    handle,
                    id as usize as *mut c_void,
                    instance,
                    null_mut(),
                );
                if button.is_null() {
                    return Err(io::Error::last_os_error());
                }
                (*state).buttons.push(button);
            }
            EVENT_TARGET.set(handle);
            for (first, last, flags) in [
                (EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND, 0),
                (
                    EVENT_SYSTEM_MINIMIZESTART,
                    EVENT_SYSTEM_MINIMIZEEND,
                    WINEVENT_SKIPOWNPROCESS,
                ),
                (
                    EVENT_OBJECT_DESTROY,
                    EVENT_OBJECT_HIDE,
                    WINEVENT_SKIPOWNPROCESS,
                ),
                (
                    EVENT_OBJECT_LOCATIONCHANGE,
                    EVENT_OBJECT_NAMECHANGE,
                    WINEVENT_SKIPOWNPROCESS,
                ),
            ] {
                let hook = SetWinEventHook(
                    first,
                    last,
                    null_mut(),
                    Some(window_event),
                    0,
                    0,
                    WINEVENT_OUTOFCONTEXT | flags,
                );
                if hook.is_null() {
                    return Err(io::Error::last_os_error());
                }
                dock.hooks.push(hook);
            }
            refresh(handle, state);
            Ok(dock)
        }
    }
}

impl Drop for Dock {
    fn drop(&mut self) {
        // Detach callback routing before releasing the HWND and its state.
        EVENT_TARGET.set(null_mut());
        REFRESH_PENDING.set(false);
        unsafe {
            for hook in self.hooks.drain(..) {
                UnhookWinEvent(hook);
            }
            DestroyWindow(self.handle);
            DeleteObject(self.state.brush);
            DeleteObject(self.state.font);
        }
    }
}

unsafe extern "system" fn window_event(
    _: HWINEVENTHOOK,
    event: u32,
    window: HWND,
    object: i32,
    child: i32,
    _: u32,
    _: u32,
) {
    if child != 0 || object != OBJID_WINDOW || window.is_null() {
        return;
    }
    // Location events from other objects can be frequent while scrolling.
    // Only the foreground window's geometry affects fullscreen suppression.
    if event == EVENT_OBJECT_LOCATIONCHANGE && unsafe { GetForegroundWindow() } != window {
        return;
    }
    let handle = EVENT_TARGET.get();
    if !handle.is_null() {
        // Out-of-context events are delivered on the registering GUI thread.
        schedule_refresh(handle);
    }
}

fn schedule_refresh(hwnd: HWND) {
    if !REFRESH_PENDING.get() && unsafe { SetTimer(hwnd, REFRESH, 100, None) } != 0 {
        REFRESH_PENDING.set(true);
    }
}

unsafe fn refresh(hwnd: HWND, state: *mut State) {
    unsafe {
        let monitor = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
        let mut display = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if GetMonitorInfoW(monitor, &mut display) == 0
            || desktop::foreground_covers_monitor(monitor, &display.rcMonitor)
        {
            ShowWindow(hwnd, SW_HIDE);
            (*state).windows.clear();
            return;
        }

        let dpi = GetDpiForWindow(hwnd).max(96);
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
                DeleteObject((*state).font);
                (*state).font = font;
                (*state).dpi = dpi;
            }
        }

        let work = display.rcWork;
        let inset = scale(5);
        let step = scale(116).min(((work.right - work.left - 2 * inset) / 2).max(1));
        let capacity =
            (((work.right - work.left - 2 * inset) / step) - 2).clamp(0, CAPACITY as i32) as usize;
        let mut windows = desktop::snapshot()
            .map(|snapshot| snapshot.windows)
            .unwrap_or_default();
        windows.sort_by_cached_key(|window| window.title.to_lowercase());
        let pin_count = (*state).pins.len().min(capacity);
        (*state).visible_pins = pin_count;
        windows.truncate(capacity - pin_count);
        (*state).windows = windows;
        let button_count = pin_count + (*state).windows.len() + 2;
        let width = button_count as i32 * step + 2 * inset;
        let height = scale(37);
        let left = work.left + ((work.right - work.left - width) / 2).max(0);
        let top = work.bottom - height - scale(8);

        let spotlight = if (*state).portuguese {
            "Buscar"
        } else {
            "Search"
        };
        let exit = if (*state).portuguese { "Sair" } else { "Exit" };
        let buttons = &(*state).buttons;
        let windows = &(*state).windows;
        let pins = &(*state).pins;
        for (index, &button) in buttons.iter().enumerate() {
            let (position, label) = match index {
                0 => (0, spotlight),
                1 => (button_count as i32 - 1, exit),
                _ => {
                    let label = if index - 2 < pin_count {
                        Some(pins[index - 2].name.as_str())
                    } else {
                        windows
                            .get(index - 2 - pin_count)
                            .map(|window| window.title.as_str())
                    };
                    let Some(label) = label else {
                        ShowWindow(button, SW_HIDE);
                        continue;
                    };
                    (index as i32 - 1, label)
                }
            };
            SetWindowTextW(button, wide(label).as_ptr());
            SetWindowPos(
                button,
                null_mut(),
                inset + position * step,
                inset,
                step - scale(3),
                height - 2 * inset,
                SWP_NOZORDER | SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
            InvalidateRect(button, std::ptr::null(), 0);
        }
        SetWindowPos(
            hwnd,
            HWND_TOPMOST,
            left,
            top,
            width,
            height,
            SWP_SHOWWINDOW | SWP_NOACTIVATE,
        );
    }
}

unsafe fn draw(state: *mut State, item: &DRAWITEMSTRUCT) {
    unsafe {
        let palette = (*state).palette;
        let area = item.rcItem;
        let pressed = item.itemState & ODS_SELECTED != 0;
        let saved = SaveDC(item.hDC);
        if saved == 0 {
            return;
        }
        SetDCBrushColor(
            item.hDC,
            if pressed {
                palette.pressed
            } else {
                palette.surface
            },
        );
        let brush = GetStockObject(DC_BRUSH) as HBRUSH;
        FillRect(item.hDC, &area, brush);

        let font = (*state).font;
        if !font.is_null() {
            SelectObject(item.hDC, font);
        }
        SetBkMode(item.hDC, TRANSPARENT as i32);
        SetTextColor(item.hDC, palette.text);
        let mut title = [0_u16; 513];
        let length = GetWindowTextW(item.hwndItem, title.as_mut_ptr(), title.len() as i32);
        let mut text_area = RECT {
            left: area.left + 5,
            right: area.right - 5,
            ..area
        };
        DrawTextW(
            item.hDC,
            title.as_mut_ptr(),
            length,
            &mut text_area,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX,
        );
        let windows = &(*state).windows;
        if item
            .CtlID
            .checked_sub(FIRST_WINDOW)
            .and_then(|index| (index as usize).checked_sub((*state).visible_pins))
            .and_then(|index| windows.get(index))
            .is_some_and(|window| window.active)
        {
            let centre = area.left + (area.right - area.left) / 2;
            let indicator = RECT {
                left: centre - 5,
                right: centre + 5,
                top: area.bottom - 2,
                bottom: area.bottom,
            };
            SetDCBrushColor(item.hDC, palette.accent);
            FillRect(item.hDC, &indicator, brush);
        }
        RestoreDC(item.hDC, saved);
    }
}

unsafe extern "system" fn window_proc(hwnd: HWND, message: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State;
        if state.is_null() {
            return DefWindowProcW(hwnd, message, wp, lp);
        }
        match message {
            WM_CLOSE => {
                PostMessageW((*state).launcher, WM_CLOSE, 0, 0);
                0
            }
            WM_COMMAND if (wp >> 16) as u32 == BN_CLICKED => {
                let id = (wp & 0xffff) as u32;
                match id {
                    SPOTLIGHT => {
                        PostMessageW((*state).launcher, SHOW_SPOTLIGHT, 0, 0);
                    }
                    EXIT => {
                        PostMessageW((*state).launcher, WM_CLOSE, 0, 0);
                    }
                    FIRST_WINDOW.. => {
                        let index = (id - FIRST_WINDOW) as usize;
                        if index < (*state).visible_pins {
                            let pins = &(*state).pins;
                            let path = pins[index].path.clone();
                            launch_shortcut(hwnd, &path);
                            return 0;
                        }
                        let windows = &(*state).windows;
                        let target = windows
                            .get(index - (*state).visible_pins)
                            .map(|window| window.target);
                        if let Some(target) = target
                            && target.toggle().is_err()
                        {
                            schedule_refresh(hwnd);
                        }
                    }
                    _ => {}
                }
                0
            }
            WM_DRAWITEM if lp != 0 => {
                draw(state, &*(lp as *const DRAWITEMSTRUCT));
                1
            }
            WM_ERASEBKGND => {
                let mut area = RECT::default();
                GetClientRect(hwnd, &mut area);
                FillRect(wp as HDC, &area, (*state).brush);
                1
            }
            WM_TIMER if wp == REFRESH => {
                KillTimer(hwnd, REFRESH);
                REFRESH_PENDING.set(false);
                refresh(hwnd, state);
                0
            }
            WM_DISPLAYCHANGE | WM_SETTINGCHANGE | WM_DPICHANGED => {
                schedule_refresh(hwnd);
                0
            }
            WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,
            _ => DefWindowProcW(hwnd, message, wp, lp),
        }
    }
}
