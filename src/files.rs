//! Native Files window. The list owns selection and requests visible rows from the directory model.

use crate::{
    file_changes::{self, Change, Outcome},
    file_clipboard::{self, MAX_PATHS},
    file_git::{self, Branch, ItemStatus},
    file_launch::{self, Tool},
    file_recycle::{self, RecycleOutcome},
    file_transfer::{self, TransferOutcome},
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
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    thread,
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
const NAME_EDIT: usize = 12;
const APPLY_NAME: usize = 13;
const CANCEL_NAME: usize = 14;
const GIT_TOGGLE: usize = 15;
const GIT_LABEL: usize = 16;
const GIT_READY: u32 = WM_APP + 17;
const CLIPBOARD_READY: u32 = WM_APP + 19;
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
    name_edit: Option<(PathBuf, Change)>,
    change: Option<Receiver<Outcome>>,
    git_enabled: bool,
    git_revision: u64,
    git: Option<file_git::Status>,
    git_job: Option<GitJob>,
    ide: Option<PathBuf>,
    external: Option<Receiver<io::Result<()>>>,
    clipboard: Option<ClipboardJob>,
    transfer_status: Option<String>,
    navigation: Navigation,
    listing: Listing,
    listing_generation: u64,
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

struct GitJob {
    revision: u64,
    result: Receiver<io::Result<Option<file_git::Status>>>,
}

struct ClipboardJob {
    result: Receiver<io::Result<ClipboardOutcome>>,
    cancel: Arc<AtomicBool>,
    close_after: bool,
}

enum ClipboardWork {
    Publish(Vec<PathBuf>, bool),
    Paste(PathBuf),
    Recycle(Vec<PathBuf>),
}

enum ClipboardOutcome {
    Copied(bool),
    Pasted(TransferOutcome),
    Recycled(RecycleOutcome),
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

pub fn run(
    initial: Option<PathBuf>,
    palette: Palette,
    portuguese: bool,
    ide: Option<PathBuf>,
) -> io::Result<()> {
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
            name_edit: None,
            change: None,
            git_enabled: false,
            git_revision: 0,
            git: None,
            git_job: None,
            ide: file_launch::configured_ide(ide),
            external: None,
            clipboard: None,
            transfer_status: None,
            navigation: Navigation::default(),
            requested: None,
            listing: Listing::default(),
            listing_generation: 0,
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
            "Git",
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
        button((*state).label("Show Git", "Exibir Git"), GIT_TOGGLE)?;
        let git = child(static_class.as_ptr(), "", SS_CENTERIMAGE, GIT_LABEL)?;
        ShowWindow(git, SW_HIDE);
        let name = child(
            wide("EDIT").as_ptr(),
            "",
            WS_TABSTOP | ES_AUTOHSCROLL as u32,
            NAME_EDIT,
        )?;
        SendMessageW(name, EM_LIMITTEXT, 255, 0);
        let apply = button((*state).label("Apply", "Aplicar"), APPLY_NAME)?;
        let cancel = button((*state).label("Cancel", "Cancelar"), CANCEL_NAME)?;
        for item in [name, apply, cancel] {
            ShowWindow(item, SW_HIDE);
        }
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
            VK_ESCAPE if (*state).name_edit.is_some() && (*state).change.is_none() => {
                end_name_edit(hwnd, state)
            }
            VK_ESCAPE if (*state).clipboard.is_some() => cancel_clipboard(hwnd, state),
            VK_RETURN if message.hwnd == GetDlgItem(hwnd, NAME_EDIT as i32) => {
                submit_name(hwnd, state)
            }
            VK_F2 if message.hwnd == list => begin_name_edit(hwnd, state, Some(focused_row(list))),
            VK_N if ctrl && GetKeyState(i32::from(VK_SHIFT)) < 0 => {
                begin_name_edit(hwnd, state, None)
            }
            VK_L if ctrl => {
                SetFocus(edit);
                SendMessageW(edit, EM_SETSEL, 0, -1);
            }
            VK_H if ctrl => action(hwnd, state, HIDDEN),
            VK_G if ctrl => action(hwnd, state, GIT_TOGGLE),
            VK_C | VK_X if ctrl && message.hwnd == list => {
                copy_selection(hwnd, state, None, vk == VK_X)
            }
            VK_V if ctrl && message.hwnd == list => paste_selection(hwnd, state),
            // Shift+Delete is permanent deletion in Explorer; Files offers recycling only.
            VK_DELETE if message.hwnd == list && GetKeyState(i32::from(VK_SHIFT)) >= 0 => {
                recycle_selection(hwnd, state, None)
            }
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
                for id in
                    (TITLE..=GIT_LABEL).chain(FIRST_FAVOURITE..FIRST_FAVOURITE + FOLDERS.len())
                {
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
        place(GIT_TOGGLE as i32, pad, scale(252), scale(128), scale(28));
        let left = pad + scale(145);
        let list_width = (width - left - pad).max(1);
        let git_heading = if (*state).git_enabled { scale(25) } else { 0 };
        place(GIT_LABEL as i32, left, scale(74), list_width, scale(22));
        place(
            LIST as i32,
            left,
            scale(74) + git_heading,
            list_width,
            height
                - scale(if (*state).name_edit.is_some() {
                    144
                } else {
                    108
                })
                - git_heading,
        );
        let list = GetDlgItem(hwnd, LIST as i32);
        let kind = scale(85);
        let git = if (*state).git_enabled { scale(165) } else { 0 };
        SendMessageW(
            list,
            LVM_SETCOLUMNWIDTH,
            0,
            (list_width - kind - git - scale(20)).max(40) as isize,
        );
        SendMessageW(list, LVM_SETCOLUMNWIDTH, 1, kind as isize);
        SendMessageW(list, LVM_SETCOLUMNWIDTH, 2, git as isize);
        place(
            STATUS as i32,
            left,
            height - scale(28),
            list_width,
            scale(22),
        );
        place(
            NAME_EDIT as i32,
            left,
            height - scale(63),
            list_width - scale(161),
            scale(25),
        );
        place(
            APPLY_NAME as i32,
            width - pad - scale(153),
            height - scale(64),
            scale(72),
            scale(26),
        );
        place(
            CANCEL_NAME as i32,
            width - pad - scale(76),
            height - scale(64),
            scale(76),
            scale(26),
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
        (*state).transfer_status = None;
        if (*state).name_edit.is_some() && (*state).change.is_none() {
            end_name_edit(hwnd, state);
        }
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
                (*state).listing = listing;
                (*state).listing_generation += 1;
                SendMessageW(list, LVM_SETITEMCOUNT, count, 0);
                if (*state).change.is_none() {
                    if let Some(label) = (*state).transfer_status.take() {
                        set_text(hwnd, STATUS, &label);
                    } else {
                        listing_status(hwnd, state);
                    }
                }
                if (*state).git_enabled {
                    refresh_git(hwnd, state);
                }
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
            GIT_TOGGLE => {
                (*state).git_enabled = !(*state).git_enabled;
                let label = if (*state).git_enabled {
                    (*state).label("Hide Git", "Ocultar Git")
                } else {
                    (*state).label("Show Git", "Exibir Git")
                };
                set_text(hwnd, GIT_TOGGLE, label);
                ShowWindow(
                    GetDlgItem(hwnd, GIT_LABEL as i32),
                    if (*state).git_enabled {
                        SW_SHOWNA
                    } else {
                        SW_HIDE
                    },
                );
                refresh_git(hwnd, state);
                layout(hwnd, state);
            }
            APPLY_NAME => submit_name(hwnd, state),
            CANCEL_NAME if (*state).change.is_none() => end_name_edit(hwnd, state),
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

fn focused_row(list: HWND) -> i32 {
    unsafe { SendMessageW(list, LVM_GETNEXTITEM, usize::MAX, LVNI_FOCUSED as isize) as i32 }
}

unsafe fn name_controls(hwnd: HWND, visible: bool, enabled: bool) {
    unsafe {
        for id in [NAME_EDIT, APPLY_NAME, CANCEL_NAME] {
            let item = GetDlgItem(hwnd, id as i32);
            EnableWindow(item, i32::from(enabled));
            ShowWindow(item, if visible { SW_SHOWNA } else { SW_HIDE });
        }
    }
}

unsafe fn begin_name_edit(hwnd: HWND, state: *mut State, row: Option<i32>) {
    unsafe {
        if (*state).requested.is_some() || (*state).change.is_some() || (*state).clipboard.is_some()
        {
            return;
        }
        let Some(directory) = (*state).navigation.current().map(Path::to_path_buf) else {
            return;
        };
        let (change, initial, title, selection) = if let Some(row) = row {
            let entries = &(*state).listing.entries;
            let Some(entry) = entries.get(row as usize) else {
                return;
            };
            let length = if entry.directory {
                -1
            } else {
                Path::new(&entry.name)
                    .file_stem()
                    .map_or(-1, |stem| stem.encode_wide().count() as isize)
            };
            (
                Change::Rename(entry.name.clone()),
                entry.name.clone(),
                (*state).label("Rename item", "Renomear item"),
                length,
            )
        } else {
            let title = (*state).label("New folder", "Nova pasta");
            (Change::CreateDirectory, OsString::from(title), title, -1)
        };
        let edit = GetDlgItem(hwnd, NAME_EDIT as i32);
        (*state).name_edit = Some((directory, change));
        SetWindowTextW(edit, wide_path(&initial).as_ptr());
        set_text(hwnd, STATUS, title);
        layout(hwnd, state);
        name_controls(hwnd, true, true);
        SetFocus(edit);
        SendMessageW(edit, EM_SETSEL, 0, selection);
    }
}

unsafe fn end_name_edit(hwnd: HWND, state: *mut State) {
    unsafe {
        (*state).name_edit = None;
        name_controls(hwnd, false, true);
        layout(hwnd, state);
        listing_status(hwnd, state);
        if GetForegroundWindow() == hwnd {
            SetFocus(GetDlgItem(hwnd, LIST as i32));
        }
    }
}

unsafe fn listing_status(hwnd: HWND, state: *mut State) {
    unsafe {
        let count = (*state).listing.entries.len();
        let label = if (*state).listing.incomplete {
            (*state).label(
                "items shown · listing incomplete",
                "itens exibidos · lista incompleta",
            )
        } else {
            (*state).label("items", "itens")
        };
        set_text(hwnd, STATUS, &format!("{count} {label}"));
    }
}

unsafe fn submit_name(hwnd: HWND, state: *mut State) {
    unsafe {
        if (*state).change.is_some() {
            return;
        }
        let Some((directory, change)) = (*state).name_edit.clone() else {
            return;
        };
        let edit = GetDlgItem(hwnd, NAME_EDIT as i32);
        let capacity = GetWindowTextLengthW(edit) + 1;
        let mut input = vec![0; capacity as usize];
        let count = GetWindowTextW(edit, input.as_mut_ptr(), capacity);
        input.truncate(count as usize);
        match file_changes::start(hwnd, directory, change, input) {
            Ok(receive) => {
                (*state).change = Some(receive);
                name_controls(hwnd, true, false);
                set_text(
                    hwnd,
                    STATUS,
                    (*state).label("Applying change…", "Aplicando alteração…"),
                );
            }
            Err(error) if error.kind() == io::ErrorKind::InvalidInput => set_text(
                hwnd,
                STATUS,
                (*state).label(
                    "Enter a valid Windows filename",
                    "Digite um nome de arquivo válido no Windows",
                ),
            ),
            Err(error) => set_text(
                hwnd,
                STATUS,
                &format!(
                    "{} · {error}",
                    (*state).label(
                        "Could not start the operation",
                        "Não foi possível iniciar a operação"
                    )
                ),
            ),
        }
    }
}

unsafe fn finish_change(hwnd: HWND, state: *mut State) {
    unsafe {
        let Some(outcome) = (*state)
            .change
            .as_ref()
            .and_then(|reply| reply.try_recv().ok())
        else {
            return;
        };
        (*state).change = None;
        match outcome.result {
            Ok(()) => {
                end_name_edit(hwnd, state);
                if (*state).navigation.current() == Some(&outcome.directory)
                    && (*state).requested.is_none()
                {
                    refresh(hwnd, state);
                }
            }
            Err(error) => {
                let title = if error.kind() == io::ErrorKind::AlreadyExists {
                    (*state).label("That name is already in use", "Esse nome já está em uso")
                } else {
                    (*state).label(
                        "Could not change this item",
                        "Não foi possível alterar este item",
                    )
                };
                set_text(hwnd, STATUS, &format!("{title} · {error}"));
                name_controls(hwnd, true, true);
                if GetForegroundWindow() == hwnd {
                    SetFocus(GetDlgItem(hwnd, NAME_EDIT as i32));
                }
            }
        }
    }
}

unsafe fn list_menu(hwnd: HWND, state: *mut State, origin: HWND, lp: LPARAM) {
    unsafe {
        let generation = (*state).listing_generation;
        let mut point = POINT {
            x: i32::from(lp as i16),
            y: i32::from((lp >> 16) as i16),
        };
        let row = if lp == -1 {
            let mut area = RECT::default();
            GetWindowRect(origin, &mut area);
            point.x = area.left + 8;
            point.y = area.top + 8;
            focused_row(origin)
        } else {
            let mut hit = LVHITTESTINFO {
                pt: point,
                ..Default::default()
            };
            ScreenToClient(origin, &mut hit.pt);
            SendMessageW(origin, LVM_HITTEST, 0, &raw mut hit as isize) as i32
        };
        let menu = CreatePopupMenu();
        if menu.is_null() {
            return;
        }
        let item_state = if row < 0 { MF_GRAYED } else { MF_ENABLED };
        let folder_state = if (*state).navigation.current().is_some() {
            MF_ENABLED
        } else {
            MF_GRAYED
        };
        for (id, label, flags) in [
            (1, (*state).label("Open", "Abrir"), item_state),
            (2, "Quick Look", item_state),
            (3, (*state).label("Rename…", "Renomear…"), item_state),
            (
                5,
                (*state).label("Open terminal here", "Abrir terminal aqui"),
                folder_state,
            ),
        ] {
            AppendMenuW(menu, MF_STRING | flags, id, wide(label).as_ptr());
        }
        if (*state).ide.is_some() {
            AppendMenuW(
                menu,
                MF_STRING | folder_state,
                6,
                wide((*state).label("Open in configured IDE", "Abrir na IDE configurada")).as_ptr(),
            );
        }
        for (id, label, flags) in [
            (7, (*state).label("Copy", "Copiar"), item_state),
            (8, (*state).label("Cut", "Recortar"), item_state),
            (9, (*state).label("Paste", "Colar"), folder_state),
            (
                10,
                (*state).label("Move to Recycle Bin", "Mover para a Lixeira"),
                item_state,
            ),
        ] {
            AppendMenuW(menu, MF_STRING | flags, id, wide(label).as_ptr());
        }
        AppendMenuW(
            menu,
            MF_STRING | folder_state,
            4,
            wide((*state).label("New folder…", "Nova pasta…")).as_ptr(),
        );
        let selected = TrackPopupMenuEx(
            menu,
            TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON,
            point.x,
            point.y,
            hwnd,
            null_mut(),
        );
        DestroyMenu(menu);
        if IsWindow(hwnd) == 0 || generation != (*state).listing_generation {
            return;
        }
        match selected {
            1 => activate(hwnd, state, row),
            2 => preview(hwnd, state, row),
            3 => begin_name_edit(hwnd, state, Some(row)),
            4 => begin_name_edit(hwnd, state, None),
            5 => launch_tool(hwnd, state, Tool::Terminal, -1),
            6 => {
                if let Some(executable) = (*state).ide.clone() {
                    launch_tool(hwnd, state, Tool::Ide(executable), row);
                }
            }
            7 | 8 => copy_selection(hwnd, state, Some(row), selected == 8),
            9 => paste_selection(hwnd, state),
            10 => recycle_selection(hwnd, state, Some(row)),
            _ => {}
        }
    }
}

unsafe fn launch_tool(hwnd: HWND, state: *mut State, tool: Tool, row: i32) {
    unsafe {
        if (*state).external.is_some() {
            return;
        }
        let Some(current) = (*state).navigation.current() else {
            return;
        };
        let entries = &(*state).listing.entries;
        let folder = entries
            .get(row as usize)
            .filter(|entry| entry.directory)
            .map_or_else(|| current.to_owned(), |entry| current.join(&entry.name));
        match file_launch::start(hwnd, tool, folder) {
            Ok(receive) => (*state).external = Some(receive),
            Err(error) => launch_error(hwnd, state, &error),
        }
    }
}

unsafe fn launch_error(hwnd: HWND, state: *mut State, error: &io::Error) {
    unsafe {
        set_text(
            hwnd,
            STATUS,
            &format!(
                "{} · {error}",
                (*state).label(
                    "Could not launch the tool",
                    "Não foi possível abrir a ferramenta"
                )
            ),
        );
    }
}

unsafe fn copy_selection(hwnd: HWND, state: *mut State, clicked: Option<i32>, cut: bool) {
    unsafe {
        if let Some(paths) = selected_paths(hwnd, state, clicked) {
            start_clipboard(hwnd, state, ClipboardWork::Publish(paths, cut));
        }
    }
}

unsafe fn recycle_selection(hwnd: HWND, state: *mut State, clicked: Option<i32>) {
    unsafe {
        if let Some(paths) = selected_paths(hwnd, state, clicked) {
            start_clipboard(hwnd, state, ClipboardWork::Recycle(paths));
        }
    }
}

/// The clicked row when it lies outside the selection, otherwise the selection.
unsafe fn selected_paths(
    hwnd: HWND,
    state: *mut State,
    clicked: Option<i32>,
) -> Option<Vec<PathBuf>> {
    unsafe {
        let folder = (*state).navigation.current()?;
        let list = GetDlgItem(hwnd, LIST as i32);
        let entries = &(*state).listing.entries;
        let mut paths = Vec::new();
        if let Some(row) = clicked.filter(|row| {
            *row >= 0
                && SendMessageW(
                    list,
                    LVM_GETITEMSTATE,
                    *row as usize,
                    LVIS_SELECTED as isize,
                ) == 0
        }) {
            if let Some(entry) = entries.get(row as usize) {
                paths.push(folder.join(&entry.name));
            }
        } else {
            let mut row = -1;
            loop {
                row = SendMessageW(list, LVM_GETNEXTITEM, row as usize, LVNI_SELECTED as isize);
                let Some(entry) = entries.get(row as usize) else {
                    break;
                };
                if paths.len() == MAX_PATHS {
                    set_text(
                        hwnd,
                        STATUS,
                        (*state).label("The selection is too large", "A seleção é grande demais"),
                    );
                    return None;
                }
                paths.push(folder.join(&entry.name));
            }
        }
        (!paths.is_empty()).then_some(paths)
    }
}

unsafe fn paste_selection(hwnd: HWND, state: *mut State) {
    unsafe {
        if (*state).requested.is_none()
            && let Some(folder) = (*state).navigation.current()
        {
            start_clipboard(hwnd, state, ClipboardWork::Paste(folder.to_owned()));
        }
    }
}

fn clipboard_work(
    window: HWND,
    work: ClipboardWork,
    cancel: Arc<AtomicBool>,
) -> io::Result<ClipboardOutcome> {
    match work {
        ClipboardWork::Publish(paths, cut) => {
            if cancel.load(Ordering::Relaxed) {
                return Err(io::Error::from(io::ErrorKind::Interrupted));
            }
            file_clipboard::publish(window, &paths, cut)?;
            Ok(ClipboardOutcome::Copied(cut))
        }
        ClipboardWork::Paste(folder) => {
            let selection = file_clipboard::read(window)?;
            let outcome = file_transfer::perform(
                window,
                &selection.paths,
                &folder,
                selection.move_items,
                cancel,
            )?;
            if matches!(outcome, TransferOutcome::Complete)
                && let Some(sequence) = selection.own_cut_sequence
            {
                file_clipboard::clear_cut(window, sequence);
            }
            Ok(ClipboardOutcome::Pasted(outcome))
        }
        ClipboardWork::Recycle(paths) => {
            file_recycle::perform(&paths, &cancel).map(ClipboardOutcome::Recycled)
        }
    }
}

unsafe fn start_clipboard(hwnd: HWND, state: *mut State, work: ClipboardWork) {
    unsafe {
        if (*state).clipboard.is_some()
            || (*state).name_edit.is_some()
            || (*state).requested.is_some()
        {
            return;
        }
        let recycling = matches!(&work, ClipboardWork::Recycle(_));
        // Both change the displayed folder, so the listing refreshes afterwards.
        let pasted = recycling || matches!(&work, ClipboardWork::Paste(_));
        let target = hwnd as usize;
        let (sender, result) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        match thread::Builder::new()
            .name("monolith-files-transfer".into())
            .spawn(move || {
                let target = target as HWND;
                if sender
                    .send(clipboard_work(target, work, worker_cancel))
                    .is_ok()
                {
                    PostMessageW(target, CLIPBOARD_READY, usize::from(pasted), 0);
                }
            }) {
            Ok(_) => {
                (*state).clipboard = Some(ClipboardJob {
                    result,
                    cancel,
                    close_after: false,
                });
                if recycling {
                    set_text(
                        hwnd,
                        STATUS,
                        (*state).label(
                            "Moving to the Recycle Bin… Escape cancels",
                            "Movendo para a Lixeira… Escape cancela",
                        ),
                    );
                } else if pasted {
                    set_text(
                        hwnd,
                        STATUS,
                        (*state).label(
                            "Transferring files… Escape cancels",
                            "Transferindo arquivos… Escape cancela",
                        ),
                    );
                }
            }
            Err(error) => set_text(
                hwnd,
                STATUS,
                &format!(
                    "{} · {error}",
                    (*state).label(
                        "Could not start the file operation",
                        "Não foi possível iniciar a operação de arquivos"
                    )
                ),
            ),
        }
    }
}

unsafe fn cancel_clipboard(hwnd: HWND, state: *mut State) {
    unsafe {
        if let Some(job) = (*state).clipboard.as_ref() {
            job.cancel.store(true, Ordering::Relaxed);
            set_text(
                hwnd,
                STATUS,
                (*state).label(
                    "Cancelling file operation…",
                    "Cancelando a operação de arquivos…",
                ),
            );
        }
    }
}

unsafe fn finish_clipboard(hwnd: HWND, state: *mut State, pasted: bool) {
    unsafe {
        let Some(job) = (*state).clipboard.take() else {
            return;
        };
        if job.close_after {
            PostMessageW(hwnd, WM_CLOSE, 0, 0);
            return;
        }
        let Ok(result) = job.result.try_recv() else {
            return;
        };
        let label = match result {
            Ok(ClipboardOutcome::Copied(false)) => (*state)
                .label(
                    "Files copied to the clipboard",
                    "Arquivos copiados para a área de transferência",
                )
                .to_owned(),
            Ok(ClipboardOutcome::Copied(true)) => (*state)
                .label(
                    "Files cut to the clipboard",
                    "Arquivos recortados para a área de transferência",
                )
                .to_owned(),
            Ok(ClipboardOutcome::Pasted(TransferOutcome::Complete)) => (*state)
                .label("Transfer complete", "Transferência concluída")
                .to_owned(),
            Ok(ClipboardOutcome::Pasted(TransferOutcome::Interrupted)) => (*state)
                .label(
                    "Transfer interrupted. Check the source and destination.",
                    "Transferência interrompida. Verifique a origem e o destino.",
                )
                .to_owned(),
            Ok(ClipboardOutcome::Recycled(RecycleOutcome { error: None, .. })) => (*state)
                .label("Moved to the Recycle Bin", "Movido para a Lixeira")
                .to_owned(),
            Ok(ClipboardOutcome::Recycled(RecycleOutcome {
                recycled,
                error: Some(error),
            })) => format!(
                "{recycled} {} · {error}",
                (*state).label(
                    "moved to the Recycle Bin before stopping",
                    "movidos para a Lixeira antes da interrupção"
                )
            ),
            Err(error) => format!(
                "{} · {error}",
                (*state).label(
                    "Could not complete the file operation",
                    "Não foi possível concluir a operação de arquivos"
                )
            ),
        };
        if pasted && (*state).requested.is_none() {
            refresh(hwnd, state);
            if (*state).requested.is_some() {
                (*state).transfer_status = Some(label);
                return;
            }
        }
        set_text(hwnd, STATUS, &label);
    }
}

unsafe fn refresh_git(hwnd: HWND, state: *mut State) {
    unsafe {
        (*state).git_revision += 1;
        (*state).git = None;
        InvalidateRect(GetDlgItem(hwnd, LIST as i32), std::ptr::null(), 0);
        if (*state).git_enabled {
            start_git(hwnd, state);
        }
    }
}

unsafe fn start_git(hwnd: HWND, state: *mut State) {
    unsafe {
        let Some(folder) = (*state).navigation.current().map(Path::to_path_buf) else {
            set_text(
                hwnd,
                GIT_LABEL,
                (*state).label("Git · choose a folder", "Git · escolha uma pasta"),
            );
            return;
        };
        set_text(
            hwnd,
            GIT_LABEL,
            (*state).label("Checking Git…", "Consultando Git…"),
        );
        if (*state).git_job.is_some() {
            return;
        }
        let target = hwnd as usize;
        let (sender, result) = mpsc::channel();
        match thread::Builder::new()
            .name("monolith-files-git".into())
            .spawn(move || {
                if sender.send(file_git::inspect(&folder)).is_ok() {
                    PostMessageW(target as HWND, GIT_READY, 0, 0);
                }
            }) {
            Ok(_) => {
                (*state).git_job = Some(GitJob {
                    revision: (*state).git_revision,
                    result,
                })
            }
            Err(_) => set_text(
                hwnd,
                GIT_LABEL,
                (*state).label("Git status unavailable", "Status do Git indisponível"),
            ),
        }
    }
}

unsafe fn finish_git(hwnd: HWND, state: *mut State) {
    unsafe {
        let Some(job) = (*state).git_job.take() else {
            return;
        };
        if !(*state).git_enabled {
            return;
        }
        if job.revision != (*state).git_revision {
            start_git(hwnd, state);
            return;
        }
        let label = match job.result.try_recv() {
            Ok(Ok(Some(repository))) => {
                let mut label = match &repository.branch {
                    Branch::Named {
                        name,
                        unborn: false,
                    } => format!("Git · {name}"),
                    Branch::Named { name, unborn: true } => format!(
                        "Git · {name} · {}",
                        (*state).label("no commits yet", "sem commits")
                    ),
                    Branch::Detached { oid } => format!(
                        "Git · {} {}",
                        (*state).label("detached at", "HEAD destacado em"),
                        oid.get(..8).unwrap_or(oid)
                    ),
                };
                if let Some(upstream) = &repository.upstream {
                    use std::fmt::Write;
                    let _ = write!(label, " · {}", upstream.name);
                    if let Some((ahead, behind)) = upstream.ahead_behind {
                        let _ = write!(
                            label,
                            " · {ahead} {} · {behind} {}",
                            (*state).label("ahead", "à frente"),
                            (*state).label("behind", "atrás")
                        );
                    }
                }
                (*state).git = Some(repository);
                label
            }
            Ok(Ok(None)) => (*state)
                .label("Git · no repository", "Git · sem repositório")
                .to_owned(),
            _ => (*state)
                .label("Git status unavailable", "Status do Git indisponível")
                .to_owned(),
        };
        set_text(hwnd, GIT_LABEL, &label);
        InvalidateRect(GetDlgItem(hwnd, LIST as i32), std::ptr::null(), 0);
    }
}

fn git_item_text(state: &State, item: Option<ItemStatus>) -> String {
    let Some(item) = item else {
        return String::new();
    };
    [
        (item.conflicted, state.label("Conflicted", "Conflito")),
        (item.staged, state.label("Staged", "Preparado")),
        (item.modified, state.label("Modified", "Modificado")),
        (item.untracked, state.label("Untracked", "Não rastreado")),
    ]
    .into_iter()
    .filter_map(|(present, label)| present.then_some(label))
    .collect::<Vec<_>>()
    .join(", ")
}

unsafe fn cell_text(state: *mut State, info: &mut NMLVDISPINFOW) {
    unsafe {
        let item = &mut info.item;
        if item.mask & LVIF_TEXT == 0 || item.pszText.is_null() || item.cchTextMax < 1 {
            return;
        }
        let entries = &(*state).listing.entries;
        let git_text;
        let content = match (item.iSubItem, entries.get(item.iItem as usize)) {
            (0, Some(entry)) => entry.display(),
            (1, Some(entry)) if entry.directory => (*state).label("Folder", "Pasta"),
            (1, Some(_)) => (*state).label("File", "Arquivo"),
            (2, Some(entry)) => {
                git_text = git_item_text(
                    &*state,
                    (*state)
                        .git
                        .as_ref()
                        .and_then(|repository| repository.item(&entry.name)),
                );
                &git_text
            }
            _ => "",
        };
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
            CLIPBOARD_READY => {
                finish_clipboard(hwnd, state, wp != 0);
                0
            }
            file_launch::READY => {
                if let Some(receive) = (*state).external.take()
                    && let Ok(Err(error)) = receive.try_recv()
                {
                    launch_error(hwnd, state, &error);
                }
                0
            }
            GIT_READY => {
                finish_git(hwnd, state);
                0
            }
            file_changes::READY => {
                finish_change(hwnd, state);
                0
            }
            WM_CONTEXTMENU if wp as HWND == GetDlgItem(hwnd, LIST as i32) => {
                list_menu(hwnd, state, wp as HWND, lp);
                0
            }
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
                if let Some(job) = (*state).clipboard.as_mut() {
                    job.close_after = true;
                    cancel_clipboard(hwnd, state);
                    return 0;
                }
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
