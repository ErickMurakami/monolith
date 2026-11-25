//! Shared palette and geometry for native shell overlays.

use crate::config::Theme;
use std::mem::size_of;
use windows_sys::Win32::{
    Foundation::RECT,
    Graphics::Gdi::*,
    UI::{
        Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW},
        Controls::{DRAWITEMSTRUCT, ODS_DISABLED, ODS_FOCUS, ODS_SELECTED},
        WindowsAndMessaging::*,
    },
};

pub const STYLE: u32 = WS_POPUP;
pub const EX_STYLE: u32 = WS_EX_TOOLWINDOW;

#[derive(Clone, Copy)]
pub struct Palette {
    pub background: u32,
    pub surface: u32,
    pub pressed: u32,
    pub text: u32,
    pub accent: u32,
}

impl Palette {
    pub fn new(theme: Theme) -> Self {
        let mut contrast = HIGHCONTRASTW {
            cbSize: size_of::<HIGHCONTRASTW>() as u32,
            ..Default::default()
        };
        unsafe {
            if SystemParametersInfoW(
                SPI_GETHIGHCONTRAST,
                contrast.cbSize,
                (&raw mut contrast).cast(),
                0,
            ) != 0
                && contrast.dwFlags & HCF_HIGHCONTRASTON != 0
            {
                return Self {
                    background: GetSysColor(COLOR_WINDOW),
                    surface: GetSysColor(COLOR_WINDOW),
                    pressed: GetSysColor(COLOR_BTNFACE),
                    text: GetSysColor(COLOR_WINDOWTEXT),
                    accent: GetSysColor(COLOR_HIGHLIGHT),
                };
            }
        }
        match theme {
            Theme::Dark => Self {
                background: rgb(8, 10, 13),
                surface: rgb(20, 24, 32),
                pressed: rgb(32, 36, 44),
                text: rgb(229, 231, 235),
                accent: rgb(41, 121, 255),
            },
            Theme::Light => Self {
                background: rgb(241, 243, 246),
                surface: rgb(255, 255, 255),
                pressed: rgb(224, 229, 235),
                text: rgb(24, 28, 34),
                accent: rgb(41, 121, 255),
            },
        }
    }
}

const fn rgb(r: u32, g: u32, b: u32) -> u32 {
    r | (g << 8) | (b << 16)
}

pub fn scale(dip: i32, dpi: u32) -> i32 {
    (dip * dpi.max(96) as i32 + 48) / 96
}

pub unsafe fn fill(hdc: HDC, bounds: &RECT, colour: u32) {
    unsafe {
        SetDCBrushColor(hdc, colour);
        FillRect(hdc, bounds, GetStockObject(DC_BRUSH) as HBRUSH);
    }
}

pub unsafe fn paint_button(item: &DRAWITEMSTRUCT, palette: Palette, font: HFONT) {
    unsafe { paint_button_at(item, palette, font, item.rcItem) };
}

pub unsafe fn paint_button_at(
    item: &DRAWITEMSTRUCT,
    palette: Palette,
    font: HFONT,
    mut text_area: RECT,
) {
    unsafe {
        let saved = SaveDC(item.hDC);
        if saved == 0 {
            return;
        }
        let pressed = item.itemState & ODS_SELECTED != 0;
        let disabled = item.itemState & ODS_DISABLED != 0;
        let colour = if pressed {
            palette.pressed
        } else {
            palette.surface
        };
        fill(item.hDC, &item.rcItem, colour);
        if !font.is_null() {
            SelectObject(item.hDC, font);
        }
        SetBkMode(item.hDC, TRANSPARENT as i32);
        SetTextColor(
            item.hDC,
            if disabled {
                GetSysColor(COLOR_GRAYTEXT)
            } else {
                palette.text
            },
        );
        let mut title = [0_u16; 64];
        let count = GetWindowTextW(item.hwndItem, title.as_mut_ptr(), title.len() as i32);
        DrawTextW(
            item.hDC,
            title.as_mut_ptr(),
            count,
            &mut text_area,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX | DT_END_ELLIPSIS,
        );
        if item.itemState & ODS_FOCUS != 0 {
            let mut bounds = item.rcItem;
            InflateRect(&mut bounds, -3, -3);
            DrawFocusRect(item.hDC, &bounds);
        }
        RestoreDC(item.hDC, saved);
    }
}

pub fn spotlight_bounds(work: RECT, dpi: u32) -> RECT {
    let work_width = (work.right - work.left).max(1);
    let work_height = (work.bottom - work.top).max(1);
    let margin = scale(16, dpi);
    let width = scale(580, dpi).min((work_width - 2 * margin).max(1));
    let height = scale(314, dpi).min((work_height - 2 * margin).max(1));
    let left = work.left + (work_width - width) / 2;
    let top = work.top + (work_height / 5).min(work_height - height);
    RECT {
        left,
        top,
        right: left + width,
        bottom: top + height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions_spotlight_on_an_offset_high_dpi_display() {
        let work = RECT {
            left: -1920,
            top: -100,
            right: 0,
            bottom: 980,
        };
        let bounds = spotlight_bounds(work, 144);
        assert_eq!(bounds.right - bounds.left, 870);
        assert_eq!(bounds.left + bounds.right, -1920);
        assert_eq!(bounds.top, 116);
    }

    #[test]
    fn keeps_spotlight_inside_a_small_work_area() {
        let work = RECT {
            left: 400,
            top: 200,
            right: 760,
            bottom: 540,
        };
        let bounds = spotlight_bounds(work, 192);
        assert!(bounds.left >= work.left && bounds.right <= work.right);
        assert!(bounds.top >= work.top && bounds.bottom <= work.bottom);
    }
}
