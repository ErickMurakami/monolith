//! Opt-in native Dock. All handles, hooks and callbacks belong to the GUI thread.

use crate::{
    config::{self, DockMonitor},
    desktop::{self, AppIdentity, WindowGroup},
    icons,
    overlay::{self, Palette},
    preview::{self, Preview},
    search::App,
    taskbar::{self, Lease, Placement},
    windows::{launch_shortcut, report_for, shell_menu, wide},
};
use std::{
    cell::Cell,
    collections::HashMap,
    ffi::c_void,
    io,
    mem::size_of,
    path::PathBuf,
    ptr::null_mut,
    rc::Rc,
    sync::mpsc::{self, Receiver},
    thread,
};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
    Graphics::Gdi::*,
    System::RemoteDesktop::{
        NOTIFY_FOR_THIS_SESSION, WTSRegisterSessionNotification, WTSUnRegisterSessionNotification,
    },
    UI::{
        Accessibility::*,
        Controls::*,
        HiDpi::GetDpiForWindow,
        Input::KeyboardAndMouse::{
            ReleaseCapture, SetCapture, TME_HOVER, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent,
        },
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::*,
    },
};

pub const SHOW_SPOTLIGHT: u32 = WM_APP + 1;
const REFRESH: usize = 1;
const HIDE: usize = 2;
const SPOTLIGHT: u32 = 1;
const EXIT: u32 = 2;
const FIRST_WINDOW: u32 = 10;
const CAPACITY: usize = 8;
const PINS_READY: u32 = WM_APP + 22;

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
    pub monitor: DockMonitor,
    pub auto_hide: bool,
    pub original_icons: bool,
    pub replace_taskbar: bool,
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
    inventory: Option<Rc<desktop::Snapshot>>,
    pins: Vec<App>,
    /// Executables launched by pins, keyed by shortcut path once the worker reports.
    pin_targets: HashMap<PathBuf, AppIdentity>,
    pin_resolver: Option<Receiver<HashMap<PathBuf, AppIdentity>>>,
    /// Running groups adopted by the visible pins, in pin order.
    pin_groups: Vec<Option<WindowGroup>>,
    order_path: PathBuf,
    drag: Option<PinDrag>,
    visible_pins: usize,
    brush: HBRUSH,
    font: HFONT,
    dpi: u32,
    monitor: DockMonitor,
    last_monitor: HMONITOR,
    auto_hide: bool,
    expanded: bool,
    menu_open: bool,
    preview: Option<(HWND, Preview)>,
    icons: Option<icons::Cache>,
    original_icons: bool,
    request_taskbar: bool,
    taskbar: Option<Lease>,
    tooltip: HWND,
    palette: Palette,
    portuguese: bool,
}

impl State {
    fn live_group(&self, id: u32) -> Option<&WindowGroup> {
        let tile = id.checked_sub(FIRST_WINDOW)? as usize;
        if tile < self.visible_pins {
            return self.pin_groups.get(tile)?.as_ref();
        }
        self.groups.get(tile.checked_sub(self.visible_pins)?)
    }

    fn icon_path(&self, id: u32) -> Option<&std::path::Path> {
        if let Some(index) = pin_index(id, self.visible_pins) {
            return Some(self.pins.get(index)?.path.as_path());
        }
        self.live_group(id)?.icon_path.as_deref()
    }

    fn request_icons(&mut self) {
        if self.icons.is_none() {
            return;
        }
        let visible = (FIRST_WINDOW..FIRST_WINDOW + CAPACITY as u32)
            .filter_map(|id| self.icon_path(id).map(std::path::Path::to_path_buf))
            .collect::<Vec<_>>();
        if let Some(cache) = self.icons.as_mut() {
            cache.request(&visible);
        }
    }
}

impl Dock {
    pub fn inventory(&self) -> Option<Rc<desktop::Snapshot>> {
        self.state.inventory.clone()
    }

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
            let previews_available = preview::register().is_ok();
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
                    inventory: None,
                    pins: settings.pins,
                    pin_targets: HashMap::new(),
                    pin_resolver: None,
                    pin_groups: Vec::new(),
                    order_path: settings.order_path,
                    drag: None,
                    visible_pins: 0,
                    brush: CreateSolidBrush(palette.background),
                    font: null_mut(),
                    dpi: 0,
                    monitor: settings.monitor,
                    last_monitor: null_mut(),
                    auto_hide: settings.auto_hide,
                    expanded: !settings.auto_hide,
                    menu_open: false,
                    preview: None,
                    icons: icons::Cache::new(handle).ok(),
                    original_icons: settings.original_icons,
                    request_taskbar: settings.replace_taskbar,
                    taskbar: None,
                    tooltip: null_mut(),
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
                    && (previews_available || (*state).pins.len() > 1)
                    && SetWindowSubclass(button, Some(dock_button), 1, state as usize) == 0
                {
                    return Err(io::Error::last_os_error());
                }
                (*state).buttons.push(button);
            }
            (*state).tooltip = tooltips(handle, instance, &(*state).buttons);
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
            resolve_pins(handle, state);
            refresh(handle, state);
            Ok(dock)
        }
    }
}

unsafe fn tooltips(hwnd: HWND, instance: *mut c_void, buttons: &[HWND]) -> HWND {
    unsafe {
        let controls = INITCOMMONCONTROLSEX {
            dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_WIN95_CLASSES,
        };
        if InitCommonControlsEx(&controls) == 0 {
            return null_mut();
        }
        let tooltip = CreateWindowExW(
            WS_EX_TOPMOST,
            TOOLTIPS_CLASSW,
            std::ptr::null(),
            WS_POPUP | TTS_NOPREFIX | TTS_ALWAYSTIP,
            0,
            0,
            0,
            0,
            hwnd,
            null_mut(),
            instance,
            null_mut(),
        );
        if tooltip.is_null() {
            return tooltip;
        }
        SendMessageW(tooltip, CCM_SETUNICODEFORMAT, 1, 0);
        for &button in buttons.iter().skip(2) {
            let tool = TTTOOLINFOW {
                // The bundled legacy common-controls implementation accepts V2.
                cbSize: std::mem::offset_of!(TTTOOLINFOW, lpReserved) as u32,
                uFlags: TTF_IDISHWND | TTF_SUBCLASS | TTF_TRANSPARENT,
                hwnd,
                uId: button as usize,
                lpszText: (-1_isize) as *mut u16, // LPSTR_TEXTCALLBACKW
                ..Default::default()
            };
            SendMessageW(tooltip, TTM_ADDTOOLW, 0, (&raw const tool) as isize);
        }
        SendMessageW(tooltip, TTM_ACTIVATE, 1, 0);
        SendMessageW(tooltip, TTM_SETDELAYTIME, TTDT_INITIAL as usize, 600);
        tooltip
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

unsafe extern "system" fn dock_button(
    button: HWND,
    message: u32,
    wp: WPARAM,
    lp: LPARAM,
    _: usize,
    reference: usize,
) -> LRESULT {
    // The parent Dock owns this state until all child windows have been destroyed.
    unsafe {
        if !matches!(
            message,
            WM_LBUTTONDOWN
                | WM_LBUTTONDBLCLK
                | WM_LBUTTONUP
                | WM_MOUSEMOVE
                | WM_MOUSEHOVER
                | WM_MOUSELEAVE
                | WM_CAPTURECHANGED
                | WM_NCDESTROY
        ) {
            return DefSubclassProc(button, message, wp, lp);
        }
        let state = reference as *mut State;
        let count = (*state).visible_pins;
        let id = GetDlgCtrlID(button) as u32;
        let parent = GetParent(button);
        if matches!(message, WM_LBUTTONDOWN | WM_LBUTTONDBLCLK | WM_MOUSELEAVE)
            && (*state)
                .preview
                .as_ref()
                .is_some_and(|(source, _)| *source == button)
        {
            drop((*state).preview.take());
        }
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
                RemoveWindowSubclass(button, Some(dock_button), 1);
            }
            WM_MOUSEMOVE if (*state).preview.is_none() && (&*state).live_group(id).is_some() => {
                let mut tracking = TRACKMOUSEEVENT {
                    cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_HOVER | TME_LEAVE,
                    hwndTrack: button,
                    dwHoverTime: 450,
                };
                TrackMouseEvent(&mut tracking);
            }
            WM_MOUSEHOVER if (*state).preview.is_none() && IsWindowVisible(button) != 0 => {
                let Some(group) = (&*state).live_group(id) else {
                    return DefSubclassProc(button, message, wp, lp);
                };
                let target = group
                    .windows
                    .iter()
                    .find(|window| window.active && !window.minimised)
                    .or_else(|| group.windows.iter().find(|window| !window.minimised))
                    .map(|window| window.target);
                if let Some(target) = target
                    && let Some(preview) =
                        Preview::open(parent, button, target, (*state).palette.background)
                {
                    (*state).preview = Some((button, preview));
                }
            }
            _ => {}
        }
        DefSubclassProc(button, message, wp, lp)
    }
}

/// Resolve pin targets once on a worker STA. Pins work as launchers until it reports.
unsafe fn resolve_pins(hwnd: HWND, state: *mut State) {
    unsafe {
        let paths: Vec<PathBuf> = (*state).pins.iter().map(|pin| pin.path.clone()).collect();
        if paths.is_empty() {
            return;
        }
        let (sender, receiver) = mpsc::sync_channel(1);
        let target = hwnd as usize;
        let spawned = thread::Builder::new()
            .name("monolith-dock-pins".into())
            .spawn(move || {
                use windows::Win32::System::Com::{
                    COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize,
                };
                if CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_err() {
                    return;
                }
                let identities = desktop::shortcut_identities(&paths);
                CoUninitialize();
                if sender.send(identities).is_ok() {
                    PostMessageW(target as HWND, PINS_READY, 0, 0);
                }
            });
        if spawned.is_ok() {
            (*state).pin_resolver = Some(receiver);
        }
    }
}

/// Move each visible pin's running group from the trailing tiles onto the pin.
fn adopt_groups(
    pins: &[App],
    targets: &HashMap<PathBuf, AppIdentity>,
    groups: &mut Vec<WindowGroup>,
) -> Vec<Option<WindowGroup>> {
    pins.iter()
        .map(|pin| {
            let identity = targets.get(&pin.path)?;
            let index = groups
                .iter()
                .position(|group| group.identity.as_ref() == Some(identity))?;
            Some(groups.remove(index))
        })
        .collect()
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
            release_taskbar(self.handle, &raw mut *self.state);
            for hook in self.hooks.drain(..) {
                UnhookWinEvent(hook);
            }
            drop(self.state.preview.take());
            drop(self.state.icons.take());
            if !self.state.tooltip.is_null() {
                DestroyWindow(self.state.tooltip);
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
    let handle = EVENT_TARGET.get();
    if handle.is_null() {
        return;
    }
    // Geometry of the foreground window and leased tray affects Dock placement.
    if event == EVENT_OBJECT_LOCATIONCHANGE && unsafe { GetForegroundWindow() } != window {
        let state = unsafe { GetWindowLongPtrW(handle, GWLP_USERDATA) as *const State };
        if state.is_null()
            || !unsafe { &*state }
                .taskbar
                .as_ref()
                .is_some_and(|lease| lease.is_tray(window))
        {
            return;
        }
    }
    // Out-of-context events are delivered on the registering GUI thread.
    schedule_refresh(handle);
}

fn schedule_refresh(hwnd: HWND) {
    if !REFRESH_PENDING.get() && unsafe { SetTimer(hwnd, REFRESH, 100, None) } != 0 {
        REFRESH_PENDING.set(true);
    }
}

unsafe fn dock_display(state: *mut State) -> Option<(HMONITOR, MONITORINFO)> {
    unsafe {
        let primary = MonitorFromPoint(POINT::default(), MONITOR_DEFAULTTOPRIMARY);
        let mut preferred = primary;
        if (*state).monitor == DockMonitor::Active {
            let foreground = GetForegroundWindow();
            let mut pid = 0;
            if GetWindowThreadProcessId(foreground, &mut pid) != 0 && pid != std::process::id() {
                let active = MonitorFromWindow(foreground, MONITOR_DEFAULTTONULL);
                if !active.is_null() {
                    (*state).last_monitor = active;
                }
            }
            if !(*state).last_monitor.is_null() {
                preferred = (*state).last_monitor;
            }
        }
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if GetMonitorInfoW(preferred, &mut info) != 0 {
            return Some((preferred, info));
        }
        (*state).last_monitor = primary;
        (GetMonitorInfoW(primary, &mut info) != 0).then_some((primary, info))
    }
}

unsafe fn refresh(hwnd: HWND, state: *mut State) {
    unsafe {
        drop((*state).preview.take());
        let Some((monitor, display)) = dock_display(state) else {
            release_taskbar(hwnd, state);
            (*state).inventory = None;
            ShowWindow(hwnd, SW_HIDE);
            return;
        };
        let band = match (*state)
            .taskbar
            .as_ref()
            .map(|lease| lease.placement(&display))
        {
            Some(Placement::Band(band)) => Some(band),
            Some(Placement::Unavailable) => {
                release_taskbar(hwnd, state);
                None
            }
            Some(Placement::Pending) | None => None,
        };
        if desktop::foreground_covers_monitor(monitor, &display.rcMonitor) {
            ShowWindow(hwnd, SW_HIDE);
            (*state).groups.clear();
            (*state).pin_groups.clear();
            (*state).inventory = None;
            if (*state).auto_hide {
                (*state).expanded = false;
                KillTimer(hwnd, HIDE);
            }
            return;
        }

        if MonitorFromWindow(hwnd, MONITOR_DEFAULTTONULL) != monitor {
            // Query the destination DPI after relocating the still-hidden window.
            SetWindowPos(
                hwnd,
                null_mut(),
                display.rcWork.left,
                display.rcWork.top,
                1,
                1,
                SWP_HIDEWINDOW | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
        let dpi = GetDpiForWindow(hwnd).max(96);
        let scale = |value| overlay::scale(value, dpi);
        if !(*state).expanded {
            let buttons = &(*state).buttons;
            for &button in buttons {
                ShowWindow(button, SW_HIDE);
            }
            (*state).visible_pins = 0;
            (*state).groups.clear();
            (*state).pin_groups.clear();
            (*state).inventory = None;
            let work = display.rcWork;
            let width = scale(120).min((work.right - work.left).max(1));
            let height = scale(3);
            let placed = SetWindowPos(
                hwnd,
                HWND_TOPMOST,
                work.left + (work.right - work.left - width) / 2,
                band.map_or(work.bottom, |band| band.bottom) - height,
                width,
                height,
                SWP_SHOWWINDOW | SWP_NOACTIVATE,
            );
            if placed != 0 {
                request_taskbar(hwnd, state, scale(37) + scale(6));
            } else {
                release_taskbar(hwnd, state);
            }
            InvalidateRect(hwnd, std::ptr::null(), 1);
            return;
        }
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
        let step = scale(76).min(((work.right - work.left - 2 * inset) / 2).max(1));
        let capacity =
            (((work.right - work.left - 2 * inset) / step) - 2).clamp(0, CAPACITY as i32) as usize;
        let inventory = desktop::snapshot().ok().map(Rc::new);
        let mut groups = inventory
            .as_ref()
            .map(|snapshot| desktop::group_windows(snapshot.windows.clone()))
            .unwrap_or_default();
        (*state).inventory = inventory;
        let pin_count = (*state).pins.len().min(capacity);
        (*state).visible_pins = pin_count;
        let adopted = {
            let state = &*state;
            adopt_groups(&state.pins[..pin_count], &state.pin_targets, &mut groups)
        };
        (*state).pin_groups = adopted;
        groups.truncate(capacity - pin_count);
        (*state).groups = groups;
        (&mut *state).request_icons();
        let button_count = pin_count + (*state).groups.len() + 2;
        let width = button_count as i32 * step + 2 * inset;
        let height = scale(37);
        let left = work.left + ((work.right - work.left - width) / 2).max(0);
        let margin = if (*state).auto_hide { 0 } else { scale(8) };
        let top = band.map_or(work.bottom - height - margin, |band| {
            band.top + (band.bottom - band.top - height) / 2
        });

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
        let placed = SetWindowPos(
            hwnd,
            HWND_TOPMOST,
            left,
            top,
            width,
            height,
            SWP_SHOWWINDOW | SWP_NOACTIVATE,
        );
        if placed != 0 {
            request_taskbar(hwnd, state, height + scale(6));
        } else {
            release_taskbar(hwnd, state);
        }
    }
}

unsafe fn request_taskbar(hwnd: HWND, state: *mut State, minimum_height: i32) {
    unsafe {
        if !std::mem::take(&mut (*state).request_taskbar)
            || WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION) == 0
        {
            return;
        }
        (*state).taskbar = Lease::start(hwnd, minimum_height).ok().flatten();
        if (*state).taskbar.is_none() {
            WTSUnRegisterSessionNotification(hwnd);
        }
    }
}

unsafe fn release_taskbar(hwnd: HWND, state: *mut State) {
    unsafe {
        if let Some(lease) = (*state).taskbar.take() {
            drop(lease);
            WTSUnRegisterSessionNotification(hwnd);
        }
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
        let fill = if pressed {
            palette.pressed
        } else {
            palette.surface
        };
        SetDCBrushColor(item.hDC, fill);
        let brush = GetStockObject(DC_BRUSH) as HBRUSH;
        FillRect(item.hDC, &area, brush);

        let font = (*state).font;
        if !font.is_null() {
            SelectObject(item.hDC, font);
        }
        SetBkMode(item.hDC, TRANSPARENT as i32);
        SetTextColor(item.hDC, palette.text);
        let icon_drawn = (&*state).icon_path(item.CtlID).is_some_and(|path| {
            (*state).icons.as_ref().is_some_and(|cache| {
                let size = overlay::scale(21, (*state).dpi).min(area.bottom - area.top - 4);
                cache.draw(path, item.hDC, area, size, fill, (*state).original_icons)
            })
        });
        if !icon_drawn {
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
        }
        let drop_target = pin_index(item.CtlID, (*state).visible_pins)
            .is_some_and(|index| (*state).drag.as_ref().and_then(|drag| drag.over) == Some(index));
        // Running tiles show a short activity dot; the active one and a drop target a wider bar.
        let group = (&*state).live_group(item.CtlID);
        if drop_target || group.is_some() {
            let emphasised = drop_target || group.is_some_and(WindowGroup::active);
            let half = overlay::scale(if emphasised { 5 } else { 2 }, (*state).dpi);
            let centre = area.left + (area.right - area.left) / 2;
            let indicator = RECT {
                left: centre - half,
                right: centre + half,
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
            WM_CONTEXTMENU => {
                (*state).menu_open = true;
                shell_menu(hwnd, (*state).launcher, (*state).portuguese);
                (*state).menu_open = false;
                schedule_refresh(hwnd);
                0
            }
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
                        if index < (*state).visible_pins && (&*state).live_group(id).is_none() {
                            let pins = &(*state).pins;
                            let path = pins[index].path.clone();
                            launch_shortcut(hwnd, &path);
                            return 0;
                        }
                        if let Some(group) = (&*state).live_group(id) {
                            (*state).menu_open = group.windows.len() > 1;
                            show_group(hwnd, lp as HWND, group);
                            if (*state).menu_open {
                                (*state).menu_open = false;
                                schedule_refresh(hwnd);
                            }
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
            WM_NOTIFY if lp != 0 => {
                let header = &*(lp as *const NMHDR);
                if header.hwndFrom == (*state).tooltip && header.code == TTN_GETDISPINFOW {
                    let tip = &mut *(lp as *mut NMTTDISPINFOW);
                    GetWindowTextW(
                        tip.hdr.idFrom as HWND,
                        tip.szText.as_mut_ptr(),
                        tip.szText.len() as i32,
                    );
                    tip.lpszText = tip.szText.as_mut_ptr();
                }
                0
            }
            PINS_READY => {
                if let Some(targets) = (*state).pin_resolver.take().and_then(|r| r.try_recv().ok())
                {
                    (*state).pin_targets = targets;
                    schedule_refresh(hwnd);
                }
                0
            }
            icons::READY => {
                if (*state).icons.as_mut().is_some_and(icons::Cache::receive) {
                    (&mut *state).request_icons();
                    repaint_buttons(state);
                }
                0
            }
            WM_ERASEBKGND => {
                let mut area = RECT::default();
                GetClientRect(hwnd, &mut area);
                if (*state).expanded {
                    FillRect(wp as HDC, &area, (*state).brush);
                } else {
                    let previous = SetDCBrushColor(wp as HDC, (*state).palette.pressed);
                    FillRect(wp as HDC, &area, GetStockObject(DC_BRUSH) as HBRUSH);
                    SetDCBrushColor(wp as HDC, previous);
                }
                1
            }
            WM_TIMER if wp == REFRESH => {
                KillTimer(hwnd, REFRESH);
                REFRESH_PENDING.set(false);
                if !(*state).menu_open {
                    refresh(hwnd, state);
                }
                0
            }
            WM_MOUSEMOVE if !(*state).expanded => {
                (*state).expanded = true;
                refresh(hwnd, state);
                if IsWindowVisible(hwnd) != 0 && (*state).expanded {
                    SetTimer(hwnd, HIDE, 400, None);
                }
                0
            }
            WM_TIMER if wp == HIDE => {
                KillTimer(hwnd, HIDE);
                let mut cursor = POINT::default();
                let mut bounds = RECT::default();
                let busy = (*state).drag.is_some() || (*state).menu_open;
                let hovered = GetCursorPos(&mut cursor) != 0
                    && GetWindowRect(hwnd, &mut bounds) != 0
                    && PtInRect(&bounds, cursor) != 0;
                if busy || hovered {
                    SetTimer(hwnd, HIDE, 400, None);
                } else {
                    (*state).expanded = false;
                    refresh(hwnd, state);
                }
                0
            }
            WM_DISPLAYCHANGE | WM_DPICHANGED | WM_WTSSESSION_CHANGE => {
                release_taskbar(hwnd, state);
                schedule_refresh(hwnd);
                0
            }
            WM_POWERBROADCAST if matches!(wp as u32, PBT_APMSUSPEND | PBT_APMRESUMEAUTOMATIC) => {
                release_taskbar(hwnd, state);
                schedule_refresh(hwnd);
                1
            }
            WM_ENDSESSION if wp != 0 => {
                release_taskbar(hwnd, state);
                0
            }
            WM_SETTINGCHANGE | taskbar::CHANGED => {
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
