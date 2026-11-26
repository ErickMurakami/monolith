//! Experimental primary-taskbar visibility lease. The helper alone changes visibility
//! during a healthy lease. Either process can restore the same Explorer window if its
//! peer terminates. This does not recover from simultaneous termination of both peers.
//!
//! Dispatch `run_helper` before creating GUI state when the first argument is
//! `HELPER_ARGUMENT`. Call `Lease::start` only for an explicitly enabled, usable Dock,
//! pass its minimum band height in physical pixels, and retain the lease on the GUI
//! thread. `CHANGED` asks the Dock to refresh on activation or release. Drop the lease
//! when its placement is unavailable, the display changes, or recovery is requested.

use std::{
    env,
    io::{self, Read, Write},
    mem::size_of,
    os::windows::{
        ffi::OsStringExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
        process::CommandExt,
    },
    path::PathBuf,
    ptr::null,
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    },
    thread,
};
use windows_sys::Win32::{
    Foundation::{ERROR_ALREADY_EXISTS, FILETIME, GetLastError, HANDLE, HWND, RECT, WAIT_TIMEOUT},
    Graphics::Gdi::{
        GetMonitorInfoW, HMONITOR, MONITOR_DEFAULTTONULL, MONITORINFO, MonitorFromWindow,
    },
    Storage::FileSystem::{FILE_TYPE_PIPE, GetFileType, SYNCHRONIZE},
    System::{
        SystemInformation::GetWindowsDirectoryW,
        Threading::{
            CREATE_NO_WINDOW, CreateEventW, EVENT_MODIFY_STATE, GetCurrentProcess, GetProcessTimes,
            INFINITE, OpenEventW, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
            QueryFullProcessImageNameW, SetEvent, WaitForMultipleObjects, WaitForSingleObject,
        },
    },
    UI::{
        HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetThreadDpiAwarenessContext},
        Shell::{
            ABE_BOTTOM, ABM_GETSTATE, ABM_GETTASKBARPOS, ABS_AUTOHIDE, APPBARDATA, SHAppBarMessage,
        },
        WindowsAndMessaging::{
            FindWindowW, GetShellWindow, GetSystemMetrics, GetWindowThreadProcessId,
            IsWindowVisible, MONITORINFOF_PRIMARY, PostMessageW, SM_CMONITORS, SW_HIDE, SW_SHOWNA,
            ShowWindowAsync, WM_APP,
        },
    },
};

pub const HELPER_ARGUMENT: &str = "--internal-taskbar-lease";
pub const CHANGED: u32 = WM_APP + 21;
const LEASE_EVENT: windows_sys::core::PCWSTR =
    windows_sys::w!("Local\\Monolith.NativeTaskbarLease.1");
const CLAIMED: u8 = b'C';
const ARM: u8 = b'A';
const ACTIVE: u8 = b'H';
const PENDING: u8 = 0;
const ENDED: u8 = 1;
const SETTLE_MS: u32 = 100;

pub enum Placement {
    Pending,
    Band(RECT),
    Unavailable,
}

pub struct Lease {
    tray: Tray,
    state: Arc<AtomicU8>,
    release: Arc<OwnedHandle>,
}

#[derive(Clone, Copy)]
struct Tray {
    window: usize,
    shell: usize,
    owner: u32,
    created: u64,
    band: RECT,
}

impl Lease {
    pub fn start(dock: HWND, minimum_height: i32) -> io::Result<Option<Self>> {
        let Some(tray) = Tray::discover(minimum_height) else {
            return Ok(None);
        };
        // Both peers retain this manual-reset event until restoration. A later
        // instance must leave the tray alone while the current lease releases.
        let handle = unsafe { CreateEventW(null(), 1, 0, LEASE_EVENT) };
        let existing = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
        let release = Arc::new(owned(handle)?);
        if existing {
            return Ok(None);
        }
        let parent_created = creation_time(unsafe { GetCurrentProcess() })?;
        let mut helper = std::process::Command::new(env::current_exe()?)
            .arg(HELPER_ARGUMENT)
            .args([
                std::process::id().to_string(),
                parent_created.to_string(),
                tray.window.to_string(),
                tray.shell.to_string(),
                tray.owner.to_string(),
                tray.created.to_string(),
                minimum_height.to_string(),
            ])
            .creation_flags(CREATE_NO_WINDOW)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()?;
        let mut input = helper.stdin.take().ok_or_else(io::Error::last_os_error)?;
        let mut output = helper.stdout.take().ok_or_else(io::Error::last_os_error)?;
        let state = Arc::new(AtomicU8::new(PENDING));
        let observer_release = Arc::clone(&release);
        let observer_state = Arc::clone(&state);
        let dock = dock as usize;
        thread::Builder::new()
            .name("taskbar-lease".into())
            .spawn(move || {
                let mut reply = [0];
                let armed = output.read_exact(&mut reply).is_ok()
                    && reply[0] == CLAIMED
                    && unsafe { WaitForSingleObject(observer_release.as_raw_handle(), 0) }
                        == WAIT_TIMEOUT
                    && input.write_all(&[ARM]).is_ok();
                drop(input);
                if armed && output.read_exact(&mut reply).is_ok() && reply[0] == ACTIVE {
                    observer_state.store(ACTIVE, Ordering::Release);
                    notify(dock);
                }
                // The helper exits after a clean release or its own termination.
                // Retain our event handle until fallback restoration is queued.
                let _ = helper.wait();
                if armed {
                    tray.restore();
                }
                drop(observer_release);
                observer_state.store(ENDED, Ordering::Release);
                notify(dock);
            })?;
        Ok(Some(Self {
            tray,
            state,
            release,
        }))
    }

    /// Use only the recorded bottom band of the still-hidden, original taskbar.
    pub fn placement(&self, display: &MONITORINFO) -> Placement {
        match self.state.load(Ordering::Acquire) {
            PENDING => Placement::Pending,
            ACTIVE
                if self.tray.matches_shell()
                    && !self.tray.visible()
                    && same_bounds(&self.tray.band, display) =>
            {
                Placement::Band(self.tray.band)
            }
            _ => Placement::Unavailable,
        }
    }

    pub fn is_tray(&self, window: HWND) -> bool {
        self.tray.window == window as usize
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        unsafe { SetEvent(self.release.as_raw_handle()) };
    }
}

/// Release a concurrently running lease on an explicit safe-mode start.
pub fn release_session() {
    if let Ok(release) = owned(unsafe { OpenEventW(EVENT_MODIFY_STATE, 0, LEASE_EVENT) }) {
        unsafe { SetEvent(release.as_raw_handle()) };
    }
}

pub fn run_helper() -> io::Result<()> {
    let mut args = env::args_os().skip(2);
    let mut number = || {
        args.next()
            .and_then(|value| value.to_str()?.parse::<u64>().ok())
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Invalid taskbar lease argument",
                )
            })
    };
    let parent = u32::try_from(number()?).map_err(io::Error::other)?;
    let parent_created = number()?;
    let window = usize::try_from(number()?).map_err(io::Error::other)?;
    let shell = usize::try_from(number()?).map_err(io::Error::other)?;
    let owner = u32::try_from(number()?).map_err(io::Error::other)?;
    let created = number()?;
    let minimum_height = i32::try_from(number()?).map_err(io::Error::other)?;
    let input = io::stdin();
    let output = io::stdout();
    if args.next().is_some()
        || unsafe {
            GetFileType(input.as_raw_handle()) != FILE_TYPE_PIPE
                || GetFileType(output.as_raw_handle()) != FILE_TYPE_PIPE
                || SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)
                    .is_null()
        }
    {
        return Ok(());
    }
    let parent =
        owned(unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE, 0, parent) })?;
    if creation_time(parent.as_raw_handle())? != parent_created
        || !image(parent.as_raw_handle())?
            .as_os_str()
            .eq_ignore_ascii_case(env::current_exe()?.as_os_str())
    {
        return Ok(());
    }
    let release = owned(unsafe { OpenEventW(SYNCHRONIZE, 0, LEASE_EVENT) })?;
    let signals = [release.as_raw_handle(), parent.as_raw_handle()];
    if wait_for_release(&signals, 0) != WAIT_TIMEOUT {
        return Ok(());
    }
    let mut input = input.lock();
    let mut output = output.lock();
    output.write_all(&[CLAIMED])?;
    output.flush()?;
    let mut request = [0];
    if input.read_exact(&mut request).is_err() || request[0] != ARM {
        return Ok(());
    }
    let Some(tray) = Tray::discover(minimum_height).filter(|tray| {
        (tray.window, tray.shell, tray.owner, tray.created) == (window, shell, owner, created)
    }) else {
        return Ok(());
    };
    if wait_for_release(&signals, 0) != WAIT_TIMEOUT {
        return Ok(());
    }
    let _restore = Restore(tray);
    if unsafe { ShowWindowAsync(tray.window as HWND, SW_HIDE) } == 0 {
        return Ok(());
    }
    // ShowWindowAsync queues work to Explorer. Use one bounded settling interval,
    // then abandon the lease if the Shell has not applied the change.
    if wait_for_release(&signals, SETTLE_MS) != WAIT_TIMEOUT
        || tray.visible()
        || !tray.matches_shell()
        || !tray.valid_geometry(minimum_height)
    {
        return Ok(());
    }
    output.write_all(&[ACTIVE])?;
    output.flush()?;
    wait_for_release(&signals, INFINITE);
    Ok(())
}

fn wait_for_release(signals: &[HANDLE; 2], timeout: u32) -> u32 {
    // The helper owns both handles for the entire wait and creates no windows.
    unsafe { WaitForMultipleObjects(signals.len() as u32, signals.as_ptr(), 0, timeout) }
}

struct Restore(Tray);

impl Drop for Restore {
    fn drop(&mut self) {
        self.0.restore();
    }
}

impl Tray {
    fn discover(minimum_height: i32) -> Option<Self> {
        let window = primary_tray();
        let shell = unsafe { GetShellWindow() };
        let owner = window_process(window);
        if owner == 0 || window_process(shell) != owner || unsafe { IsWindowVisible(window) } == 0 {
            return None;
        }
        let process = process(owner).ok()?;
        let mut directory = [0; 260];
        let length = unsafe { GetWindowsDirectoryW(directory.as_mut_ptr(), directory.len() as u32) }
            as usize;
        if length == 0 || length >= directory.len() {
            return None;
        }
        let explorer =
            PathBuf::from(std::ffi::OsString::from_wide(&directory[..length])).join("explorer.exe");
        if !image(process.as_raw_handle())
            .ok()?
            .as_os_str()
            .eq_ignore_ascii_case(explorer.as_os_str())
        {
            return None;
        }
        let mut data = appbar_data();
        if unsafe { SHAppBarMessage(ABM_GETTASKBARPOS, &mut data) } == 0 || data.uEdge != ABE_BOTTOM
        {
            return None;
        }
        let tray = Self {
            window: window as usize,
            shell: shell as usize,
            owner,
            created: creation_time(process.as_raw_handle()).ok()?,
            band: data.rc,
        };
        (tray.matches_shell() && tray.valid_geometry(minimum_height)).then_some(tray)
    }

    fn matches_shell(self) -> bool {
        let window = primary_tray();
        let shell = unsafe { GetShellWindow() };
        window as usize == self.window
            && shell as usize == self.shell
            && window_process(window) == self.owner
            && window_process(shell) == self.owner
    }

    fn visible(self) -> bool {
        unsafe { IsWindowVisible(self.window as HWND) != 0 }
    }

    fn valid_geometry(self, minimum_height: i32) -> bool {
        let monitor: HMONITOR =
            unsafe { MonitorFromWindow(self.window as HWND, MONITOR_DEFAULTTONULL) };
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        minimum_height > 0
            && unsafe {
                GetSystemMetrics(SM_CMONITORS) == 1
                    && FindWindowW(windows_sys::w!("Shell_SecondaryTrayWnd"), null()).is_null()
                    && GetMonitorInfoW(monitor, &mut info) != 0
            }
            && native_auto_hide_off()
            && self.matches_shell()
            && same_bounds(&self.band, &info)
            && self.band.bottom.saturating_sub(self.band.top) >= minimum_height
    }

    fn restore(self) {
        if self.matches_shell()
            && process(self.owner)
                .and_then(|process| creation_time(process.as_raw_handle()))
                .is_ok_and(|created| created == self.created)
            && native_auto_hide_off()
            && self.matches_shell()
        {
            // Queue even when Explorer has yet to apply a pending hide request.
            unsafe { ShowWindowAsync(self.window as HWND, SW_SHOWNA) };
        }
    }
}

fn same_bounds(band: &RECT, display: &MONITORINFO) -> bool {
    display.dwFlags & MONITORINFOF_PRIMARY != 0
        && band.left == display.rcMonitor.left
        && band.right == display.rcMonitor.right
        && band.top == display.rcWork.bottom
        && band.bottom == display.rcMonitor.bottom
}

fn primary_tray() -> HWND {
    unsafe { FindWindowW(windows_sys::w!("Shell_TrayWnd"), null()) }
}

fn window_process(window: HWND) -> u32 {
    let mut process = 0;
    unsafe { GetWindowThreadProcessId(window, &mut process) };
    process
}

fn appbar_data() -> APPBARDATA {
    APPBARDATA {
        cbSize: size_of::<APPBARDATA>() as u32,
        ..Default::default()
    }
}

fn native_auto_hide_off() -> bool {
    (unsafe { SHAppBarMessage(ABM_GETSTATE, &mut appbar_data()) } & ABS_AUTOHIDE as usize) == 0
}

fn notify(dock: usize) {
    if window_process(dock as HWND) == std::process::id() {
        unsafe { PostMessageW(dock as HWND, CHANGED, 0, 0) };
    }
}

fn process(pid: u32) -> io::Result<OwnedHandle> {
    owned(unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) })
}

fn owned(handle: HANDLE) -> io::Result<OwnedHandle> {
    if handle.is_null() {
        return Err(io::Error::last_os_error());
    }
    // Win32 returned a newly owned kernel handle.
    Ok(unsafe { OwnedHandle::from_raw_handle(handle) })
}

fn creation_time(handle: HANDLE) -> io::Result<u64> {
    let (mut created, mut exit, mut kernel, mut user) = Default::default();
    if unsafe { GetProcessTimes(handle, &mut created, &mut exit, &mut kernel, &mut user) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let FILETIME {
        dwHighDateTime,
        dwLowDateTime,
    } = created;
    Ok((u64::from(dwHighDateTime) << 32) | u64::from(dwLowDateTime))
}

fn image(handle: HANDLE) -> io::Result<PathBuf> {
    let mut path = vec![0; 32768];
    let mut length = path.len() as u32;
    if unsafe { QueryFullProcessImageNameW(handle, 0, path.as_mut_ptr(), &mut length) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(PathBuf::from(std::ffi::OsString::from_wide(
        &path[..length as usize],
    )))
}
