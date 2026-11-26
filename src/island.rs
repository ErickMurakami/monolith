//! Borderless control surface for real Windows media sessions.

use crate::{
    desktop,
    media::{self, Action, Media, NowPlaying},
    overlay::{self, Palette},
    windows::{shell_menu, wide},
};
use std::{cell::Cell, ffi::c_void, io, mem::size_of, ptr::null_mut};
use windows_sys::Win32::{
    Foundation::{FILETIME, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
    Graphics::Gdi::*,
    System::{
        SystemInformation::GetSystemTimeAsFileTime,
        SystemServices::{SS_CENTERIMAGE, SS_ENDELLIPSIS, SS_NOTIFY, SS_OWNERDRAW},
    },
    UI::{
        Accessibility::*,
        Controls::*,
        HiDpi::GetDpiForWindow,
        Input::KeyboardAndMouse::EnableWindow,
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::*,
    },
};

const LABEL: usize = 1;
const PREVIOUS: usize = 2;
const TOGGLE: usize = 3;
const NEXT: usize = 4;
const ARTWORK: usize = 5;
const REPOSITION: usize = 1;
const PROGRESS: usize = 2;

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
    progress: HWND,
    artwork: HWND,
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
                    progress: null_mut(),
                    artwork: null_mut(),
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
            island.state.progress = progress_control(hwnd, instance, &raw mut *island.state);
            island.state.artwork = CreateWindowExW(
                0,
                wide("STATIC").as_ptr(),
                wide("Album artwork").as_ptr(),
                WS_CHILD | SS_OWNERDRAW,
                0,
                0,
                0,
                0,
                hwnd,
                ARTWORK as *mut c_void,
                instance,
                null_mut(),
            );
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

unsafe fn progress_control(hwnd: HWND, instance: *mut c_void, state: *mut State) -> HWND {
    unsafe {
        let controls = INITCOMMONCONTROLSEX {
            dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_PROGRESS_CLASS,
        };
        if InitCommonControlsEx(&controls) == 0 {
            return null_mut();
        }
        let progress = CreateWindowExW(
            0,
            PROGRESS_CLASSW,
            wide("Playback progress").as_ptr(),
            WS_CHILD | PBS_SMOOTH,
            0,
            0,
            0,
            0,
            hwnd,
            null_mut(),
            instance,
            null_mut(),
        );
        if !progress.is_null() {
            if SetWindowSubclass(progress, Some(progress_proc), 1, state as usize) == 0 {
                DestroyWindow(progress);
                return null_mut();
            }
            SetWindowLongW(
                progress,
                GWL_STYLE,
                GetWindowLongW(progress, GWL_STYLE) & !(WS_BORDER as i32),
            );
            SetWindowLongW(
                progress,
                GWL_EXSTYLE,
                GetWindowLongW(progress, GWL_EXSTYLE)
                    & !((WS_EX_STATICEDGE | WS_EX_CLIENTEDGE | WS_EX_WINDOWEDGE) as i32),
            );
            SetWindowPos(
                progress,
                null_mut(),
                0,
                0,
                0,
                0,
                SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
            SendMessageW(progress, PBM_SETRANGE32, 0, 1000);
        }
        progress
    }
}

unsafe extern "system" fn progress_proc(
    hwnd: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
    _: usize,
    reference: usize,
) -> LRESULT {
    // The Island owns this state until it destroys all child windows.
    unsafe {
        match msg {
            WM_ERASEBKGND => return 1,
            WM_NCPAINT | WM_NCCALCSIZE => return 0,
            WM_PAINT => {
                let palette = (*(reference as *const State)).palette;
                let mut paint = PAINTSTRUCT::default();
                let dc = BeginPaint(hwnd, &mut paint);
                let mut bounds = RECT::default();
                GetClientRect(hwnd, &mut bounds);
                let fraction = DefSubclassProc(hwnd, PBM_GETPOS, 0, 0).clamp(0, 1000) as i32;
                let brush = GetStockObject(DC_BRUSH) as HBRUSH;
                let previous = SetDCBrushColor(dc, palette.surface);
                FillRect(dc, &bounds, brush);
                bounds.right = bounds.left + (bounds.right - bounds.left) * fraction / 1000;
                SetDCBrushColor(dc, palette.accent);
                FillRect(dc, &bounds, brush);
                SetDCBrushColor(dc, previous);
                EndPaint(hwnd, &paint);
                return 0;
            }
            WM_NCDESTROY => {
                RemoveWindowSubclass(hwnd, Some(progress_proc), 1);
            }
            _ => {}
        }
        DefSubclassProc(hwnd, msg, wp, lp)
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

pub fn host_changed(host: HWND) {
    let target = TARGET.get();
    if !target.is_null() && unsafe { GetWindow(target, GW_OWNER) } == host {
        schedule(target);
    }
}

unsafe fn layout(hwnd: HWND, state: *mut State) {
    unsafe {
        KillTimer(hwnd, PROGRESS);
        let Some(current) = (*state).current.as_ref() else {
            if let Some(media) = (*state).media.as_ref() {
                media.request_artwork(false);
            }
            ShowWindow(hwnd, SW_HIDE);
            return;
        };
        let monitor = MonitorFromPoint(POINT::default(), MONITOR_DEFAULTTOPRIMARY);
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let host = (*state).host;
        if GetMonitorInfoW(monitor, &mut info) == 0
            || if host.is_null() {
                desktop::foreground_covers_monitor(monitor, &info.rcMonitor)
            } else {
                IsWindowVisible(host) == 0
            }
        {
            if let Some(media) = (*state).media.as_ref() {
                media.request_artwork(false);
            }
            ShowWindow(hwnd, SW_HIDE);
            return;
        }
        // Copy the presentation before calling APIs that can send window messages.
        let expanded = (*state).expanded;
        let artwork_view = (*state).artwork;
        if let Some(media) = (*state).media.as_ref() {
            media.request_artwork(expanded && !artwork_view.is_null());
        }
        let show_artwork = expanded && current.artwork.is_some() && !artwork_view.is_null();
        let title = match (current.title.is_empty(), current.artist.is_empty()) {
            (true, _) => current.artist.clone(),
            (_, true) => current.title.clone(),
            _ => format!("{}   ·   {}", current.title, current.artist),
        };
        let playing = current.playing;
        let show_progress = expanded && current.timeline.is_some() && !(*state).progress.is_null();
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
        let label_height = scale(if show_artwork { 37 } else { 27 });
        let height = if show_progress {
            label_height + scale(46)
        } else if expanded {
            label_height + scale(36)
        } else {
            label_height
        };
        let inset = scale(8);
        let artwork_size = scale(28);
        let label_left = if show_artwork {
            inset + artwork_size + scale(7)
        } else {
            inset
        };
        let left = work.left + (work_width - width) / 2;
        let mut top = work.top + scale(4);
        let host = (*state).host;
        if !host.is_null() {
            let mut rect = RECT::default();
            if GetWindowRect(host, &mut rect) != 0 {
                top = if work_width < scale(780) {
                    rect.bottom + scale(4)
                } else {
                    rect.top
                };
            }
        }
        SetWindowTextW(controls[0], wide(&title).as_ptr());
        if show_artwork {
            SetWindowPos(
                artwork_view,
                null_mut(),
                inset,
                (label_height - artwork_size) / 2,
                artwork_size,
                artwork_size,
                SWP_SHOWWINDOW | SWP_NOZORDER | SWP_NOACTIVATE,
            );
            InvalidateRect(artwork_view, std::ptr::null(), 1);
        } else if !artwork_view.is_null() {
            ShowWindow(artwork_view, SW_HIDE);
        }
        SetWindowPos(
            controls[0],
            null_mut(),
            label_left,
            0,
            (width - label_left - inset).max(1),
            label_height,
            SWP_NOZORDER | SWP_NOACTIVATE | SWP_SHOWWINDOW | SWP_NOCOPYBITS,
        );
        InvalidateRect(controls[0], std::ptr::null(), 1);
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
        let progress = (*state).progress;
        if show_progress {
            SetWindowPos(
                progress,
                null_mut(),
                inset,
                label_height + scale(35),
                (width - 2 * inset).max(1),
                scale(3),
                SWP_SHOWWINDOW | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        } else if !progress.is_null() {
            ShowWindow(progress, SW_HIDE);
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
        InvalidateRect(hwnd, std::ptr::null(), 1);
        if show_progress {
            update_progress(hwnd, state);
        }
    }
}

unsafe fn update_progress(hwnd: HWND, state: *mut State) {
    unsafe {
        KillTimer(hwnd, PROGRESS);
        let bar = (*state).progress;
        if IsWindowVisible(bar) == 0 {
            return;
        }
        let Some(current) = (*state).current.as_ref() else {
            return;
        };
        let Some(timeline) = current.timeline else {
            return;
        };
        let mut timestamp = FILETIME::default();
        GetSystemTimeAsFileTime(&mut timestamp);
        let now = (i64::from(timestamp.dwHighDateTime) << 32) | i64::from(timestamp.dwLowDateTime);
        let (position, continues) = timeline.position_at(now, current.playing);
        SendMessageW(bar, PBM_SETPOS, position as usize, 0);
        InvalidateRect(bar, std::ptr::null(), 0);
        if continues {
            SetTimer(hwnd, PROGRESS, 1000, None);
        }
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
                shell_menu(hwnd, (*state).launcher, (*state).portuguese);
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
            WM_TIMER if wp == PROGRESS => {
                update_progress(hwnd, state);
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
                SetBkMode(wp as HDC, OPAQUE as i32);
                SetTextColor(wp as HDC, (*state).palette.text);
                (*state).brush as isize
            }
            WM_DRAWITEM if lp != 0 => {
                let item = &*(lp as *const DRAWITEMSTRUCT);
                if item.CtlID as usize == ARTWORK {
                    FillRect(item.hDC, &item.rcItem, (*state).brush);
                    if let Some(artwork) = (*state)
                        .current
                        .as_ref()
                        .and_then(|current| current.artwork.as_ref())
                    {
                        artwork.draw(item.hDC, item.rcItem, (*state).palette.background);
                    }
                } else {
                    overlay::paint_button(item, (*state).palette, (*state).font);
                }
                1
            }
            WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}
