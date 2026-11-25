//! Borderless control surface for real Windows media sessions.

use crate::{
    desktop,
    media::{self, Action, Media, NowPlaying},
    overlay::{self, Palette},
    top_bar,
    windows::wide,
};
use std::{cell::Cell, ffi::c_void, io, mem::size_of, ptr::null_mut};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
    Graphics::Gdi::*,
    System::SystemServices::{SS_CENTERIMAGE, SS_ENDELLIPSIS, SS_NOTIFY},
    UI::{
        Accessibility::*, Controls::*, HiDpi::GetDpiForWindow,
        Input::KeyboardAndMouse::EnableWindow, WindowsAndMessaging::*,
    },
};

const LABEL: usize = 1;
const PREVIOUS: usize = 2;
const TOGGLE: usize = 3;
const NEXT: usize = 4;
const REPOSITION: usize = 1;

thread_local! {
    static TARGET: Cell<HWND> = const { Cell::new(null_mut()) };
    static PENDING: Cell<bool> = const { Cell::new(false) };
}

pub struct Island {
    hwnd: HWND,
    hooks: Vec<HWINEVENTHOOK>,
    state: Box<State>,
}

struct State {
    launcher: HWND,
    host: HWND,
    controls: [HWND; 4],
    font: HFONT,
    dpi: u32,
    brush: HBRUSH,
    palette: Palette,
    portuguese: bool,
    media: Option<Media>,
    current: Option<NowPlaying>,
    expanded: bool,
}

impl Island {
    pub fn new(
        instance: *mut c_void,
        launcher: HWND,
        host: HWND,
        palette: Palette,
        portuguese: bool,
    ) -> io::Result<Self> {
        // Only this GUI thread owns the window and its native controls.
        unsafe {
            let class_name = wide("MonolithIsland");
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
                wide("Monolith Island").as_ptr(),
                overlay::STYLE | WS_CLIPCHILDREN,
                0,
                0,
                0,
                0,
                host,
                null_mut(),
                instance,
                null_mut(),
            );
            if hwnd.is_null() {
                return Err(io::Error::last_os_error());
            }
            let mut island = Self {
                hwnd,
                hooks: Vec::new(),
                state: Box::new(State {
                    launcher,
                    host,
                    controls: [null_mut(); 4],
                    font: null_mut(),
                    dpi: 0,
                    brush: CreateSolidBrush(palette.background),
                    palette,
                    portuguese,
                    media: None,
                    current: None,
                    expanded: false,
                }),
            };
            if island.state.brush.is_null() {
                return Err(io::Error::last_os_error());
            }
            for (index, control) in island.state.controls.iter_mut().enumerate() {
                let (class, style) = if index == 0 {
                    ("STATIC", SS_NOTIFY | SS_CENTERIMAGE | SS_ENDELLIPSIS)
                } else {
                    ("BUTTON", BS_OWNERDRAW as u32)
                };
                *control = CreateWindowExW(
                    0,
                    wide(class).as_ptr(),
                    std::ptr::null(),
                    WS_CHILD | style,
                    0,
                    0,
                    0,
                    0,
                    hwnd,
                    (index + 1) as *mut c_void,
                    instance,
                    null_mut(),
                );
                if control.is_null() {
                    return Err(io::Error::last_os_error());
                }
            }
            island.state.media = Some(Media::connect(hwnd)?);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, (&raw mut *island.state) as isize);
            TARGET.set(hwnd);
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
                island.hooks.push(hook);
            }
            Ok(island)
        }
    }
}

impl Drop for Island {
    fn drop(&mut self) {
        TARGET.set(null_mut());
        PENDING.set(false);
        drop(self.state.media.take());
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
    hwnd: HWND,
    object: i32,
    child: i32,
    _: u32,
    _: u32,
) {
    if child == 0
        && object == OBJID_WINDOW
        && !hwnd.is_null()
        && unsafe { GetForegroundWindow() } == hwnd
    {
        schedule(TARGET.get());
    }
}

fn schedule(hwnd: HWND) {
    if !hwnd.is_null() && !PENDING.get() && unsafe { SetTimer(hwnd, REPOSITION, 100, None) } != 0 {
        PENDING.set(true);
    }
}

unsafe fn layout(hwnd: HWND, state: *mut State) {
    unsafe {
        let Some(current) = (*state).current.as_ref() else {
            ShowWindow(hwnd, SW_HIDE);
            return;
        };
        let monitor = MonitorFromPoint(POINT::default(), MONITOR_DEFAULTTOPRIMARY);
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if GetMonitorInfoW(monitor, &mut info) == 0
            || desktop::foreground_covers_monitor(monitor, &info.rcMonitor)
        {
            ShowWindow(hwnd, SW_HIDE);
            return;
        }
        // Copy the presentation before calling APIs that can send window messages.
        let expanded = (*state).expanded;
        let title = match (current.title.is_empty(), current.artist.is_empty()) {
            (true, _) => current.artist.clone(),
            (_, true) => current.title.clone(),
            _ => format!("{}   ·   {}", current.title, current.artist),
        };
        let playing = current.playing;
        let availability = [current.previous, current.toggle, current.next];
        let controls = (*state).controls;
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
                for child in controls {
                    SendMessageW(child, WM_SETFONT, font as usize, 0);
                }
                DeleteObject((*state).font);
                (*state).font = font;
                (*state).dpi = dpi;
            }
        }
        let work = info.rcWork;
        let work_width = (work.right - work.left).max(1);
        let width = scale(346).min(work_width.saturating_sub(scale(16))).max(1);
        let label_height = scale(27);
        let height = if expanded { scale(63) } else { label_height };
        let inset = scale(8);
        let left = work.left + (work_width - width) / 2;
        let mut top = work.top + scale(4);
        let host = (*state).host;
        if !host.is_null() && work_width < scale(780) {
            let mut rect = RECT::default();
            if GetWindowRect(host, &mut rect) != 0 {
                top = rect.bottom + scale(4);
            }
        }
        SetWindowTextW(controls[0], wide(&title).as_ptr());
        SetWindowPos(
            controls[0],
            null_mut(),
            inset,
            0,
            (width - 2 * inset).max(1),
            label_height,
            SWP_NOZORDER | SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
        let portuguese = (*state).portuguese;
        let captions = match (portuguese, playing) {
            (false, true) => ["Previous", "Pause", "Next"],
            (false, false) => ["Previous", "Play", "Next"],
            (true, true) => ["Anterior", "Pausar", "Próxima"],
            (true, false) => ["Anterior", "Reproduzir", "Próxima"],
        };
        let step = (width - 2 * inset).max(3) / 3;
        for (index, &button) in controls.iter().skip(1).enumerate() {
            if expanded {
                SetWindowTextW(button, wide(captions[index]).as_ptr());
                EnableWindow(button, i32::from(availability[index]));
                SetWindowPos(
                    button,
                    null_mut(),
                    inset + step * index as i32,
                    label_height + scale(3),
                    (step - scale(3)).max(1),
                    scale(27),
                    SWP_SHOWWINDOW | SWP_NOZORDER | SWP_NOACTIVATE,
                );
                InvalidateRect(button, std::ptr::null(), 1);
            } else {
                ShowWindow(button, SW_HIDE);
            }
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

unsafe fn paint_button(item: &DRAWITEMSTRUCT, palette: Palette, font: HFONT) {
    unsafe {
        let saved = SaveDC(item.hDC);
        if saved == 0 {
            return;
        }
        let pressed = item.itemState & ODS_SELECTED != 0;
        let disabled = item.itemState & ODS_DISABLED != 0;
        SetDCBrushColor(
            item.hDC,
            if pressed {
                palette.pressed
            } else {
                palette.surface
            },
        );
        FillRect(item.hDC, &item.rcItem, GetStockObject(DC_BRUSH) as HBRUSH);
        if !font.is_null() {
            SelectObject(item.hDC, font);
        }
        SetBkMode(item.hDC, TRANSPARENT as i32);
        SetTextColor(
            item.hDC,
            if disabled {
                GetSysColor(COLOR_GRAYTEXT)
            } else {
                palette.text
            },
        );
        let mut title = [0_u16; 32];
        let count = GetWindowTextW(item.hwndItem, title.as_mut_ptr(), title.len() as i32);
        let mut bounds = item.rcItem;
        DrawTextW(
            item.hDC,
            title.as_mut_ptr(),
            count,
            &mut bounds,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
        );
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
            media::READY => {
                (*state).current = (*state).media.as_ref().and_then(Media::read);
                if (*state).current.is_none() {
                    (*state).expanded = false;
                }
                layout(hwnd, state);
                0
            }
            WM_COMMAND if wp >> 16 == 0 => {
                let id = wp & 0xffff;
                if id == LABEL {
                    (*state).expanded = !(*state).expanded;
                    layout(hwnd, state);
                } else {
                    let action = match id {
                        PREVIOUS => Action::Previous,
                        TOGGLE => Action::Toggle,
                        NEXT => Action::Next,
                        _ => return 0,
                    };
                    if let (Some(media), Some(current)) =
                        ((*state).media.as_ref(), (*state).current.as_ref())
                    {
                        media.perform(current.generation, action);
                    }
                }
                0
            }
            WM_CONTEXTMENU => {
                top_bar::menu(hwnd, (*state).launcher, (*state).portuguese);
                0
            }
            WM_CLOSE => {
                PostMessageW((*state).launcher, WM_CLOSE, 0, 0);
                0
            }
            WM_TIMER if wp == REPOSITION => {
                KillTimer(hwnd, REPOSITION);
                PENDING.set(false);
                layout(hwnd, state);
                0
            }
            WM_DISPLAYCHANGE | WM_DPICHANGED | WM_SETTINGCHANGE => {
                schedule(hwnd);
                0
            }
            WM_ERASEBKGND => {
                let mut area = RECT::default();
                GetClientRect(hwnd, &mut area);
                FillRect(wp as HDC, &area, (*state).brush);
                1
            }
            WM_CTLCOLORSTATIC => {
                SetBkColor(wp as HDC, (*state).palette.background);
                SetTextColor(wp as HDC, (*state).palette.text);
                (*state).brush as isize
            }
            WM_DRAWITEM if lp != 0 => {
                paint_button(
                    &*(lp as *const DRAWITEMSTRUCT),
                    (*state).palette,
                    (*state).font,
                );
                1
            }
            WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}
