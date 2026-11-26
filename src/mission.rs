//! A disposable window overview for the current desktop, with native DWM previews.

use crate::{
    desktop::{self, Snapshot},
    overlay::{self, Palette},
    preview::Thumbnail,
    windows::wide,
};
use std::{ffi::c_void, io, mem::size_of, ptr::null_mut, rc::Rc};
use windows_sys::Win32::{
    Foundation::{ERROR_CLASS_ALREADY_EXISTS, HWND, LPARAM, LRESULT, RECT, WPARAM},
    Graphics::Gdi::*,
    System::SystemServices::SS_CENTERIMAGE,
    UI::{
        Controls::DRAWITEMSTRUCT, HiDpi::GetDpiForWindow, Input::KeyboardAndMouse::*,
        WindowsAndMessaging::*,
    },
};

pub const OPEN: u32 = WM_APP + 9;
pub const CLOSE: u32 = WM_APP + 10;

const TITLE: usize = 1;
const TASK_VIEW: usize = 2;
const PREVIOUS: usize = 3;
const NEXT: usize = 4;
const DESKTOP_LEFT: usize = 5;
const DESKTOP_RIGHT: usize = 6;
const FIRST_TILE: usize = 10;
const MAX_TILES: usize = 12;

pub struct Mission {
    hwnd: HWND,
    state: Box<State>,
}

struct Tile {
    button: HWND,
    thumbnail: Option<Thumbnail>,
}

struct State {
    launcher: HWND,
    previous: HWND,
    inventory: Option<Rc<Snapshot>>,
    first: usize,
    columns: usize,
    capacity: usize,
    tiles: Vec<Tile>,
    palette: Palette,
    portuguese: bool,
    background: HBRUSH,
    font: HFONT,
    dpi: u32,
}

impl State {
    fn label(&self, english: &'static str, portuguese: &'static str) -> &'static str {
        if self.portuguese { portuguese } else { english }
    }
}

impl Mission {
    pub fn new(
        instance: *mut c_void,
        launcher: HWND,
        source: HWND,
        palette: Palette,
        portuguese: bool,
        inventory: Option<Rc<Snapshot>>,
    ) -> io::Result<Self> {
        unsafe {
            let previous = GetForegroundWindow();
            let class_name = wide("MonolithMissionControl");
            let class = WNDCLASSW {
                lpszClassName: class_name.as_ptr(),
                hInstance: instance,
                lpfnWndProc: Some(window_proc),
                hCursor: LoadCursorW(null_mut(), IDC_ARROW),
                ..Default::default()
            };
            if RegisterClassW(&class) == 0 {
                let error = io::Error::last_os_error();
                if error.raw_os_error() != Some(ERROR_CLASS_ALREADY_EXISTS as i32) {
                    return Err(error);
                }
            }
            let anchor = if source.is_null() { previous } else { source };
            let monitor = if anchor.is_null() {
                let mut point = Default::default();
                GetCursorPos(&mut point);
                MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST)
            } else {
                MonitorFromWindow(anchor, MONITOR_DEFAULTTONEAREST)
            };
            let mut info = MONITORINFO {
                cbSize: size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            if GetMonitorInfoW(monitor, &mut info) == 0 {
                return Err(io::Error::last_os_error());
            }
            let hwnd = CreateWindowExW(
                overlay::EX_STYLE | WS_EX_TOPMOST,
                class_name.as_ptr(),
                wide("Monolith Mission Control").as_ptr(),
                overlay::STYLE | WS_CLIPCHILDREN,
                info.rcWork.left,
                info.rcWork.top,
                1,
                1,
                null_mut(),
                null_mut(),
                instance,
                null_mut(),
            );
            if hwnd.is_null() {
                return Err(io::Error::last_os_error());
            }
            let mut overview = Self {
                hwnd,
                state: Box::new(State {
                    launcher,
                    previous,
                    palette,
                    portuguese,
                    first: 0,
                    columns: 1,
                    capacity: 1,
                    inventory: inventory.or_else(|| desktop::snapshot().ok().map(Rc::new)),
                    tiles: Vec::with_capacity(MAX_TILES),
                    background: CreateSolidBrush(palette.background),
                    font: null_mut(),
                    dpi: 0,
                }),
            };
            if overview.state.background.is_null() {
                return Err(io::Error::last_os_error());
            }
            let child = |class, text: &str, styles, id| {
                let child = CreateWindowExW(
                    0,
                    class,
                    wide(text).as_ptr(),
                    WS_CHILD | styles,
                    0,
                    0,
                    0,
                    0,
                    hwnd,
                    id as *mut c_void,
                    instance,
                    null_mut(),
                );
                if child.is_null() {
                    Err(io::Error::last_os_error())
                } else {
                    Ok(child)
                }
            };
            let button_class = wide("BUTTON");
            let button = |caption, id| {
                child(
                    button_class.as_ptr(),
                    caption,
                    WS_TABSTOP | BS_OWNERDRAW as u32,
                    id,
                )
            };
            child(wide("STATIC").as_ptr(), "", SS_CENTERIMAGE, TITLE)?;
            for index in 0..MAX_TILES {
                overview.state.tiles.push(Tile {
                    button: button("", FIRST_TILE + index)?,
                    thumbnail: None,
                });
            }
            button(
                overview.state.label("Previous desktop", "Área anterior"),
                DESKTOP_LEFT,
            )?;
            button(
                overview
                    .state
                    .label("Windows Task View", "Visão de Tarefas do Windows"),
                TASK_VIEW,
            )?;
            button(
                overview.state.label("Next desktop", "Próxima área"),
                DESKTOP_RIGHT,
            )?;
            button(
                overview.state.label("Previous page", "Página anterior"),
                PREVIOUS,
            )?;
            button(overview.state.label("Next page", "Próxima página"), NEXT)?;
            let state = &raw mut *overview.state;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, state as isize);
            render(hwnd, state, info.rcWork);
            ShowWindow(hwnd, SW_SHOW);
            SetForegroundWindow(hwnd);
            SetFocus(hwnd);
            Ok(overview)
        }
    }

    pub fn handle(&self) -> HWND {
        self.hwnd
    }

    pub fn focus(&self) {
        unsafe { SetForegroundWindow(self.hwnd) };
    }

    pub fn handle_key(&self, msg: &MSG) -> bool {
        let hwnd = self.hwnd;
        unsafe {
            if msg.message != WM_KEYDOWN || (msg.hwnd != hwnd && IsChild(hwnd, msg.hwnd) == 0) {
                return false;
            }
            let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State;
            match msg.wParam as u16 {
                VK_ESCAPE => close(hwnd, state, true),
                VK_F5 => reload(hwnd, state),
                VK_PRIOR => page(hwnd, state, false),
                VK_NEXT => page(hwnd, state, true),
                VK_TAB => {
                    let child = if msg.hwnd == hwnd {
                        null_mut()
                    } else {
                        msg.hwnd
                    };
                    let backwards = i32::from(GetKeyState(i32::from(VK_SHIFT)) < 0);
                    SetFocus(GetNextDlgTabItem(hwnd, child, backwards));
                }
                VK_RETURN if msg.hwnd != hwnd => {
                    SendMessageW(msg.hwnd, BM_CLICK, 0, 0);
                }
                key @ (VK_LEFT | VK_RIGHT | VK_UP | VK_DOWN) => {
                    focus_tile(hwnd, state, msg.hwnd, key)
                }
                _ => return false,
            }
            true
        }
    }
}

impl Drop for Mission {
    fn drop(&mut self) {
        self.state.tiles.clear();
        unsafe {
            SetWindowLongPtrW(self.hwnd, GWLP_USERDATA, 0);
            DestroyWindow(self.hwnd);
            DeleteObject(self.state.background);
            DeleteObject(self.state.font);
        }
    }
}

struct Grid {
    columns: usize,
    capacity: usize,
    bounds: RECT,
    pad: i32,
    header: i32,
    tile_width: i32,
    tile_height: i32,
    width: i32,
    foot: i32,
}

fn geometry(work: RECT, dpi: u32, count: usize) -> Grid {
    let scale = |dip| overlay::scale(dip, dpi);
    let padding = scale(10);
    let header = scale(33);
    let footer = scale(26);
    let available_width = (work.right - work.left - 2 * padding).max(1);
    let available_height = (work.bottom - work.top - 2 * padding).max(1);
    let columns =
        (((available_width - padding) / scale(218)).clamp(1, 4) as usize).min(count.max(1));
    let upper_rows =
        ((available_height - header - footer - 2 * padding) / scale(120)).clamp(1, 3) as usize;
    let rows = count.div_ceil(columns).clamp(1, upper_rows);
    let tile_width = ((available_width - padding) / columns as i32 - padding).clamp(1, scale(230));
    let tile_height = ((available_height - header - footer - 2 * padding) / rows as i32 - padding)
        .clamp(1, scale(142));
    let width = (tile_width * columns as i32 + padding * (columns + 1) as i32)
        .max(scale(424).min(available_width));
    let height = header + (tile_height + padding) * rows as i32 + footer + padding;
    let x = work.left + (work.right - work.left - width) / 2;
    let y = work.top + ((work.bottom - work.top - height) / 3).max(0);
    Grid {
        columns,
        capacity: columns * rows,
        bounds: RECT {
            left: x,
            top: y,
            right: x + width,
            bottom: y + height,
        },
        pad: padding,
        header,
        tile_width: (width - padding * (columns + 1) as i32) / columns as i32,
        tile_height,
        width,
        foot: height - footer - padding,
    }
}

unsafe fn render(hwnd: HWND, state: *mut State, work: RECT) {
    unsafe {
        let dpi = GetDpiForWindow(hwnd);
        let scale = |dip| overlay::scale(dip, dpi);
        let inventory = (*state).inventory.clone();
        let windows = inventory
            .as_ref()
            .map_or([].as_slice(), |data| &data.windows);
        let grid = geometry(work, dpi, windows.len());
        (*state).columns = grid.columns;
        (*state).capacity = grid.capacity;
        let first =
            (*state).first.min(windows.len().saturating_sub(1)) / grid.capacity * grid.capacity;
        (*state).first = first;
        let font = if (*state).dpi == dpi {
            null_mut()
        } else {
            CreateFontW(
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
                wide("Segoe UI").as_ptr(),
            )
        };
        if !font.is_null() {
            let old = (*state).font;
            (*state).font = font;
            (*state).dpi = dpi;
            for id in [
                TITLE,
                TASK_VIEW,
                PREVIOUS,
                NEXT,
                DESKTOP_LEFT,
                DESKTOP_RIGHT,
            ]
            .into_iter()
            .chain(FIRST_TILE..FIRST_TILE + MAX_TILES)
            {
                SendMessageW(GetDlgItem(hwnd, id as i32), WM_SETFONT, font as usize, 0);
            }
            DeleteObject(old);
        }
        let bounds = grid.bounds;
        SetWindowPos(
            hwnd,
            null_mut(),
            bounds.left,
            bounds.top,
            bounds.right - bounds.left,
            bounds.bottom - bounds.top,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
        let window_count = windows.len();
        let title = if inventory.is_none() {
            (*state).label("Window inventory unavailable", "Janelas indisponíveis")
        } else if window_count == 0 {
            (*state).label("No open windows", "Nenhuma janela aberta")
        } else {
            (*state).label("Open windows", "Janelas abertas")
        };
        // The overview shows windows on the current desktop; name that desktop.
        match crate::workspaces::read()
            .ok()
            .and_then(|workspaces| workspaces.summary((*state).portuguese))
        {
            Some(workspace) => set_title(hwnd, &format!("{workspace} · {title}")),
            None => set_title(hwnd, title),
        }
        let position = |id, x, y, width, height| {
            SetWindowPos(
                GetDlgItem(hwnd, id),
                null_mut(),
                x,
                y,
                width,
                height,
                SWP_NOZORDER | SWP_NOACTIVATE | SWP_SHOWWINDOW | SWP_NOCOPYBITS,
            )
        };
        let show_paging = window_count > grid.capacity;
        let width = grid.width - 2 * grid.pad;
        let gap = scale(4);
        let page_width = scale(101).min(width / 3).max(1);
        position(
            TITLE as i32,
            grid.pad,
            scale(5),
            if show_paging {
                width - 2 * (page_width + gap)
            } else {
                width
            },
            scale(24),
        );
        let navigation_width = ((width - 2 * gap) / 3).clamp(1, scale(164));
        let navigation_left = (grid.width - 3 * navigation_width - 2 * gap) / 2;
        for (index, id) in [DESKTOP_LEFT, TASK_VIEW, DESKTOP_RIGHT]
            .into_iter()
            .enumerate()
        {
            position(
                id as i32,
                navigation_left + index as i32 * (navigation_width + gap),
                grid.foot,
                navigation_width,
                scale(26),
            );
        }
        for index in 0..2 {
            let id = PREVIOUS as i32 + index;
            let page_button = GetDlgItem(hwnd, id);
            if show_paging {
                position(
                    id,
                    grid.width - grid.pad - (2 - index) * page_width - (1 - index) * gap,
                    scale(5),
                    page_width,
                    scale(24),
                );
                EnableWindow(
                    page_button,
                    i32::from(if index == 0 {
                        first > 0
                    } else {
                        first + grid.capacity < window_count
                    }),
                );
            } else {
                ShowWindow(page_button, SW_HIDE);
            }
        }
        for index in 0..MAX_TILES {
            let tile = (*state).tiles.as_mut_ptr().add(index);
            drop((*tile).thumbnail.take());
            let Some(window) = windows.get(first + index).filter(|_| index < grid.capacity) else {
                ShowWindow((*tile).button, SW_HIDE);
                continue;
            };
            let x = grid.pad + (index % grid.columns) as i32 * (grid.tile_width + grid.pad);
            let y = grid.header + (index / grid.columns) as i32 * (grid.tile_height + grid.pad);
            position(
                (FIRST_TILE + index) as i32,
                x,
                y,
                grid.tile_width,
                grid.tile_height,
            );
            let thumbnail = Thumbnail::register(hwnd, window.target).filter(|thumbnail| {
                thumbnail.show(RECT {
                    left: x + grid.pad,
                    top: y + grid.pad,
                    right: x + grid.tile_width - grid.pad,
                    bottom: y + grid.tile_height - scale(29),
                })
            });
            let caption = if window.minimised {
                format!(
                    "{} · {}",
                    window.title,
                    (*state).label("Minimised", "Minimizada")
                )
            } else {
                window.title.clone()
            };
            SetWindowTextW((*tile).button, wide(&caption).as_ptr());
            (*tile).thumbnail = thumbnail;
            InvalidateRect((*tile).button, std::ptr::null(), 0);
        }
        InvalidateRect(hwnd, std::ptr::null(), 1);
    }
}

unsafe fn layout(hwnd: HWND, state: *mut State) {
    unsafe {
        let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if GetMonitorInfoW(monitor, &mut info) != 0 {
            render(hwnd, state, info.rcWork);
        }
    }
}

unsafe fn reload(hwnd: HWND, state: *mut State) {
    unsafe {
        (*state).inventory = desktop::snapshot().ok().map(Rc::new);
        layout(hwnd, state);
    }
}

unsafe fn page(hwnd: HWND, state: *mut State, forwards: bool) {
    unsafe {
        let first = (*state).first;
        let capacity = (*state).capacity;
        let next = if forwards {
            first + capacity
        } else {
            first.saturating_sub(capacity)
        };
        if next != first
            && (*state)
                .inventory
                .as_ref()
                .is_some_and(|item| next < item.windows.len())
        {
            (*state).first = next;
            layout(hwnd, state);
            SetFocus(GetDlgItem(hwnd, FIRST_TILE as i32));
        }
    }
}

unsafe fn focus_tile(hwnd: HWND, state: *mut State, child: HWND, key: u16) {
    unsafe {
        let index = GetDlgCtrlID(child) - FIRST_TILE as i32;
        let columns = (*state).columns as i32;
        let next = if !(0..MAX_TILES as i32).contains(&index) {
            0
        } else {
            index
                + match key {
                    VK_LEFT => -1,
                    VK_RIGHT => 1,
                    VK_UP => -columns,
                    _ => columns,
                }
        };
        if (0..MAX_TILES as i32).contains(&next) {
            let button = GetDlgItem(hwnd, FIRST_TILE as i32 + next);
            if IsWindowVisible(button) != 0 {
                SetFocus(button);
            }
        }
    }
}

unsafe fn close(hwnd: HWND, state: *mut State, restore: bool) {
    unsafe {
        if restore
            && GetForegroundWindow() == hwnd
            && IsWindowVisible((*state).previous) != 0
            && IsIconic((*state).previous) == 0
        {
            SetForegroundWindow((*state).previous);
        }
        PostMessageW(
            (*state).launcher,
            CLOSE,
            hwnd as usize,
            isize::from(restore),
        );
    }
}

unsafe fn set_title(hwnd: HWND, title: &str) {
    unsafe {
        SetWindowTextW(GetDlgItem(hwnd, TITLE as i32), wide(title).as_ptr());
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
                close(hwnd, state, true);
                0
            }
            WM_ACTIVATE if wp as u16 == WA_INACTIVE as u16 => {
                close(hwnd, state, false);
                0
            }
            WM_DPICHANGED | WM_DISPLAYCHANGE => {
                layout(hwnd, state);
                0
            }
            WM_COMMAND if lp != 0 && (wp >> 16) as u16 == BN_CLICKED as u16 => {
                let id = wp as u16 as usize;
                match id {
                    TASK_VIEW | DESKTOP_LEFT | DESKTOP_RIGHT => {
                        let accepted = GetForegroundWindow() == hwnd
                            && match id {
                                TASK_VIEW => crate::workspaces::press(&[VK_LWIN, VK_TAB], 1),
                                DESKTOP_LEFT => {
                                    crate::workspaces::press(&[VK_LWIN, VK_LCONTROL, VK_LEFT], 1)
                                }
                                _ => crate::workspaces::press(&[VK_LWIN, VK_LCONTROL, VK_RIGHT], 1),
                            };
                        if accepted {
                            ShowWindow(hwnd, SW_HIDE);
                            PostMessageW((*state).launcher, CLOSE, hwnd as usize, 1);
                        } else {
                            let hint = if id == TASK_VIEW {
                                (*state).label(
                                    "Open Task View with Win+Tab",
                                    "Abra a Visão de Tarefas com Win+Tab",
                                )
                            } else {
                                (*state).label(
                                    "Switch desktops with Win+Ctrl+Left or Right",
                                    "Alterne áreas com Win+Ctrl+Esquerda ou Direita",
                                )
                            };
                            set_title(hwnd, hint);
                        }
                    }
                    PREVIOUS => page(hwnd, state, false),
                    NEXT => page(hwnd, state, true),
                    FIRST_TILE.. => {
                        let index = id - FIRST_TILE;
                        let target = (*state)
                            .inventory
                            .as_ref()
                            .and_then(|data| data.windows.get((*state).first + index))
                            .map(|window| window.target);
                        match target.map(desktop::WindowTarget::activate) {
                            Some(Ok(())) => {
                                ShowWindow(hwnd, SW_HIDE);
                                PostMessageW((*state).launcher, CLOSE, hwnd as usize, 1);
                            }
                            Some(Err(desktop::ActivationError::Refused)) => set_title(
                                hwnd,
                                (*state).label(
                                    "Windows refused to switch to that window",
                                    "O Windows impediu a troca de janela",
                                ),
                            ),
                            _ => reload(hwnd, state),
                        }
                    }
                    _ => {}
                }
                0
            }
            WM_DRAWITEM if lp != 0 => {
                let item = &*(lp as *const DRAWITEMSTRUCT);
                let mut text = item.rcItem;
                let tiles = &(*state).tiles;
                if (item.CtlID as usize)
                    .checked_sub(FIRST_TILE)
                    .and_then(|index| tiles.get(index))
                    .is_some_and(|tile| tile.thumbnail.is_some())
                {
                    text.top =
                        (text.bottom - overlay::scale(25, GetDpiForWindow(hwnd))).max(text.top);
                }
                overlay::paint_button_at(item, (*state).palette, (*state).font, text);
                1
            }
            WM_CTLCOLORSTATIC => {
                SetBkColor(wp as HDC, (*state).palette.background);
                SetTextColor(wp as HDC, (*state).palette.text);
                (*state).background as isize
            }
            WM_ERASEBKGND => {
                let mut bounds = RECT::default();
                GetClientRect(hwnd, &mut bounds);
                FillRect(wp as HDC, &bounds, (*state).background);
                1
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounds_preview_count_and_centres_the_grid_on_an_offset_display() {
        let work = RECT {
            left: -1920,
            top: -150,
            right: 0,
            bottom: 890,
        };
        let grid = geometry(work, 144, 90);
        assert_eq!(grid.capacity, MAX_TILES);
        assert!((grid.bounds.left + grid.bounds.right).abs_diff(work.left + work.right) <= 1);
        assert!(grid.bounds.top >= work.top && grid.bounds.bottom <= work.bottom);
        assert!(geometry(work, 144, 2).bounds.bottom < grid.bounds.bottom);
    }
}
