use crate::{
    calculator::{self, CalculationError},
    config::{self, Language},
    control_center::{self, ControlCenter},
    desktop::{self, ActivationError},
    dock::{self, Dock},
    files,
    hotkey::{self, Hotkey, Notice},
    icons,
    island::Island,
    mission::{self, Mission},
    notifications::{self, Notifications},
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
const SPOTLIGHT_FADE: usize = 0x51;
const SPOTLIGHT_FADE_MS: u64 = 120;
// Spotlight geometry in device-independent pixels.
const FIELD_TOP: i32 = 14;
const FIELD_HEIGHT: i32 = 42;
const GLYPH: i32 = 40;
const CHIP: i32 = 124;
const RESULTS_TOP: i32 = FIELD_TOP + FIELD_HEIGHT + 10;
const FOOTER: i32 = 36;

thread_local! {
    static FADE_STARTED: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

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
    /// Shell icons for application rows, extracted off the GUI thread.
    icons: Option<icons::Cache>,
    small_font: HFONT,
    glyph_font: HFONT,
    /// The registered opening chord shown in the search field.
    chord: Option<&'static str>,
}

struct SurfaceOptions {
    dock: Option<dock::Settings>,
    top_bar: bool,
    island: bool,
    notifications: bool,
    hide_notification_content: bool,
    control_center: bool,
    mission: bool,
}

impl Drop for State {
    fn drop(&mut self) {
        // The window and its children have already been destroyed.
        unsafe {
            DeleteObject(self.font);
            DeleteObject(self.small_font);
            DeleteObject(self.glyph_font);
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
            ("Files", "Arquivos", files::OPEN),
            ("Mission Control", "Controle de Janelas", mission::OPEN),
            ("Notifications", "Notificações", notifications::OPEN),
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
            let mut bounds = RECT::default();
            let mut display = MONITORINFO {
                cbSize: size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            let mut align = TPM_TOPALIGN;
            if GetWindowRect(hwnd, &mut bounds) != 0
                && GetMonitorInfoW(
                    MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST),
                    &mut display,
                ) != 0
            {
                // Keep the popup clear of its topmost anchor, including the first menu row.
                if display.rcWork.bottom - bounds.bottom >= bounds.top - display.rcWork.top {
                    point.y = bounds.bottom;
                } else {
                    point.y = bounds.top;
                    align = TPM_BOTTOMALIGN;
                }
            }
            SetForegroundWindow(hwnd);
            let selected = TrackPopupMenuEx(
                menu,
                TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON | align,
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

/// Start menu shortcuts, plus Apps folder entries such as packaged apps that have
/// no shortcut file.
static PREPARED: std::sync::Mutex<Option<std::sync::mpsc::Receiver<Vec<App>>>> =
    std::sync::Mutex::new(None);

fn start_catalogue() {
    let (sender, receiver) = std::sync::mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("monolith-catalogue".into())
        .spawn(move || {
            let _ = sender.send(catalogue());
        });
    if spawned.is_ok()
        && let Ok(mut prepared) = PREPARED.lock()
    {
        *prepared = Some(receiver);
    }
}

/// The prepared catalogue, waiting for the worker if it is still scanning; built
/// here only if no worker could start or it failed.
fn take_catalogue() -> Vec<App> {
    let prepared = PREPARED
        .lock()
        .ok()
        .and_then(|mut prepared| prepared.take());
    prepared
        .and_then(|receiver| receiver.recv().ok())
        .unwrap_or_else(catalogue)
}

fn catalogue() -> Vec<App> {
    let mut apps = search::discover(roots());
    search::merge(&mut apps, search::shell_apps());
    apps
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
    if safe {
        crate::taskbar::release_session();
    }
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
        let palette = Palette::new(settings.theme);
        let portuguese = matches!(settings.language, Language::Portuguese);
        if !safe && let Some(index) = args.iter().position(|arg| arg == "--files") {
            let folder = args
                .get(index + 1)
                .filter(|value| !value.as_encoded_bytes().starts_with(b"--"))
                .map(PathBuf::from);
            return files::run(folder, palette, portuguese, settings.files_ide);
        }
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
            portuguese,
            can_hide: false,
            previous_foreground: null_mut(),
            icons: None,
            small_font: null_mut(),
            glyph_font: null_mut(),
            chord: None,
        });
        let hwnd = CreateWindowExW(
            overlay::EX_STYLE | WS_EX_TOPMOST | WS_EX_LAYERED,
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
        // A layered window stays invisible until it has an opacity.
        SetLayeredWindowAttributes(hwnd, 0, 255, LWA_ALPHA);
        // The callback takes exclusive state access only where it does not
        // dispatch messages that can re-enter a state-reading branch.
        let state_ptr = Box::into_raw(state);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, state_ptr as isize);
        let replace_taskbar = !safe && args.iter().any(|arg| arg == "--replace-taskbar");
        let show_dock =
            !safe && (settings.dock || replace_taskbar || args.iter().any(|arg| arg == "--dock"));
        let show_top_bar = !safe && (settings.top_bar || args.iter().any(|arg| arg == "--top-bar"));
        let show_island = !safe && (settings.island || args.iter().any(|arg| arg == "--island"));
        // Build the application catalogue off the GUI thread before Spotlight first opens.
        start_catalogue();
        let hotkey = Hotkey::bind(
            hwnd,
            settings.shortcut,
            settings.win_space_override,
            settings.workspace_shortcuts,
            settings.desktop_moves,
            settings.fullscreen_workspaces,
            settings.close_empty_workspaces,
        );
        (*state_ptr).chord = hotkey.chord();
        let order_path = config_path.with_file_name("dock-order.toml");
        let pins = if show_dock && !settings.dock_pins.is_empty() {
            let apps = (*state_ptr).apps.get_or_insert_with(take_catalogue);
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
                    replace_taskbar,
                }),
                top_bar: show_top_bar,
                island: show_island,
                notifications: !safe
                    && (settings.notifications || args.iter().any(|arg| arg == "--notifications")),
                hide_notification_content: settings.hide_notification_content,
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
                | LBS_NOINTEGRALHEIGHT as u32
                | LBS_OWNERDRAWFIXED as u32
                | LBS_HASSTRINGS as u32,
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
        (*state).icons = icons::Cache::new(hwnd).ok();
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
        // Register the top AppBar before the Dock leases the existing bottom band.
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
        // Lives until the GUI loop ends; dropping it removes the hook and stops the worker.
        let notifications = if surfaces.notifications {
            match Notifications::new(
                instance,
                (*state).palette,
                (*state).portuguese,
                surfaces.hide_notification_content,
            ) {
                Ok(notifications) => Some(notifications),
                Err(error) => {
                    report(&format!("Could not start notifications ({error})"));
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
            if msg.hwnd == hwnd && msg.message == files::OPEN {
                if let Err(error) = env::current_exe()
                    .and_then(|exe| std::process::Command::new(exe).arg("--files").spawn())
                {
                    report(&format!("Could not open Files ({error})"));
                }
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
            if msg.hwnd == hwnd && msg.message == notifications::OPEN {
                match &notifications {
                    Some(notifications) => notifications.open_centre(),
                    None => report(if (*state).portuguese {
                        "Ative notifications = true na configuração"
                    } else {
                        "Set notifications = true in the configuration"
                    }),
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
            if msg.hwnd == hwnd
                && msg.message == WM_HOTKEY
                && msg.wParam == hotkey::MISSION_ID as usize
            {
                // Win+Tab toggles Mission Control; the hotkey grants foreground rights.
                match &mission {
                    Some(overview) if overview.handle() == GetForegroundWindow() => {
                        PostMessageW(overview.handle(), WM_CLOSE, 0, 0);
                    }
                    _ => {
                        PostMessageW(hwnd, mission::OPEN, 0, 0);
                    }
                }
                continue;
            }
            if msg.hwnd == hwnd && msg.message == WM_HOTKEY && msg.wParam == hotkey::ID as usize {
                if IsWindowVisible(hwnd) != 0 && GetForegroundWindow() == hwnd {
                    dismiss(hwnd, state, true);
                } else {
                    show(hwnd, state, dock.as_ref());
                }
                hotkey.handled();
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
                        // Ctrl+Enter opens the application in a new native workspace.
                        if GetKeyState(i32::from(VK_CONTROL)) < 0 {
                            open_selected_in_new_workspace(hwnd, state);
                        } else {
                            open_selected(hwnd, state);
                        }
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
        let opening = IsWindowVisible(hwnd) == 0;
        if opening {
            (*state).previous_foreground = GetForegroundWindow();
            (*state).desktop = dock.and_then(Dock::inventory);
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
        }
        if opening && overlay::animations_enabled() {
            // Fade in; keyboard focus is immediate because opacity does not affect input.
            SetLayeredWindowAttributes(hwnd, 0, 0, LWA_ALPHA);
            SetTimer(hwnd, SPOTLIGHT_FADE, 15, None);
        }
        ShowWindow(hwnd, SW_SHOW);
        SetForegroundWindow(hwnd);
        SetFocus((*state).edit);
        SendMessageW((*state).edit, EM_SETSEL, 0, -1);
        if (*state).apps.is_none() {
            (*state).apps = Some(take_catalogue());
        }
        if opening {
            refresh(state);
            fit_results(hwnd, state);
            // Start the fade clock after the synchronous first-open work, so the
            // transition is visible rather than skipped.
            FADE_STARTED.set(windows_sys::Win32::System::SystemInformation::GetTickCount64());
        }
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
            (*state).small_font =
                replace_font((*state).small_font, scale(12), FW_NORMAL, "Segoe UI");
            (*state).glyph_font = replace_font(
                (*state).glyph_font,
                scale(16),
                FW_NORMAL,
                "Segoe Fluent Icons",
            );
            SendMessageW(results, LB_SETITEMHEIGHT, 0, scale(44) as isize);
        }
        let mut area = RECT::default();
        GetClientRect(hwnd, &mut area);
        let width = (area.right - scale(32)).max(1);
        // The edit sits inside the painted field, after the search glyph and before
        // the shortcut chip.
        let chip = if (*state).chord.is_some() {
            scale(CHIP)
        } else {
            0
        };
        MoveWindow(
            edit,
            scale(16 + GLYPH),
            scale(FIELD_TOP + (FIELD_HEIGHT - 27) / 2),
            (width - scale(GLYPH) - chip - scale(12)).max(1),
            scale(27),
            1,
        );
        MoveWindow(
            results,
            scale(16),
            scale(RESULTS_TOP),
            width,
            (area.bottom - scale(RESULTS_TOP + FOOTER)).max(1),
            1,
        );
        InvalidateRect(hwnd, std::ptr::null(), 1);
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
            overlay::scale(FIELD_TOP * 2 + FIELD_HEIGHT, dpi)
        } else {
            ShowWindow(results, SW_SHOWNA);
            let row = (SendMessageW(results, LB_GETITEMHEIGHT, 0, 0) as i32).max(1);
            overlay::scale(RESULTS_TOP + FOOTER, dpi) + count * row
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
            request_icons(state);
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

/// Create a font and release the one it replaces; keeps the old one on failure.
unsafe fn replace_font(old: HFONT, height: i32, weight: u32, face: &str) -> HFONT {
    unsafe {
        let font = CreateFontW(
            -height,
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
            wide(face).as_ptr(),
        );
        if font.is_null() {
            return old;
        }
        if !old.is_null() {
            DeleteObject(old);
        }
        font
    }
}

/// Ask the icon worker for the application rows that can be visible.
unsafe fn request_icons(state: *mut State) {
    unsafe {
        let Some(apps) = (*state).apps.as_ref() else {
            return;
        };
        let paths: Vec<PathBuf> = (*state)
            .matches
            .iter()
            .take(12)
            .filter_map(|found| match found {
                Match::App(index) => apps.get(*index).map(|app| app.path.clone()),
                Match::Window(_) => None,
            })
            .collect();
        if let Some(cache) = (*state).icons.as_mut() {
            cache.request(&paths);
        }
    }
}

/// Blend `accent` into `base` by `weight` out of 255.
fn tint(base: u32, accent: u32, weight: u32) -> u32 {
    let channel = |shift: u32| {
        let (b, a) = ((base >> shift) & 0xff, (accent >> shift) & 0xff);
        (b * (255 - weight) + a * weight) / 255
    };
    channel(0) | (channel(8) << 8) | (channel(16) << 16)
}

unsafe fn draw_text(hdc: HDC, font: HFONT, colour: u32, text: &str, mut bounds: RECT, flags: u32) {
    unsafe {
        let mut wide: Vec<u16> = text.encode_utf16().collect();
        SelectObject(hdc, font);
        SetTextColor(hdc, colour);
        DrawTextW(
            hdc,
            wide.as_mut_ptr(),
            wide.len() as i32,
            &mut bounds,
            flags | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX,
        );
    }
}

/// One result row: icon, name, kind and, when selected, the Enter action.
unsafe fn draw_result(state: *mut State, item: &windows_sys::Win32::UI::Controls::DRAWITEMSTRUCT) {
    unsafe {
        if item.itemID == u32::MAX {
            return;
        }
        let palette = (*state).palette;
        let dpi = GetDpiForWindow(item.hwndItem);
        let scale = |value| overlay::scale(value, dpi);
        let selected = item.itemState & windows_sys::Win32::UI::Controls::ODS_SELECTED != 0;
        let area = item.rcItem;
        let background = if selected {
            tint(palette.surface, palette.accent, 46)
        } else {
            palette.surface
        };
        let saved = SaveDC(item.hDC);
        overlay::fill(item.hDC, &area, background);
        SetBkMode(item.hDC, TRANSPARENT as i32);
        let length = SendMessageW(item.hwndItem, LB_GETTEXTLEN, item.itemID as usize, 0);
        let mut label = vec![0_u16; length.max(0) as usize + 1];
        SendMessageW(
            item.hwndItem,
            LB_GETTEXT,
            item.itemID as usize,
            label.as_mut_ptr() as isize,
        );
        let label = String::from_utf16_lossy(&label[..length.max(0) as usize]);
        let portuguese = (*state).portuguese;
        let matches = &(*state).matches;
        let found = matches.get(item.itemID as usize);
        let (kind, action, glyph) = match found {
            Some(Match::App(_)) => (
                if portuguese {
                    "Aplicativo"
                } else {
                    "Application"
                },
                if portuguese { "Abrir" } else { "Open" },
                None,
            ),
            Some(Match::Window(_)) => (
                if portuguese {
                    "Janela aberta"
                } else {
                    "Open window"
                },
                if portuguese { "Alternar" } else { "Switch" },
                // Segoe Fluent Icons: window.
                Some("\u{E737}"),
            ),
            // Calculations and status lines carry no action.
            None => ("", "", Some("\u{E8EF}")),
        };
        let icon_size = scale(24);
        let icon = RECT {
            left: area.left + scale(10),
            top: area.top + (area.bottom - area.top - icon_size) / 2,
            right: area.left + scale(10) + icon_size,
            bottom: area.top + (area.bottom - area.top - icon_size) / 2 + icon_size,
        };
        let drawn = match found {
            Some(Match::App(index)) => (*state)
                .apps
                .as_ref()
                .and_then(|apps| apps.get(*index))
                .zip((*state).icons.as_ref())
                .is_some_and(|(app, cache)| {
                    cache.draw(&app.path, item.hDC, icon, icon_size, background, false)
                }),
            _ => false,
        };
        if !drawn && let Some(glyph) = glyph {
            draw_text(
                item.hDC,
                (*state).glyph_font,
                palette.text,
                glyph,
                icon,
                DT_CENTER,
            );
        }
        let left = icon.right + scale(12);
        let action_width = if selected && !action.is_empty() {
            scale(96)
        } else {
            0
        };
        let right = area.right - scale(12) - action_width;
        let secondary = tint(palette.text, palette.surface, 110);
        if kind.is_empty() {
            let line = RECT {
                left,
                right,
                ..area
            };
            draw_text(item.hDC, (*state).font, palette.text, &label, line, DT_LEFT);
        } else {
            let middle = area.top + (area.bottom - area.top) / 2;
            let name = RECT {
                left,
                top: area.top + scale(3),
                right,
                bottom: middle + scale(2),
            };
            draw_text(item.hDC, (*state).font, palette.text, &label, name, DT_LEFT);
            let detail = RECT {
                left,
                top: middle + scale(1),
                right,
                bottom: area.bottom - scale(4),
            };
            draw_text(
                item.hDC,
                (*state).small_font,
                secondary,
                kind,
                detail,
                DT_LEFT,
            );
        }
        if action_width > 0 {
            let hint = RECT {
                left: area.right - scale(12) - action_width,
                right: area.right - scale(12),
                ..area
            };
            draw_text(
                item.hDC,
                (*state).small_font,
                palette.accent,
                &format!("{action}  \u{21B5}"),
                hint,
                DT_RIGHT,
            );
        }
        RestoreDC(item.hDC, saved);
    }
}

/// The search field frame, glyph and shortcut chip, and the keyboard guidance footer.
unsafe fn paint_spotlight(hwnd: HWND, state: *mut State) {
    unsafe {
        let mut paint = PAINTSTRUCT::default();
        let hdc = BeginPaint(hwnd, &mut paint);
        if hdc.is_null() {
            return;
        }
        let palette = (*state).palette;
        let scale = |value| overlay::scale(value, GetDpiForWindow(hwnd));
        let mut area = RECT::default();
        GetClientRect(hwnd, &mut area);
        overlay::fill(hdc, &area, palette.background);
        SetBkMode(hdc, TRANSPARENT as i32);
        let field = RECT {
            left: scale(16),
            top: scale(FIELD_TOP),
            right: area.right - scale(16),
            bottom: scale(FIELD_TOP + FIELD_HEIGHT),
        };
        // A restrained blue outline marks keyboard focus in the field.
        let focused = GetFocus() == (*state).edit;
        let outline = if focused {
            palette.accent
        } else {
            tint(palette.surface, palette.text, 40)
        };
        overlay::fill(hdc, &field, outline);
        let border = scale(1).max(1);
        let inner = RECT {
            left: field.left + border,
            top: field.top + border,
            right: field.right - border,
            bottom: field.bottom - border,
        };
        overlay::fill(hdc, &inner, palette.surface);
        let glyph = RECT {
            left: field.left,
            right: field.left + scale(GLYPH),
            ..field
        };
        let secondary = tint(palette.text, palette.surface, 110);
        // Segoe Fluent Icons: search.
        draw_text(
            hdc,
            (*state).glyph_font,
            secondary,
            "\u{E721}",
            glyph,
            DT_CENTER,
        );
        if let Some(chord) = (*state).chord {
            let chip = RECT {
                left: field.right - scale(CHIP),
                top: field.top + scale(9),
                right: field.right - scale(8),
                bottom: field.bottom - scale(9),
            };
            overlay::fill(hdc, &chip, palette.pressed);
            draw_text(hdc, (*state).small_font, secondary, chord, chip, DT_CENTER);
        }
        if IsWindowVisible((*state).results) != 0 {
            let portuguese = (*state).portuguese;
            let guidance = if portuguese {
                "\u{2191} \u{2193}  Navegar      \u{21B5}  Abrir      Ctrl+\u{21B5}  Nova área      Esc  Fechar"
            } else {
                "\u{2191} \u{2193}  Navigate      \u{21B5}  Open      Ctrl+\u{21B5}  New workspace      Esc  Close"
            };
            let footer = RECT {
                left: scale(20),
                top: area.bottom - scale(FOOTER),
                right: area.right - scale(20),
                bottom: area.bottom,
            };
            draw_text(
                hdc,
                (*state).small_font,
                secondary,
                guidance,
                footer,
                DT_LEFT,
            );
        }
        EndPaint(hwnd, &paint);
    }
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

/// Open the selected application in a new native workspace. Windows have no
/// dedicated workspace, so a window selection falls back to switching to it.
unsafe fn open_selected_in_new_workspace(hwnd: HWND, state: *mut State) {
    unsafe {
        let selection = SendMessageW((*state).results, LB_GETCURSEL, 0, 0);
        let matches = &(*state).matches;
        let Some(&Match::App(index)) = matches.get(selection as usize) else {
            open_selected(hwnd, state);
            return;
        };
        let Some(path) = (*state)
            .apps
            .as_ref()
            .and_then(|apps| apps.get(index))
            .map(|app| app.path.clone())
        else {
            return;
        };
        // Hide first so Spotlight does not follow the user to the new desktop.
        dismiss(hwnd, state, false);
        if let Err(error) = crate::workspaces::open_in_new(path) {
            report_for(hwnd, &format!("Could not open a new workspace ({error})"));
        }
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
            WM_TIMER if wp == SPOTLIGHT_FADE => {
                let elapsed = windows_sys::Win32::System::SystemInformation::GetTickCount64()
                    .saturating_sub(FADE_STARTED.get());
                let (alpha, finished) = overlay::fade_alpha(true, elapsed, SPOTLIGHT_FADE_MS);
                SetLayeredWindowAttributes(hwnd, 0, alpha, LWA_ALPHA);
                if finished {
                    KillTimer(hwnd, SPOTLIGHT_FADE);
                }
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
            WM_PAINT => {
                paint_spotlight(hwnd, state);
                0
            }
            WM_DRAWITEM if wp == RESULTS && lp != 0 => {
                draw_result(
                    state,
                    &*(lp as *const windows_sys::Win32::UI::Controls::DRAWITEMSTRUCT),
                );
                1
            }
            icons::READY => {
                if (*state).icons.as_mut().is_some_and(icons::Cache::receive) {
                    request_icons(state);
                    InvalidateRect((*state).results, std::ptr::null(), 0);
                }
                0
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
                if id == EDIT && matches!(notification, EN_SETFOCUS | EN_KILLFOCUS) {
                    // Repaint the field outline that marks keyboard focus.
                    InvalidateRect(hwnd, std::ptr::null(), 0);
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
