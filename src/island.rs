//! Borderless control surface for real Windows media sessions.

use crate::{
    desktop,
    media::{self, Action, Media, NowPlaying, Repeat},
    overlay::{self, Palette},
    windows::{shell_menu, wide},
};
use std::{cell::Cell, ffi::c_void, io, mem::size_of, ptr::null_mut};
use windows_sys::Win32::{
    Foundation::{FILETIME, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM},
    Graphics::Gdi::*,
    System::{
        SystemInformation::{GetSystemTimeAsFileTime, GetTickCount64},
        SystemServices::{SS_NOTIFY, SS_OWNERDRAW},
    },
    UI::{
        Accessibility::*,
        Controls::*,
        HiDpi::GetDpiForWindow,
        Input::KeyboardAndMouse::EnableWindow,
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::*,
    },
};

const LABEL: usize = 1;
const PREVIOUS: usize = 2;
const TOGGLE: usize = 3;
const NEXT: usize = 4;
const ARTWORK: usize = 5;
const SHUFFLE: usize = 6;
const REPEAT: usize = 7;
const TIME: usize = 8;
const REPOSITION: usize = 1;
const PROGRESS: usize = 2;
const ANIMATE: usize = 3;
const ANIMATION_MS: u64 = 180;

#[derive(Clone, Copy)]
struct Animation {
    from: i32,
    to: i32,
    started: u64,
}

thread_local! {
    static TARGET: Cell<HWND> = const { Cell::new(null_mut()) };
    static PENDING: Cell<bool> = const { Cell::new(false) };
}

/// Fonts for the painted Island, recreated when the DPI changes.
struct Fonts {
    body: HFONT,
    strong: HFONT,
    title: HFONT,
    small: HFONT,
    glyphs: HFONT,
    large_glyphs: HFONT,
    /// Solid transport symbols, which Segoe Fluent only offers as outlines.
    symbols: HFONT,
    large_symbols: HFONT,
}

impl Default for Fonts {
    fn default() -> Self {
        Self {
            body: null_mut(),
            strong: null_mut(),
            title: null_mut(),
            small: null_mut(),
            glyphs: null_mut(),
            large_glyphs: null_mut(),
            symbols: null_mut(),
            large_symbols: null_mut(),
        }
    }
}

impl Fonts {
    fn release(&mut self) {
        for font in [
            self.body,
            self.strong,
            self.title,
            self.small,
            self.glyphs,
            self.large_glyphs,
            self.symbols,
            self.large_symbols,
        ] {
            unsafe { DeleteObject(font) };
        }
        *self = Self::default();
    }
}

pub struct Island {
    hwnd: HWND,
    hooks: Vec<HWINEVENTHOOK>,
    state: Box<State>,
}

struct State {
    launcher: HWND,
    host: HWND,
    controls: [HWND; 4],
    progress: HWND,
    artwork: HWND,
    fonts: Fonts,
    dpi: u32,
    brush: HBRUSH,
    palette: Palette,
    portuguese: bool,
    media: Option<Media>,
    current: Option<NowPlaying>,
    expanded: bool,
    /// Shuffle and repeat, shown only when the session supports them.
    extras: [HWND; 2],
    /// Elapsed and total time beside the progress bar.
    time: HWND,
    animation: Option<Animation>,
}

impl Island {
    pub fn new(
        instance: *mut c_void,
        launcher: HWND,
        host: HWND,
        palette: Palette,
        portuguese: bool,
    ) -> io::Result<Self> {
        // Only this GUI thread owns the window and its native controls.
        unsafe {
            let class_name = wide("MonolithIsland");
            let class = WNDCLASSW {
                hInstance: instance,
                lpfnWndProc: Some(window_proc),
                hCursor: LoadCursorW(null_mut(), IDC_ARROW),
                lpszClassName: class_name.as_ptr(),
                ..Default::default()
            };
            if RegisterClassW(&class) == 0 {
                return Err(io::Error::last_os_error());
            }
            let hwnd = CreateWindowExW(
                overlay::EX_STYLE | WS_EX_TOPMOST | WS_EX_NOACTIVATE,
                class_name.as_ptr(),
                wide("Monolith Island").as_ptr(),
                overlay::STYLE | WS_CLIPCHILDREN,
                0,
                0,
                0,
                0,
                host,
                null_mut(),
                instance,
                null_mut(),
            );
            if hwnd.is_null() {
                return Err(io::Error::last_os_error());
            }
            let mut island = Self {
                hwnd,
                hooks: Vec::new(),
                state: Box::new(State {
                    launcher,
                    host,
                    controls: [null_mut(); 4],
                    progress: null_mut(),
                    artwork: null_mut(),
                    fonts: Fonts::default(),
                    dpi: 0,
                    brush: CreateSolidBrush(palette.background),
                    palette,
                    portuguese,
                    media: None,
                    current: None,
                    expanded: false,
                    extras: [null_mut(); 2],
                    time: null_mut(),
                    animation: None,
                }),
            };
            if island.state.brush.is_null() {
                return Err(io::Error::last_os_error());
            }
            island.state.progress = progress_control(hwnd, instance, &raw mut *island.state);
            island.state.artwork = CreateWindowExW(
                0,
                wide("STATIC").as_ptr(),
                wide("Album artwork").as_ptr(),
                WS_CHILD | SS_OWNERDRAW,
                0,
                0,
                0,
                0,
                hwnd,
                ARTWORK as *mut c_void,
                instance,
                null_mut(),
            );
            for (index, control) in island.state.controls.iter_mut().enumerate() {
                let (class, style) = if index == 0 {
                    ("STATIC", SS_NOTIFY | SS_OWNERDRAW)
                } else {
                    ("BUTTON", BS_OWNERDRAW as u32)
                };
                *control = CreateWindowExW(
                    0,
                    wide(class).as_ptr(),
                    std::ptr::null(),
                    WS_CHILD | style,
                    0,
                    0,
                    0,
                    0,
                    hwnd,
                    (index + 1) as *mut c_void,
                    instance,
                    null_mut(),
                );
                if control.is_null() {
                    return Err(io::Error::last_os_error());
                }
            }
            for (id, class, style) in [
                (SHUFFLE, "BUTTON", BS_OWNERDRAW as u32),
                (REPEAT, "BUTTON", BS_OWNERDRAW as u32),
                (TIME, "STATIC", SS_OWNERDRAW),
            ] {
                let child = CreateWindowExW(
                    0,
                    wide(class).as_ptr(),
                    std::ptr::null(),
                    WS_CHILD | style,
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
                    return Err(io::Error::last_os_error());
                }
                match id {
                    SHUFFLE => island.state.extras[0] = child,
                    REPEAT => island.state.extras[1] = child,
                    _ => island.state.time = child,
                }
            }
            // Rounded corners, so the expanded panel reads as one floating surface.
            let corner = windows_sys::Win32::Graphics::Dwm::DWMWCP_ROUND;
            windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute(
                hwnd,
                windows_sys::Win32::Graphics::Dwm::DWMWA_WINDOW_CORNER_PREFERENCE as u32,
                (&raw const corner).cast(),
                size_of::<i32>() as u32,
            );
            island.state.media = Some(Media::connect(hwnd)?);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, (&raw mut *island.state) as isize);
            TARGET.set(hwnd);
            for event in [EVENT_SYSTEM_FOREGROUND, EVENT_OBJECT_LOCATIONCHANGE] {
                let hook = SetWinEventHook(
                    event,
                    event,
                    null_mut(),
                    Some(foreground_event),
                    0,
                    0,
                    WINEVENT_OUTOFCONTEXT,
                );
                if hook.is_null() {
                    return Err(io::Error::last_os_error());
                }
                island.hooks.push(hook);
            }
            Ok(island)
        }
    }
}

unsafe fn progress_control(hwnd: HWND, instance: *mut c_void, state: *mut State) -> HWND {
    unsafe {
        let controls = INITCOMMONCONTROLSEX {
            dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_PROGRESS_CLASS,
        };
        if InitCommonControlsEx(&controls) == 0 {
            return null_mut();
        }
        let progress = CreateWindowExW(
            0,
            PROGRESS_CLASSW,
            wide("Playback progress").as_ptr(),
            WS_CHILD | PBS_SMOOTH,
            0,
            0,
            0,
            0,
            hwnd,
            null_mut(),
            instance,
            null_mut(),
        );
        if !progress.is_null() {
            if SetWindowSubclass(progress, Some(progress_proc), 1, state as usize) == 0 {
                DestroyWindow(progress);
                return null_mut();
            }
            SetWindowLongW(
                progress,
                GWL_STYLE,
                GetWindowLongW(progress, GWL_STYLE) & !(WS_BORDER as i32),
            );
            SetWindowLongW(
                progress,
                GWL_EXSTYLE,
                GetWindowLongW(progress, GWL_EXSTYLE)
                    & !((WS_EX_STATICEDGE | WS_EX_CLIENTEDGE | WS_EX_WINDOWEDGE) as i32),
            );
            SetWindowPos(
                progress,
                null_mut(),
                0,
                0,
                0,
                0,
                SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
            SendMessageW(progress, PBM_SETRANGE32, 0, 1000);
        }
        progress
    }
}

unsafe extern "system" fn progress_proc(
    hwnd: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
    _: usize,
    reference: usize,
) -> LRESULT {
    // The Island owns this state until it destroys all child windows.
    unsafe {
        match msg {
            WM_ERASEBKGND => return 1,
            WM_NCPAINT | WM_NCCALCSIZE => return 0,
            WM_LBUTTONUP => {
                let state = reference as *const State;
                if let (Some(media), Some(current)) =
                    ((*state).media.as_ref(), (*state).current.as_ref())
                    && current.seek
                {
                    let mut bounds = RECT::default();
                    GetClientRect(hwnd, &mut bounds);
                    let x = (lp & 0xffff) as i16 as i32;
                    let width = (bounds.right - bounds.left).max(1);
                    let permille = (x.clamp(0, width) * 1000 / width) as u32;
                    media.perform(current.generation, Action::Seek(permille));
                }
                return 0;
            }
            WM_PAINT => {
                let palette = (*(reference as *const State)).palette;
                let mut paint = PAINTSTRUCT::default();
                let dc = BeginPaint(hwnd, &mut paint);
                let mut bounds = RECT::default();
                GetClientRect(hwnd, &mut bounds);
                let fraction = DefSubclassProc(hwnd, PBM_GETPOS, 0, 0).clamp(0, 1000) as i32;
                overlay::fill(dc, &bounds, palette.background);
                // A rounded track centred in the taller hit area, with a knob at
                // the playback position, inset so the knob never clips.
                let dpi = GetDpiForWindow(hwnd);
                let thickness = overlay::scale(4, dpi);
                let knob = overlay::scale(12, dpi);
                let middle = (bounds.top + bounds.bottom) / 2;
                let track = RECT {
                    left: bounds.left + knob / 2,
                    top: middle - thickness / 2,
                    right: bounds.right - knob / 2,
                    bottom: middle - thickness / 2 + thickness,
                };
                let track_colour = overlay::mix(palette.background, palette.text, 18);
                overlay::rounded(dc, &track, thickness / 2, track_colour, None);
                let position = track.left + (track.right - track.left) * fraction / 1000;
                let played = RECT {
                    right: position.max(track.left + thickness),
                    ..track
                };
                overlay::rounded(dc, &played, thickness / 2, palette.accent, None);
                let handle = RECT {
                    left: position - knob / 2,
                    top: middle - knob / 2,
                    right: position - knob / 2 + knob,
                    bottom: middle - knob / 2 + knob,
                };
                overlay::rounded(dc, &handle, knob / 2, palette.text, None);
                EndPaint(hwnd, &paint);
                return 0;
            }
            WM_NCDESTROY => {
                RemoveWindowSubclass(hwnd, Some(progress_proc), 1);
            }
            _ => {}
        }
        DefSubclassProc(hwnd, msg, wp, lp)
    }
}

impl Drop for Island {
    fn drop(&mut self) {
        TARGET.set(null_mut());
        PENDING.set(false);
        drop(self.state.media.take());
        unsafe {
            for hook in self.hooks.drain(..) {
                UnhookWinEvent(hook);
            }
            DestroyWindow(self.hwnd);
            DeleteObject(self.state.brush);
            self.state.fonts.release();
        }
    }
}

unsafe extern "system" fn foreground_event(
    _: HWINEVENTHOOK,
    _: u32,
    hwnd: HWND,
    object: i32,
    child: i32,
    _: u32,
    _: u32,
) {
    if child == 0
        && object == OBJID_WINDOW
        && !hwnd.is_null()
        && unsafe { GetForegroundWindow() } == hwnd
    {
        schedule(TARGET.get());
    }
}

fn schedule(hwnd: HWND) {
    if !hwnd.is_null() && !PENDING.get() && unsafe { SetTimer(hwnd, REPOSITION, 100, None) } != 0 {
        PENDING.set(true);
    }
}

pub fn host_changed(host: HWND) {
    let target = TARGET.get();
    if !target.is_null() && unsafe { GetWindow(target, GW_OWNER) } == host {
        schedule(target);
    }
}

unsafe fn layout(hwnd: HWND, state: *mut State) {
    unsafe {
        KillTimer(hwnd, PROGRESS);
        let Some(current) = (*state).current.as_ref() else {
            if let Some(media) = (*state).media.as_ref() {
                media.request_artwork(false);
            }
            ShowWindow(hwnd, SW_HIDE);
            return;
        };
        let monitor = MonitorFromPoint(POINT::default(), MONITOR_DEFAULTTOPRIMARY);
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let host = (*state).host;
        if GetMonitorInfoW(monitor, &mut info) == 0
            || if host.is_null() {
                desktop::foreground_covers_monitor(monitor, &info.rcMonitor)
            } else {
                IsWindowVisible(host) == 0 || crate::top_bar::collapsed()
            }
        {
            if let Some(media) = (*state).media.as_ref() {
                media.request_artwork(false);
            }
            ShowWindow(hwnd, SW_HIDE);
            return;
        }
        // Copy the presentation before calling APIs that can send window messages.
        let expanded = (*state).expanded;
        let artwork_view = (*state).artwork;
        if let Some(media) = (*state).media.as_ref() {
            media.request_artwork(!artwork_view.is_null());
        }
        let show_artwork = current.artwork.is_some() && !artwork_view.is_null();
        let title = match (current.title.is_empty(), current.artist.is_empty()) {
            (true, _) => current.artist.clone(),
            (_, true) => current.title.clone(),
            _ => format!("{}   ·   {}", current.title, current.artist),
        };
        let title = if expanded && !current.album.is_empty() {
            format!("{title}   ·   {}", current.album)
        } else {
            title
        };
        let shuffle = current.shuffle;
        let repeat = current.repeat;
        let extras = (*state).extras;
        let time = (*state).time;
        let playing = current.playing;
        let show_progress = expanded && current.timeline.is_some() && !(*state).progress.is_null();
        let availability = [current.previous, current.toggle, current.next];
        let controls = (*state).controls;
        let dpi = GetDpiForWindow(hwnd);
        let scale = |value| overlay::scale(value, dpi);
        if (*state).dpi != dpi {
            let create = |size, weight: u32, face: &str| {
                CreateFontW(
                    -scale(size),
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
                    CLEARTYPE_QUALITY as u32,
                    0,
                    wide(face).as_ptr(),
                )
            };
            let fonts = Fonts {
                body: create(13, FW_NORMAL, "Segoe UI"),
                strong: create(13, FW_SEMIBOLD, "Segoe UI"),
                title: create(16, FW_SEMIBOLD, "Segoe UI"),
                small: create(12, FW_NORMAL, "Segoe UI"),
                glyphs: create(16, FW_NORMAL, "Segoe Fluent Icons"),
                large_glyphs: create(20, FW_NORMAL, "Segoe Fluent Icons"),
                symbols: create(17, FW_NORMAL, "Segoe UI Symbol"),
                large_symbols: create(19, FW_NORMAL, "Segoe UI Symbol"),
            };
            for child in controls.into_iter().chain(extras).chain([time]) {
                SendMessageW(child, WM_SETFONT, fonts.body as usize, 0);
            }
            (*state).fonts.release();
            (*state).fonts = fonts;
            (*state).dpi = dpi;
        }
        let work = info.rcWork;
        let work_width = (work.right - work.left).max(1);
        // Both states share one width so expanding only animates the height.
        let width = scale(372).min(work_width.saturating_sub(scale(16))).max(1);
        let band = scale(27);
        let inset = scale(16);
        let art = scale(64);
        let row_top = inset + art + scale(if show_progress { 48 } else { 16 });
        let height = if expanded {
            row_top + scale(44) + scale(12)
        } else {
            band
        };
        let left = work.left + (work_width - width) / 2;
        let mut top = work.top + scale(4);
        let host = (*state).host;
        if !host.is_null() {
            let mut rect = RECT::default();
            if GetWindowRect(host, &mut rect) != 0 {
                top = if work_width < scale(780) {
                    rect.bottom + scale(4)
                } else {
                    rect.top
                };
            }
        }
        let place = |child: HWND, x: i32, y: i32, w: i32, h: i32| {
            SetWindowPos(
                child,
                null_mut(),
                x,
                y,
                w.max(1),
                h.max(1),
                SWP_SHOWWINDOW | SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOCOPYBITS,
            );
            InvalidateRect(child, std::ptr::null(), 0);
        };
        SetWindowTextW(controls[0], wide(&title).as_ptr());
        let thumb = scale(19);
        let label_left = if !show_artwork {
            if expanded { inset } else { scale(10) }
        } else if expanded {
            place(artwork_view, inset, inset, art, art);
            inset + art + scale(14)
        } else {
            place(artwork_view, scale(6), (band - thumb) / 2, thumb, thumb);
            scale(6) + thumb + scale(8)
        };
        if !show_artwork && !artwork_view.is_null() {
            ShowWindow(artwork_view, SW_HIDE);
        }
        if expanded {
            place(
                controls[0],
                label_left,
                inset,
                width - label_left - inset,
                art,
            );
        } else {
            place(
                controls[0],
                label_left,
                0,
                width - label_left - band - scale(6),
                band,
            );
        }
        let portuguese = (*state).portuguese;
        let captions = match (portuguese, playing) {
            (false, true) => ["Previous", "Pause", "Next"],
            (false, false) => ["Previous", "Play", "Next"],
            (true, true) => ["Anterior", "Pausar", "Próxima"],
            (true, false) => ["Anterior", "Reproduzir", "Próxima"],
        };
        for (index, &button) in controls.iter().skip(1).enumerate() {
            SetWindowTextW(button, wide(captions[index]).as_ptr());
            EnableWindow(button, i32::from(availability[index]));
        }
        // Transport sits in five fixed slots so it stays centred whether or not
        // shuffle and repeat are supported.
        let slot = (width - 2 * inset) / 5;
        let in_slot = |index: i32, side: i32| {
            (
                inset + slot * index + (slot - side) / 2,
                row_top + (scale(44) - side) / 2,
            )
        };
        if expanded {
            for (index, &button) in controls.iter().skip(1).enumerate() {
                let side = scale(if index == 1 { 44 } else { 40 });
                let (x, y) = in_slot(index as i32 + 1, side);
                place(button, x, y, side, side);
            }
        } else {
            ShowWindow(controls[1], SW_HIDE);
            ShowWindow(controls[3], SW_HIDE);
            place(controls[2], width - band - scale(6), 0, band, band);
        }
        let shuffle_caption = shuffle.map(|active| match (portuguese, active) {
            (false, false) => "Shuffle",
            (false, true) => "Shuffle ●",
            (true, false) => "Aleatório",
            (true, true) => "Aleatório ●",
        });
        let repeat_caption = repeat.map(|mode| match (portuguese, mode) {
            (false, Repeat::Off) => "Repeat",
            (false, Repeat::List) => "Repeat all",
            (false, Repeat::Track) => "Repeat one",
            (true, Repeat::Off) => "Repetir",
            (true, Repeat::List) => "Repetir tudo",
            (true, Repeat::Track) => "Repetir uma",
        });
        for (button, caption, index) in [
            (extras[0], shuffle_caption, 0),
            (extras[1], repeat_caption, 4),
        ] {
            match caption.filter(|_| expanded) {
                Some(caption) => {
                    SetWindowTextW(button, wide(caption).as_ptr());
                    let side = scale(40);
                    let (x, y) = in_slot(index, side);
                    place(button, x, y, side, side);
                }
                None => {
                    ShowWindow(button, SW_HIDE);
                }
            }
        }
        let progress = (*state).progress;
        if show_progress {
            // A taller hit area than the drawn bar makes seeking practical.
            place(
                progress,
                inset,
                inset + art + scale(12),
                width - 2 * inset,
                scale(14),
            );
            place(
                time,
                inset,
                inset + art + scale(26),
                width - 2 * inset,
                scale(16),
            );
        } else {
            if !progress.is_null() {
                ShowWindow(progress, SW_HIDE);
            }
            ShowWindow(time, SW_HIDE);
        }
        // Expanding or collapsing in place animates the height; everything else snaps.
        let mut previous = RECT::default();
        let start_height = (IsWindowVisible(hwnd) != 0
            && GetWindowRect(hwnd, &mut previous) != 0
            && (previous.left, previous.top, previous.right - previous.left) == (left, top, width)
            && previous.bottom - previous.top != height
            && overlay::animations_enabled())
        .then_some(previous.bottom - previous.top);
        SetWindowPos(
            hwnd,
            HWND_TOPMOST,
            left,
            top,
            width,
            start_height.unwrap_or(height),
            SWP_SHOWWINDOW | SWP_NOACTIVATE,
        );
        match start_height {
            Some(from) => {
                (*state).animation = Some(Animation {
                    from,
                    to: height,
                    started: GetTickCount64(),
                });
                SetTimer(hwnd, ANIMATE, 15, None);
            }
            None => {
                KillTimer(hwnd, ANIMATE);
                (*state).animation = None;
            }
        }
        InvalidateRect(hwnd, std::ptr::null(), 1);
        if show_progress {
            update_progress(hwnd, state);
        }
    }
}

/// Ease-out cubic height between `from` and `to` after `elapsed` milliseconds.
fn animated_height(from: i32, to: i32, elapsed: u64) -> (i32, bool) {
    if elapsed >= ANIMATION_MS {
        return (to, true);
    }
    let progress = elapsed as f64 / ANIMATION_MS as f64;
    let eased = 1.0 - (1.0 - progress).powi(3);
    (from + ((to - from) as f64 * eased).round() as i32, false)
}

unsafe fn step_animation(hwnd: HWND, state: *mut State) {
    unsafe {
        let Some(animation) = (*state).animation else {
            KillTimer(hwnd, ANIMATE);
            return;
        };
        let (height, finished) = animated_height(
            animation.from,
            animation.to,
            GetTickCount64().saturating_sub(animation.started),
        );
        let mut bounds = RECT::default();
        GetWindowRect(hwnd, &mut bounds);
        SetWindowPos(
            hwnd,
            null_mut(),
            0,
            0,
            bounds.right - bounds.left,
            height,
            SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
        if finished {
            // No timer remains once the Island is stationary.
            KillTimer(hwnd, ANIMATE);
            (*state).animation = None;
        }
    }
}

unsafe fn update_progress(hwnd: HWND, state: *mut State) {
    unsafe {
        KillTimer(hwnd, PROGRESS);
        let bar = (*state).progress;
        if IsWindowVisible(bar) == 0 {
            return;
        }
        let Some(current) = (*state).current.as_ref() else {
            return;
        };
        let Some(timeline) = current.timeline else {
            return;
        };
        let mut timestamp = FILETIME::default();
        GetSystemTimeAsFileTime(&mut timestamp);
        let now = (i64::from(timestamp.dwHighDateTime) << 32) | i64::from(timestamp.dwLowDateTime);
        let (position, continues) = timeline.position_at(now, current.playing);
        let (elapsed, total) = timeline.seconds_at(now, current.playing);
        let text = format!("{} / {}", media::clock(elapsed), media::clock(total));
        SetWindowTextW((*state).time, wide(&text).as_ptr());
        SendMessageW(bar, PBM_SETPOS, position as usize, 0);
        InvalidateRect(bar, std::ptr::null(), 0);
        if continues {
            SetTimer(hwnd, PROGRESS, 1000, None);
        }
    }
}

/// Paint the Island's controls: artwork with rounded corners, the title block,
/// elapsed and total time, and monochrome transport glyphs. Window texts stay as
/// the controls' accessible names.
unsafe fn draw_item(state: &State, item: &DRAWITEMSTRUCT) {
    unsafe {
        let dc = item.hDC;
        let rect = item.rcItem;
        let palette = state.palette;
        let fonts = &state.fonts;
        let dpi = state.dpi;
        let scale = |value| overlay::scale(value, dpi);
        let secondary = overlay::mix(palette.text, palette.background, 35);
        let muted = overlay::mix(palette.text, palette.background, 55);
        overlay::fill(dc, &rect, palette.background);
        SetBkMode(dc, TRANSPARENT as i32);
        let Some(current) = state.current.as_ref() else {
            return;
        };
        let line = |value: &str, area: RECT, font, colour, format| {
            overlay::text(dc, value, area, font, colour, format)
        };
        match item.CtlID as usize {
            ARTWORK => {
                let Some(artwork) = current.artwork.as_ref() else {
                    return;
                };
                let radius = scale(if state.expanded { 12 } else { 5 });
                let region = CreateRoundRectRgn(
                    rect.left,
                    rect.top,
                    rect.right + 1,
                    rect.bottom + 1,
                    2 * radius,
                    2 * radius,
                );
                SelectClipRgn(dc, region);
                artwork.draw(dc, rect, palette.background);
                SelectClipRgn(dc, null_mut());
                DeleteObject(region);
            }
            LABEL if state.expanded => {
                let rows = [
                    (current.title.as_str(), fonts.title, palette.text, scale(24)),
                    (current.artist.as_str(), fonts.body, secondary, scale(20)),
                    (current.album.as_str(), fonts.body, muted, scale(20)),
                ];
                let filled: Vec<_> = rows.iter().filter(|row| !row.0.is_empty()).collect();
                let total: i32 = filled.iter().map(|row| row.3).sum();
                let mut top = rect.top + (rect.bottom - rect.top - total) / 2;
                for (value, font, colour, height) in filled {
                    let area = RECT {
                        top,
                        bottom: top + height,
                        ..rect
                    };
                    line(value, area, *font, *colour, DT_LEFT);
                    top += height;
                }
            }
            LABEL => {
                // Compact: the title in semibold, then the artist in a quieter tone.
                let title: Vec<u16> = current.title.encode_utf16().collect();
                SelectObject(dc, fonts.strong);
                let mut size = SIZE::default();
                GetTextExtentPoint32W(dc, title.as_ptr(), title.len() as i32, &mut size);
                let width = rect.right - rect.left;
                let title_width = if current.artist.is_empty() {
                    width
                } else {
                    size.cx.min(width * 2 / 3)
                };
                line(
                    &current.title,
                    RECT {
                        right: rect.left + title_width,
                        ..rect
                    },
                    fonts.strong,
                    palette.text,
                    DT_LEFT,
                );
                if !current.artist.is_empty() {
                    line(
                        &current.artist,
                        RECT {
                            left: rect.left + title_width + scale(8),
                            ..rect
                        },
                        fonts.body,
                        secondary,
                        DT_LEFT,
                    );
                }
            }
            TIME => {
                let mut buffer = [0_u16; 64];
                let length =
                    GetWindowTextW(item.hwndItem, buffer.as_mut_ptr(), buffer.len() as i32);
                let value = String::from_utf16_lossy(&buffer[..length.max(0) as usize]);
                let (elapsed, total) = value.split_once(" / ").unwrap_or((&value, ""));
                line(elapsed, rect, fonts.small, secondary, DT_LEFT);
                line(total, rect, fonts.small, secondary, DT_RIGHT);
            }
            id => {
                let disabled = item.itemState & ODS_DISABLED != 0;
                let pressed = item.itemState & ODS_SELECTED != 0;
                let (glyph, active) = match id {
                    PREVIOUS => ("\u{23EE}", false),
                    NEXT => ("\u{23ED}", false),
                    TOGGLE if current.playing => ("\u{23F8}", false),
                    TOGGLE => ("\u{25B6}", false),
                    SHUFFLE => ("\u{E8B1}", current.shuffle == Some(true)),
                    REPEAT if current.repeat == Some(Repeat::Track) => ("\u{E8ED}", true),
                    REPEAT => ("\u{E8EE}", current.repeat == Some(Repeat::List)),
                    _ => return,
                };
                let prominent = id == TOGGLE && state.expanded;
                if prominent || pressed {
                    let side = (rect.right - rect.left).min(rect.bottom - rect.top);
                    let circle = RECT {
                        left: rect.left + (rect.right - rect.left - side) / 2,
                        top: rect.top + (rect.bottom - rect.top - side) / 2,
                        right: rect.left + (rect.right - rect.left + side) / 2,
                        bottom: rect.top + (rect.bottom - rect.top + side) / 2,
                    };
                    let colour = if pressed {
                        palette.pressed
                    } else {
                        palette.surface
                    };
                    overlay::rounded(dc, &circle, side / 2, colour, None);
                }
                let colour = if disabled {
                    muted
                } else if active {
                    palette.accent
                } else {
                    palette.text
                };
                let transport = matches!(id, PREVIOUS | NEXT | TOGGLE);
                let font = match (transport, prominent) {
                    (true, true) => fonts.large_symbols,
                    (true, false) => fonts.symbols,
                    (false, _) => fonts.glyphs,
                };
                line(glyph, rect, font, colour, DT_CENTER);
            }
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
            media::READY => {
                (*state).current = (*state).media.as_ref().and_then(Media::read);
                if (*state).current.is_none() {
                    (*state).expanded = false;
                }
                layout(hwnd, state);
                0
            }
            WM_COMMAND if wp >> 16 == 0 => {
                let id = wp & 0xffff;
                if id == LABEL {
                    (*state).expanded = !(*state).expanded;
                    layout(hwnd, state);
                } else {
                    let action = match id {
                        PREVIOUS => Action::Previous,
                        TOGGLE => Action::Toggle,
                        NEXT => Action::Next,
                        SHUFFLE => Action::Shuffle,
                        REPEAT => Action::Repeat,
                        _ => return 0,
                    };
                    if let (Some(media), Some(current)) =
                        ((*state).media.as_ref(), (*state).current.as_ref())
                    {
                        media.perform(current.generation, action);
                    }
                }
                0
            }
            WM_CONTEXTMENU => {
                shell_menu(hwnd, (*state).launcher, (*state).portuguese);
                0
            }
            WM_CLOSE => {
                PostMessageW((*state).launcher, WM_CLOSE, 0, 0);
                0
            }
            WM_TIMER if wp == REPOSITION => {
                KillTimer(hwnd, REPOSITION);
                PENDING.set(false);
                layout(hwnd, state);
                0
            }
            WM_TIMER if wp == ANIMATE => {
                step_animation(hwnd, state);
                0
            }
            WM_TIMER if wp == PROGRESS => {
                update_progress(hwnd, state);
                0
            }
            WM_DISPLAYCHANGE | WM_DPICHANGED | WM_SETTINGCHANGE => {
                schedule(hwnd);
                0
            }
            WM_ERASEBKGND => {
                let mut area = RECT::default();
                GetClientRect(hwnd, &mut area);
                FillRect(wp as HDC, &area, (*state).brush);
                1
            }
            WM_CTLCOLORSTATIC => {
                SetBkColor(wp as HDC, (*state).palette.background);
                SetBkMode(wp as HDC, OPAQUE as i32);
                SetTextColor(wp as HDC, (*state).palette.text);
                (*state).brush as isize
            }
            WM_DRAWITEM if lp != 0 => {
                draw_item(&*state, &*(lp as *const DRAWITEMSTRUCT));
                1
            }
            WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eases_out_and_finishes_at_the_target() {
        assert_eq!(animated_height(27, 100, 0), (27, false));
        let (middle, finished) = animated_height(27, 100, ANIMATION_MS / 2);
        // Ease-out covers most of the distance in the first half.
        assert!(!finished && middle > 27 + (100 - 27) / 2, "{middle}");
        assert_eq!(animated_height(27, 100, ANIMATION_MS), (100, true));
        assert_eq!(animated_height(100, 27, ANIMATION_MS * 2), (27, true));
        let (shrinking, _) = animated_height(100, 27, ANIMATION_MS / 2);
        assert!(shrinking < 100 - (100 - 27) / 2, "{shrinking}");
    }
}
