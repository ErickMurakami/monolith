//! Native Files window. The list owns selection and requests visible rows from the directory model.

use crate::{
    files_directory::{self, Listing, Move, Navigation, Reader},
    overlay::{self, Palette},
    quick_look::{self, QuickLook},
    windows::{report_for, wide},
};
use std::{
    ffi::{OsStr, OsString, c_void},
    io,
    mem::size_of,
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
    ptr::null_mut,
    sync::{Arc, atomic::AtomicBool},
};
use windows_sys::{
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
        Graphics::Gdi::*,
        System::{
            Com::CoTaskMemFree, LibraryLoader::GetModuleHandleW, SystemServices::SS_CENTERIMAGE,
        },
        UI::{
            Controls::*,
            HiDpi::GetDpiForWindow,
            Input::KeyboardAndMouse::*,
            Shell::{
                FOLDERID_Desktop, FOLDERID_Documents, FOLDERID_Downloads, FOLDERID_Profile,
                KF_FLAG_DONT_VERIFY, SHGetKnownFolderPath, ShellExecuteW,
            },
            WindowsAndMessaging::*,
        },
    },
    core::GUID,
};

const TITLE: usize = 1;
pub const OPEN: u32 = WM_APP + 12;
const MINIMISE: usize = 2;
const MAXIMISE: usize = 3;
const CLOSE: usize = 4;
const BACK: usize = 5;
const FORWARD: usize = 6;
const UP: usize = 7;
const PATH: usize = 8;
const LIST: usize = 9;
const STATUS: usize = 10;
const HIDDEN: usize = 11;
const FIRST_FAVOURITE: usize = 20;
const FOLDERS: [(GUID, &str, &str); 4] = [
    (FOLDERID_Profile, "Home", "Início"),
    (FOLDERID_Desktop, "Desktop", "Área de Trabalho"),
    (FOLDERID_Documents, "Documents", "Documentos"),
    (FOLDERID_Downloads, "Downloads", "Downloads"),
];

struct State {
    reader: Option<Reader>,
    preview: Option<QuickLook>,
    preview_busy: Arc<AtomicBool>,
    navigation: Navigation,
    listing: Listing,
    requested: Option<(PathBuf, Move)>,
    favourites: [Option<PathBuf>; FOLDERS.len()],
    hidden: bool,
    portuguese: bool,
    palette: Palette,
    background: HBRUSH,
    surface: HBRUSH,
    font: HFONT,
    dpi: u32,
}

impl State {
    fn label(&self, english: &'static str, portuguese: &'static str) -> &'static str {
        if self.portuguese { portuguese } else { english }
    }
}

impl Drop for State {
    fn drop(&mut self) {
        unsafe {
            DeleteObject(self.background);
            DeleteObject(self.surface);
            DeleteObject(self.font);
        }
    }
}

fn wide_path(path: &OsStr) -> Vec<u16> {
    path.encode_wide().chain([0]).collect()
}

fn known_folder(folder: &GUID) -> Option<PathBuf> {
    unsafe {
        let mut raw = null_mut();
        let result = SHGetKnownFolderPath(folder, KF_FLAG_DONT_VERIFY as u32, null_mut(), &mut raw);
        let value = if result >= 0 && !raw.is_null() {
            let mut length = 0;
            while *raw.add(length) != 0 {
                length += 1;
            }
            Some(PathBuf::from(OsString::from_wide(
                std::slice::from_raw_parts(raw, length),
            )))
        } else {
            None
        };
        CoTaskMemFree(raw.cast());
        value
    }
}

pub fn run(initial: Option<PathBuf>, palette: Palette, portuguese: bool) -> io::Result<()> {
    unsafe {
        let instance = GetModuleHandleW(std::ptr::null());
        let name = wide("MonolithFiles");
        let class = WNDCLASSW {
            lpszClassName: name.as_ptr(),
            hInstance: instance,
            lpfnWndProc: Some(window_proc),
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            ..Default::default()
        };
        if RegisterClassW(&class) == 0 {
            return Err(io::Error::last_os_error());
        }
        let controls = INITCOMMONCONTROLSEX {
            dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_LISTVIEW_CLASSES,
        };
        if InitCommonControlsEx(&controls) == 0 {
            return Err(io::Error::last_os_error());
        }
        let hwnd = CreateWindowExW(
            0,
            name.as_ptr(),
            wide("Monolith Files").as_ptr(),
            WS_THICKFRAME | WS_SYSMENU | WS_MINIMIZEBOX | WS_MAXIMIZEBOX | WS_CLIPCHILDREN,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            900,
            570,
            null_mut(),
            null_mut(),
            instance,
            null_mut(),
        );
        if hwnd.is_null() {
            return Err(io::Error::last_os_error());
        }
        // CreateWindowEx adds a caption to an overlapped window. Remove it after creation
        // so Windows still calculates a resizable frame and taskbar presence.
        let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
        SetWindowLongW(hwnd, GWL_STYLE, (style & !WS_CAPTION) as i32);
        SetWindowPos(
            hwnd,
            null_mut(),
            0,
            0,
            0,
            0,
            SWP_NOSIZE | SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        );
        let mut state = Box::new(State {
            reader: None,
            preview: None,
            preview_busy: Arc::default(),
            navigation: Navigation::default(),
            requested: None,
            listing: Listing::default(),
            favourites: FOLDERS.map(|(id, _, _)| known_folder(&id)),
            hidden: false,
            portuguese,
            palette,
            background: CreateSolidBrush(palette.background),
            surface: CreateSolidBrush(palette.surface),
            font: null_mut(),
            dpi: 0,
        });
        let state = &raw mut *state;
        let result =
            initialise(hwnd, instance, state, initial).and_then(|()| run_loop(hwnd, state));
        if IsWindow(hwnd) != 0 {
            drop((*state).preview.take());
            drop((*state).reader.take());
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            DestroyWindow(hwnd);
        }
        result
    }
}

unsafe fn initialise(
    hwnd: HWND,
    instance: *mut c_void,
    state: *mut State,
    initial: Option<PathBuf>,
) -> io::Result<()> {
    unsafe {
        if (*state).background.is_null() || (*state).surface.is_null() {
            return Err(io::Error::last_os_error());
        }
        let child = |class, caption: &str, style, id| {
            let handle = CreateWindowExW(
                0,
                class,
                wide(caption).as_ptr(),
                WS_CHILD | WS_VISIBLE | style,
                0,
                0,
                0,
                0,
                hwnd,
                id as *mut c_void,
                instance,
                null_mut(),
            );
            if handle.is_null() {
                Err(io::Error::last_os_error())
            } else {
                Ok(handle)
            }
        };
        let static_class = wide("STATIC");
        let button_class = wide("BUTTON");
        let button = |caption, id| {
            child(
                button_class.as_ptr(),
                caption,
                WS_TABSTOP | BS_OWNERDRAW as u32,
                id,
            )
        };
        child(static_class.as_ptr(), "Files", SS_CENTERIMAGE, TITLE)?;
        for (id, english, translated) in [
            (MINIMISE, "Minimise", "Minimizar"),
            (MAXIMISE, "Maximise", "Maximizar"),
            (CLOSE, "Close", "Fechar"),
            (BACK, "Back", "Voltar"),
            (FORWARD, "Forward", "Avançar"),
            (UP, "Up", "Acima"),
        ] {
            button((*state).label(english, translated), id)?;
        }
        let edit = child(
            wide("EDIT").as_ptr(),
            "",
            WS_TABSTOP | ES_AUTOHSCROLL as u32,
            PATH,
        )?;
        SendMessageW(edit, EM_LIMITTEXT, 32_767, 0);
        SendMessageW(
            edit,
            EM_SETCUEBANNER,
            0,
            wide((*state).label("Folder path", "Caminho da pasta")).as_ptr() as isize,
        );
        let list = child(
            WC_LISTVIEWW,
            "",
            WS_TABSTOP | LVS_REPORT | LVS_OWNERDATA | LVS_NOCOLUMNHEADER | LVS_SHOWSELALWAYS,
            LIST,
        )?;
        SendMessageW(
            list,
            LVM_SETEXTENDEDLISTVIEWSTYLE,
            LVS_EX_FULLROWSELECT as usize,
            LVS_EX_FULLROWSELECT as isize,
        );
        SendMessageW(
            list,
            LVM_SETBKCOLOR,
            0,
            (*state).palette.background as isize,
        );
        SendMessageW(
            list,
            LVM_SETTEXTBKCOLOR,
            0,
            (*state).palette.background as isize,
        );
        SendMessageW(list, LVM_SETTEXTCOLOR, 0, (*state).palette.text as isize);
        for (index, heading) in [
            (*state).label("Name", "Nome"),
            (*state).label("Kind", "Tipo"),
        ]
        .into_iter()
        .enumerate()
        {
            let mut text = wide(heading);
            let column = LVCOLUMNW {
                mask: LVCF_TEXT | LVCF_WIDTH,
                cx: 100,
                pszText: text.as_mut_ptr(),
                ..Default::default()
            };
            if SendMessageW(list, LVM_INSERTCOLUMNW, index, &raw const column as isize) < 0 {
                return Err(io::Error::last_os_error());
            }
        }
        for (index, (_, english, translated)) in FOLDERS.iter().enumerate() {
            let favourite = button((*state).label(english, translated), FIRST_FAVOURITE + index)?;
            EnableWindow(
                favourite,
                i32::from((&(*state).favourites)[index].is_some()),
            );
        }
        button((*state).label("Show hidden", "Mostrar ocultos"), HIDDEN)?;
        child(static_class.as_ptr(), "", SS_CENTERIMAGE, STATUS)?;
        (*state).reader = Some(Reader::start(hwnd)?);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, state as isize);
        layout(hwnd, state);
        let base = std::env::current_dir()?;
        let path = initial
            .or_else(|| (&(*state).favourites)[0].clone())
            .unwrap_or_else(|| base.clone());
        match files_directory::resolve(&base, &path) {
            Ok(path) => request(hwnd, state, path, Move::Open),
            Err(error) => show_error(hwnd, state, &error),
        }
        ShowWindow(hwnd, SW_SHOW);
        SetForegroundWindow(hwnd);
        SetFocus(list);
        Ok(())
    }
}

unsafe fn run_loop(hwnd: HWND, state: *mut State) -> io::Result<()> {
    unsafe {
        let mut message = MSG::default();
        loop {
            match GetMessageW(&mut message, null_mut(), 0, 0) {
                -1 => return Err(io::Error::last_os_error()),
                0 => return Ok(()),
                _ => {}
            }
            if key(hwnd, state, &message) {
                continue;
            }
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

unsafe fn key(hwnd: HWND, state: *mut State, message: &MSG) -> bool {
    unsafe {
        if let Some(preview) = (*state)
            .preview
            .as_ref()
            .filter(|preview| preview.contains(message.hwnd))
        {
            return preview.handle_key(message);
        }
        let edit = GetDlgItem(hwnd, PATH as i32);
        let list = GetDlgItem(hwnd, LIST as i32);
        let vk = message.wParam as u16;
        if message.message == WM_SYSKEYDOWN {
            let button = match vk {
                VK_LEFT => BACK,
                VK_RIGHT => FORWARD,
                VK_UP => UP,
                _ => return false,
            };
            action(hwnd, state, button);
            return true;
        }
        if message.message != WM_KEYDOWN {
            return false;
        }
        let ctrl = GetKeyState(i32::from(VK_CONTROL)) < 0;
        match vk {
            VK_L if ctrl => {
                SetFocus(edit);
                SendMessageW(edit, EM_SETSEL, 0, -1);
            }
            VK_H if ctrl => action(hwnd, state, HIDDEN),
            VK_F5 => refresh(hwnd, state),
            VK_RETURN if message.hwnd == edit => {
                let length = GetWindowTextLengthW(edit);
                let mut text = vec![0; length as usize + 1];
                let count = GetWindowTextW(edit, text.as_mut_ptr(), length + 1);
                if count > 0 {
                    let input = PathBuf::from(OsString::from_wide(&text[..count as usize]));
                    let base = (*state)
                        .navigation
                        .current()
                        .map(Path::to_path_buf)
                        .map_or_else(std::env::current_dir, Ok);
                    match base.and_then(|base| files_directory::resolve(&base, &input)) {
                        Ok(path) => request(hwnd, state, path, Move::Open),
                        Err(error) => show_error(hwnd, state, &error),
                    }
                }
            }
            VK_RETURN if message.hwnd == list => {
                let selected =
                    SendMessageW(list, LVM_GETNEXTITEM, usize::MAX, LVNI_FOCUSED as isize);
                activate(hwnd, state, selected as i32);
            }
            VK_SPACE if message.hwnd == list => {
                if message.lParam & (1 << 30) == 0 {
                    let selected =
                        SendMessageW(list, LVM_GETNEXTITEM, usize::MAX, LVNI_FOCUSED as isize);
                    preview(hwnd, state, selected as i32);
                }
            }
            VK_RETURN if message.hwnd != hwnd => {
                SendMessageW(message.hwnd, BM_CLICK, 0, 0);
            }
            VK_TAB => {
                let from = if message.hwnd == hwnd {
                    null_mut()
                } else {
                    message.hwnd
                };
                SetFocus(GetNextDlgTabItem(
                    hwnd,
                    from,
                    i32::from(GetKeyState(i32::from(VK_SHIFT)) < 0),
                ));
            }
            _ => return false,
        }
        true
    }
}

unsafe fn layout(hwnd: HWND, state: *mut State) {
    unsafe {
        let dpi = GetDpiForWindow(hwnd);
        let scale = |dip| overlay::scale(dip, dpi);
        if (*state).dpi != dpi {
            let font = CreateFontW(
                -scale(15),
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
                for id in (TITLE..=HIDDEN).chain(FIRST_FAVOURITE..FIRST_FAVOURITE + FOLDERS.len()) {
                    SendMessageW(GetDlgItem(hwnd, id as i32), WM_SETFONT, font as usize, 0);
                }
                DeleteObject((*state).font);
                (*state).font = font;
                (*state).dpi = dpi;
            }
        }
        let mut area = RECT::default();
        GetClientRect(hwnd, &mut area);
        let pad = scale(10);
        let width = area.right;
        let height = area.bottom;
        let place = |id, left, top, width: i32, height: i32| {
            let child = GetDlgItem(hwnd, id);
            SetWindowPos(
                child,
                null_mut(),
                left,
                top,
                width.max(1),
                height.max(1),
                SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOCOPYBITS,
            );
            InvalidateRect(child, std::ptr::null(), 0);
        };
        place(TITLE as i32, pad, 0, scale(150), scale(31));
        for id in MINIMISE..=CLOSE {
            place(
                id as i32,
                width - pad - scale(82 * (CLOSE + 1 - id) as i32),
                scale(4),
                scale(79),
                scale(26),
            );
        }
        for (index, id) in [BACK, FORWARD, UP].into_iter().enumerate() {
            place(
                id as i32,
                pad + scale(index as i32 * 75),
                scale(38),
                scale(71),
                scale(29),
            );
        }
        let path_left = pad + scale(225);
        place(
            PATH as i32,
            path_left,
            scale(41),
            width - path_left - pad,
            scale(24),
        );
        SendMessageW(
            GetDlgItem(hwnd, PATH as i32),
            EM_SETMARGINS,
            3,
            (scale(7) | scale(7) << 16) as isize,
        );
        for index in 0..FOLDERS.len() {
            place(
                (FIRST_FAVOURITE + index) as i32,
                pad,
                scale(79 + index as i32 * 33),
                scale(128),
                scale(28),
            );
        }
        place(HIDDEN as i32, pad, scale(219), scale(128), scale(28));
        let left = pad + scale(145);
        let list_width = (width - left - pad).max(1);
        place(
            LIST as i32,
            left,
            scale(74),
            list_width,
            height - scale(108),
        );
        let list = GetDlgItem(hwnd, LIST as i32);
        let kind = scale(85);
        SendMessageW(
            list,
            LVM_SETCOLUMNWIDTH,
            0,
            (list_width - kind - scale(20)).max(40) as isize,
        );
        SendMessageW(list, LVM_SETCOLUMNWIDTH, 1, kind as isize);
        place(
            STATUS as i32,
            left,
            height - scale(28),
            list_width,
            scale(22),
        );
        InvalidateRect(hwnd, std::ptr::null(), 1);
    }
}

unsafe fn set_text(hwnd: HWND, id: usize, text: &str) {
    unsafe {
        SetWindowTextW(GetDlgItem(hwnd, id as i32), wide(text).as_ptr());
    }
}

unsafe fn request(hwnd: HWND, state: *mut State, path: PathBuf, movement: Move) {
    unsafe {
        if let Some(reader) = &(*state).reader {
            SetWindowTextW(
                GetDlgItem(hwnd, PATH as i32),
                wide_path(path.as_os_str()).as_ptr(),
            );
            set_text(
                hwnd,
                STATUS,
                (*state).label("Loading folder…", "Abrindo pasta…"),
            );
            (*state).requested = Some((path.clone(), movement));
            reader.load(path, movement, (*state).hidden);
        }
    }
}

unsafe fn refresh(hwnd: HWND, state: *mut State) {
    unsafe {
        let current = (*state).requested.clone().or_else(|| {
            (*state)
                .navigation
                .current()
                .map(|path| (path.to_owned(), Move::Refresh))
        });
        if let Some((path, movement)) = current {
            request(hwnd, state, path, movement);
        }
    }
}

unsafe fn show_error(hwnd: HWND, state: *mut State, error: &io::Error) {
    unsafe {
        let label = (*state).label("Could not open folder", "Não foi possível abrir a pasta");
        let status = format!("{label} · {error}");
        set_text(hwnd, STATUS, &status);
        if let Some(path) = (*state).navigation.current() {
            SetWindowTextW(
                GetDlgItem(hwnd, PATH as i32),
                wide_path(path.as_os_str()).as_ptr(),
            );
        }
    }
}

unsafe fn complete(hwnd: HWND, state: *mut State) {
    unsafe {
        let Some(result) = (*state).reader.as_ref().and_then(Reader::completed) else {
            return;
        };
        (*state).requested = None;
        match result.listing {
            Ok(listing) => {
                let list = GetDlgItem(hwnd, LIST as i32);
                SendMessageW(list, LVM_DELETEALLITEMS, 0, 0);
                (*state).navigation.commit(result.path, result.movement);
                let count = listing.entries.len();
                let status = if listing.incomplete {
                    format!(
                        "{count} {}",
                        (*state).label(
                            "items shown · listing incomplete",
                            "itens exibidos · lista incompleta"
                        )
                    )
                } else {
                    format!("{count} {}", (*state).label("items", "itens"))
                };
                (*state).listing = listing;
                SendMessageW(list, LVM_SETITEMCOUNT, count, 0);
                set_text(hwnd, STATUS, &status);
            }
            Err(error) => show_error(hwnd, state, &error),
        }
        EnableWindow(
            GetDlgItem(hwnd, BACK as i32),
            i32::from((*state).navigation.history(false).is_some()),
        );
        EnableWindow(
            GetDlgItem(hwnd, FORWARD as i32),
            i32::from((*state).navigation.history(true).is_some()),
        );
        EnableWindow(
            GetDlgItem(hwnd, UP as i32),
            i32::from(
                (*state)
                    .navigation
                    .current()
                    .and_then(Path::parent)
                    .is_some(),
            ),
        );
    }
}

unsafe fn activate(hwnd: HWND, state: *mut State, row: i32) {
    unsafe {
        let entries = &(*state).listing.entries;
        let Some((folder, entry)) = (*state).navigation.current().zip(entries.get(row as usize))
        else {
            return;
        };
        let path = folder.join(&entry.name);
        if entry.directory {
            request(hwnd, state, path, Move::Open);
        } else {
            let result = ShellExecuteW(
                hwnd,
                wide("open").as_ptr(),
                wide_path(path.as_os_str()).as_ptr(),
                std::ptr::null(),
                wide_path(folder.as_os_str()).as_ptr(),
                SW_SHOWNORMAL,
            );
            if result as isize <= 32 {
                report_for(
                    hwnd,
                    (*state).label(
                        "Could not open this item with the Windows default app.",
                        "Não foi possível abrir este item no aplicativo padrão do Windows.",
                    ),
                );
            }
        }
    }
}

unsafe fn preview(hwnd: HWND, state: *mut State, row: i32) {
    unsafe {
        let entries = &(*state).listing.entries;
        let Some((folder, entry)) = (*state).navigation.current().zip(entries.get(row as usize))
        else {
            return;
        };
        let path = folder.join(&entry.name);
        drop((*state).preview.take());
        match QuickLook::open(
            hwnd,
            path,
            (*state).palette,
            (*state).portuguese,
            Arc::clone(&(*state).preview_busy),
        ) {
            Ok(preview) => (*state).preview = Some(preview),
            Err(error) => report_for(
                hwnd,
                &format!(
                    "{} · {error}",
                    (*state).label(
                        "Could not open Quick Look",
                        "Não foi possível abrir a prévia"
                    )
                ),
            ),
        }
    }
}

unsafe fn action(hwnd: HWND, state: *mut State, id: usize) {
    unsafe {
        match id {
            MINIMISE => {
                ShowWindow(hwnd, SW_MINIMIZE);
            }
            MAXIMISE => {
                ShowWindow(
                    hwnd,
                    if IsZoomed(hwnd) == 0 {
                        SW_MAXIMIZE
                    } else {
                        SW_RESTORE
                    },
                );
            }
            CLOSE => {
                PostMessageW(hwnd, WM_CLOSE, 0, 0);
            }
            BACK | FORWARD => {
                if let Some((path, movement)) = (*state).navigation.history(id == FORWARD) {
                    request(hwnd, state, path, movement);
                }
            }
            UP => {
                if let Some(parent) = (*state)
                    .navigation
                    .current()
                    .and_then(Path::parent)
                    .map(Path::to_path_buf)
                {
                    request(hwnd, state, parent, Move::Open);
                }
            }
            HIDDEN => {
                (*state).hidden = !(*state).hidden;
                let caption = if (*state).hidden {
                    (*state).label("Hide hidden", "Esconder ocultos")
                } else {
                    (*state).label("Show hidden", "Mostrar ocultos")
                };
                set_text(hwnd, HIDDEN, caption);
                refresh(hwnd, state);
            }
            FIRST_FAVOURITE.. => {
                let favourites = &(*state).favourites;
                if let Some(Some(path)) = favourites.get(id - FIRST_FAVOURITE) {
                    request(hwnd, state, path.clone(), Move::Open);
                }
            }
            _ => {}
        }
    }
}

unsafe fn cell_text(state: *mut State, info: &mut NMLVDISPINFOW) {
    unsafe {
        let item = &mut info.item;
        if item.mask & LVIF_TEXT == 0 || item.pszText.is_null() || item.cchTextMax < 1 {
            return;
        }
        let entries = &(*state).listing.entries;
        let content = entries.get(item.iItem as usize).map_or("", |entry| {
            if item.iSubItem == 0 {
                entry.display()
            } else if entry.directory {
                (*state).label("Folder", "Pasta")
            } else {
                (*state).label("File", "Arquivo")
            }
        });
        let text = std::slice::from_raw_parts_mut(item.pszText, item.cchTextMax as usize);
        let available = text.len() - 1;
        let count = content
            .encode_utf16()
            .zip(text.iter_mut().take(available))
            .map(|(unit, output)| {
                *output = unit;
            })
            .count();
        text[count] = 0;
    }
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State;
        if state.is_null() {
            return DefWindowProcW(hwnd, msg, wp, lp);
        }
        match msg {
            quick_look::CLOSE => {
                if (*state).preview.as_ref().is_some_and(|preview| {
                    preview.handle() == wp as HWND
                        && (lp != 0 || GetForegroundWindow() != preview.handle())
                }) {
                    if lp != 0 && GetForegroundWindow() == wp as HWND {
                        SetForegroundWindow(hwnd);
                    }
                    drop((*state).preview.take());
                    if lp != 0 && GetForegroundWindow() == hwnd {
                        SetFocus(GetDlgItem(hwnd, LIST as i32));
                    }
                }
                0
            }
            files_directory::READY => {
                complete(hwnd, state);
                0
            }
            WM_COMMAND if lp != 0 && (wp >> 16) as u16 == BN_CLICKED as u16 => {
                action(hwnd, state, wp as u16 as usize);
                0
            }
            WM_NOTIFY if lp != 0 => {
                let header = &*(lp as *const NMHDR);
                if header.idFrom == LIST {
                    match header.code {
                        LVN_GETDISPINFOW => cell_text(state, &mut *(lp as *mut NMLVDISPINFOW)),
                        LVN_ITEMACTIVATE => {
                            activate(hwnd, state, (*(lp as *const NMITEMACTIVATE)).iItem)
                        }
                        LVN_ODFINDITEMW => return -1,
                        _ => {}
                    }
                }
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
            WM_SIZE => {
                if wp != SIZE_MINIMIZED as usize {
                    let title = if wp == SIZE_MAXIMIZED as usize {
                        (*state).label("Restore", "Restaurar")
                    } else {
                        (*state).label("Maximise", "Maximizar")
                    };
                    set_text(hwnd, MAXIMISE, title);
                    layout(hwnd, state);
                }
                0
            }
            WM_GETMINMAXINFO if lp != 0 => {
                let limits = &mut *(lp as *mut MINMAXINFO);
                let dpi = GetDpiForWindow(hwnd);
                limits.ptMinTrackSize.x = overlay::scale(550, dpi);
                limits.ptMinTrackSize.y = overlay::scale(310, dpi);
                0
            }
            WM_NCHITTEST => {
                let base = DefWindowProcW(hwnd, msg, wp, lp);
                if base != HTCLIENT as isize {
                    return base;
                }
                let mut point = POINT {
                    x: i32::from(lp as i16),
                    y: i32::from((lp >> 16) as i16),
                };
                ScreenToClient(hwnd, &mut point);
                let dpi = GetDpiForWindow(hwnd);
                let mut client = RECT::default();
                GetClientRect(hwnd, &mut client);
                if point.y < overlay::scale(35, dpi)
                    && point.x < client.right - overlay::scale(256, dpi)
                {
                    HTCAPTION as isize
                } else {
                    base
                }
            }
            WM_DRAWITEM if lp != 0 => {
                overlay::paint_button(
                    &*(lp as *const DRAWITEMSTRUCT),
                    (*state).palette,
                    (*state).font,
                );
                1
            }
            WM_CTLCOLORSTATIC | WM_CTLCOLOREDIT => {
                let (background, brush) = if msg == WM_CTLCOLOREDIT {
                    ((*state).palette.surface, (*state).surface)
                } else {
                    ((*state).palette.background, (*state).background)
                };
                SetBkColor(wp as HDC, background);
                SetTextColor(wp as HDC, (*state).palette.text);
                brush as isize
            }
            WM_ERASEBKGND => {
                let mut area = RECT::default();
                GetClientRect(hwnd, &mut area);
                FillRect(wp as HDC, &area, (*state).background);
                1
            }
            WM_CLOSE => {
                drop((*state).preview.take());
                drop((*state).reader.take());
                DestroyWindow(hwnd);
                0
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                0
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}
