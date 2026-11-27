use crate::{
    audio::{self, Audio, Volume},
    brightness::{self, Brightness},
    overlay::{self, Palette},
    radios::{self, Radios},
    windows::{report_for, wide},
};
use std::{ffi::c_void, io, mem::size_of, ptr::null_mut};
use windows_sys::Win32::{
    Foundation::{ERROR_CLASS_ALREADY_EXISTS, HWND, LPARAM, LRESULT, RECT, WPARAM},
    Graphics::Gdi::*,
    System::{
        Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS},
        SystemServices::SS_CENTERIMAGE,
    },
    UI::{
        Controls::*, HiDpi::GetDpiForWindow, Input::KeyboardAndMouse::*, Shell::ShellExecuteW,
        WindowsAndMessaging::*,
    },
};

pub const OPEN: u32 = WM_APP + 6;
pub const CLOSE: u32 = WM_APP + 7;

const VOLUME_LABEL: usize = 1;
const SLIDER: usize = 2;
const MUTE: usize = 3;
const SETTINGS_LABEL: usize = 4;
const BRIGHTNESS_LABEL: usize = 5;
const BRIGHTNESS_SLIDER: usize = 6;
const FIRST_SETTING: usize = 10;
// The SDK defines this trackbar message as WM_USER + 0.
const TBM_GETPOS: u32 = WM_USER;
const SETTINGS: [(&str, &str, &str); 8] = [
    ("Wi-Fi", "Wi-Fi", "ms-settings:network-wifi"),
    ("Bluetooth", "Bluetooth", "ms-settings:bluetooth"),
    ("Display", "Tela", "ms-settings:display"),
    ("Night light", "Luz noturna", "ms-settings:nightlight"),
    ("Sound devices", "Dispositivos de som", "ms-settings:sound"),
    ("Focus", "Foco", "ms-settings:quiethours"),
    ("Power", "Energia", "ms-settings:powersleep"),
    ("All settings", "Todas as configurações", "ms-settings:"),
];

pub struct ControlCenter {
    hwnd: HWND,
    previous: HWND,
    state: Box<State>,
}

struct State {
    launcher: HWND,
    audio: Option<Audio>,
    volume: Option<Volume>,
    drag_endpoint: Option<u64>,
    brightness: Option<Brightness>,
    radios: Option<Radios>,
    /// The internal panel level, once the worker reports one.
    brightness_level: Option<u8>,
    palette: Palette,
    portuguese: bool,
    font: HFONT,
    dpi: u32,
    background: HBRUSH,
}

fn label(portuguese: bool, english: &'static str, translated: &'static str) -> &'static str {
    if portuguese { translated } else { english }
}

impl ControlCenter {
    pub fn new(
        instance: *mut c_void,
        launcher: HWND,
        source: HWND,
        palette: Palette,
        portuguese: bool,
    ) -> io::Result<Self> {
        unsafe {
            let previous = GetForegroundWindow();
            let name = wide("MonolithControlCenter");
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
            let controls = INITCOMMONCONTROLSEX {
                dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
                dwICC: ICC_BAR_CLASSES,
            };
            if InitCommonControlsEx(&controls) == 0 {
                return Err(io::Error::last_os_error());
            }
            let hwnd = CreateWindowExW(
                overlay::EX_STYLE | WS_EX_TOPMOST,
                name.as_ptr(),
                wide(label(
                    portuguese,
                    "Monolith Control Center",
                    "Central de Controle do Monolith",
                ))
                .as_ptr(),
                overlay::STYLE | WS_CLIPCHILDREN,
                0,
                0,
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
            let mut panel = Self {
                hwnd,
                previous,
                state: Box::new(State {
                    launcher,
                    audio: None,
                    volume: None,
                    drag_endpoint: None,
                    brightness: None,
                    radios: None,
                    brightness_level: None,
                    palette,
                    portuguese,
                    font: null_mut(),
                    dpi: 0,
                    background: CreateSolidBrush(palette.background),
                }),
            };
            if panel.state.background.is_null() {
                return Err(io::Error::last_os_error());
            }
            let child = |class, title: &str, styles, id| {
                let window = CreateWindowExW(
                    0,
                    class,
                    wide(title).as_ptr(),
                    WS_CHILD | WS_VISIBLE | styles,
                    0,
                    0,
                    0,
                    0,
                    hwnd,
                    id as *mut c_void,
                    instance,
                    null_mut(),
                );
                if window.is_null() {
                    Err(io::Error::last_os_error())
                } else {
                    Ok(window)
                }
            };
            let static_class = wide("STATIC");
            let button_class = wide("BUTTON");
            let volume = label(portuguese, "Volume", "Volume");
            child(static_class.as_ptr(), volume, SS_CENTERIMAGE, VOLUME_LABEL)?;
            let slider = child(
                TRACKBAR_CLASSW,
                volume,
                TBS_NOTICKS | WS_TABSTOP | WS_DISABLED,
                SLIDER,
            )?;
            SendMessageW(slider, TBM_SETRANGEMAX, 0, 100);
            SendMessageW(slider, TBM_SETLINESIZE, 0, 1);
            child(
                button_class.as_ptr(),
                label(portuguese, "Mute", "Silenciar"),
                BS_OWNERDRAW as u32 | WS_TABSTOP | WS_DISABLED,
                MUTE,
            )?;
            // Hidden until the worker confirms an internal panel exposes brightness.
            let brightness_name = label(portuguese, "Brightness", "Brilho");
            let brightness_label = child(
                static_class.as_ptr(),
                brightness_name,
                SS_CENTERIMAGE,
                BRIGHTNESS_LABEL,
            )?;
            let brightness_slider = child(
                TRACKBAR_CLASSW,
                brightness_name,
                TBS_NOTICKS | WS_TABSTOP,
                BRIGHTNESS_SLIDER,
            )?;
            SendMessageW(brightness_slider, TBM_SETRANGEMAX, 0, 100);
            SendMessageW(brightness_slider, TBM_SETLINESIZE, 0, 1);
            ShowWindow(brightness_label, SW_HIDE);
            ShowWindow(brightness_slider, SW_HIDE);
            child(
                static_class.as_ptr(),
                label(portuguese, "Windows settings", "Configurações do Windows"),
                SS_CENTERIMAGE,
                SETTINGS_LABEL,
            )?;
            for (index, (english, translated, _)) in SETTINGS.iter().enumerate() {
                child(
                    button_class.as_ptr(),
                    label(portuguese, english, translated),
                    BS_OWNERDRAW as u32 | WS_TABSTOP,
                    FIRST_SETTING + index,
                )?;
            }
            let state = &raw mut *panel.state;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, state as isize);
            let anchor = if source.is_null() { previous } else { source };
            let monitor = if anchor.is_null() {
                let mut pointer = Default::default();
                GetCursorPos(&mut pointer);
                MonitorFromPoint(pointer, MONITOR_DEFAULTTONEAREST)
            } else {
                MonitorFromWindow(anchor, MONITOR_DEFAULTTONEAREST)
            };
            place(hwnd, state, monitor);
            update_power(hwnd, portuguese);
            (*state).audio = Audio::start(hwnd).ok();
            (*state).brightness = Brightness::start(hwnd).ok();
            (*state).radios = Radios::start(hwnd).ok();
            if (*state).audio.is_none() {
                refresh(hwnd, state, None);
            }
            ShowWindow(hwnd, SW_SHOW);
            SetForegroundWindow(hwnd);
            SetFocus(hwnd);
            Ok(panel)
        }
    }

    pub fn handle(&self) -> HWND {
        self.hwnd
    }

    pub fn focus(&self) {
        unsafe { SetForegroundWindow(self.hwnd) };
    }

    pub fn restore_focus(&self) {
        unsafe {
            if GetForegroundWindow() == self.hwnd
                && IsWindowVisible(self.previous) != 0
                && IsIconic(self.previous) == 0
            {
                SetForegroundWindow(self.previous);
            }
        }
    }

    pub fn handle_key(&self, msg: &MSG) -> bool {
        unsafe {
            if msg.message != WM_KEYDOWN
                || (msg.hwnd != self.hwnd && IsChild(self.hwnd, msg.hwnd) == 0)
            {
                return false;
            }
            match msg.wParam as u16 {
                VK_ESCAPE => {
                    PostMessageW(self.state.launcher, CLOSE, self.hwnd as usize, 1);
                    true
                }
                VK_TAB => {
                    let start = if msg.hwnd == self.hwnd {
                        null_mut()
                    } else {
                        msg.hwnd
                    };
                    let backwards = i32::from(GetKeyState(i32::from(VK_SHIFT)) < 0);
                    SetFocus(GetNextDlgTabItem(self.hwnd, start, backwards));
                    true
                }
                VK_RETURN
                    if msg.hwnd != self.hwnd
                        && msg.hwnd != GetDlgItem(self.hwnd, SLIDER as i32) =>
                {
                    SendMessageW(msg.hwnd, BM_CLICK, 0, 0);
                    true
                }
                _ => false,
            }
        }
    }
}

impl Drop for ControlCenter {
    fn drop(&mut self) {
        drop(self.state.audio.take());
        unsafe {
            SetWindowLongPtrW(self.hwnd, GWLP_USERDATA, 0);
            DestroyWindow(self.hwnd);
            DeleteObject(self.state.background);
            DeleteObject(self.state.font);
        }
    }
}

unsafe fn place(hwnd: HWND, state: *mut State, monitor: HMONITOR) {
    unsafe {
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if GetMonitorInfoW(monitor, &mut info) == 0 {
            return;
        }
        let work = info.rcWork;
        if monitor != MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) {
            // Position the hidden panel on the target before reading its DPI.
            SetWindowPos(
                hwnd,
                null_mut(),
                work.left,
                work.top,
                0,
                0,
                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
        let dpi = GetDpiForWindow(hwnd);
        let scale = |dip| overlay::scale(dip, dpi);
        let margin = scale(8);
        let width = scale(324).min((work.right - work.left - 2 * margin).max(1));
        // A brightness row shifts the settings section down.
        let shift = if (*state).brightness_level.is_some() {
            60
        } else {
            0
        };
        let height = scale(248 + shift);
        let x = (work.right - width - margin).max(work.left);
        let y = (work.top + scale(36))
            .min(work.bottom - height)
            .max(work.top);
        SetWindowPos(
            hwnd,
            null_mut(),
            x,
            y,
            width,
            height,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );

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
        let padding = scale(12);
        let body = (width - 2 * padding).max(1);
        let gap = scale(4);
        let half = ((body - gap) / 2).max(1);
        let layout = |id: usize, left, top, width: i32, height| {
            let child = GetDlgItem(hwnd, id as i32);
            if !font.is_null() {
                SendMessageW(child, WM_SETFONT, font as usize, 0);
            }
            MoveWindow(child, left, scale(top), width.max(1), scale(height), 1);
        };
        layout(VOLUME_LABEL, padding, 9, body, 22);
        let mute_width = scale(82).min(half);
        layout(SLIDER, padding, 33, body - mute_width - gap, 29);
        layout(MUTE, padding + body - mute_width, 33, mute_width, 29);
        if shift > 0 {
            layout(BRIGHTNESS_LABEL, padding, 69, body, 22);
            layout(BRIGHTNESS_SLIDER, padding, 93, body, 29);
        }
        layout(SETTINGS_LABEL, padding, 72 + shift, body, 25);
        for index in 0..SETTINGS.len() {
            let left = padding + (index as i32 % 2) * (half + gap);
            layout(
                FIRST_SETTING + index,
                left,
                100 + shift + index as i32 / 2 * 35,
                half,
                31,
            );
        }
        if !font.is_null() {
            DeleteObject((*state).font);
            (*state).font = font;
            (*state).dpi = dpi;
        }
    }
}

unsafe fn update_power(hwnd: HWND, portuguese: bool) {
    unsafe {
        let mut power = SYSTEM_POWER_STATUS::default();
        let name = label(portuguese, "Power", "Energia");
        let caption = if GetSystemPowerStatus(&mut power) != 0 && power.BatteryLifePercent <= 100 {
            format!("{name} · {}%", power.BatteryLifePercent)
        } else {
            name.to_owned()
        };
        SetWindowTextW(
            GetDlgItem(hwnd, (FIRST_SETTING + 6) as i32),
            wide(&caption).as_ptr(),
        );
    }
}

unsafe fn refresh(hwnd: HWND, state: *mut State, volume: Option<Volume>) {
    unsafe {
        let slider = GetDlgItem(hwnd, SLIDER as i32);
        let mute = GetDlgItem(hwnd, MUTE as i32);
        if let Some(Volume {
            endpoint,
            level,
            muted,
        }) = volume
        {
            let dragging = GetCapture() == slider && (*state).drag_endpoint == Some(endpoint);
            if !dragging {
                SendMessageW(slider, TBM_SETPOS, 1, usize::from(level) as isize);
                SetWindowTextW(
                    GetDlgItem(hwnd, VOLUME_LABEL as i32),
                    wide(&format!("Volume · {level}%")).as_ptr(),
                );
            }
            let text = if muted {
                label((*state).portuguese, "Unmute", "Reativar som")
            } else {
                label((*state).portuguese, "Mute", "Silenciar")
            };
            SetWindowTextW(mute, wide(text).as_ptr());
        } else {
            let text = label(
                (*state).portuguese,
                "Audio output unavailable",
                "Saída de áudio indisponível",
            );
            SetWindowTextW(GetDlgItem(hwnd, VOLUME_LABEL as i32), wide(text).as_ptr());
        }
        (*state).volume = volume;
        EnableWindow(slider, i32::from(volume.is_some()));
        EnableWindow(mute, i32::from(volume.is_some()));
    }
}

unsafe fn change_volume(hwnd: HWND, state: *mut State, wp: WPARAM, slider: HWND) {
    unsafe {
        let action = wp as u16 as u32;
        if action == TB_ENDTRACK {
            (*state).drag_endpoint = None;
            return;
        }
        let Some(volume) = (*state).volume else {
            return;
        };
        let level = SendMessageW(slider, TBM_GETPOS, 0, 0).clamp(0, 100) as u8;
        if action == TB_THUMBTRACK {
            if (*state).drag_endpoint.is_none() {
                (*state).drag_endpoint = Some(volume.endpoint);
            }
            SetWindowTextW(
                GetDlgItem(hwnd, VOLUME_LABEL as i32),
                wide(&format!("Volume · {level}%")).as_ptr(),
            );
            return;
        }
        let endpoint = (*state).drag_endpoint.unwrap_or(volume.endpoint);
        if action == TB_THUMBPOSITION {
            (*state).drag_endpoint = None;
        }
        if endpoint == volume.endpoint {
            if let Some(audio) = (*state).audio.as_ref() {
                audio.set_level(endpoint, level);
            }
        } else {
            refresh(hwnd, state, Some(volume));
        }
    }
}

unsafe fn paint_slider(hwnd: HWND, draw: &NMCUSTOMDRAW, palette: Palette) {
    unsafe {
        let slider = draw.hdr.hwndFrom;
        let mut channel = RECT::default();
        let mut thumb = RECT::default();
        SendMessageW(slider, TBM_GETCHANNELRECT, 0, &raw mut channel as isize);
        SendMessageW(slider, TBM_GETTHUMBRECT, 0, &raw mut thumb as isize);
        overlay::fill(draw.hdc, &draw.rc, palette.background);
        let middle = (thumb.top + thumb.bottom) / 2;
        let thickness = overlay::scale(4, GetDpiForWindow(hwnd));
        let track = RECT {
            top: middle - thickness / 2,
            bottom: middle - thickness / 2 + thickness,
            ..channel
        };
        overlay::fill(draw.hdc, &track, palette.pressed);
        if IsWindowEnabled(slider) != 0 {
            let position = (thumb.left + thumb.right) / 2;
            overlay::fill(
                draw.hdc,
                &RECT {
                    right: position,
                    ..track
                },
                palette.accent,
            );
            let size = overlay::scale(10, GetDpiForWindow(hwnd));
            let thumb = RECT {
                left: position - size / 2,
                top: middle - size / 2,
                right: position - size / 2 + size,
                bottom: middle - size / 2 + size,
            };
            overlay::fill(draw.hdc, &thumb, palette.text);
            if GetFocus() == slider {
                DrawFocusRect(draw.hdc, &draw.rc);
            }
        }
    }
}

/// The radio behind a settings button: Wi-Fi and Bluetooth are the first two.
fn radio_for(id: usize) -> Option<radios::Kind> {
    match id.checked_sub(FIRST_SETTING)? {
        0 => Some(radios::Kind::WiFi),
        1 => Some(radios::Kind::Bluetooth),
        _ => None,
    }
}

fn radio_caption(name: &str, state: Option<bool>, portuguese: bool) -> String {
    match (state, portuguese) {
        (Some(true), false) => format!("{name} · On"),
        (Some(false), false) => format!("{name} · Off"),
        (Some(true), true) => format!("{name} · Ligado"),
        (Some(false), true) => format!("{name} · Desligado"),
        (None, _) => name.to_owned(),
    }
}

unsafe fn update_radios(hwnd: HWND, state: *mut State) {
    unsafe {
        let Some(radios) = &(*state).radios else {
            return;
        };
        let states = radios.states();
        for (index, (english, translated, _)) in SETTINGS.iter().take(2).enumerate() {
            let id = FIRST_SETTING + index;
            let name = label((*state).portuguese, english, translated);
            let kind = radio_for(id).expect("first two settings are radios");
            let caption = radio_caption(name, states.get(kind), (*state).portuguese);
            let button = GetDlgItem(hwnd, id as i32);
            SetWindowTextW(button, wide(&caption).as_ptr());
            InvalidateRect(button, std::ptr::null(), 1);
        }
    }
}

unsafe fn set_brightness_label(hwnd: HWND, state: *mut State, level: u8) {
    unsafe {
        let name = label((*state).portuguese, "Brightness", "Brilho");
        SetWindowTextW(
            GetDlgItem(hwnd, BRIGHTNESS_LABEL as i32),
            wide(&format!("{name} · {level}%")).as_ptr(),
        );
    }
}

/// Apply a level reported by the worker, revealing the row the first time.
unsafe fn show_brightness(hwnd: HWND, state: *mut State, reported: WPARAM) {
    unsafe {
        if reported == brightness::UNAVAILABLE {
            return;
        }
        let level = reported.min(100) as u8;
        let first = (*state).brightness_level.is_none();
        (*state).brightness_level = Some(level);
        let label_window = GetDlgItem(hwnd, BRIGHTNESS_LABEL as i32);
        let slider = GetDlgItem(hwnd, BRIGHTNESS_SLIDER as i32);
        set_brightness_label(hwnd, state, level);
        // Do not move the thumb under a drag in progress.
        if GetCapture() != slider {
            SendMessageW(slider, TBM_SETPOS, 1, level as isize);
        }
        if first {
            for child in [label_window, slider] {
                SendMessageW(child, WM_SETFONT, (*state).font as usize, 0);
            }
            place(
                hwnd,
                state,
                MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST),
            );
            ShowWindow(label_window, SW_SHOW);
            ShowWindow(slider, SW_SHOW);
            // The panel grew; erase stale pixels left by the moved settings section.
            RedrawWindow(
                hwnd,
                std::ptr::null(),
                null_mut(),
                RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN,
            );
        } else {
            InvalidateRect(slider, std::ptr::null(), 1);
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
            audio::READY => {
                let volume = (*state).audio.as_ref().and_then(Audio::take_update);
                refresh(hwnd, state, volume);
                0
            }
            brightness::READY => {
                show_brightness(hwnd, state, wp);
                0
            }
            radios::READY => {
                update_radios(hwnd, state);
                0
            }
            WM_CLOSE | WM_ACTIVATE if msg == WM_CLOSE || wp as u16 == WA_INACTIVE as u16 => {
                PostMessageW(
                    (*state).launcher,
                    CLOSE,
                    hwnd as usize,
                    isize::from(msg == WM_CLOSE),
                );
                0
            }
            WM_DPICHANGED | WM_DISPLAYCHANGE => {
                let monitor = if msg == WM_DPICHANGED {
                    MonitorFromRect(lp as *const RECT, MONITOR_DEFAULTTONEAREST)
                } else {
                    MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST)
                };
                place(hwnd, state, monitor);
                0
            }
            WM_POWERBROADCAST => {
                update_power(hwnd, (*state).portuguese);
                1
            }
            WM_COMMAND if (wp >> 16) as u16 == BN_CLICKED as u16 && lp != 0 => {
                let id = wp as u16 as usize;
                if id == MUTE {
                    if let (Some(audio), Some(volume)) = ((*state).audio.as_ref(), (*state).volume)
                    {
                        audio.set_mute(volume.endpoint, !volume.muted);
                    }
                } else if let Some(kind) = radio_for(id).filter(|kind| {
                    (*state)
                        .radios
                        .as_ref()
                        .is_some_and(|radios| radios.states().get(*kind).is_some())
                }) {
                    // A direct control; the Settings page remains the fallback below.
                    if let Some(radios) = &(*state).radios {
                        // Show the requested state at once; the StateChanged event
                        // confirms it, or restores the real state if Windows refuses.
                        let requested = radios.states().get(kind).map(|on| !on);
                        radios.toggle(kind);
                        let index = id - FIRST_SETTING;
                        let (english, translated, _) = SETTINGS[index];
                        let name = label((*state).portuguese, english, translated);
                        let caption = format!(
                            "{}  \u{2026}",
                            radio_caption(name, requested, (*state).portuguese)
                        );
                        SetWindowTextW(lp as HWND, wide(&caption).as_ptr());
                        InvalidateRect(lp as HWND, std::ptr::null(), 1);
                    }
                } else if let Some((_, _, uri)) = id
                    .checked_sub(FIRST_SETTING)
                    .and_then(|index| SETTINGS.get(index))
                {
                    let result = ShellExecuteW(
                        hwnd,
                        wide("open").as_ptr(),
                        wide(uri).as_ptr(),
                        std::ptr::null(),
                        std::ptr::null(),
                        SW_SHOWNORMAL,
                    );
                    if result as isize > 32 {
                        PostMessageW((*state).launcher, CLOSE, hwnd as usize, 2);
                    } else {
                        report_for(
                            hwnd,
                            label(
                                (*state).portuguese,
                                "Could not open Windows settings.",
                                "Não foi possível abrir as configurações do Windows.",
                            ),
                        );
                    }
                }
                0
            }
            WM_HSCROLL if lp as HWND == GetDlgItem(hwnd, SLIDER as i32) => {
                change_volume(hwnd, state, wp, lp as HWND);
                0
            }
            WM_HSCROLL if lp as HWND == GetDlgItem(hwnd, BRIGHTNESS_SLIDER as i32) => {
                let level = SendMessageW(lp as HWND, TBM_GETPOS, 0, 0).clamp(0, 100) as u8;
                set_brightness_label(hwnd, state, level);
                InvalidateRect(lp as HWND, std::ptr::null(), 1);
                if let Some(brightness) = &(*state).brightness {
                    brightness.set(level);
                }
                0
            }
            WM_NOTIFY if lp != 0 => {
                let header = &*(lp as *const NMHDR);
                if matches!(header.idFrom, SLIDER | BRIGHTNESS_SLIDER)
                    && header.code == NM_CUSTOMDRAW
                {
                    let draw = &*(lp as *const NMCUSTOMDRAW);
                    if draw.dwDrawStage == CDDS_PREPAINT {
                        paint_slider(hwnd, draw, (*state).palette);
                        return CDRF_SKIPDEFAULT as LRESULT;
                    }
                }
                DefWindowProcW(hwnd, msg, wp, lp)
            }
            WM_DRAWITEM if lp != 0 => {
                overlay::paint_button(
                    &*(lp as *const DRAWITEMSTRUCT),
                    (*state).palette,
                    (*state).font,
                );
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
