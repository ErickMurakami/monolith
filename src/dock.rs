//! Opt-in native Dock. All handles, hooks and callbacks belong to the GUI thread.

use crate::{
    config,
    desktop::{self, WindowGroup},
    overlay::{self, Palette},
    search::App,
    windows::{launch_shortcut, report_for, wide},
};
use std::{cell::Cell, ffi::c_void, io, mem::size_of, path::PathBuf, ptr::null_mut};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
    Graphics::Gdi::*,
    UI::{
        Accessibility::*,
        Controls::*,
        HiDpi::GetDpiForWindow,
        Input::KeyboardAndMouse::{ReleaseCapture, SetCapture},
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::*,
    },
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

pub struct Settings {
    pub pins: Vec<App>,
    pub order_path: PathBuf,
}

struct PinDrag {
    start: POINT,
    from: usize,
    over: Option<usize>,
    moved: bool,
}

struct State {
    launcher: HWND,
    buttons: Vec<HWND>,
    groups: Vec<WindowGroup>,
    pins: Vec<App>,
    order_path: PathBuf,
    drag: Option<PinDrag>,
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
        settings: Settings,
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
                    groups: Vec::new(),
                    pins: settings.pins,
                    order_path: settings.order_path,
                    drag: None,
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
                if id >= FIRST_WINDOW
                    && (*state).pins.len() > 1
                    && SetWindowSubclass(button, Some(pin_button), 1, state as usize) == 0
                {
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

fn pin_index(id: u32, count: usize) -> Option<usize> {
    id.checked_sub(FIRST_WINDOW)
        .map(|index| index as usize)
        .filter(|&index| index < count)
}

fn mouse_point(button: HWND, coordinates: LPARAM) -> Option<POINT> {
    let mut point = POINT {
        x: coordinates as i16 as i32,
        y: (coordinates >> 16) as i16 as i32,
    };
    (unsafe { ClientToScreen(button, &mut point) } != 0).then_some(point)
}

unsafe fn repaint_buttons(state: *mut State) {
    unsafe {
        let buttons = &(*state).buttons;
        for &button in buttons {
            InvalidateRect(button, std::ptr::null(), 0);
        }
    }
}

unsafe extern "system" fn pin_button(
    button: HWND,
    message: u32,
    wp: WPARAM,
    lp: LPARAM,
    _: usize,
    reference: usize,
) -> LRESULT {
    // The parent Dock owns this state until all child windows have been destroyed.
    unsafe {
        let state = reference as *mut State;
        let count = (*state).visible_pins;
        let id = GetDlgCtrlID(button) as u32;
        let parent = GetParent(button);
        match message {
            WM_LBUTTONDOWN | WM_LBUTTONDBLCLK if count > 1 => {
                if let Some(from) = pin_index(id, count)
                    && let Some(start) = mouse_point(button, lp)
                {
                    SetCapture(button);
                    (*state).drag = Some(PinDrag {
                        start,
                        from,
                        over: None,
                        moved: false,
                    });
                    return 0;
                }
            }
            WM_MOUSEMOVE | WM_LBUTTONUP if (*state).drag.is_some() => {
                if let Some(point) = mouse_point(button, lp) {
                    let hit = WindowFromPoint(point);
                    let over = if !hit.is_null() && GetParent(hit) == parent {
                        pin_index(GetDlgCtrlID(hit) as u32, count)
                    } else {
                        None
                    };
                    let threshold = overlay::scale(5, (*state).dpi) as u32;
                    if let Some(drag) = (*state).drag.as_mut() {
                        drag.moved |= drag.start.x.abs_diff(point.x) >= threshold
                            || drag.start.y.abs_diff(point.y) >= threshold;
                        let target = over.filter(|&to| drag.moved && to != drag.from);
                        if drag.over != target {
                            drag.over = target;
                            repaint_buttons(state);
                        }
                    }
                    if message == WM_LBUTTONUP {
                        let drag = (*state).drag.take();
                        ReleaseCapture();
                        repaint_buttons(state);
                        if let Some(drag) = drag {
                            if let Some(to) = drag.over {
                                reorder_pins(parent, state, drag.from, to);
                            } else if !drag.moved && over == Some(drag.from) {
                                PostMessageW(parent, WM_COMMAND, id as usize, button as isize);
                            }
                        }
                    }
                    return 0;
                }
                if message == WM_LBUTTONUP {
                    (*state).drag = None;
                    ReleaseCapture();
                    repaint_buttons(state);
                    return 0;
                }
            }
            WM_CAPTURECHANGED => {
                if (*state).drag.take().is_some() {
                    repaint_buttons(state);
                }
            }
            WM_NCDESTROY => {
                RemoveWindowSubclass(button, Some(pin_button), 1);
            }
            _ => {}
        }
        DefSubclassProc(button, message, wp, lp)
    }
}

unsafe fn reorder_pins(hwnd: HWND, state: *mut State, from: usize, to: usize) {
    unsafe {
        let count = (*state).visible_pins;
        if from >= count || to >= count || from == to {
            return;
        }
        let mut pins = (*state).pins.clone();
        let pin = pins.remove(from);
        pins.insert(to, pin);
        let order = pins.iter().map(|pin| pin.name.clone()).collect();
        if let Err(error) = config::save_dock_order(&(*state).order_path, order) {
            let label = if (*state).portuguese {
                "Não foi possível salvar a ordem do Dock"
            } else {
                "Could not save the Dock order"
            };
            report_for(hwnd, &format!("{label} ({error})"));
            return;
        }
        (*state).pins = pins;
        refresh(hwnd, state);
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
            (*state).groups.clear();
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
        let mut groups = desktop::snapshot()
            .map(|snapshot| desktop::group_windows(snapshot.windows))
            .unwrap_or_default();
        let pin_count = (*state).pins.len().min(capacity);
        (*state).visible_pins = pin_count;
        groups.truncate(capacity - pin_count);
        (*state).groups = groups;
        let button_count = pin_count + (*state).groups.len() + 2;
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
        let groups = &(*state).groups;
        let pins = &(*state).pins;
        for (index, &button) in buttons.iter().enumerate() {
            let (position, label) = match index {
                0 => (0, spotlight.to_owned()),
                1 => (button_count as i32 - 1, exit.to_owned()),
                _ => {
                    let label = if index - 2 < pin_count {
                        Some(pins[index - 2].name.clone())
                    } else {
                        groups.get(index - 2 - pin_count).map(WindowGroup::caption)
                    };
                    let Some(label) = label else {
                        ShowWindow(button, SW_HIDE);
                        continue;
                    };
                    (index as i32 - 1, label)
                }
            };
            SetWindowTextW(button, wide(&label).as_ptr());
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
        let groups = &(*state).groups;
        let drop_target = pin_index(item.CtlID, (*state).visible_pins)
            .is_some_and(|index| (*state).drag.as_ref().and_then(|drag| drag.over) == Some(index));
        if drop_target
            || item
                .CtlID
                .checked_sub(FIRST_WINDOW)
                .and_then(|index| (index as usize).checked_sub((*state).visible_pins))
                .and_then(|index| groups.get(index))
                .is_some_and(WindowGroup::active)
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
                        let groups = &(*state).groups;
                        if let Some(group) = groups.get(index - (*state).visible_pins) {
                            show_group(hwnd, lp as HWND, group);
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

unsafe fn show_group(hwnd: HWND, button: HWND, group: &WindowGroup) {
    unsafe {
        if group.windows.len() == 1 {
            let target = group.windows[0].target;
            if target.toggle().is_err() {
                schedule_refresh(hwnd);
            }
            return;
        }
        let menu = CreatePopupMenu();
        if menu.is_null() {
            return;
        }
        // TrackPopupMenuEx pumps nested messages. Do not access group after it starts.
        let mut targets = Vec::with_capacity(group.windows.len());
        for window in &group.windows {
            let title = wide(&window.title.replace('&', "&&"));
            let id = targets.len() + 1;
            let checked = if window.active { MF_CHECKED } else { 0 };
            if AppendMenuW(menu, MF_STRING | checked, id, title.as_ptr()) != 0 {
                targets.push(window.target);
            }
        }
        let mut rect = RECT::default();
        if GetWindowRect(button, &mut rect) != 0 && !targets.is_empty() {
            SetForegroundWindow(hwnd);
            let command = TrackPopupMenuEx(
                menu,
                TPM_BOTTOMALIGN | TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON,
                rect.left,
                rect.top,
                hwnd,
                null_mut(),
            );
            PostMessageW(hwnd, WM_NULL, 0, 0);
            if let Some(target) = (command as usize)
                .checked_sub(1)
                .and_then(|index| targets.get(index))
                && target.activate().is_err()
            {
                schedule_refresh(hwnd);
            }
        }
        DestroyMenu(menu);
    }
}
