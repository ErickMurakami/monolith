//! Mission Control: the desktops in spatial order with live DWM previews of their
//! windows, a strip to switch or add desktops, and paging between neighbours.

use crate::{
    desktop::{self, Snapshot, Window, WindowTarget},
    overlay::{self, Palette},
    preview::Thumbnail,
    windows::wide,
};
use std::{ffi::c_void, io, mem::size_of, ptr::null_mut, rc::Rc};
use windows_sys::Win32::{
    Foundation::{ERROR_CLASS_ALREADY_EXISTS, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
    Graphics::Gdi::*,
    UI::{
        Controls::{DRAWITEMSTRUCT, ODS_FOCUS, ODS_SELECTED},
        HiDpi::GetDpiForWindow,
        Input::KeyboardAndMouse::*,
        WindowsAndMessaging::*,
    },
};

pub const OPEN: u32 = WM_APP + 9;
pub const CLOSE: u32 = WM_APP + 10;

const TASK_VIEW: usize = 2;
const PREVIOUS: usize = 3;
const NEXT: usize = 4;
const ADD: usize = 5;
const FIRST_PILL: usize = 20;
const FIRST_CARD: usize = 30;
const FIRST_TILE: usize = 40;

const MAX_PILLS: usize = 9;
const MAX_CARDS: usize = 5;
const PER_CARD: usize = 4;
const MAX_TILES: usize = MAX_CARDS * PER_CARD;
const FIRST_CLOSE: usize = FIRST_TILE + MAX_TILES;
const RELOAD_TIMER: usize = 1;

// Segoe Fluent Icons code points.
const GLYPH_ADD: &str = "\u{E710}";
const GLYPH_TASK_VIEW: &str = "\u{E7C4}";
const GLYPH_PREVIOUS: &str = "\u{E76B}";
const GLYPH_NEXT: &str = "\u{E76C}";
const GLYPH_FULLSCREEN: &str = "\u{E740}";
const GLYPH_CLOSE: &str = "\u{E711}";

pub struct Mission {
    hwnd: HWND,
    state: Box<State>,
}

struct Tile {
    button: HWND,
    thumbnail: Option<Thumbnail>,
    window: Option<Rc<Window>>,
}

struct Desk {
    name: String,
    windows: Vec<Rc<Window>>,
}

#[derive(Clone, Copy)]
struct Colours {
    background: u32,
    panel: u32,
    card: u32,
    tile: u32,
    border: u32,
    pressed: u32,
    text: u32,
    secondary: u32,
    accent: u32,
}

impl Colours {
    fn new(palette: Palette) -> Self {
        Self {
            background: palette.background,
            panel: palette.surface,
            card: mix(palette.surface, palette.text, 4),
            tile: mix(palette.background, palette.surface, 60),
            border: mix(palette.surface, palette.text, 10),
            pressed: palette.pressed,
            text: palette.text,
            secondary: mix(palette.text, palette.background, 35),
            accent: palette.accent,
        }
    }
}

/// `from` moved `percent` of the way towards `to`, per channel.
fn mix(from: u32, to: u32, percent: u32) -> u32 {
    (0..3).fold(0, |colour, channel| {
        let shift = channel * 8;
        let a = (from >> shift) & 0xff;
        let b = (to >> shift) & 0xff;
        let value = (a * (100 - percent) + b * percent) / 100;
        colour | (value << shift)
    })
}

struct State {
    launcher: HWND,
    previous: HWND,
    desks: Vec<Desk>,
    current: usize,
    layout: Layout,
    tiles: Vec<Tile>,
    colours: Colours,
    portuguese: bool,
    font: HFONT,
    label: HFONT,
    icons: HFONT,
    small_icons: HFONT,
    dpi: u32,
    status: Option<&'static str>,
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
            let (desks, current) = desks(portuguese, inventory);
            let bounds = info.rcMonitor;
            let hwnd = CreateWindowExW(
                overlay::EX_STYLE | WS_EX_TOPMOST,
                class_name.as_ptr(),
                wide("Monolith Mission Control").as_ptr(),
                overlay::STYLE | WS_CLIPCHILDREN,
                bounds.left,
                bounds.top,
                bounds.right - bounds.left,
                bounds.bottom - bounds.top,
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
                    desks,
                    current,
                    layout: Layout::default(),
                    tiles: Vec::with_capacity(MAX_TILES),
                    colours: Colours::new(palette),
                    portuguese,
                    font: null_mut(),
                    label: null_mut(),
                    icons: null_mut(),
                    small_icons: null_mut(),
                    dpi: 0,
                    status: None,
                }),
            };
            let button = |caption: &str, id: usize| {
                let child = CreateWindowExW(
                    0,
                    wide("BUTTON").as_ptr(),
                    wide(caption).as_ptr(),
                    WS_CHILD | WS_TABSTOP | BS_OWNERDRAW as u32,
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
            for index in 0..MAX_PILLS {
                button("", FIRST_PILL + index)?;
            }
            button(overview.state.label("New desktop", "Nova área"), ADD)?;
            button(
                overview
                    .state
                    .label("Windows Task View", "Visão de Tarefas do Windows"),
                TASK_VIEW,
            )?;
            for index in 0..MAX_CARDS {
                button("", FIRST_CARD + index)?;
                button(
                    overview.state.label("Close desktop", "Fechar área"),
                    FIRST_CLOSE + index,
                )?;
            }
            for index in 0..MAX_TILES {
                overview.state.tiles.push(Tile {
                    button: button("", FIRST_TILE + index)?,
                    thumbnail: None,
                    window: None,
                });
            }
            button(
                overview.state.label("Previous desktop", "Área anterior"),
                PREVIOUS,
            )?;
            button(overview.state.label("Next desktop", "Próxima área"), NEXT)?;
            let state = &raw mut *overview.state;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, state as isize);
            render(hwnd, state);
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
                key @ 0x31..=0x39 => select_desk(hwnd, state, usize::from(key - 0x31)),
                VK_PRIOR => step(hwnd, state, false),
                VK_NEXT => step(hwnd, state, true),
                key @ (VK_TAB | VK_LEFT | VK_RIGHT | VK_UP | VK_DOWN) => {
                    let child = if msg.hwnd == hwnd {
                        null_mut()
                    } else {
                        msg.hwnd
                    };
                    let backwards = match key {
                        VK_TAB => GetKeyState(i32::from(VK_SHIFT)) < 0,
                        _ => matches!(key, VK_LEFT | VK_UP),
                    };
                    SetFocus(GetNextDlgTabItem(hwnd, child, i32::from(backwards)));
                }
                VK_RETURN if msg.hwnd != hwnd => {
                    SendMessageW(msg.hwnd, BM_CLICK, 0, 0);
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
            DeleteObject(self.state.font);
            DeleteObject(self.state.label);
            DeleteObject(self.state.icons);
            DeleteObject(self.state.small_icons);
        }
    }
}

/// Desktops in Explorer's order with their windows, and the current one. Without
/// readable desktop state, the current desktop's inventory stands in as the only one.
fn desks(portuguese: bool, inventory: Option<Rc<Snapshot>>) -> (Vec<Desk>, usize) {
    let fallback = || {
        let windows = inventory
            .clone()
            .or_else(|| desktop::snapshot().ok().map(Rc::new))
            .map(|snapshot| snapshot.windows.clone())
            .unwrap_or_default();
        (
            vec![Desk {
                name: desk_name(None, 0, portuguese),
                windows,
            }],
            0,
        )
    };
    match crate::workspaces::read() {
        Ok(workspaces) if !workspaces.ids.is_empty() && workspaces.current.is_some() => {
            let groups = desktop::windows_by_desktop(&workspaces.ids);
            let desks = workspaces
                .ids
                .iter()
                .zip(groups)
                .enumerate()
                .map(|(index, (id, windows))| {
                    // A dedicated fullscreen desktop is named after its app, as in macOS.
                    let dedicated = match windows.as_slice() {
                        [only] if only.fullscreen => Some(only.title.clone()),
                        _ => None,
                    };
                    Desk {
                        name: desk_name(
                            crate::workspaces::name(id).or(dedicated),
                            index,
                            portuguese,
                        ),
                        windows,
                    }
                })
                .collect();
            (desks, workspaces.current.unwrap_or(0))
        }
        _ => fallback(),
    }
}

fn desk_name(name: Option<String>, index: usize, portuguese: bool) -> String {
    name.unwrap_or_else(|| {
        let generic = if portuguese { "Área" } else { "Desktop" };
        format!("{generic} {}", index + 1)
    })
}

#[derive(Default)]
struct Layout {
    pills: Vec<RECT>,
    add: RECT,
    task_view: RECT,
    status: RECT,
    panel: RECT,
    /// Index of the desktop shown in the first card.
    first: usize,
    cards: Vec<RECT>,
    /// Window tiles of each card, in Z order.
    tiles: Vec<Vec<RECT>>,
    pager: RECT,
    previous: RECT,
    next: RECT,
    dots: Vec<RECT>,
}

const HEADER: i32 = 52;
/// Space a tile adds around its preview: the inset above and the caption below.
const TILE_CHROME: i32 = 40;

/// Place the desktops strip, the panel of desktop cards around `current` and the
/// pager inside a `width` by `height` client area. `counts` holds each desktop's
/// window count. Cards size to their previews, which keep the screen's proportions,
/// and the bottom stays clear for the Dock.
fn plan(width: i32, height: i32, dpi: u32, counts: &[usize], current: usize) -> Layout {
    let scale = |dip| overlay::scale(dip, dpi);
    let count = counts.len();
    let gap = scale(12);
    let side = scale(48);
    let margin = scale(32);
    let pills = count.min(MAX_PILLS) as i32;
    let available = width - 2 * margin - 2 * (side + gap) - gap * (pills - 1).max(0);
    let pill_width = (available / pills.max(1)).clamp(scale(56), scale(184));
    let strip_width = pills * (pill_width + gap) + side + gap + side;
    let top = scale(56);
    let mut x = (width - strip_width) / 2;
    let mut square = |width| {
        let rect = RECT {
            left: x,
            top,
            right: x + width,
            bottom: top + side,
        };
        x += width + gap;
        rect
    };
    let pill_rects = (0..pills).map(|_| square(pill_width)).collect();
    let add = square(side);
    let task_view = square(side);
    let status = RECT {
        left: margin,
        top: top + side,
        right: width - margin,
        bottom: top + side + scale(40),
    };
    let padding = scale(24);
    let card_gap = scale(16);
    // The pager and the Dock below it keep the lowest band of the screen.
    let lowest = (height - scale(168)).max(status.bottom + scale(240));
    let visible = count.min(MAX_CARDS);
    let first = current
        .saturating_sub(visible / 2)
        .min(count.saturating_sub(visible));
    let weights: Vec<i32> = (first..first + visible)
        .map(|index| 1 + i32::from(index == current && counts[index] > 1))
        .collect();
    let units: i32 = weights.iter().sum();
    let inner = width - 2 * margin - 2 * padding - card_gap * (visible as i32 - 1).max(0);
    // A single unit never grows past a comfortable card width; the row centres.
    let unit = (inner / units.max(1)).min(scale(440));
    let row = unit * units + card_gap * (visible as i32 - 1).max(0);
    let card_top = status.bottom + padding;
    let tallest = lowest - padding;
    let mut left = (width - row) / 2;
    let mut cards: Vec<RECT> = weights
        .iter()
        .map(|weight| {
            let card = RECT {
                left,
                top: card_top,
                right: left + unit * weight,
                bottom: tallest,
            };
            left += unit * weight + card_gap;
            card
        })
        .collect();
    let aspect = (height.max(1), width.max(1));
    let tiles: Vec<Vec<RECT>> = cards
        .iter()
        .zip(first..)
        .map(|(card, index)| tiles(*card, counts[index], dpi, aspect))
        .collect();
    let content = tiles
        .iter()
        .flatten()
        .map(|tile| tile.bottom + scale(16))
        .max()
        .unwrap_or(0);
    let bottom = content
        .max(card_top + scale(HEADER) + scale(200))
        .min(tallest);
    for card in &mut cards {
        card.bottom = bottom;
    }
    let panel = RECT {
        left: cards.first().map_or(margin, |card| card.left) - padding,
        top: status.bottom,
        right: cards.last().map_or(width - margin, |card| card.right) + padding,
        bottom: bottom + padding,
    };
    let pager_width = scale(232);
    let pager = RECT {
        left: (width - pager_width) / 2,
        top: panel.bottom + scale(24),
        right: (width + pager_width) / 2,
        bottom: panel.bottom + scale(24) + side,
    };
    let previous = RECT {
        right: pager.left + side,
        ..pager
    };
    let next = RECT {
        left: pager.right - side,
        ..pager
    };
    let dot = scale(8);
    let dot_gap = scale(10);
    let dots_count = count.min(12) as i32;
    let dots_width = dots_count * dot + (dots_count - 1).max(0) * dot_gap;
    let dots_left = (pager.left + pager.right - dots_width) / 2;
    let dots_top = (pager.top + pager.bottom - dot) / 2;
    let dots = (0..dots_count)
        .map(|index| {
            let left = dots_left + index * (dot + dot_gap);
            RECT {
                left,
                top: dots_top,
                right: left + dot,
                bottom: dots_top + dot,
            }
        })
        .collect();
    Layout {
        pills: pill_rects,
        add,
        task_view,
        status,
        panel,
        first,
        cards,
        tiles,
        pager,
        previous,
        next,
        dots,
    }
}

/// Window tiles for a card holding `count` windows: one large tile, a stacked pair,
/// or a two-column grid. Previews keep the screen's `aspect` (height, width).
fn tiles(card: RECT, count: usize, dpi: u32, aspect: (i32, i32)) -> Vec<RECT> {
    let scale = |dip| overlay::scale(dip, dpi);
    let shown = count.min(PER_CARD);
    if shown == 0 {
        return Vec::new();
    }
    let padding = scale(16);
    let gap = scale(12);
    let left = card.left + padding;
    let top = card.top + scale(HEADER);
    let width = (card.right - padding - left).max(1);
    let height = (card.bottom - padding - top).max(1);
    let columns = if shown == 1 || (shown == 2 && width < scale(560)) {
        1
    } else {
        2
    };
    let rows = shown.div_ceil(columns) as i32;
    let columns = columns as i32;
    let chrome = scale(TILE_CHROME);
    let mut tile_width = (width - gap * (columns - 1)) / columns;
    let mut tile_height = (tile_width - scale(16)) * aspect.0 / aspect.1 + chrome;
    let room = (height - gap * (rows - 1)) / rows;
    if tile_height > room {
        // Shrink both sides so previews keep their proportions.
        tile_height = room.max(chrome + 1);
        tile_width = (tile_height - chrome) * aspect.1 / aspect.0 + scale(16);
    }
    let offset = (width - tile_width * columns - gap * (columns - 1)) / 2;
    (0..shown as i32)
        .map(|index| {
            let x = left + offset + (index % columns) * (tile_width + gap);
            let y = top + (index / columns) * (tile_height + gap);
            RECT {
                left: x,
                top: y,
                right: x + tile_width,
                bottom: y + tile_height,
            }
        })
        .collect()
}

unsafe fn fonts(state: &mut State, dpi: u32) {
    unsafe {
        if state.dpi == dpi {
            return;
        }
        let create = |size, weight, face: &str| {
            CreateFontW(
                -overlay::scale(size, dpi),
                0,
                0,
                0,
                weight,
                0,
                0,
                0,
                1,
                0,
                0,
                CLEARTYPE_QUALITY as u32,
                0,
                wide(face).as_ptr(),
            )
        };
        for old in [state.font, state.label, state.icons, state.small_icons] {
            DeleteObject(old);
        }
        state.font = create(13, FW_NORMAL as i32, "Segoe UI");
        state.label = create(15, FW_SEMIBOLD as i32, "Segoe UI");
        state.icons = create(16, FW_NORMAL as i32, "Segoe Fluent Icons");
        state.small_icons = create(12, FW_NORMAL as i32, "Segoe Fluent Icons");
        state.dpi = dpi;
    }
}

unsafe fn render(hwnd: HWND, state: *mut State) {
    let state = unsafe { &mut *state };
    unsafe {
        let dpi = GetDpiForWindow(hwnd);
        fonts(state, dpi);
        let mut client = RECT::default();
        GetClientRect(hwnd, &mut client);
        let current = state.current;
        let counts: Vec<usize> = state.desks.iter().map(|desk| desk.windows.len()).collect();
        let layout = plan(client.right, client.bottom, dpi, &counts, current);
        let place = |id: usize, rect: RECT, visible: bool| {
            let child = GetDlgItem(hwnd, id as i32);
            if visible {
                SetWindowPos(
                    child,
                    null_mut(),
                    rect.left,
                    rect.top,
                    rect.right - rect.left,
                    rect.bottom - rect.top,
                    SWP_NOZORDER | SWP_NOACTIVATE | SWP_SHOWWINDOW | SWP_NOCOPYBITS,
                );
                InvalidateRect(child, std::ptr::null(), 0);
            } else {
                ShowWindow(child, SW_HIDE);
            }
            child
        };
        for index in 0..MAX_PILLS {
            let rect = layout.pills.get(index).copied().unwrap_or_default();
            let child = place(FIRST_PILL + index, rect, index < layout.pills.len());
            if let Some(desk) = state.desks.get(index) {
                SetWindowTextW(child, wide(&desk.name).as_ptr());
            }
        }
        place(ADD, layout.add, true);
        place(TASK_VIEW, layout.task_view, true);
        let paging = state.desks.len() > 1;
        place(PREVIOUS, layout.previous, paging);
        place(NEXT, layout.next, paging);
        for index in 0..MAX_CARDS {
            let card = layout.cards.get(index).copied().unwrap_or_default();
            let shown = index < layout.cards.len();
            let header = RECT {
                bottom: card.top + overlay::scale(HEADER, dpi),
                ..card
            };
            // The last remaining desktop cannot be closed.
            let closable = shown && state.desks.len() > 1;
            let close = close_button(header, dpi);
            place(FIRST_CLOSE + index, close, closable);
            let header = RECT {
                right: if closable { close.left } else { header.right },
                ..header
            };
            let child = place(FIRST_CARD + index, header, shown);
            if let Some(desk) = state.desks.get(layout.first + index) {
                SetWindowTextW(child, wide(&desk.name).as_ptr());
            }
        }
        for tile in &mut state.tiles {
            drop(tile.thumbnail.take());
            tile.window = None;
        }
        let caption = overlay::scale(TILE_CHROME - 8, dpi);
        let inset = overlay::scale(8, dpi);
        for (card_index, rects) in layout.tiles.iter().enumerate() {
            let desk = &state.desks[layout.first + card_index];
            for (slot, &rect) in rects.iter().enumerate() {
                let tile = &mut state.tiles[card_index * PER_CARD + slot];
                let window = desk.windows[slot].clone();
                tile.thumbnail = Thumbnail::register(hwnd, window.target).filter(|thumbnail| {
                    thumbnail.show(RECT {
                        left: rect.left + inset,
                        top: rect.top + inset,
                        right: rect.right - inset,
                        bottom: rect.bottom - caption,
                    })
                });
                SetWindowTextW(tile.button, wide(&window.title).as_ptr());
                tile.window = Some(window);
            }
        }
        for (index, tile) in state.tiles.iter().enumerate() {
            let card = index / PER_CARD;
            let slot = index % PER_CARD;
            let rect = layout
                .tiles
                .get(card)
                .and_then(|rects| rects.get(slot).copied())
                .unwrap_or_default();
            place(FIRST_TILE + index, rect, tile.window.is_some());
        }
        state.layout = layout;
        // The accessible name says which desktop is current, as in "Desktop 2 · 2 of 3".
        let name = crate::workspaces::read()
            .ok()
            .and_then(|workspaces| workspaces.summary(state.portuguese))
            .map_or_else(
                || "Mission Control".to_owned(),
                |summary| format!("Mission Control · {summary}"),
            );
        SetWindowTextW(hwnd, wide(&name).as_ptr());
        InvalidateRect(hwnd, std::ptr::null(), 1);
    }
}

unsafe fn reload(hwnd: HWND, state: *mut State) {
    unsafe {
        let (desks, current) = desks((*state).portuguese, None);
        (*state).desks = desks;
        (*state).current = current;
        (*state).status = None;
        render(hwnd, state);
    }
}

/// Switch to desktop `index` and dismiss the overview, or just dismiss it for the
/// current desktop.
unsafe fn select_desk(hwnd: HWND, state: *mut State, index: usize) {
    unsafe {
        if index >= (*state).desks.len() {
            return;
        }
        if index == (*state).current {
            close(hwnd, state, true);
            return;
        }
        if GetForegroundWindow() == hwnd && crate::workspaces::switch_to(index) {
            dismiss(hwnd, state);
        } else {
            refuse(
                hwnd,
                state,
                (*state).label(
                    "Switch desktops with Win+Ctrl+Left or Right",
                    "Alterne áreas com Win+Ctrl+Esquerda ou Direita",
                ),
            );
        }
    }
}

unsafe fn step(hwnd: HWND, state: *mut State, forwards: bool) {
    unsafe {
        let current = (*state).current;
        let target = if forwards {
            current + 1
        } else {
            current.wrapping_sub(1)
        };
        select_desk(hwnd, state, target);
    }
}

unsafe fn press(hwnd: HWND, state: *mut State, keys: &[u16], hint: &'static str) {
    unsafe {
        if GetForegroundWindow() == hwnd && crate::workspaces::press(keys, 1) {
            dismiss(hwnd, state);
        } else {
            refuse(hwnd, state, hint);
        }
    }
}

unsafe fn refuse(hwnd: HWND, state: *mut State, hint: &'static str) {
    unsafe {
        (*state).status = Some(hint);
        InvalidateRect(hwnd, &(*state).layout.status, 1);
    }
}

unsafe fn dismiss(hwnd: HWND, state: *mut State) {
    unsafe {
        ShowWindow(hwnd, SW_HIDE);
        PostMessageW((*state).launcher, CLOSE, hwnd as usize, 1);
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

fn contains(rect: &RECT, point: POINT) -> bool {
    point.x >= rect.left && point.x < rect.right && point.y >= rect.top && point.y < rect.bottom
}

unsafe fn rounded(dc: HDC, rect: &RECT, radius: i32, fill: u32, border: Option<(u32, i32)>) {
    unsafe {
        let brush = CreateSolidBrush(fill);
        let pen = match border {
            Some((colour, width)) => CreatePen(PS_INSIDEFRAME, width, colour),
            None => CreatePen(PS_NULL, 0, 0),
        };
        let old_brush = SelectObject(dc, brush);
        let old_pen = SelectObject(dc, pen);
        let grow = i32::from(border.is_none());
        RoundRect(
            dc,
            rect.left,
            rect.top,
            rect.right + grow,
            rect.bottom + grow,
            2 * radius,
            2 * radius,
        );
        SelectObject(dc, old_brush);
        SelectObject(dc, old_pen);
        DeleteObject(brush);
        DeleteObject(pen);
    }
}

unsafe fn fill(dc: HDC, rect: &RECT, colour: u32) {
    unsafe {
        let brush = CreateSolidBrush(colour);
        FillRect(dc, rect, brush);
        DeleteObject(brush);
    }
}

unsafe fn text(
    dc: HDC,
    value: &str,
    rect: RECT,
    font: HFONT,
    colour: u32,
    format: DRAW_TEXT_FORMAT,
) {
    unsafe {
        let mut rect = rect;
        let mut value: Vec<u16> = value.encode_utf16().collect();
        SelectObject(dc, font);
        SetTextColor(dc, colour);
        DrawTextW(
            dc,
            value.as_mut_ptr(),
            value.len() as i32,
            &mut rect,
            format | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX | DT_END_ELLIPSIS,
        );
    }
}

unsafe fn paint(hwnd: HWND, state: &State) {
    unsafe {
        let mut paint = PAINTSTRUCT::default();
        let screen = BeginPaint(hwnd, &mut paint);
        let mut client = RECT::default();
        GetClientRect(hwnd, &mut client);
        // Compose off screen so the panel and cards appear at once.
        let dc = CreateCompatibleDC(screen);
        let bitmap = CreateCompatibleBitmap(screen, client.right, client.bottom);
        let old_bitmap = SelectObject(dc, bitmap);
        let colours = state.colours;
        let layout = &state.layout;
        let dpi = state.dpi;
        let scale = |dip| overlay::scale(dip, dpi);
        fill(dc, &client, colours.background);
        SetBkMode(dc, TRANSPARENT as i32);
        rounded(
            dc,
            &layout.panel,
            scale(24),
            colours.panel,
            Some((colours.border, 1)),
        );
        for (index, card) in layout.cards.iter().enumerate() {
            let desk = layout.first + index;
            let border = if desk == state.current {
                (colours.accent, scale(2))
            } else {
                (colours.border, 1)
            };
            rounded(dc, card, scale(16), colours.card, Some(border));
            if state.desks[desk].windows.is_empty() {
                text(
                    dc,
                    state.label("No windows", "Nenhuma janela"),
                    RECT {
                        top: card.top + scale(HEADER),
                        ..*card
                    },
                    state.font,
                    colours.secondary,
                    DT_CENTER,
                );
            }
        }
        if let Some(status) = state.status {
            text(
                dc,
                status,
                layout.status,
                state.font,
                colours.secondary,
                DT_CENTER,
            );
        }
        if state.desks.len() > 1 {
            rounded(
                dc,
                &layout.pager,
                (layout.pager.bottom - layout.pager.top) / 2,
                colours.card,
                Some((colours.border, 1)),
            );
            for (index, dot) in layout.dots.iter().enumerate() {
                let colour = if index == state.current {
                    colours.accent
                } else {
                    colours.border
                };
                rounded(dc, dot, (dot.right - dot.left) / 2, colour, None);
            }
        }
        BitBlt(screen, 0, 0, client.right, client.bottom, dc, 0, 0, SRCCOPY);
        SelectObject(dc, old_bitmap);
        DeleteObject(bitmap);
        DeleteDC(dc);
        EndPaint(hwnd, &paint);
    }
}

unsafe fn draw_item(state: &State, item: &DRAWITEMSTRUCT) {
    unsafe {
        let colours = state.colours;
        let dpi = state.dpi;
        let scale = |dip| overlay::scale(dip, dpi);
        let id = item.CtlID as usize;
        let dc = item.hDC;
        let rect = item.rcItem;
        let pressed = item.itemState & ODS_SELECTED != 0;
        let focused = item.itemState & ODS_FOCUS != 0;
        let current = state.current;
        let layout = &state.layout;
        let mut caption = [0_u16; 256];
        let length = GetWindowTextW(item.hwndItem, caption.as_mut_ptr(), caption.len() as i32);
        let caption = String::from_utf16_lossy(&caption[..length.max(0) as usize]);
        SetBkMode(dc, TRANSPARENT as i32);
        let focus = |colour| {
            if focused {
                (colours.accent, scale(2))
            } else {
                colour
            }
        };
        match id {
            FIRST_PILL..FIRST_CARD => {
                let desk = id - FIRST_PILL;
                fill(dc, &rect, colours.background);
                let border = if desk == current {
                    (colours.accent, scale(2))
                } else {
                    focus((colours.border, 1))
                };
                let surface = if pressed {
                    colours.pressed
                } else {
                    colours.card
                };
                rounded(dc, &rect, scale(12), surface, Some(border));
                let number = RECT {
                    left: rect.left + scale(18),
                    right: rect.left + scale(42),
                    ..rect
                };
                text(
                    dc,
                    &(desk + 1).to_string(),
                    number,
                    state.font,
                    colours.secondary,
                    DT_LEFT,
                );
                let label = RECT {
                    left: number.right,
                    right: rect.right - scale(14),
                    ..rect
                };
                let name = desk_label(&caption, desk);
                text(dc, name, label, state.label, colours.text, DT_LEFT);
            }
            ADD | TASK_VIEW | PREVIOUS | NEXT => {
                let (behind, surface, radius, glyph) = match id {
                    ADD => (colours.background, colours.card, scale(12), GLYPH_ADD),
                    TASK_VIEW => (colours.background, colours.card, scale(12), GLYPH_TASK_VIEW),
                    PREVIOUS => (colours.card, colours.card, scale(24), GLYPH_PREVIOUS),
                    _ => (colours.card, colours.card, scale(24), GLYPH_NEXT),
                };
                if matches!(id, PREVIOUS | NEXT) {
                    // Repaint this button's slice of the pager capsule the parent draws.
                    let own = if id == NEXT {
                        layout.next
                    } else {
                        layout.previous
                    };
                    let mut capsule = layout.pager;
                    OffsetRect(&mut capsule, -own.left, -own.top);
                    fill(dc, &rect, colours.background);
                    rounded(dc, &capsule, radius, behind, Some((colours.border, 1)));
                    if pressed || focused {
                        let mut inner = rect;
                        InflateRect(&mut inner, -scale(6), -scale(6));
                        let ring = focused.then_some((colours.accent, scale(2)));
                        rounded(dc, &inner, scale(18), colours.pressed, ring);
                    }
                } else {
                    fill(dc, &rect, behind);
                    let fill_colour = if pressed { colours.pressed } else { surface };
                    rounded(
                        dc,
                        &rect,
                        radius,
                        fill_colour,
                        Some(focus((colours.border, 1))),
                    );
                }
                text(dc, glyph, rect, state.icons, colours.text, DT_CENTER);
            }
            FIRST_CARD..FIRST_TILE => {
                let card = id - FIRST_CARD;
                let desk_index = layout.first + card;
                let Some(desk) = state.desks.get(desk_index) else {
                    return;
                };
                fill(dc, &rect, colours.panel);
                let mut whole = layout.cards[card];
                OffsetRect(&mut whole, -whole.left, -whole.top);
                let border = if desk_index == current {
                    (colours.accent, scale(2))
                } else {
                    (colours.border, 1)
                };
                let surface = if pressed {
                    colours.pressed
                } else {
                    colours.card
                };
                rounded(dc, &whole, scale(16), surface, Some(border));
                if focused {
                    let mut ring = rect;
                    InflateRect(&mut ring, -scale(6), -scale(6));
                    rounded(
                        dc,
                        &ring,
                        scale(10),
                        surface,
                        Some((colours.accent, scale(2))),
                    );
                }
                let number = RECT {
                    left: rect.left + scale(20),
                    right: rect.left + scale(44),
                    ..rect
                };
                text(
                    dc,
                    &(desk_index + 1).to_string(),
                    number,
                    state.font,
                    colours.secondary,
                    DT_LEFT,
                );
                let trailing = RECT {
                    left: rect.right - scale(48),
                    right: rect.right - scale(if state.desks.len() > 1 { 4 } else { 20 }),
                    ..rect
                };
                let label = RECT {
                    left: number.right,
                    right: trailing.left,
                    ..rect
                };
                text(
                    dc,
                    desk_label(&caption, desk_index),
                    label,
                    state.label,
                    colours.text,
                    DT_LEFT,
                );
                if desk.windows.len() == 1 && desk.windows[0].fullscreen {
                    text(
                        dc,
                        GLYPH_FULLSCREEN,
                        trailing,
                        state.icons,
                        colours.secondary,
                        DT_RIGHT,
                    );
                } else if desk.windows.len() > PER_CARD {
                    let more = format!("+{}", desk.windows.len() - PER_CARD);
                    text(dc, &more, trailing, state.font, colours.secondary, DT_RIGHT);
                }
            }
            FIRST_CLOSE.. => {
                let card = id - FIRST_CLOSE;
                let Some(&whole) = layout.cards.get(card) else {
                    return;
                };
                let own = close_button(
                    RECT {
                        bottom: whole.top + scale(HEADER),
                        ..whole
                    },
                    dpi,
                );
                let mut shape = whole;
                OffsetRect(&mut shape, -own.left, -own.top);
                let border = if layout.first + card == current {
                    (colours.accent, scale(2))
                } else {
                    (colours.border, 1)
                };
                fill(dc, &rect, colours.panel);
                rounded(dc, &shape, scale(16), colours.card, Some(border));
                let mut circle = rect;
                InflateRect(&mut circle, -scale(8), -scale(8));
                if pressed || focused {
                    let ring = focused.then_some((colours.accent, scale(2)));
                    rounded(dc, &circle, scale(16), colours.pressed, ring);
                }
                text(
                    dc,
                    GLYPH_CLOSE,
                    rect,
                    state.small_icons,
                    colours.secondary,
                    DT_CENTER,
                );
            }
            _ => {
                let tile = &state.tiles[id - FIRST_TILE];
                let Some(window) = &tile.window else {
                    return;
                };
                fill(dc, &rect, colours.card);
                let surface = if pressed {
                    colours.pressed
                } else {
                    colours.tile
                };
                rounded(
                    dc,
                    &rect,
                    scale(10),
                    surface,
                    Some(focus((colours.border, 1))),
                );
                let inset = scale(10);
                if tile.thumbnail.is_some() {
                    let label = RECT {
                        left: rect.left + inset,
                        right: rect.right - inset,
                        top: rect.bottom - scale(30),
                        bottom: rect.bottom - scale(2),
                    };
                    text(dc, &caption, label, state.font, colours.secondary, DT_LEFT);
                } else {
                    let mut label = rect;
                    InflateRect(&mut label, -inset, -inset);
                    let middle = (label.top + label.bottom) / 2;
                    text(
                        dc,
                        &caption,
                        RECT {
                            bottom: middle,
                            ..label
                        },
                        state.font,
                        colours.text,
                        DT_CENTER,
                    );
                    let note = if window.minimised {
                        state.label("Minimised", "Minimizada")
                    } else {
                        state.label("Preview unavailable", "Prévia indisponível")
                    };
                    text(
                        dc,
                        note,
                        RECT {
                            top: middle,
                            ..label
                        },
                        state.font,
                        colours.secondary,
                        DT_CENTER,
                    );
                }
            }
        }
    }
}

/// The desktop name without the generic numbered fallback, which the number shows.
fn desk_label(name: &str, index: usize) -> &str {
    let number = format!(" {}", index + 1);
    match name.strip_suffix(&number) {
        Some(generic) if matches!(generic, "Desktop" | "Área") => generic,
        _ => name,
    }
}

/// The close control at the right end of a card's header.
fn close_button(header: RECT, dpi: u32) -> RECT {
    let side = overlay::scale(44, dpi);
    let right = header.right - overlay::scale(6, dpi);
    let top = header.top + (header.bottom - header.top - side) / 2;
    RECT {
        left: right - side,
        top,
        right,
        bottom: top + side,
    }
}

/// Close desktop `index`. Its windows move to a neighbouring desktop, as when a
/// desktop is closed in Task View, so no window is lost.
unsafe fn close_desk(hwnd: HWND, state: *mut State, index: usize) {
    unsafe {
        let current = (*state).current;
        if (*state).desks.len() < 2 || index >= (*state).desks.len() {
            return;
        }
        if index == current {
            // Windows closes the current desktop and moves to the one on its left.
            press(
                hwnd,
                state,
                &[VK_LWIN, VK_LCONTROL, VK_F4],
                (*state).label(
                    "Close this desktop with Win+Ctrl+F4",
                    "Feche esta área com Win+Ctrl+F4",
                ),
            );
        } else if crate::space_bridge::remove_desktop(index, current) {
            SetTimer(hwnd, RELOAD_TIMER, 300, None);
        } else {
            refuse(
                hwnd,
                state,
                (*state).label(
                    "Closing another desktop needs desktop_moves, fullscreen_workspaces or close_empty_workspaces",
                    "Fechar outra área requer desktop_moves, fullscreen_workspaces ou close_empty_workspaces",
                ),
            );
        }
    }
}

unsafe fn activate(hwnd: HWND, state: *mut State, target: WindowTarget) {
    unsafe {
        match target.activate() {
            Ok(()) => dismiss(hwnd, state),
            Err(desktop::ActivationError::Refused) => refuse(
                hwnd,
                state,
                (*state).label(
                    "Windows refused to switch to that window",
                    "O Windows impediu a troca de janela",
                ),
            ),
            Err(desktop::ActivationError::Unavailable) => reload(hwnd, state),
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
            WM_CLOSE => {
                close(hwnd, state, true);
                0
            }
            WM_ACTIVATE if wp as u16 == WA_INACTIVE as u16 => {
                close(hwnd, state, false);
                0
            }
            WM_DPICHANGED | WM_DISPLAYCHANGE => {
                render(hwnd, state);
                0
            }
            WM_TIMER if wp == RELOAD_TIMER => {
                KillTimer(hwnd, RELOAD_TIMER);
                reload(hwnd, state);
                0
            }
            WM_LBUTTONUP => {
                let point = POINT {
                    x: (lp & 0xffff) as i16 as i32,
                    y: ((lp >> 16) & 0xffff) as i16 as i32,
                };
                let layout = &(*state).layout;
                if let Some(card) = layout.cards.iter().position(|card| contains(card, point)) {
                    select_desk(hwnd, state, layout.first + card);
                } else if !contains(&layout.panel, point) && !contains(&layout.pager, point) {
                    // As in macOS, clicking outside the desktops dismisses the overview.
                    close(hwnd, state, true);
                }
                0
            }
            WM_COMMAND if lp != 0 && (wp >> 16) as u16 == BN_CLICKED as u16 => {
                let id = wp as u16 as usize;
                match id {
                    TASK_VIEW => press(
                        hwnd,
                        state,
                        &[VK_LWIN, VK_TAB],
                        (*state).label(
                            "Open Task View with Win+Tab",
                            "Abra a Visão de Tarefas com Win+Tab",
                        ),
                    ),
                    ADD => press(
                        hwnd,
                        state,
                        &[VK_LWIN, VK_LCONTROL, u16::from(b'D')],
                        (*state).label(
                            "Add a desktop with Win+Ctrl+D",
                            "Adicione uma área com Win+Ctrl+D",
                        ),
                    ),
                    PREVIOUS => step(hwnd, state, false),
                    NEXT => step(hwnd, state, true),
                    FIRST_PILL..FIRST_CARD => select_desk(hwnd, state, id - FIRST_PILL),
                    FIRST_CARD..FIRST_TILE => {
                        select_desk(hwnd, state, (*state).layout.first + id - FIRST_CARD)
                    }
                    FIRST_CLOSE.. => {
                        close_desk(hwnd, state, (*state).layout.first + id - FIRST_CLOSE)
                    }
                    FIRST_TILE.. => {
                        let target = (&(*state))
                            .tiles
                            .get(id - FIRST_TILE)
                            .and_then(|tile| tile.window.as_ref())
                            .map(|window| window.target);
                        if let Some(target) = target {
                            activate(hwnd, state, target);
                        }
                    }
                    _ => {}
                }
                0
            }
            WM_DRAWITEM if lp != 0 => {
                draw_item(&*state, &*(lp as *const DRAWITEMSTRUCT));
                1
            }
            WM_ERASEBKGND => 1,
            WM_PAINT => {
                paint(hwnd, &*state);
                0
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn centres_the_strip_and_keeps_the_current_desktop_among_the_cards() {
        let layout = plan(1920, 1080, 96, &[1; 8], 6);
        assert!(
            layout.pager.bottom <= 1080 - 96,
            "the pager stays clear of the Dock"
        );
        assert_eq!(layout.pills.len(), 8);
        let strip_left = layout.pills[0].left;
        let strip_right = layout.task_view.right;
        assert!((strip_left + strip_right).abs_diff(1920) <= 2);
        assert_eq!(layout.cards.len(), MAX_CARDS);
        assert!((layout.first..layout.first + MAX_CARDS).contains(&6));
        assert!(layout.first + MAX_CARDS <= 8);
        assert!(layout.pager.top >= layout.panel.bottom && layout.pager.bottom <= 1080);
    }

    #[test]
    fn widens_the_weighted_card_and_fits_cards_inside_the_panel() {
        let layout = plan(1920, 1080, 144, &[2, 1, 1], 0);
        let width = |rect: &RECT| rect.right - rect.left;
        assert!(width(&layout.cards[0]) > width(&layout.cards[1]));
        for card in &layout.cards {
            assert!(card.left >= layout.panel.left && card.right <= layout.panel.right);
        }
    }

    #[test]
    fn arranges_window_tiles_by_count() {
        let card = RECT {
            left: 0,
            top: 0,
            right: 800,
            bottom: 600,
        };
        assert!(tiles(card, 0, 96, (1080, 1920)).is_empty());
        assert_eq!(tiles(card, 1, 96, (1080, 1920)).len(), 1);
        let pair = tiles(card, 2, 96, (1080, 1920));
        assert_eq!(pair[0].top, pair[1].top);
        assert_eq!(tiles(card, 9, 96, (1080, 1920)).len(), PER_CARD);
        for tile in tiles(card, 4, 96, (1080, 1920)) {
            assert!(tile.right <= card.right && tile.bottom <= card.bottom);
        }
    }

    #[test]
    fn hides_generic_desktop_names_behind_their_number() {
        assert_eq!(desk_label("Desktop 3", 2), "Desktop");
        assert_eq!(desk_label("Dev", 0), "Dev");
    }

    #[test]
    fn mixes_colours_per_channel() {
        assert_eq!(mix(0, 0x00ff_ffff, 50), 0x007f_7f7f);
        assert_eq!(mix(0x0000_00ff, 0x0000_ff00, 100), 0x0000_ff00);
    }
}
