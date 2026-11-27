//! Native Windows virtual desktops as ordered Monolith workspaces.
//!
//! Windows publishes no API for desktop order or the current desktop. Explorer
//! persists both under the user's registry hive, which PowerToys FancyZones also
//! reads. Monolith only reads these values and switches through the documented
//! Win+Ctrl+Left/Right shortcuts, so Explorer remains the owner of desktop state.

use crate::space_bridge::INJECTED;
use std::{
    io,
    mem::{size_of, size_of_val},
    ptr::null_mut,
};
use windows_sys::Win32::{
    Foundation::ERROR_SUCCESS,
    System::{
        Registry::{HKEY_CURRENT_USER, RRF_RT_REG_BINARY, RRF_RT_REG_SZ, RegGetValueW},
        RemoteDesktop::ProcessIdToSessionId,
        Threading::GetCurrentProcessId,
    },
    UI::Input::KeyboardAndMouse::*,
};

const DESKTOPS: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\VirtualDesktops";

pub type DesktopId = [u8; 16];

pub struct Workspaces {
    pub ids: Vec<DesktopId>,
    pub current: Option<usize>,
}

pub fn read() -> io::Result<Workspaces> {
    let ids = parse_ids(&binary(DESKTOPS, "VirtualDesktopIDs")?);
    let mut session = 0;
    let session_key = (unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &mut session) } != 0)
        .then(|| format!(r"Software\Microsoft\Windows\CurrentVersion\Explorer\SessionInfo\{session}\VirtualDesktops"));
    // Explorer keeps the current desktop per session once the user has switched.
    let current = session_key
        .and_then(|key| binary(&key, "CurrentVirtualDesktop").ok())
        .or_else(|| binary(DESKTOPS, "CurrentVirtualDesktop").ok())
        .and_then(|value| value.try_into().ok())
        .and_then(|current: DesktopId| ids.iter().position(|id| *id == current));
    Ok(Workspaces { ids, current })
}

fn parse_ids(value: &[u8]) -> Vec<DesktopId> {
    value.as_chunks::<16>().0.to_vec()
}

fn binary(key: &str, value: &str) -> io::Result<Vec<u8>> {
    let key: Vec<u16> = key.encode_utf16().chain([0]).collect();
    let value: Vec<u16> = value.encode_utf16().chain([0]).collect();
    let mut buffer = vec![0_u8; 16 * 64];
    let mut size = buffer.len() as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_BINARY,
            null_mut(),
            buffer.as_mut_ptr().cast(),
            &mut size,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    buffer.truncate(size as usize);
    Ok(buffer)
}

impl Workspaces {
    /// A short description of the current desktop, such as "Work · 2 of 3", when
    /// more than one desktop exists or the current one has a user-assigned name.
    pub fn summary(&self, portuguese: bool) -> Option<String> {
        let index = self.current?;
        let name = name(&self.ids[index]);
        if self.ids.len() < 2 && name.is_none() {
            return None;
        }
        let number = index + 1;
        let label = name.unwrap_or_else(|| {
            if portuguese {
                format!("Área {number}")
            } else {
                format!("Desktop {number}")
            }
        });
        let of = if portuguese { "de" } else { "of" };
        Some(format!("{label} · {number} {of} {}", self.ids.len()))
    }
}

/// The name a user gave the desktop in Task View, if any.
fn name(id: &DesktopId) -> Option<String> {
    let key: Vec<u16> = format!(r"{DESKTOPS}\Desktops\{}", guid_text(id))
        .encode_utf16()
        .chain([0])
        .collect();
    let value: Vec<u16> = "Name".encode_utf16().chain([0]).collect();
    let mut buffer = [0_u16; 256];
    let mut size = size_of_val(&buffer) as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_SZ,
            null_mut(),
            buffer.as_mut_ptr().cast(),
            &mut size,
        )
    };
    if status != ERROR_SUCCESS {
        return None;
    }
    let length = buffer
        .iter()
        .position(|&unit| unit == 0)
        .unwrap_or(buffer.len());
    let name = String::from_utf16_lossy(&buffer[..length]);
    let name = name.trim();
    (!name.is_empty()).then(|| name.to_owned())
}

/// Registry key spelling of a GUID stored in its little-endian binary layout.
fn guid_text(id: &DesktopId) -> String {
    let d1 = u32::from_le_bytes([id[0], id[1], id[2], id[3]]);
    let d2 = u16::from_le_bytes([id[4], id[5]]);
    let d3 = u16::from_le_bytes([id[6], id[7]]);
    let tail: String = id[10..].iter().map(|byte| format!("{byte:02X}")).collect();
    format!(
        "{{{d1:08X}-{d2:04X}-{d3:04X}-{:02X}{:02X}-{tail}}}",
        id[8], id[9]
    )
}

/// Steps from the current desktop to `target`, or `None` when either is unknown.
fn steps(workspaces: &Workspaces, target: usize) -> Option<i32> {
    let current = workspaces.current?;
    (target < workspaces.ids.len()).then(|| target as i32 - current as i32)
}

/// Select the workspace at `index` in Explorer's order. Returns false if it does not exist.
pub fn switch_to(index: usize) -> bool {
    let Some(delta) = read().ok().and_then(|workspaces| steps(&workspaces, index)) else {
        return false;
    };
    delta == 0
        || press(
            &[
                VK_LWIN,
                VK_LCONTROL,
                if delta < 0 { VK_LEFT } else { VK_RIGHT },
            ],
            delta.unsigned_abs() as usize,
        )
}

/// Create a native desktop, wait until Explorer reports it as current, then open
/// `target` there. Runs on its own thread; if the desktop cannot be created, the
/// target still opens on the current desktop.
pub fn open_in_new(target: std::path::PathBuf) -> io::Result<()> {
    std::thread::Builder::new()
        .name("monolith-new-workspace".into())
        .spawn(move || {
            let before = read()
                .ok()
                .and_then(|workspaces| workspaces.current.map(|index| workspaces.ids[index]));
            // The user may still hold Ctrl from Ctrl+Enter; the chord needs clean modifiers.
            let released = wait_until(2_000, || !any_modifier_held());
            let current = || {
                read()
                    .ok()
                    .and_then(|workspaces| workspaces.current.map(|index| workspaces.ids[index]))
            };
            let created = (released && press(&[VK_LWIN, VK_LCONTROL, VK_D], 1))
                .then(|| {
                    wait_until(1_500, || current().is_some_and(|id| Some(id) != before));
                    current().filter(|id| Some(*id) != before)
                })
                .flatten();
            launch(&target);
            // Track the desktop only once its application has shown a window there,
            // so a slow launch cannot make it look empty.
            if let Some(created) = created
                && wait_until(10_000, || {
                    current() == Some(created)
                        && crate::desktop::snapshot()
                            .is_ok_and(|snapshot| !snapshot.windows.is_empty())
                })
            {
                track(before, created);
            }
        })
        .map(drop)
}

/// A desktop Monolith created for an application, and the desktop it came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Dedicated {
    origin: Option<DesktopId>,
    created: DesktopId,
}

#[derive(Debug, PartialEq, Eq)]
enum Step {
    Keep,
    /// The user closed or replaced the desktop; stop tracking it.
    Forget,
    /// The desktop is current and empty: close it and return to `origin`.
    Return {
        origin: Option<DesktopId>,
    },
}

static DEDICATED: std::sync::Mutex<Vec<Dedicated>> = std::sync::Mutex::new(Vec::new());
static WATCHING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn decide(entry: Dedicated, workspaces: &Workspaces, has_windows: bool) -> Step {
    let Some(created) = workspaces.ids.iter().position(|id| *id == entry.created) else {
        return Step::Forget;
    };
    if workspaces.current != Some(created) || has_windows {
        return Step::Keep;
    }
    Step::Return {
        origin: entry
            .origin
            .filter(|origin| workspaces.ids.contains(origin)),
    }
}

/// Track `created` once its application has shown a window there.
fn track(origin: Option<DesktopId>, created: DesktopId) {
    if let Ok(mut entries) = DEDICATED.lock() {
        entries.push(Dedicated { origin, created });
    }
    start_watcher();
}

thread_local! {
    static WATCH_THREAD: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

const CHECK: u32 = windows_sys::Win32::UI::WindowsAndMessaging::WM_APP + 1;

unsafe extern "system" fn window_destroyed(
    _: windows_sys::Win32::UI::Accessibility::HWINEVENTHOOK,
    _: u32,
    _: windows_sys::Win32::Foundation::HWND,
    object: i32,
    child: i32,
    _: u32,
    _: u32,
) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{OBJID_WINDOW, PostThreadMessageW};
    if object == OBJID_WINDOW && child == 0 {
        unsafe { PostThreadMessageW(WATCH_THREAD.get(), CHECK, 0, 0) };
    }
}

/// Runs only while dedicated desktops are tracked. Window destruction schedules a
/// check after a short delay so closing applications can finish tearing down.
fn watch() {
    use windows_sys::Win32::{
        System::Threading::GetCurrentThreadId,
        UI::{
            Accessibility::{SetWinEventHook, UnhookWinEvent},
            WindowsAndMessaging::{
                EVENT_OBJECT_DESTROY, GetMessageW, KillTimer, MSG, SetTimer, WINEVENT_OUTOFCONTEXT,
                WINEVENT_SKIPOWNPROCESS, WM_TIMER,
            },
        },
    };
    WATCH_THREAD.set(unsafe { GetCurrentThreadId() });
    let hook = unsafe {
        SetWinEventHook(
            EVENT_OBJECT_DESTROY,
            EVENT_OBJECT_DESTROY,
            null_mut(),
            Some(window_destroyed),
            0,
            0,
            WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
        )
    };
    let mut timer = 0;
    let mut msg = MSG::default();
    while !hook.is_null() && unsafe { GetMessageW(&mut msg, null_mut(), 0, 0) } > 0 {
        match msg.message {
            CHECK if timer == 0 => timer = unsafe { SetTimer(null_mut(), 0, 400, None) },
            WM_TIMER if msg.wParam == timer && timer != 0 => {
                unsafe { KillTimer(null_mut(), timer) };
                timer = 0;
                if !check() {
                    break;
                }
            }
            _ => {}
        }
    }
    if !hook.is_null() {
        unsafe { UnhookWinEvent(hook) };
    }
    WATCHING.store(false, std::sync::atomic::Ordering::Release);
    // An entry added while stopping still needs a watcher.
    if DEDICATED.lock().is_ok_and(|entries| !entries.is_empty()) {
        start_watcher();
    }
}

fn start_watcher() {
    if !WATCHING.swap(true, std::sync::atomic::Ordering::AcqRel)
        && std::thread::Builder::new()
            .name("monolith-dedicated-workspaces".into())
            .spawn(watch)
            .is_err()
    {
        WATCHING.store(false, std::sync::atomic::Ordering::Release);
    }
}

/// Apply the policy to every tracked desktop. False once nothing is tracked.
fn check() -> bool {
    let Ok(workspaces) = read() else {
        return true;
    };
    let has_windows = crate::desktop::snapshot().is_ok_and(|snapshot| !snapshot.windows.is_empty());
    let Ok(mut entries) = DEDICATED.lock() else {
        return false;
    };
    let mut returning = None;
    entries.retain(|entry| match decide(*entry, &workspaces, has_windows) {
        Step::Keep => true,
        Step::Forget => false,
        Step::Return { origin } => {
            returning = Some((entry.created, origin));
            false
        }
    });
    let remaining = !entries.is_empty();
    drop(entries);
    if let Some((created, origin)) = returning {
        // Close the empty desktop Monolith created, then return to where it started.
        if press(&[VK_LWIN, VK_LCONTROL, VK_F4], 1) {
            wait_until(1_500, || {
                read().is_ok_and(|workspaces| !workspaces.ids.contains(&created))
            });
        }
        if let Some(index) =
            origin.and_then(|origin| read().ok()?.ids.iter().position(|id| *id == origin))
        {
            switch_to(index);
        }
    }
    remaining
}

/// A fullscreen window moved to a desktop of its own, and the desktop it left.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Space {
    window: isize,
    origin: DesktopId,
    created: DesktopId,
}

#[derive(Debug, PartialEq, Eq)]
enum FullscreenStep {
    /// Give this fullscreen window its own desktop.
    Enter(isize),
    /// The tracked window at this position left fullscreen or closed.
    Leave(usize),
    Idle,
}

fn plan(
    spaces: &[Space],
    fullscreen_foreground: Option<isize>,
    still_fullscreen: impl Fn(isize) -> bool,
    current: Option<DesktopId>,
) -> FullscreenStep {
    if let Some(index) = spaces
        .iter()
        .position(|space| !still_fullscreen(space.window))
    {
        return FullscreenStep::Leave(index);
    }
    match fullscreen_foreground {
        // Never nest: a window already tracked, or one shown on a created desktop, stays.
        Some(window)
            if !spaces
                .iter()
                .any(|space| space.window == window || Some(space.created) == current) =>
        {
            FullscreenStep::Enter(window)
        }
        _ => FullscreenStep::Idle,
    }
}

/// Opt-in: fullscreen applications occupy their own native desktop and return to
/// where they came from when they leave fullscreen. Desktops are created and closed
/// with the documented shortcuts; only moving the window uses the private adapter.
#[derive(Default)]
pub struct FullscreenSpaces {
    spaces: Vec<Space>,
}

impl FullscreenSpaces {
    pub fn check(&mut self, mover: &crate::virtual_desktops::Mover) {
        use windows_sys::Win32::Foundation::HWND;
        // A tracked window on another desktop is cloaked, which says nothing about
        // fullscreen; only a closed window or a visible non-fullscreen one returns.
        let fullscreen = |window: isize| {
            let hwnd = window as HWND;
            (unsafe { windows_sys::Win32::UI::WindowsAndMessaging::IsWindow(hwnd) } != 0)
                && (crate::desktop::is_cloaked(hwnd) || crate::desktop::is_fullscreen(hwnd))
        };
        // Each step changes desktop state, so re-plan after every one; bound the work.
        for _ in 0..4 {
            let current = read()
                .ok()
                .and_then(|workspaces| workspaces.current.map(|index| workspaces.ids[index]));
            let foreground = crate::desktop::fullscreen_foreground().map(|hwnd| hwnd as isize);
            match plan(&self.spaces, foreground, fullscreen, current) {
                FullscreenStep::Idle => return,
                FullscreenStep::Leave(index) => {
                    let space = self.spaces.remove(index);
                    leave(mover, space);
                }
                FullscreenStep::Enter(window) => match enter(mover, window) {
                    Some(space) => self.spaces.push(space),
                    None => return,
                },
            }
        }
    }
}

fn current_id() -> Option<DesktopId> {
    read()
        .ok()
        .and_then(|workspaces| workspaces.current.map(|index| workspaces.ids[index]))
}

fn index_of(id: DesktopId) -> Option<usize> {
    read()
        .ok()?
        .ids
        .iter()
        .position(|candidate| *candidate == id)
}

fn enter(mover: &crate::virtual_desktops::Mover, window: isize) -> Option<Space> {
    use windows_sys::Win32::{Foundation::HWND, UI::WindowsAndMessaging::SetForegroundWindow};
    let origin = current_id()?;
    if !wait_until(1_000, || !any_modifier_held()) || !press(&[VK_LWIN, VK_LCONTROL, VK_D], 1) {
        return None;
    }
    wait_until(1_500, || current_id().is_some_and(|id| id != origin));
    let created = current_id().filter(|id| *id != origin)?;
    let index = index_of(created)?;
    if !mover.move_window(windows::Win32::Foundation::HWND(window as _), index) {
        // Undo: the new desktop is current and empty.
        close_current_if_empty(created);
        if let Some(index) = index_of(origin) {
            switch_to(index);
        }
        return None;
    }
    unsafe { SetForegroundWindow(window as HWND) };
    Some(Space {
        window,
        origin,
        created,
    })
}

fn leave(mover: &crate::virtual_desktops::Mover, space: Space) {
    use windows_sys::Win32::{Foundation::HWND, UI::WindowsAndMessaging::IsWindow};
    let alive = unsafe { IsWindow(space.window as HWND) } != 0;
    if alive && let Some(index) = index_of(space.origin) {
        mover.move_window(windows::Win32::Foundation::HWND(space.window as _), index);
    }
    // Close the created desktop only while it is current and holds no other windows.
    if current_id() == Some(space.created) {
        close_current_if_empty(space.created);
        if let Some(index) = index_of(space.origin) {
            switch_to(index);
        }
    }
}

fn close_current_if_empty(created: DesktopId) {
    let empty = crate::desktop::snapshot().is_ok_and(|snapshot| snapshot.windows.is_empty());
    if empty
        && current_id() == Some(created)
        && wait_until(1_000, || !any_modifier_held())
        && press(&[VK_LWIN, VK_LCONTROL, VK_F4], 1)
    {
        wait_until(1_500, || index_of(created).is_none());
    }
}

fn any_modifier_held() -> bool {
    [VK_LWIN, VK_RWIN, VK_SHIFT, VK_CONTROL, VK_MENU]
        .into_iter()
        .any(|key| unsafe { GetAsyncKeyState(i32::from(key)) } < 0)
}

/// Poll `condition` every 25 ms for at most `limit_ms`; true when it held.
fn wait_until(limit_ms: u64, mut condition: impl FnMut() -> bool) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(limit_ms);
    loop {
        if condition() {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
}

fn launch(target: &std::path::Path) {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL};
    let path: Vec<u16> = target.as_os_str().encode_wide().chain([0]).collect();
    let verb: Vec<u16> = "open".encode_utf16().chain([0]).collect();
    unsafe {
        ShellExecuteW(
            null_mut(),
            verb.as_ptr(),
            path.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        );
    }
}

/// Press `keys` as a chord, repeating the final key `repeat` times, when no modifier
/// is physically held. Input is tagged so the Monolith keyboard bridge ignores it.
pub fn press(keys: &[u16], repeat: usize) -> bool {
    let Some((&last, held)) = keys.split_last() else {
        return false;
    };
    let modifiers = [VK_LWIN, VK_RWIN, VK_SHIFT, VK_CONTROL, VK_MENU];
    if modifiers
        .into_iter()
        .chain(keys.iter().copied())
        .any(|key| unsafe { GetAsyncKeyState(i32::from(key)) } < 0)
    {
        return false;
    }
    let input = |key: u16, flags| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: key,
                dwFlags: if matches!(key, VK_LWIN | VK_LEFT | VK_RIGHT) {
                    flags | KEYEVENTF_EXTENDEDKEY
                } else {
                    flags
                },
                dwExtraInfo: INJECTED,
                ..Default::default()
            },
        },
    };
    let mut events: Vec<_> = held.iter().map(|&key| input(key, 0)).collect();
    for _ in 0..repeat.max(1) {
        events.push(input(last, 0));
        events.push(input(last, KEYEVENTF_KEYUP));
    }
    events.extend(held.iter().rev().map(|&key| input(key, KEYEVENTF_KEYUP)));
    let size = size_of::<INPUT>() as i32;
    let sent = unsafe { SendInput(events.len() as u32, events.as_ptr(), size) } as usize;
    if sent != events.len() && sent > 0 {
        // Never leave a modifier logically held after a partial insertion.
        let release: Vec<_> = held
            .iter()
            .rev()
            .map(|&key| input(key, KEYEVENTF_KEYUP))
            .collect();
        unsafe { SendInput(release.len() as u32, release.as_ptr(), size) };
    }
    sent == events.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ordered_identities_and_ignores_a_partial_trailer() {
        let mut value: Vec<u8> = (0..32).collect();
        value.extend([1, 2, 3]);
        let ids = parse_ids(&value);
        assert_eq!(ids.len(), 2);
        assert_eq!(ids[1][0], 16);
    }

    #[test]
    fn computes_directional_steps_within_bounds() {
        let workspaces = Workspaces {
            ids: vec![[0; 16], [1; 16], [2; 16]],
            current: Some(2),
        };
        assert_eq!(steps(&workspaces, 0), Some(-2));
        assert_eq!(steps(&workspaces, 2), Some(0));
        assert_eq!(steps(&workspaces, 3), None);
        let unknown = Workspaces {
            current: None,
            ..workspaces
        };
        assert_eq!(steps(&unknown, 0), None);
    }

    #[test]
    fn summarises_unnamed_desktops_by_position() {
        // Identities without a registry entry fall back to a numbered label.
        let workspaces = Workspaces {
            ids: vec![[0xa5; 16], [0x5a; 16], [0x3c; 16]],
            current: Some(1),
        };
        assert_eq!(
            workspaces.summary(false).as_deref(),
            Some("Desktop 2 · 2 of 3")
        );
        assert_eq!(workspaces.summary(true).as_deref(), Some("Área 2 · 2 de 3"));
        let single = Workspaces {
            ids: vec![[0xa5; 16]],
            current: Some(0),
        };
        assert_eq!(single.summary(false), None);
    }

    #[test]
    fn plans_fullscreen_spaces_without_nesting() {
        let space = Space {
            window: 7,
            origin: [1; 16],
            created: [2; 16],
        };
        // A new fullscreen window gets a desktop.
        assert_eq!(
            plan(&[], Some(7), |_| true, Some([1; 16])),
            FullscreenStep::Enter(7)
        );
        // Already tracked, or another window on a created desktop: nothing to do.
        assert_eq!(
            plan(&[space], Some(7), |_| true, Some([2; 16])),
            FullscreenStep::Idle
        );
        assert_eq!(
            plan(&[space], Some(9), |_| true, Some([2; 16])),
            FullscreenStep::Idle
        );
        // Leaving fullscreen returns the window before anything else happens.
        assert_eq!(
            plan(&[space], Some(9), |window| window != 7, Some([1; 16])),
            FullscreenStep::Leave(0)
        );
        assert_eq!(
            plan(&[], None, |_| true, Some([1; 16])),
            FullscreenStep::Idle
        );
    }

    #[test]
    fn returns_only_from_an_empty_current_dedicated_desktop() {
        let entry = Dedicated {
            origin: Some([1; 16]),
            created: [3; 16],
        };
        let at = |current| Workspaces {
            ids: vec![[1; 16], [2; 16], [3; 16]],
            current: Some(current),
        };
        assert_eq!(
            decide(entry, &at(2), false),
            Step::Return {
                origin: Some([1; 16])
            }
        );
        // Still in use, or the user is elsewhere: leave it alone.
        assert_eq!(decide(entry, &at(2), true), Step::Keep);
        assert_eq!(decide(entry, &at(0), false), Step::Keep);
        // The user closed the dedicated desktop.
        let closed = Workspaces {
            ids: vec![[1; 16], [2; 16]],
            current: Some(0),
        };
        assert_eq!(decide(entry, &closed, false), Step::Forget);
        // The origin was closed meanwhile: close the empty desktop without a target.
        let orphan = Workspaces {
            ids: vec![[2; 16], [3; 16]],
            current: Some(1),
        };
        assert_eq!(decide(entry, &orphan, false), Step::Return { origin: None });
    }

    #[test]
    fn spells_registry_guid_keys() {
        // 83578b8d-d29f-42dc-8bb0-b2c1e3cb7dd4 in its binary layout.
        let id = [
            0x8d, 0x8b, 0x57, 0x83, 0x9f, 0xd2, 0xdc, 0x42, 0x8b, 0xb0, 0xb2, 0xc1, 0xe3, 0xcb,
            0x7d, 0xd4,
        ];
        assert_eq!(guid_text(&id), "{83578B8D-D29F-42DC-8BB0-B2C1E3CB7DD4}");
    }

    #[test]
    #[ignore = "reads the interactive user's Explorer state"]
    fn reads_the_live_desktop_order() {
        let workspaces = read().unwrap();
        assert!(!workspaces.ids.is_empty());
        assert!(workspaces.current.is_some());
    }
}
