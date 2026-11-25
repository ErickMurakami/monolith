//! On-demand native preview for a selection in Files.

use crate::{
    artwork::Artwork,
    overlay::{self, Palette},
    quick_content::{self, Content},
    windows::wide,
};
use std::{
    io,
    mem::size_of,
    path::PathBuf,
    ptr::null_mut,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};
use windows_sys::Win32::{
    Foundation::{ERROR_CLASS_ALREADY_EXISTS, HWND, LPARAM, LRESULT, RECT, WPARAM},
    Graphics::Gdi::*,
    System::{
        LibraryLoader::GetModuleHandleW,
        SystemServices::{SS_CENTERIMAGE, SS_ENDELLIPSIS},
    },
    UI::{
        Controls::DRAWITEMSTRUCT, HiDpi::GetDpiForWindow, Input::KeyboardAndMouse::*,
        WindowsAndMessaging::*,
    },
};

pub const CLOSE: u32 = WM_APP + 13;
const READY: u32 = WM_APP + 14;
const NAME: usize = 1;
const TEXT: usize = 2;
const STATUS: usize = 3;
const DISMISS: usize = 4;

pub struct QuickLook {
    hwnd: HWND,
    state: Box<State>,
}

struct State {
    owner: HWND,
    palette: Palette,
    portuguese: bool,
    image: Option<Artwork>,
    delivery: Arc<Mutex<Delivery>>,
    font: HFONT,
    text_font: HFONT,
    dpi: u32,
    background: HBRUSH,
}

#[derive(Default)]
struct Delivery {
    closed: bool,
    result: Option<Content>,
}

struct Loading(Arc<AtomicBool>);

impl Drop for Loading {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

fn load(
    window: HWND,
    path: PathBuf,
    delivery: Arc<Mutex<Delivery>>,
    busy: Arc<AtomicBool>,
) -> bool {
    if busy.swap(true, Ordering::AcqRel) {
        return false;
    }
    let loading = Loading(busy);
    let window = window as usize;
    thread::Builder::new()
        .name("monolith-quick-look".into())
        .spawn(move || {
            let result = quick_content::read(&path);
            drop(loading);
            let mut delivery = delivery.lock().unwrap();
            if !delivery.closed {
                delivery.result = Some(result);
                unsafe { PostMessageW(window as HWND, READY, 0, 0) };
            }
        })
        .is_ok()
}

impl State {
    fn label(&self, english: &'static str, portuguese: &'static str) -> &'static str {
        if self.portuguese { portuguese } else { english }
    }
}

impl QuickLook {
    pub fn open(
        owner: HWND,
        path: PathBuf,
        palette: Palette,
        portuguese: bool,
        busy: Arc<AtomicBool>,
    ) -> io::Result<Self> {
        unsafe {
            let instance = GetModuleHandleW(std::ptr::null());
            let name = wide("MonolithQuickLook");
            let class = WNDCLASSW {
                lpszClassName: name.as_ptr(),
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
            let monitor = MonitorFromWindow(owner, MONITOR_DEFAULTTONEAREST);
            let mut display = MONITORINFO {
                cbSize: size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            if GetMonitorInfoW(monitor, &mut display) == 0 {
                return Err(io::Error::last_os_error());
            }
            let work = display.rcWork;
            let hwnd = CreateWindowExW(
                overlay::EX_STYLE,
                name.as_ptr(),
                wide("Monolith Quick Look").as_ptr(),
                overlay::STYLE | WS_CLIPCHILDREN,
                work.left,
                work.top,
                1,
                1,
                owner,
                null_mut(),
                instance,
                null_mut(),
            );
            if hwnd.is_null() {
                return Err(io::Error::last_os_error());
            }
            let mut preview = Self {
                hwnd,
                state: Box::new(State {
                    owner,
                    palette,
                    portuguese,
                    image: None,
                    delivery: Arc::new(Mutex::new(Delivery::default())),
                    font: null_mut(),
                    text_font: null_mut(),
                    dpi: 0,
                    background: CreateSolidBrush(palette.background),
                }),
            };
            if preview.state.background.is_null() {
                return Err(io::Error::last_os_error());
            }
            let child = |class, caption: &str, style, id| {
                let child = CreateWindowExW(
                    0,
                    class,
                    wide(caption).as_ptr(),
                    WS_CHILD | style,
                    0,
                    0,
                    0,
                    0,
                    hwnd,
                    id as *mut _,
                    instance,
                    null_mut(),
                );
                if child.is_null() {
                    Err(io::Error::last_os_error())
                } else {
                    Ok(child)
                }
            };
            let static_class = wide("STATIC");
            let file = path
                .file_name()
                .map(|value| value.to_string_lossy())
                .unwrap_or_default();
            child(
                static_class.as_ptr(),
                &file,
                WS_VISIBLE | SS_CENTERIMAGE | SS_ENDELLIPSIS,
                NAME,
            )?;
            child(
                wide("EDIT").as_ptr(),
                "",
                WS_TABSTOP
                    | WS_VSCROLL
                    | ES_MULTILINE as u32
                    | ES_AUTOVSCROLL as u32
                    | ES_READONLY as u32,
                TEXT,
            )?;
            child(
                static_class.as_ptr(),
                preview
                    .state
                    .label("Loading preview…", "Carregando prévia…"),
                WS_VISIBLE | SS_CENTERIMAGE,
                STATUS,
            )?;
            child(
                wide("BUTTON").as_ptr(),
                preview.state.label("Close", "Fechar"),
                WS_VISIBLE | WS_TABSTOP | BS_OWNERDRAW as u32,
                DISMISS,
            )?;
            let state = &raw mut *preview.state;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, state as isize);
            let dpi = GetDpiForWindow(hwnd);
            let width = overlay::scale(740, dpi).min((work.right - work.left - 32).max(1));
            let height = overlay::scale(478, dpi).min((work.bottom - work.top - 32).max(1));
            let x = work.left + (work.right - work.left - width) / 2;
            let y = work.top + (work.bottom - work.top - height) / 2;
            SetWindowPos(
                hwnd,
                null_mut(),
                x,
                y,
                width,
                height,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            layout(hwnd, state);
            if !load(hwnd, path, Arc::clone(&(*state).delivery), busy) {
                status(
                    hwnd,
                    (*state).label(
                        "Preview is temporarily unavailable",
                        "A prévia está temporariamente indisponível",
                    ),
                );
            }
            ShowWindow(hwnd, SW_SHOW);
            SetForegroundWindow(hwnd);
            SetFocus(GetDlgItem(hwnd, DISMISS as i32));
            Ok(preview)
        }
    }

    pub fn handle(&self) -> HWND {
        self.hwnd
    }

    pub fn contains(&self, hwnd: HWND) -> bool {
        self.hwnd == hwnd || unsafe { IsChild(self.hwnd, hwnd) != 0 }
    }

    pub fn handle_key(&self, message: &MSG) -> bool {
        unsafe {
            if message.message != WM_KEYDOWN
                || (message.hwnd != self.hwnd && IsChild(self.hwnd, message.hwnd) == 0)
            {
                return false;
            }
            match message.wParam as u16 {
                VK_ESCAPE | VK_SPACE => {
                    // A held key should never close the preview it just opened.
                    if message.lParam & (1 << 30) == 0 {
                        dismiss(self.hwnd, self.state.owner, true);
                    }
                }
                VK_RETURN if message.hwnd == GetDlgItem(self.hwnd, DISMISS as i32) => {
                    dismiss(self.hwnd, self.state.owner, true)
                }
                VK_TAB => {
                    SetFocus(GetNextDlgTabItem(
                        self.hwnd,
                        message.hwnd,
                        i32::from(GetKeyState(i32::from(VK_SHIFT)) < 0),
                    ));
                }
                _ => return false,
            }
            true
        }
    }
}

impl Drop for QuickLook {
    fn drop(&mut self) {
        {
            let mut delivery = self.state.delivery.lock().unwrap();
            delivery.closed = true;
            delivery.result = None;
        }
        unsafe {
            SetWindowLongPtrW(self.hwnd, GWLP_USERDATA, 0);
            DestroyWindow(self.hwnd);
            DeleteObject(self.state.background);
            DeleteObject(self.state.font);
            DeleteObject(self.state.text_font);
        }
    }
}

unsafe fn layout(hwnd: HWND, state: *mut State) {
    unsafe {
        let dpi = GetDpiForWindow(hwnd);
        let scale = |dip| overlay::scale(dip, dpi);
        if dpi != (*state).dpi {
            let font = |face, size| {
                CreateFontW(
                    -scale(size),
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
                    wide(face).as_ptr(),
                )
            };
            let regular = font("Segoe UI", 14);
            let mono = font("Consolas", 14);
            if !regular.is_null() && !mono.is_null() {
                for id in [NAME, STATUS, DISMISS] {
                    SendMessageW(GetDlgItem(hwnd, id as i32), WM_SETFONT, regular as usize, 0);
                }
                SendMessageW(GetDlgItem(hwnd, TEXT as i32), WM_SETFONT, mono as usize, 0);
                DeleteObject((*state).font);
                DeleteObject((*state).text_font);
                (*state).font = regular;
                (*state).text_font = mono;
                (*state).dpi = dpi;
            } else {
                DeleteObject(regular);
                DeleteObject(mono);
            }
        }
        let mut bounds = RECT::default();
        GetClientRect(hwnd, &mut bounds);
        let pad = scale(12);
        let width = (bounds.right - 2 * pad).max(1);
        MoveWindow(
            GetDlgItem(hwnd, NAME as i32),
            pad,
            scale(9),
            (width - scale(76)).max(1),
            scale(26),
            1,
        );
        MoveWindow(
            GetDlgItem(hwnd, DISMISS as i32),
            (bounds.right - pad - scale(68)).max(pad),
            scale(9),
            scale(68),
            scale(26),
            1,
        );
        MoveWindow(
            GetDlgItem(hwnd, STATUS as i32),
            pad,
            (bounds.bottom - scale(29)).max(scale(38)),
            width,
            scale(23),
            1,
        );
        MoveWindow(
            GetDlgItem(hwnd, TEXT as i32),
            pad,
            scale(43),
            width,
            (bounds.bottom - scale(76)).max(1),
            1,
        );
    }
}

unsafe fn status(hwnd: HWND, value: &str) {
    unsafe {
        SetWindowTextW(GetDlgItem(hwnd, STATUS as i32), wide(value).as_ptr());
    }
}

unsafe fn update(hwnd: HWND, state: *mut State) {
    unsafe {
        let result = (*state).delivery.lock().unwrap().result.take();
        let Some(content) = result else {
            return;
        };
        let text = GetDlgItem(hwnd, TEXT as i32);
        match content {
            Content::Text { value, cut } => {
                SetWindowTextW(text, wide(&value).as_ptr());
                ShowWindow(text, SW_SHOWNA);
                if GetForegroundWindow() == hwnd {
                    SetFocus(text);
                }
                let message = if cut {
                    (*state).label(
                        "Text preview limited to 256 KiB",
                        "Prévia de texto limitada a 256 KiB",
                    )
                } else {
                    (*state).label("Text preview", "Prévia de texto")
                };
                status(hwnd, message);
            }
            Content::Image(image) => {
                (*state).image = Some(image);
                status(hwnd, "");
            }
            Content::Pdf { image, pages } => {
                (*state).image = Some(image);
                status(
                    hwnd,
                    &format!("{} 1 / {pages}", (*state).label("Page", "Página")),
                );
            }
            Content::Directory => status(hwnd, (*state).label("Folder", "Pasta")),
            Content::Details(bytes) => status(hwnd, &format!("{bytes} bytes")),
            Content::TooLarge => status(
                hwnd,
                (*state).label(
                    "Preview exceeds the 16 MiB limit",
                    "A prévia excede o limite de 16 MiB",
                ),
            ),
            Content::Unavailable => status(
                hwnd,
                (*state).label("Preview unavailable", "Prévia indisponível"),
            ),
        }
        InvalidateRect(hwnd, std::ptr::null(), 1);
    }
}

unsafe fn dismiss(hwnd: HWND, owner: HWND, restore: bool) {
    unsafe {
        PostMessageW(owner, CLOSE, hwnd as usize, isize::from(restore));
    }
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State;
        if state.is_null() {
            return DefWindowProcW(hwnd, msg, wp, lp);
        }
        match msg {
            READY => {
                update(hwnd, state);
                0
            }
            WM_CLOSE => {
                dismiss(hwnd, (*state).owner, true);
                0
            }
            WM_ACTIVATE if wp as u16 == WA_INACTIVE as u16 => {
                dismiss(hwnd, (*state).owner, false);
                0
            }
            WM_COMMAND
                if wp as u16 as usize == DISMISS
                    && lp != 0
                    && (wp >> 16) as u16 == BN_CLICKED as u16 =>
            {
                dismiss(hwnd, (*state).owner, true);
                0
            }
            WM_DPICHANGED if lp != 0 => {
                let bounds = &*(lp as *const RECT);
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
                0
            }
            WM_PAINT => {
                let mut paint = PAINTSTRUCT::default();
                let dc = BeginPaint(hwnd, &mut paint);
                if let Some(image) = &(*state).image {
                    let mut area = RECT::default();
                    GetClientRect(hwnd, &mut area);
                    let scale = |dip| overlay::scale(dip, GetDpiForWindow(hwnd));
                    area.left += scale(12);
                    area.right -= scale(12);
                    area.top += scale(43);
                    area.bottom -= scale(33);
                    image.draw(dc, area, (*state).palette.background);
                }
                EndPaint(hwnd, &paint);
                0
            }
            WM_ERASEBKGND => {
                let mut bounds = RECT::default();
                GetClientRect(hwnd, &mut bounds);
                FillRect(wp as HDC, &bounds, (*state).background);
                1
            }
            WM_CTLCOLORSTATIC => {
                SetBkColor(wp as HDC, (*state).palette.background);
                SetTextColor(wp as HDC, (*state).palette.text);
                (*state).background as isize
            }
            WM_DRAWITEM if lp != 0 => {
                overlay::paint_button(
                    &*(lp as *const DRAWITEMSTRUCT),
                    (*state).palette,
                    (*state).font,
                );
                1
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}
