//! Bounded asynchronous Shell icon discovery and native bitmap drawing.

use std::{
    collections::{HashMap, HashSet},
    io,
    mem::size_of,
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
    ptr::null_mut,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread,
};
use windows_sys::Win32::{
    Foundation::{HWND, RECT},
    Graphics::Gdi::*,
    System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize},
    UI::{
        Shell::{SHFILEINFOW, SHGFI_ICON, SHGetFileInfoW},
        WindowsAndMessaging::{DI_NORMAL, DestroyIcon, DrawIconEx, PostMessageW, WM_APP},
    },
};

pub const READY: u32 = WM_APP + 4;
const SIDE: i32 = 32;
const PIXELS: usize = (SIDE * SIDE) as usize;
const LIMIT: usize = 32;

struct Icon(Box<[u32; PIXELS]>);
type IconResult = (PathBuf, Option<Icon>);

pub struct Cache {
    jobs: SyncSender<PathBuf>,
    results: Receiver<IconResult>,
    alive: Arc<AtomicBool>,
    pending: HashSet<PathBuf>,
    entries: HashMap<PathBuf, Option<Icon>>,
}

impl Cache {
    pub fn new(owner: HWND) -> io::Result<Self> {
        let (jobs, work) = mpsc::sync_channel::<PathBuf>(8);
        let (complete, results) = mpsc::sync_channel::<IconResult>(16);
        let alive = Arc::new(AtomicBool::new(true));
        let worker_active = Arc::clone(&alive);
        let receiver = owner as isize;
        thread::Builder::new()
            .name("monolith-shell-icons".into())
            .spawn(move || {
                // Shell handlers can block. Keep all extraction and COM off the GUI thread.
                let apartment =
                    unsafe { CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32) };
                if apartment < 0 {
                    return;
                }
                while worker_active.load(Ordering::Acquire) {
                    let Ok(path) = work.recv() else { break };
                    let icon = extract(&path);
                    if !worker_active.load(Ordering::Acquire)
                        || complete.send((path, icon)).is_err()
                    {
                        break;
                    }
                    unsafe { PostMessageW(receiver as HWND, READY, 0, 0) };
                }
                unsafe { CoUninitialize() };
            })?;
        Ok(Self {
            jobs,
            results,
            alive,
            pending: HashSet::new(),
            entries: HashMap::new(),
        })
    }

    pub fn request(&mut self, visible: &[PathBuf]) {
        if self.entries.len() >= LIMIT {
            self.entries.retain(|path, _| visible.contains(path));
        }
        for path in visible {
            if self.pending.len() < 16
                && !self.entries.contains_key(path)
                && !self.pending.contains(path)
                && self.jobs.try_send(path.clone()).is_ok()
            {
                self.pending.insert(path.clone());
            }
        }
    }

    pub fn receive(&mut self) -> bool {
        let mut changed = false;
        while let Ok((path, icon)) = self.results.try_recv() {
            self.pending.remove(&path);
            self.entries.insert(path, icon);
            changed = true;
        }
        changed
    }

    pub fn draw(
        &self,
        path: &Path,
        dc: HDC,
        target: RECT,
        size: i32,
        fill: u32,
        original: bool,
    ) -> bool {
        let Some(Some(icon)) = self.entries.get(path) else {
            return false;
        };
        if size <= 0 {
            return false;
        }
        let mut pixels = [0_u32; PIXELS];
        for (output, &pixel) in pixels.iter_mut().zip(icon.0.iter()) {
            *output = composite(pixel, fill, original);
        }
        unsafe {
            StretchDIBits(
                dc,
                target.left + (target.right - target.left - size) / 2,
                target.top + (target.bottom - target.top - size) / 2,
                size,
                size,
                0,
                0,
                SIDE,
                SIDE,
                pixels.as_ptr().cast(),
                &bitmap_info(),
                DIB_RGB_COLORS,
                SRCCOPY,
            ) > 0
        }
    }
}

impl Drop for Cache {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::Release);
    }
}

fn bitmap_info() -> BITMAPINFO {
    BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: SIDE,
            biHeight: -SIDE,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..Default::default()
        },
        ..Default::default()
    }
}

fn extract(path: &Path) -> Option<Icon> {
    let path: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    let mut info = SHFILEINFOW::default();
    // SHGFI_ICON gives the caller ownership of the icon handle.
    unsafe {
        if SHGetFileInfoW(
            path.as_ptr(),
            0,
            &mut info,
            size_of::<SHFILEINFOW>() as u32,
            SHGFI_ICON,
        ) == 0
            || info.hIcon.is_null()
        {
            return None;
        }
        let icon = rasterise(info.hIcon);
        DestroyIcon(info.hIcon);
        icon
    }
}

unsafe fn rasterise(icon: windows_sys::Win32::UI::WindowsAndMessaging::HICON) -> Option<Icon> {
    unsafe {
        let dc = CreateCompatibleDC(null_mut());
        if dc.is_null() {
            return None;
        }
        let mut data = null_mut();
        let bitmap = CreateDIBSection(dc, &bitmap_info(), DIB_RGB_COLORS, &mut data, null_mut(), 0);
        let old = if bitmap.is_null() {
            null_mut()
        } else {
            SelectObject(dc, bitmap)
        };
        let pixels = if !old.is_null() && !data.is_null() {
            let data = std::slice::from_raw_parts_mut(data.cast::<u32>(), PIXELS);
            data.fill(0);
            let first = DrawIconEx(dc, 0, 0, icon, SIDE, SIDE, 0, null_mut(), DI_NORMAL);
            GdiFlush();
            let mut black = Box::new([0; PIXELS]);
            black.copy_from_slice(data);
            data.fill(0x00ff_ffff);
            let second = DrawIconEx(dc, 0, 0, icon, SIDE, SIDE, 0, null_mut(), DI_NORMAL);
            GdiFlush();
            for (colour, &white) in black.iter_mut().zip(data.iter()) {
                // Comparing black and white backdrops also handles legacy mask icons.
                let background = [0, 8, 16]
                    .into_iter()
                    .map(|shift| channel(white, shift).saturating_sub(channel(*colour, shift)))
                    .max()
                    .unwrap_or(0);
                *colour = (*colour & 0x00ff_ffff) | ((255 - background) << 24);
            }
            SelectObject(dc, old);
            (first != 0 && second != 0 && black.iter().any(|pixel| pixel >> 24 != 0))
                .then_some(Icon(black))
        } else {
            None
        };
        DeleteObject(bitmap);
        DeleteDC(dc);
        pixels
    }
}

fn channel(pixel: u32, shift: u32) -> u32 {
    (pixel >> shift) & 255
}

fn composite(pixel: u32, fill: u32, original: bool) -> u32 {
    let alpha = pixel >> 24;
    let grey =
        (2126 * channel(pixel, 16) + 7152 * channel(pixel, 8) + 722 * channel(pixel, 0)) / 10_000;
    let grey = grey.clamp(alpha * 65 / 255, alpha * 205 / 255);
    let blend = |source: u32, background: u32| (source + background * (255 - alpha) / 255).min(255);
    let ink = |shift| {
        if original {
            channel(pixel, shift)
        } else {
            grey
        }
    };
    blend(ink(0), channel(fill, 16))
        | (blend(ink(8), channel(fill, 8)) << 8)
        | (blend(ink(16), channel(fill, 0)) << 16)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_transparency_and_converts_colours_to_greyscale() {
        assert_eq!(composite(0, 0x00332211, false), 0x00112233);
        let pixel = composite(0xffff_0000, 0, false);
        assert_eq!(channel(pixel, 0), channel(pixel, 8));
        assert_eq!(channel(pixel, 8), channel(pixel, 16));
        assert_eq!(composite(0xffff_0000, 0, true), 0x00ff0000);
    }
}
