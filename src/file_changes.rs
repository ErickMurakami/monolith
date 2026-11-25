//! Single-item file changes that leave an existing destination intact.

use std::{
    ffi::{OsStr, OsString},
    fs, io,
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver},
    thread,
};
use windows_sys::Win32::{
    Foundation::HWND,
    Storage::FileSystem::MoveFileExW,
    UI::WindowsAndMessaging::{PostMessageW, WM_APP},
};

pub const READY: u32 = WM_APP + 15;

#[derive(Clone)]
pub enum Change {
    CreateDirectory,
    Rename(OsString),
}

pub struct Outcome {
    pub directory: PathBuf,
    pub result: io::Result<()>,
}

pub fn valid_name(name: &str) -> bool {
    if name.is_empty()
        || name.ends_with([' ', '.'])
        || name.chars().any(|ch| {
            ch <= '\u{1f}' || matches!(ch, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')
        })
    {
        return false;
    }
    let stem = name
        .split('.')
        .next()
        .unwrap_or(name)
        .trim_end_matches(' ')
        .to_ascii_uppercase();
    !matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) && !stem
        .strip_prefix("COM")
        .or_else(|| stem.strip_prefix("LPT"))
        .is_some_and(|suffix| {
            matches!(
                suffix,
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        })
}

fn extended(path: &Path) -> Vec<u16> {
    let encoded = path.as_os_str().encode_wide().collect::<Vec<_>>();
    let mut result = if encoded.starts_with(&[92, 92, 63, 92]) {
        encoded
    } else if encoded.starts_with(&[92, 92]) {
        r"\\?\UNC\"
            .encode_utf16()
            .chain(encoded.into_iter().skip(2))
            .collect()
    } else {
        r"\\?\".encode_utf16().chain(encoded).collect()
    };
    result.push(0);
    result
}

fn apply(directory: &Path, change: Change, name: &OsStr) -> io::Result<()> {
    let destination = directory.join(name);
    match change {
        Change::CreateDirectory => fs::create_dir(destination),
        Change::Rename(source) => {
            if source == name {
                return Ok(());
            }
            let source = extended(&directory.join(source));
            let destination = extended(&destination);
            // A same-directory rename must fail if the destination already exists.
            if unsafe { MoveFileExW(source.as_ptr(), destination.as_ptr(), 0) } == 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            }
        }
    }
}

pub fn start(
    window: HWND,
    directory: PathBuf,
    change: Change,
    name: Vec<u16>,
) -> io::Result<Receiver<Outcome>> {
    let value = String::from_utf16(&name).map_err(|_| invalid_name())?;
    if !valid_name(&value) {
        return Err(invalid_name());
    }
    let name = OsString::from_wide(&name);
    let window = window as usize;
    let (sender, receiver) = mpsc::channel();
    thread::Builder::new()
        .name("monolith-file-change".into())
        .spawn(move || {
            let result = apply(&directory, change, &name);
            if sender.send(Outcome { directory, result }).is_ok() {
                unsafe { PostMessageW(window as HWND, READY, 0, 0) };
            }
        })?;
    Ok(receiver)
}

fn invalid_name() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, "Invalid Windows filename")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_paths_streams_and_reserved_device_names() {
        for name in [
            "",
            ".",
            "..",
            r"..\other",
            "a:b",
            "name.",
            "name ",
            "NUL.txt",
            "com¹.log",
            "aux .txt",
        ] {
            assert!(!valid_name(name), "{name}");
        }
        for name in ["Música", "foo.bar.txt", ".gitignore", "package com1"] {
            assert!(valid_name(name), "{name}");
        }
    }

    #[test]
    fn preserves_extended_paths_and_adds_the_unc_prefix() {
        let visible = |wide: Vec<u16>| String::from_utf16_lossy(&wide[..wide.len() - 1]);
        assert_eq!(
            visible(extended(Path::new(r"C:\test\folder"))),
            r"\\?\C:\test\folder"
        );
        assert_eq!(
            visible(extended(Path::new(r"\\server\share\file"))),
            r"\\?\UNC\server\share\file"
        );
        assert_eq!(visible(extended(Path::new(r"\\?\C:\test"))), r"\\?\C:\test");
    }
}
