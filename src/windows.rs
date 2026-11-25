use crate::{
    calculator::{self, CalculationError},
    config::{self, Language},
    control_center::{self, ControlCenter},
    desktop::{self, ActivationError},
    dock::{self, Dock},
    hotkey::{self, Hotkey, Notice},
    island::Island,
    mission::{self, Mission},
    overlay::{self, Palette},
    search::{self, App},
    top_bar::TopBar,
};
use std::{
    env, ffi::c_void, io, mem::size_of, os::windows::ffi::OsStrExt, path::PathBuf, ptr::null_mut,
    rc::Rc,
};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
    Graphics::Gdi::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        Controls::{EM_LIMITTEXT, EM_SETCUEBANNER, EM_SETMARGINS, EM_SETSEL},
        HiDpi::*,
        Input::KeyboardAndMouse::*,
        Shell::ShellExecuteW,
        WindowsAndMessaging::*,
    },
};

const EDIT: usize = 100;
const RESULTS: usize = 101;
const RESULT_LIMIT: usize = 60;
const DISMISS_INACTIVE: u32 = WM_APP + 2;

enum Match {
    App(usize),
    Window(usize),
}

struct State {
    apps: Option<Vec<App>>,
    desktop: Option<Rc<desktop::Snapshot>>,
    matches: Vec<Match>,
    edit: HWND,
    results: HWND,
    font: HFONT,
    font_dpi: u32,
    background: HBRUSH,
    surface: HBRUSH,
    palette: Palette,
    portuguese: bool,
    can_hide: bool,
    previous_foreground: HWND,
}

struct SurfaceOptions {
    dock: Option<dock::Settings>,
    top_bar: bool,
    island: bool,
    control_center: bool,
    mission: bool,
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

pub fn report(message: &str) {
    report_for(null_mut(), message);
}

pub(crate) fn report_for(owner: HWND, message: &str) {
    // Strings remain valid until the modal call returns.
    unsafe {
        MessageBoxW(
            owner,
            wide(message).as_ptr(),
            wide("Monolith").as_ptr(),
            MB_OK | MB_ICONWARNING,
        );
    }
}

pub(crate) unsafe fn shell_menu(hwnd: HWND, launcher: HWND, portuguese: bool) {
    unsafe {
        let menu = CreatePopupMenu();
        if menu.is_null() {
            return;
        }
        let actions = [
            ("Search", "Buscar", dock::SHOW_SPOTLIGHT),
            ("Mission Control", "Controle de Janelas", mission::OPEN),
            (
                "Control Center",
                "Central de Controle",
                control_center::OPEN,
            ),
            ("Exit", "Sair", WM_CLOSE),
        ];
        for (index, (english, translated, _)) in actions.iter().enumerate() {
            let label = if portuguese { translated } else { english };
            AppendMenuW(menu, MF_STRING, index + 1, wide(label).as_ptr());
        }
        let mut point = POINT::default();
        if GetCursorPos(&mut point) != 0 {
            SetForegroundWindow(hwnd);
            let selected = TrackPopupMenuEx(
                menu,
                TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON,
                point.x,
                point.y,
                hwnd,
                null_mut(),
            );
            PostMessageW(hwnd, WM_NULL, 0, 0);
            if let Some((_, _, action)) = (selected as usize)
                .checked_sub(1)
                .and_then(|index| actions.get(index))
            {
                PostMessageW(launcher, *action, hwnd as usize, 0);
            }
        }
        DestroyMenu(menu);
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

        let palette = Palette::new(settings.theme);
        let mut state = Box::new(State {
            apps: None,
            desktop: None,
            matches: Vec::new(),
            edit: null_mut(),
            results: null_mut(),
            font: null_mut(),
            font_dpi: 0,
            background: CreateSolidBrush(palette.background),
            surface: CreateSolidBrush(palette.surface),
            palette,
            portuguese: matches!(settings.language, Language::Portuguese),
            can_hide: false,
            previous_foreground: null_mut(),
        });
        let hwnd = CreateWindowExW(
            overlay::EX_STYLE | WS_EX_TOPMOST,
            class_name.as_ptr(),
            wide("Monolith").as_ptr(),
            overlay::STYLE | WS_CLIPCHILDREN,
            0,
            0,
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
        let show_top_bar = !safe && (settings.top_bar || args.iter().any(|arg| arg == "--top-bar"));
        let show_island = !safe && (settings.island || args.iter().any(|arg| arg == "--island"));
        let hotkey = Hotkey::bind(hwnd, settings.shortcut);
        let order_path = config_path.with_file_name("dock-order.toml");
        let pins = if show_dock && !settings.dock_pins.is_empty() {
            let apps = (*state_ptr)
                .apps
                .get_or_insert_with(|| search::discover(roots()));
            let mut pins = search::resolve_pins(apps, &settings.dock_pins);
            search::apply_pin_order(&mut pins, &config::load_dock_order(&order_path));
            pins
        } else {
            Vec::new()
        };
        let result = run_window(
            hwnd,
            instance,
            state_ptr,
            &hotkey,
            resident,
            SurfaceOptions {
                dock: show_dock.then_some(dock::Settings {
                    pins,
                    order_path,
                    monitor: settings.dock_monitor,
                    auto_hide: settings.dock_auto_hide,
                    original_icons: settings.dock_original_icons,
                }),
                top_bar: show_top_bar,
                island: show_island,
                control_center: !safe && args.iter().any(|arg| arg == "--control-center"),
                mission: !safe && args.iter().any(|arg| arg == "--mission-control"),
            },
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
    hotkey: &Hotkey,
    resident: bool,
    surfaces: SurfaceOptions,
) -> io::Result<()> {
    unsafe {
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
        SendMessageW(edit, EM_SETCUEBANNER, 1, wide(cue).as_ptr() as isize);
        SendMessageW(edit, EM_LIMITTEXT, 256, 0);
        layout(hwnd, state);

        if let Some(notice) = &hotkey.notice {
            let detail = match (notice, (*state).portuguese) {
                (Notice::Alternate, _) => "Ctrl+Alt+Space".to_owned(),
                (Notice::Unavailable(error), true) => format!("atalho indisponível, Win32 {error}"),
                (Notice::Unavailable(error), false) => {
                    format!("shortcut unavailable, Win32 {error}")
                }
            };
            SendMessageW(
                edit,
                EM_SETCUEBANNER,
                1,
                wide(&format!("{cue} ({detail})")).as_ptr() as isize,
            );
        }
        let dock = if let Some(settings) = surfaces.dock {
            match Dock::new(
                instance,
                hwnd,
                (*state).palette,
                (*state).portuguese,
                settings,
            ) {
                Ok(dock) => Some(dock),
                Err(error) => {
                    report(&format!("Could not open the Dock ({error})"));
                    None
                }
            }
        } else {
            None
        };
        let top_bar = if surfaces.top_bar {
            match TopBar::new(instance, hwnd, (*state).palette, (*state).portuguese) {
                Ok(bar) => Some(bar),
                Err(error) => {
                    report(&format!("Could not open the Top Bar ({error})"));
                    None
                }
            }
        } else {
            None
        };
        let island = if surfaces.island {
            match Island::new(
                instance,
                hwnd,
                top_bar.as_ref().map_or(null_mut(), TopBar::handle),
                (*state).palette,
                (*state).portuguese,
            ) {
                Ok(island) => Some(island),
                Err(error) => {
                    report(&format!("Could not open the Island ({error})"));
                    None
                }
            }
        } else {
            None
        };
        (*state).can_hide = dock.is_some()
            || top_bar.is_some()
            || ((resident || island.is_some()) && hotkey.available());
        let mut mission = if surfaces.mission {
            Some(Mission::new(
                instance,
                hwnd,
                null_mut(),
                (*state).palette,
                (*state).portuguese,
                dock.as_ref().and_then(Dock::inventory),
            )?)
        } else {
            None
        };
        let mut center = if surfaces.control_center && mission.is_none() {
            Some(ControlCenter::new(
                instance,
                hwnd,
                null_mut(),
                (*state).palette,
                (*state).portuguese,
            )?)
        } else {
            None
        };
        if !(*state).can_hide && center.is_none() && mission.is_none() {
            show(hwnd, state, dock.as_ref());
        }

        let mut msg = MSG::default();
        loop {
            match GetMessageW(&mut msg, null_mut(), 0, 0) {
                -1 => return Err(io::Error::last_os_error()),
                0 => return Ok(()),
                _ => {}
            }
            if msg.hwnd == hwnd && msg.message == dock::SHOW_SPOTLIGHT {
                show(hwnd, state, dock.as_ref());
                continue;
            }
            if msg.hwnd == hwnd && msg.message == control_center::OPEN {
                if let Some(panel) = &center {
                    panel.focus();
                } else {
                    match ControlCenter::new(
                        instance,
                        hwnd,
                        msg.wParam as HWND,
                        (*state).palette,
                        (*state).portuguese,
                    ) {
                        Ok(panel) => center = Some(panel),
                        Err(error) => report(&format!("Could not open Control Center ({error})")),
                    }
                }
                continue;
            }
            if msg.hwnd == hwnd && msg.message == control_center::CLOSE {
                if let Some(panel) = center.as_ref().filter(|panel| {
                    panel.handle() == msg.wParam as HWND
                        && (msg.lParam != 0 || panel.handle() != GetForegroundWindow())
                }) {
                    if msg.lParam == 1 {
                        panel.restore_focus();
                    }
                    center = None;
                    if !(*state).can_hide && mission.is_none() && IsWindowVisible(hwnd) == 0 {
                        return Ok(());
                    }
                }
                continue;
            }
            if msg.hwnd == hwnd && msg.message == mission::OPEN {
                if let Some(overview) = &mission {
                    overview.focus();
                } else {
                    match Mission::new(
                        instance,
                        hwnd,
                        msg.wParam as HWND,
                        (*state).palette,
                        (*state).portuguese,
                        dock.as_ref().and_then(Dock::inventory),
                    ) {
                        Ok(overview) => mission = Some(overview),
                        Err(error) => report(&format!("Could not open Mission Control ({error})")),
                    }
                }
                continue;
            }
            if msg.hwnd == hwnd && msg.message == mission::CLOSE {
                if mission.as_ref().is_some_and(|overview| {
                    overview.handle() == msg.wParam as HWND
                        && (msg.lParam != 0 || overview.handle() != GetForegroundWindow())
                }) {
                    mission = None;
                    if !(*state).can_hide && center.is_none() && IsWindowVisible(hwnd) == 0 {
                        return Ok(());
                    }
                }
                continue;
            }
            if mission
                .as_ref()
                .is_some_and(|overview| overview.handle_key(&msg))
            {
                continue;
            }
            if center.as_ref().is_some_and(|panel| panel.handle_key(&msg)) {
                continue;
            }
            if msg.hwnd == hwnd && msg.message == WM_HOTKEY && msg.wParam == hotkey::ID as usize {
                if IsWindowVisible(hwnd) != 0 && GetForegroundWindow() == hwnd {
                    dismiss(hwnd, state, true);
                } else {
                    show(hwnd, state, dock.as_ref());
                }
                continue;
            }
            if msg.message == WM_KEYDOWN && (msg.hwnd == edit || msg.hwnd == results) {
                match msg.wParam as u16 {
                    VK_ESCAPE => {
                        dismiss(hwnd, state, true);
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
                        SetFocus(if msg.hwnd == edit && IsWindowVisible(results) != 0 {
                            results
                        } else {
                            edit
                        });
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

unsafe fn show(hwnd: HWND, state: *mut State, dock: Option<&Dock>) {
    unsafe {
        if (*state).apps.is_none() {
            (*state).apps = Some(search::discover(roots()));
        }
        if IsWindowVisible(hwnd) == 0 {
            (*state).previous_foreground = GetForegroundWindow();
            (*state).desktop = dock.and_then(Dock::inventory);
            refresh(state);
            let foreground = GetForegroundWindow();
            let monitor = if foreground.is_null() {
                let mut cursor = POINT::default();
                GetCursorPos(&mut cursor);
                MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST)
            } else {
                MonitorFromWindow(foreground, MONITOR_DEFAULTTONEAREST)
            };
            let mut info = MONITORINFO {
                cbSize: size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            if GetMonitorInfoW(monitor, &mut info) != 0 {
                // Move the hidden window first so GetDpiForWindow uses the target display.
                SetWindowPos(
                    hwnd,
                    null_mut(),
                    info.rcWork.left,
                    info.rcWork.top,
                    1,
                    1,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
                let bounds = overlay::spotlight_bounds(info.rcWork, GetDpiForWindow(hwnd));
                SetWindowPos(
                    hwnd,
                    null_mut(),
                    bounds.left,
                    bounds.top,
                    bounds.right - bounds.left,
                    bounds.bottom - bounds.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
                layout(hwnd, state);
            }
            fit_results(hwnd, state);
        }
        ShowWindow(hwnd, SW_SHOW);
        SetForegroundWindow(hwnd);
        SetFocus((*state).edit);
        SendMessageW((*state).edit, EM_SETSEL, 0, -1);
    }
}

unsafe fn dismiss(hwnd: HWND, state: *mut State, return_focus: bool) {
    unsafe {
        let previous = std::mem::replace(&mut (*state).previous_foreground, null_mut());
        if return_focus
            && GetForegroundWindow() == hwnd
            && previous != hwnd
            && IsWindowVisible(previous) != 0
            && IsIconic(previous) == 0
        {
            SetForegroundWindow(previous);
        }
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
        let dpi = GetDpiForWindow(hwnd);
        let scale = |value| overlay::scale(value, dpi);
        let edit = (*state).edit;
        let results = (*state).results;
        if dpi != (*state).font_dpi {
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
            if !font.is_null() {
                SendMessageW(edit, WM_SETFONT, font as usize, 1);
                SendMessageW(results, WM_SETFONT, font as usize, 1);
                DeleteObject((*state).font);
                (*state).font = font;
                (*state).font_dpi = dpi;
            }
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

unsafe fn fit_results(hwnd: HWND, state: *mut State) {
    unsafe {
        let results = (*state).results;
        let count = SendMessageW(results, LB_GETCOUNT, 0, 0).clamp(0, 10) as i32;
        let dpi = GetDpiForWindow(hwnd);
        let height = if count == 0 {
            if GetFocus() == results {
                SetFocus((*state).edit);
            }
            ShowWindow(results, SW_HIDE);
            overlay::scale(61, dpi)
        } else {
            ShowWindow(results, SW_SHOWNA);
            let row = (SendMessageW(results, LB_GETITEMHEIGHT, 0, 0) as i32).max(1);
            overlay::scale(73, dpi) + count * row
        };
        let mut frame = RECT::default();
        if GetWindowRect(hwnd, &mut frame) == 0 {
            return;
        }
        let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let height = if GetMonitorInfoW(monitor, &mut info) != 0 {
            height.min((info.rcWork.bottom - frame.top).max(1))
        } else {
            height
        };
        if height != frame.bottom - frame.top {
            SetWindowPos(
                hwnd,
                null_mut(),
                0,
                0,
                frame.right - frame.left,
                height,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
            layout(hwnd, state);
        }
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
        let hwnd = GetParent((*state).edit);
        if IsWindowVisible(hwnd) != 0 {
            fit_results(hwnd, state);
        }
    }
}

unsafe fn add_row(results: HWND, label: &str) -> bool {
    unsafe { SendMessageW(results, LB_ADDSTRING, 0, wide(label).as_ptr() as isize) >= 0 }
}

unsafe fn fill_windows(state: *mut State, results: HWND, title: &str) {
    unsafe {
        let portuguese = (*state).portuguese;
        if (*state).desktop.is_none() {
            (*state).desktop = desktop::snapshot().ok().map(Rc::new);
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
            report_for(
                hwnd,
                &format!("Windows could not open the shortcut ({})", result as isize),
            );
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
            Ok(()) => dismiss(hwnd, state, false),
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
                report_for(hwnd, message);
            }
        }
    }
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State;
        match msg {
            WM_DESTROY => {
                PostQuitMessage(0);
                0
            }
            _ if state.is_null() => DefWindowProcW(hwnd, msg, wp, lp),
            WM_ACTIVATE if wp & 0xffff == WA_INACTIVE as usize => {
                PostMessageW(hwnd, DISMISS_INACTIVE, 0, 0);
                0
            }
            DISMISS_INACTIVE => {
                let foreground = GetForegroundWindow();
                if IsWindowVisible(hwnd) != 0
                    && foreground != hwnd
                    && GetAncestor(foreground, GA_ROOTOWNER) != hwnd
                {
                    dismiss(hwnd, state, false);
                }
                0
            }
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
                SetBkColor(wp as HDC, (*state).palette.surface);
                SetTextColor(wp as HDC, (*state).palette.text);
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
