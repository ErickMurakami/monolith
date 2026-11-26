//! Windows file-list clipboard interchange with bounded validation.

use std::{
    ffi::{OsString, c_void},
    io,
    mem::size_of,
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::{Component, Path, PathBuf, Prefix},
    ptr::{self, null_mut},
    thread,
    time::Duration,
};
use windows_sys::{
    Win32::{
        Foundation::{GlobalFree, HGLOBAL, HWND},
        System::{
            DataExchange::{
                CloseClipboard, EmptyClipboard, GetClipboardData, GetClipboardOwner,
                GetClipboardSequenceNumber, OpenClipboard, RegisterClipboardFormatW,
                SetClipboardData,
            },
            Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock},
            Ole::{CF_HDROP, DROPEFFECT_COPY, DROPEFFECT_MOVE},
        },
        UI::Shell::DROPFILES,
    },
    core::w,
};

const LIMIT_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_PATHS: usize = 4096;

struct Clipboard;

impl Clipboard {
    fn open(window: HWND) -> io::Result<Self> {
        let mut retries = 5;
        loop {
            if unsafe { OpenClipboard(window) } != 0 {
                return Ok(Self);
            }
            let error = io::Error::last_os_error();
            if retries == 0 || error.kind() != io::ErrorKind::PermissionDenied {
                return Err(error);
            }
            // Other clipboard participants often take the lock briefly.
            retries -= 1;
            thread::sleep(Duration::from_millis(15));
        }
    }
}

impl Drop for Clipboard {
    fn drop(&mut self) {
        unsafe { CloseClipboard() };
    }
}

struct Block(HGLOBAL);

impl Block {
    fn new(bytes: usize, write: impl FnOnce(*mut c_void)) -> io::Result<Self> {
        let handle = unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        let block = Self(handle);
        let lock = LockedBlock::new(handle)?;
        write(lock.pointer);
        Ok(block)
    }

    fn publish(mut self, format: u32) -> io::Result<()> {
        if unsafe { SetClipboardData(format, self.0) }.is_null() {
            return Err(io::Error::last_os_error());
        }
        self.0 = null_mut();
        Ok(())
    }
}

impl Drop for Block {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { GlobalFree(self.0) };
        }
    }
}

struct LockedBlock {
    handle: HGLOBAL,
    pointer: *mut c_void,
}

impl LockedBlock {
    fn new(handle: HGLOBAL) -> io::Result<Self> {
        let pointer = unsafe { GlobalLock(handle) };
        if pointer.is_null() {
            Err(io::Error::last_os_error())
        } else {
            Ok(Self { handle, pointer })
        }
    }
}

impl Drop for LockedBlock {
    fn drop(&mut self) {
        unsafe { GlobalUnlock(self.handle) };
    }
}

fn preferred_effect() -> u32 {
    unsafe { RegisterClipboardFormatW(w!("Preferred DropEffect")) }
}

fn absolute_file(path: &Path) -> bool {
    path.is_absolute()
        && path.file_name().is_some()
        && matches!(path.components().next(), Some(Component::Prefix(prefix)) if matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_) | Prefix::UNC(..) | Prefix::VerbatimUNC(..)))
        && !path.components().any(|component| match component {
            Component::ParentDir => true,
            Component::Normal(name) => name.encode_wide().any(|unit| unit == u16::from(b':')),
            _ => false,
        })
}

fn malformed() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "Invalid or oversized clipboard file list",
    )
}

pub fn publish(window: HWND, paths: &[PathBuf], move_items: bool) -> io::Result<()> {
    if paths.is_empty() || paths.len() > MAX_PATHS {
        return Err(malformed());
    }
    let header = size_of::<DROPFILES>();
    let mut names = Vec::new();
    for path in paths {
        if !absolute_file(path) {
            return Err(malformed());
        }
        names.extend(path.as_os_str().encode_wide().chain([0]));
        if header + 2 * (names.len() + 1) > LIMIT_BYTES {
            return Err(malformed());
        }
    }
    names.push(0);
    let payload = Block::new(header + names.len() * 2, |pointer| unsafe {
        pointer.cast::<DROPFILES>().write(DROPFILES {
            pFiles: header as u32,
            fWide: 1,
            ..Default::default()
        });
        ptr::copy_nonoverlapping(
            names.as_ptr(),
            pointer.cast::<u8>().add(header).cast::<u16>(),
            names.len(),
        );
    })?;
    let format = preferred_effect();
    if format == 0 {
        return Err(io::Error::last_os_error());
    }
    let effect = if move_items {
        DROPEFFECT_MOVE
    } else {
        DROPEFFECT_COPY
    };
    let preferred = Block::new(size_of::<u32>(), |pointer| unsafe {
        pointer.cast::<u32>().write(effect)
    })?;
    let _clipboard = Clipboard::open(window)?;
    if unsafe { EmptyClipboard() } == 0 {
        return Err(io::Error::last_os_error());
    }
    preferred.publish(format)?;
    payload.publish(u32::from(CF_HDROP))
}

pub struct Selection {
    pub paths: Vec<PathBuf>,
    pub move_items: bool,
    pub own_cut_sequence: Option<u32>,
}

pub fn read(window: HWND) -> io::Result<Selection> {
    let _clipboard = Clipboard::open(window)?;
    let handle = unsafe { GetClipboardData(u32::from(CF_HDROP)) };
    if handle.is_null() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "Clipboard has no files",
        ));
    }
    let bytes = unsafe { GlobalSize(handle) };
    if !(size_of::<DROPFILES>()..=LIMIT_BYTES).contains(&bytes) {
        return Err(malformed());
    }
    let payload = LockedBlock::new(handle)?;
    let contents = unsafe { std::slice::from_raw_parts(payload.pointer.cast::<u8>(), bytes) };
    let paths = parse(contents)?;
    let format = preferred_effect();
    let move_items = format != 0
        && unsafe {
            let handle = GetClipboardData(format);
            !handle.is_null()
                && GlobalSize(handle) >= size_of::<u32>()
                && LockedBlock::new(handle)
                    .is_ok_and(|block| block.pointer.cast::<u32>().read() == DROPEFFECT_MOVE)
        };
    let own_cut_sequence = (move_items && unsafe { GetClipboardOwner() } == window)
        .then(|| unsafe { GetClipboardSequenceNumber() });
    Ok(Selection {
        paths,
        move_items,
        own_cut_sequence,
    })
}

pub fn clear_cut(window: HWND, sequence: u32) {
    if let Ok(_clipboard) = Clipboard::open(window) {
        unsafe {
            if GetClipboardSequenceNumber() == sequence && GetClipboardOwner() == window {
                EmptyClipboard();
            }
        }
    }
}

fn parse(bytes: &[u8]) -> io::Result<Vec<PathBuf>> {
    let header = size_of::<DROPFILES>();
    if bytes.len() < header {
        return Err(malformed());
    }
    let drop = unsafe { bytes.as_ptr().cast::<DROPFILES>().read_unaligned() };
    let start = drop.pFiles as usize;
    if drop.fWide == 0 || start < header || !start.is_multiple_of(2) {
        return Err(malformed());
    }
    let names = bytes
        .get(start..)
        .filter(|names| names.len() >= 4)
        .ok_or_else(malformed)?;
    let mut paths = Vec::new();
    let mut next = Vec::new();
    for &bytes in names.as_chunks::<2>().0 {
        let unit = u16::from_le_bytes(bytes);
        if unit == 0 {
            if next.is_empty() {
                return if paths.is_empty() {
                    Err(malformed())
                } else {
                    Ok(paths)
                };
            }
            let path = PathBuf::from(OsString::from_wide(&next));
            if !absolute_file(&path) || paths.len() == MAX_PATHS {
                return Err(malformed());
            }
            paths.push(path);
            next.clear();
        } else {
            if next.len() == 32_767 {
                return Err(malformed());
            }
            next.push(unit);
        }
    }
    Err(malformed())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(names: &str) -> Vec<u8> {
        let mut result = vec![0; size_of::<DROPFILES>()];
        result[..4].copy_from_slice(&(size_of::<DROPFILES>() as u32).to_le_bytes());
        result[size_of::<DROPFILES>() - 4] = 1;
        result.extend(names.encode_utf16().flat_map(u16::to_le_bytes));
        result
    }

    #[test]
    fn reads_multiple_unicode_paths_and_rejects_malformed_or_device_input() {
        let data = payload("C:\\música.txt\0\\\\server\\share\\file.txt\0\0");
        assert_eq!(
            parse(&data).unwrap(),
            [
                PathBuf::from(r"C:\música.txt"),
                PathBuf::from(r"\\server\share\file.txt")
            ]
        );
        assert!(parse(&data[..data.len() - 2]).is_err());
        assert!(parse(&payload("..\\outside.txt\0\0")).is_err());
        assert!(parse(&payload("\\\\.\\PhysicalDrive0\0\0")).is_err());
    }
}
