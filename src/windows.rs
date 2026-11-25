use crate::{
    calculator::{self, CalculationError},
    config::{self, Language, Shortcut, Theme},
    desktop::{self, ActivationError},
    dock::{self, Dock},
    search::{self, App},
};
use std::{
    env, ffi::c_void, io, mem::size_of, os::windows::ffi::OsStrExt, path::PathBuf, ptr::null_mut,
};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
    Graphics::{
        Dwm::{DWMWA_USE_IMMERSIVE_DARK_MODE, DwmSetWindowAttribute},
        Gdi::*,
    },
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        Controls::{EM_LIMITTEXT, EM_SETCUEBANNER, EM_SETMARGINS, EM_SETSEL},
        HiDpi::*,
        Input::KeyboardAndMouse::*,
        Shell::ShellExecuteW,
        WindowsAndMessaging::*,
    },
};

const HOTKEY: i32 = 1;
const EDIT: usize = 100;
const RESULTS: usize = 101;
const RESULT_LIMIT: usize = 60;

enum Match {
    App(usize),
    Window(usize),
}

struct State {
    apps: Option<Vec<App>>,
    desktop: Option<desktop::Snapshot>,
    matches: Vec<Match>,
    edit: HWND,
    results: HWND,
    font: HFONT,
    background: HBRUSH,
    surface: HBRUSH,
    dark: bool,
    portuguese: bool,
    can_hide: bool,
}

impl Drop for State {
    fn drop(&mut self) {
        // The window and its children have already been destroyed.
        unsafe {
            DeleteObject(self.font);
            DeleteObject(self.background);
            DeleteObject(self.surface);
        }
    }
}

pub(crate) fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain([0]).collect()
}

pub(crate) fn rgb(r: u32, g: u32, b: u32) -> u32 {
    r | (g << 8) | (b << 16)
}

pub fn report(message: &str) {
    // Strings remain valid until the modal call returns.
    unsafe {
        MessageBoxW(
            null_mut(),
            wide(message).as_ptr(),
            wide("Monolith").as_ptr(),
            MB_OK | MB_ICONWARNING,
        );
    }
}

fn roots() -> impl Iterator<Item = PathBuf> {
    ["APPDATA", "ProgramData"]
        .into_iter()
        .filter_map(env::var_os)
        .map(|path| {
            PathBuf::from(path)
                .join("Microsoft")
                .join("Windows")
                .join("Start Menu")
                .join("Programs")
        })
}

// Window handles and state belong to the single GUI thread. Only the main
// loop handles keys that require direct access to State.
pub fn run() -> io::Result<()> {
    let args: Vec<_> = env::args_os().skip(1).collect();
    let safe = args.iter().any(|arg| arg == "--safe-mode");
    let resident = !safe && args.iter().any(|arg| arg == "--resident");
    let config_path = env::var_os("LOCALAPPDATA")
        .map(|base| PathBuf::from(base).join("Monolith").join("config.toml"))
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "LOCALAPPDATA is unavailable"))?;
    let (settings, warning) = config::load(&config_path, safe);
    if let Some(message) = warning {
        report(&message);
    }

    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let instance = GetModuleHandleW(std::ptr::null());
        let class_name = wide("MonolithSpotlight");
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

        let dark = matches!(settings.theme, Theme::Dark);
        let mut state = Box::new(State {
            apps: None,
            desktop: None,
            matches: Vec::new(),
            edit: null_mut(),
            results: null_mut(),
            font: null_mut(),
            background: CreateSolidBrush(if dark {
                rgb(8, 10, 13)
            } else {
                rgb(241, 243, 246)
            }),
            surface: CreateSolidBrush(if dark {
                rgb(20, 24, 32)
            } else {
                rgb(255, 255, 255)
            }),
            dark,
            portuguese: matches!(settings.language, Language::Portuguese),
            can_hide: false,
        });
        let hwnd = CreateWindowExW(
            0,
            class_name.as_ptr(),
            wide("Monolith").as_ptr(),
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            540,
            350,
            null_mut(),
            null_mut(),
            instance,
            null_mut(),
        );
        if hwnd.is_null() {
            return Err(io::Error::last_os_error());
        }
        // The callback takes exclusive state access only where it does not
        // dispatch messages that can re-enter a state-reading branch.
        let state_ptr = Box::into_raw(state);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, state_ptr as isize);
        let show_dock = !safe && (settings.dock || args.iter().any(|arg| arg == "--dock"));
        let pins = if show_dock && !settings.dock_pins.is_empty() {
            let apps = (*state_ptr)
                .apps
                .get_or_insert_with(|| search::discover(roots()));
            search::resolve_pins(apps, &settings.dock_pins)
        } else {
            Vec::new()
        };
        let result = run_window(
            hwnd,
            instance,
            state_ptr,
            settings.shortcut,
            resident,
            show_dock,
            pins,
        );
        if IsWindow(hwnd) != 0 {
            DestroyWindow(hwnd);
        }
        state = Box::from_raw(state_ptr);
        drop(state);
        result
    }
}

unsafe fn run_window(
    hwnd: HWND,
    instance: *mut c_void,
    state: *mut State,
    shortcut: Shortcut,
    resident: bool,
    show_dock: bool,
    pins: Vec<App>,
) -> io::Result<()> {
    unsafe {
        let dark = i32::from((*state).dark);
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE as u32,
            (&raw const dark).cast(),
            size_of::<i32>() as u32,
        );
        let edit = CreateWindowExW(
            0,
            wide("EDIT").as_ptr(),
            std::ptr::null(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | ES_AUTOHSCROLL as u32,
            0,
            0,
            0,
            0,
            hwnd,
            EDIT as *mut c_void,
            instance,
            null_mut(),
        );
        let results = CreateWindowExW(
            0,
            wide("LISTBOX").as_ptr(),
            std::ptr::null(),
            WS_CHILD
                | WS_VISIBLE
                | WS_TABSTOP
                | WS_VSCROLL
                | LBS_NOTIFY as u32
                | LBS_NOINTEGRALHEIGHT as u32,
            0,
            0,
            0,
            0,
            hwnd,
            RESULTS as *mut c_void,
            instance,
            null_mut(),
        );
        if edit.is_null() || results.is_null() {
            return Err(io::Error::last_os_error());
        }
        (*state).edit = edit;
        (*state).results = results;
        let cue = if (*state).portuguese {
            "Buscar aplicativos"
        } else {
            "Search applications"
        };
        SendMessageW(edit, EM_SETCUEBANNER, 0, wide(cue).as_ptr() as isize);
        SendMessageW(edit, EM_LIMITTEXT, 256, 0);
        layout(hwnd, state);

        let modifiers = match shortcut {
            Shortcut::WinSpace => Some(MOD_WIN),
            Shortcut::CtrlAltSpace => Some(MOD_CONTROL | MOD_ALT),
            Shortcut::Disabled => None,
        };
        let registered = modifiers.is_some_and(|mods| {
            RegisterHotKey(hwnd, HOTKEY, mods | MOD_NOREPEAT, VK_SPACE.into()) != 0
        });
        if modifiers.is_some() && !registered {
            SetWindowTextW(
                hwnd,
                wide(if (*state).portuguese {
                    "Monolith | Atalho indisponivel"
                } else {
                    "Monolith | Shortcut unavailable"
                })
                .as_ptr(),
            );
        }
        let dock = if show_dock {
            match Dock::new(instance, hwnd, (*state).dark, (*state).portuguese, pins) {
                Ok(dock) => Some(dock),
                Err(error) => {
                    report(&format!("Could not open the Dock ({error})"));
                    None
                }
            }
        } else {
            None
        };
        (*state).can_hide = dock.is_some() || (resident && registered);
        if !(*state).can_hide {
            show(hwnd, state);
        }

        let mut msg = MSG::default();
        loop {
            match GetMessageW(&mut msg, null_mut(), 0, 0) {
                -1 => return Err(io::Error::last_os_error()),
                0 => return Ok(()),
                _ => {}
            }
            if msg.hwnd == hwnd
                && (msg.message == dock::SHOW_SPOTLIGHT
                    || (msg.message == WM_HOTKEY && msg.wParam == HOTKEY as usize))
            {
                show(hwnd, state);
                continue;
            }
            if msg.message == WM_KEYDOWN && (msg.hwnd == edit || msg.hwnd == results) {
                match msg.wParam as u16 {
                    VK_ESCAPE => {
                        dismiss(hwnd, state);
                        continue;
                    }
                    VK_F5 => {
                        (*state).desktop = None;
                        refresh(state);
                        continue;
                    }
                    VK_RETURN => {
                        open_selected(hwnd, state);
                        continue;
                    }
                    VK_TAB => {
                        SetFocus(if msg.hwnd == edit { results } else { edit });
                        continue;
                    }
                    VK_DOWN | VK_UP if msg.hwnd == edit => {
                        let index = SendMessageW(results, LB_GETCURSEL, 0, 0);
                        let next = if msg.wParam as u16 == VK_DOWN {
                            index + 1
                        } else {
                            index - 1
                        };
                        if next >= 0 && next < SendMessageW(results, LB_GETCOUNT, 0, 0) {
                            SendMessageW(results, LB_SETCURSEL, next as usize, 0);
                        }
                        continue;
                    }
                    _ => {}
                }
            }
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

unsafe fn show(hwnd: HWND, state: *mut State) {
    unsafe {
        if (*state).apps.is_none() {
            (*state).apps = Some(search::discover(roots()));
        }
        if IsWindowVisible(hwnd) == 0 {
            refresh(state);
            let mut cursor = POINT::default();
            if GetCursorPos(&mut cursor) != 0 {
                let monitor = MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST);
                let mut info = MONITORINFO {
                    cbSize: size_of::<MONITORINFO>() as u32,
                    ..Default::default()
                };
                let mut frame = RECT::default();
                if GetMonitorInfoW(monitor, &mut info) != 0 && GetWindowRect(hwnd, &mut frame) != 0
                {
                    let area = info.rcWork;
                    let width = frame.right - frame.left;
                    let height = frame.bottom - frame.top;
                    SetWindowPos(
                        hwnd,
                        null_mut(),
                        area.left + ((area.right - area.left - width) / 2).max(0),
                        area.top + ((area.bottom - area.top - height) / 2).max(0),
                        0,
                        0,
                        SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOSIZE,
                    );
                }
            }
        }
        ShowWindow(hwnd, SW_RESTORE);
        SetForegroundWindow(hwnd);
        SetFocus((*state).edit);
        SendMessageW((*state).edit, EM_SETSEL, 0, -1);
    }
}

unsafe fn dismiss(hwnd: HWND, state: *mut State) {
    unsafe {
        if (*state).can_hide {
            (*state).desktop = None;
            ShowWindow(hwnd, SW_HIDE);
        } else {
            DestroyWindow(hwnd);
        }
    }
}

unsafe fn layout(hwnd: HWND, state: *mut State) {
    unsafe {
        let dpi = GetDpiForWindow(hwnd).max(96) as i32;
        let scale = |value| value * dpi / 96;
        let font = CreateFontW(
            -scale(16),
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
        let edit = (*state).edit;
        let results = (*state).results;
        if !font.is_null() {
            SendMessageW(edit, WM_SETFONT, font as usize, 1);
            SendMessageW(results, WM_SETFONT, font as usize, 1);
            DeleteObject((*state).font);
            (*state).font = font;
        }
        let mut area = RECT::default();
        GetClientRect(hwnd, &mut area);
        let width = (area.right - scale(32)).max(1);
        MoveWindow(edit, scale(16), scale(18), width, scale(27), 1);
        MoveWindow(
            results,
            scale(16),
            scale(57),
            width,
            (area.bottom - scale(73)).max(1),
            1,
        );
        SendMessageW(
            edit,
            EM_SETMARGINS,
            3,
            (scale(8) | (scale(8) << 16)) as isize,
        );
    }
}

unsafe fn refresh(state: *mut State) {
    unsafe {
        let mut query = [0u16; 257];
        let len = GetWindowTextW((*state).edit, query.as_mut_ptr(), query.len() as i32);
        let results = (*state).results;
        let Some(apps) = (*state).apps.as_ref() else {
            return;
        };
        let query = String::from_utf16_lossy(&query[..len as usize]);
        SendMessageW(results, WM_SETREDRAW, 0, 0);
        SendMessageW(results, LB_RESETCONTENT, 0, 0);
        (*state).matches.clear();
        let query = query.trim_start();
        if let Some(title) = query.strip_prefix('@') {
            fill_windows(state, results, title);
        } else if let Some(expression) = query.strip_prefix('=') {
            (*state).desktop = None;
            if !expression.trim().is_empty() {
                add_row(results, &calculation_label(expression, (*state).portuguese));
            }
        } else {
            (*state).desktop = None;
            for index in search::search(
                apps.iter().map(|app| app.name.as_str()),
                query,
                RESULT_LIMIT,
            ) {
                if add_row(results, &apps[index].name) {
                    (*state).matches.push(Match::App(index));
                }
            }
        }
        if !(*state).matches.is_empty() {
            SendMessageW(results, LB_SETCURSEL, 0, 0);
        }
        SendMessageW(results, WM_SETREDRAW, 1, 0);
        InvalidateRect(results, std::ptr::null(), 1);
    }
}

unsafe fn add_row(results: HWND, label: &str) -> bool {
    unsafe { SendMessageW(results, LB_ADDSTRING, 0, wide(label).as_ptr() as isize) >= 0 }
}

unsafe fn fill_windows(state: *mut State, results: HWND, title: &str) {
    unsafe {
        let portuguese = (*state).portuguese;
        if (*state).desktop.is_none() {
            (*state).desktop = desktop::snapshot().ok();
        }
        let Some(desktop) = (*state).desktop.as_ref() else {
            add_row(
                results,
                if portuguese {
                    "Janelas indisponíveis"
                } else {
                    "Windows unavailable"
                },
            );
            return;
        };
        let hits = search::search(
            desktop.windows.iter().map(|window| window.title.as_str()),
            title,
            RESULT_LIMIT,
        );
        for index in hits {
            let window = &desktop.windows[index];
            let mut label = window.title.clone();
            if window.minimised {
                label.push_str(if portuguese {
                    "   ·   Minimizada"
                } else {
                    "   ·   Minimised"
                });
            } else if window.fullscreen {
                label.push_str(if portuguese {
                    "   ·   Tela cheia"
                } else {
                    "   ·   Fullscreen"
                });
            }
            if desktop.monitor_count > 1
                && let Some(monitor) = window.monitor
            {
                use std::fmt::Write;
                let _ = write!(label, "   ·   Monitor {}", monitor + 1);
            }
            if add_row(results, &label) {
                (*state).matches.push(Match::Window(index));
            }
        }
    }
}

fn calculation_label(expression: &str, portuguese: bool) -> String {
    match calculator::calculate(expression) {
        Ok(value) => value.to_string(),
        Err(error) => match (error, portuguese) {
            (CalculationError::Syntax, false) => "Invalid expression",
            (CalculationError::Syntax, true) => "Express\u{e3}o inv\u{e1}lida",
            (CalculationError::DivisionByZero, false) => "Division by zero",
            (CalculationError::DivisionByZero, true) => "Divis\u{e3}o por zero",
            (CalculationError::OutOfRange, false) => "Result out of range",
            (CalculationError::OutOfRange, true) => "Resultado fora do intervalo",
        }
        .into(),
    }
}

unsafe fn open_selected(hwnd: HWND, state: *mut State) {
    unsafe {
        let selection = SendMessageW((*state).results, LB_GETCURSEL, 0, 0);
        let matches = &(*state).matches;
        let index = match matches.get(selection as usize) {
            Some(&Match::App(index)) => index,
            Some(&Match::Window(index)) => {
                switch_window(hwnd, state, index);
                return;
            }
            None => return,
        };
        let Some(app) = (*state).apps.as_ref().and_then(|apps| apps.get(index)) else {
            return;
        };
        launch_shortcut(hwnd, &app.path);
    }
}

/// The path must come from the local Start menu catalogue.
pub(crate) unsafe fn launch_shortcut(hwnd: HWND, path: &std::path::Path) {
    let target: Vec<_> = path.as_os_str().encode_wide().chain([0]).collect();
    unsafe {
        let result = ShellExecuteW(
            hwnd,
            wide("open").as_ptr(),
            target.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        );
        if result as isize <= 32 {
            report(&format!(
                "Windows could not open the shortcut ({})",
                result as isize
            ));
        }
    }
}

unsafe fn switch_window(hwnd: HWND, state: *mut State, index: usize) {
    unsafe {
        let Some(window) = (*state)
            .desktop
            .as_ref()
            .and_then(|desktop| desktop.windows.get(index))
        else {
            return;
        };
        let target = window.target;
        match target.activate() {
            Ok(()) => dismiss(hwnd, state),
            Err(error) => {
                (*state).desktop = None;
                refresh(state);
                let message = match (error, (*state).portuguese) {
                    (ActivationError::Unavailable, false) => "This window is no longer available.",
                    (ActivationError::Unavailable, true) => "Esta janela não está mais disponível.",
                    (ActivationError::Refused, false) => "Windows could not switch to this window.",
                    (ActivationError::Refused, true) => {
                        "O Windows não conseguiu ativar esta janela."
                    }
                };
                report(message);
            }
        }
    }
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State;
        match msg {
            WM_DESTROY => {
                UnregisterHotKey(hwnd, HOTKEY);
                PostQuitMessage(0);
                0
            }
            _ if state.is_null() => DefWindowProcW(hwnd, msg, wp, lp),
            WM_DPICHANGED => {
                let rect = &*(lp as *const RECT);
                SetWindowPos(
                    hwnd,
                    null_mut(),
                    rect.left,
                    rect.top,
                    rect.right - rect.left,
                    rect.bottom - rect.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
                layout(hwnd, state);
                0
            }
            WM_ERASEBKGND => {
                let mut rect = RECT::default();
                GetClientRect(hwnd, &mut rect);
                FillRect(wp as HDC, &rect, (*state).background);
                1
            }
            WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX => {
                SetBkColor(
                    wp as HDC,
                    if (*state).dark {
                        rgb(20, 24, 32)
                    } else {
                        rgb(255, 255, 255)
                    },
                );
                SetTextColor(
                    wp as HDC,
                    if (*state).dark {
                        rgb(229, 231, 235)
                    } else {
                        rgb(24, 28, 34)
                    },
                );
                (*state).surface as isize
            }
            WM_COMMAND => {
                let id = wp & 0xffff;
                let notification = (wp >> 16) as u32 & 0xffff;
                if id == EDIT && notification == EN_CHANGE {
                    refresh(state);
                }
                if id == RESULTS && notification == LBN_DBLCLK {
                    open_selected(hwnd, state);
                }
                0
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}
