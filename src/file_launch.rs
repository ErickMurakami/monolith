//! User-initiated terminal and editor launches from a Files directory.

use std::{
    env,
    ffi::OsString,
    io,
    os::windows::ffi::OsStringExt,
    path::{Component, Path, PathBuf, Prefix},
    process::Command,
    sync::mpsc::{self, Receiver},
    thread,
};
use windows_sys::Win32::{
    Foundation::HWND,
    System::SystemInformation::GetSystemDirectoryW,
    UI::WindowsAndMessaging::{PostMessageW, WM_APP},
};

pub const READY: u32 = WM_APP + 18;

pub enum Tool {
    Terminal,
    Ide(PathBuf),
}

pub fn configured_ide(path: Option<PathBuf>) -> Option<PathBuf> {
    path.filter(|path| {
        path.is_absolute() && matches!(path.components().next(), Some(Component::Prefix(prefix)) if matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_)))
            && path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
    })
}

pub fn start(window: HWND, tool: Tool, folder: PathBuf) -> io::Result<Receiver<io::Result<()>>> {
    let window = window as usize;
    let (sender, receive) = mpsc::channel();
    thread::Builder::new()
        .name("monolith-files-launch".into())
        .spawn(move || {
            let result = match tool {
                Tool::Terminal => open_terminal(&folder),
                Tool::Ide(executable) => Command::new(executable)
                    .arg(&folder)
                    .current_dir(&folder)
                    .spawn()
                    .map(|_| ()),
            };
            if sender.send(result).is_ok() {
                unsafe { PostMessageW(window as HWND, READY, 0, 0) };
            }
        })?;
    Ok(receive)
}

fn terminal_command(executable: &Path, folder: &Path) -> Command {
    // Windows Terminal parses semicolons in its own argument syntax. Set the
    // real working directory separately and pass a fixed relative directory.
    let mut command = Command::new(executable);
    command
        .args(["-w", "new", "new-tab", "-d", "."])
        .current_dir(folder);
    command
}

fn open_terminal(folder: &Path) -> io::Result<()> {
    if let Some(local) = env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
    {
        let alias = local.join("Microsoft").join("WindowsApps").join("wt.exe");
        if terminal_command(&alias, folder).spawn().is_ok() {
            return Ok(());
        }
    }
    let executable = system_directory()?
        .join("WindowsPowerShell")
        .join("v1.0")
        .join("powershell.exe");
    Command::new(executable)
        .args(["-NoLogo", "-NoProfile"])
        .current_dir(folder)
        .spawn()
        .map(|_| ())
}

fn system_directory() -> io::Result<PathBuf> {
    let mut buffer = [0_u16; 260];
    let size = unsafe { GetSystemDirectoryW(buffer.as_mut_ptr(), buffer.len() as u32) } as usize;
    if size == 0 {
        return Err(io::Error::last_os_error());
    }
    if size >= buffer.len() {
        return Err(io::Error::other("System directory path is too long"));
    }
    Ok(PathBuf::from(OsString::from_wide(&buffer[..size])))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn treats_the_selected_folder_as_a_working_directory_not_terminal_syntax() {
        let folder = Path::new(r"C:\Projects\A; next tab");
        let command = terminal_command(Path::new(r"C:\Terminal\wt.exe"), folder);
        assert_eq!(command.get_current_dir(), Some(folder));
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            ["-w", "new", "new-tab", "-d", "."]
        );
    }

    #[test]
    fn accepts_only_a_configured_local_executable() {
        assert!(configured_ide(Some(r"C:\Tools\Code.exe".into())).is_some());
        assert!(configured_ide(Some(r"..\Code.exe".into())).is_none());
        assert!(configured_ide(Some(r"\\server\tools\Code.exe".into())).is_none());
        assert!(configured_ide(Some(r"C:\Tools\Code.cmd".into())).is_none());
    }
}
