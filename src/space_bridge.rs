//! Opt-in Win+Space, Win+Tab and Win+1-9 for Monolith on builds where Windows
//! reserves these chords.
//!
//! A low-level keyboard hook on a dedicated thread swallows the chord key while the
//! logo key is held and pairs the logo key with a harmless marker so Start does not
//! open. After the logo key is released, it sends a registered fallback chord so
//! Windows delivers a real `WM_HOTKEY` with foreground rights, or switches desktops
//! through the documented shortcuts for Win+1-9. Keys typed between the Spotlight chord
//! and Spotlight taking focus are held in a small buffer and replayed as native input
//! once Spotlight reports focus, so dead keys and modifiers keep their layout meaning.
//! The buffer exists only during that window and is never stored or logged.

use crate::workspaces;
use std::{
    cell::RefCell,
    io,
    mem::size_of,
    ptr::null_mut,
    sync::{atomic::Ordering, mpsc},
    thread::{self, JoinHandle},
};
use windows_sys::Win32::{
    Foundation::{LPARAM, LRESULT, WPARAM},
    System::{LibraryLoader::GetModuleHandleW, Threading::GetCurrentThreadId},
    UI::{
        Accessibility::{SetWinEventHook, UnhookWinEvent},
        Input::KeyboardAndMouse::*,
        WindowsAndMessaging::*,
    },
};

/// Tags Monolith's own synthetic input so the hook passes it through.
pub const INJECTED: usize = 0x4d4f_4e4f;
// Give the system a harmless key to pair with the logo key instead of opening Start.
const LOGO_MARKER: u16 = 0xff;
const RELEASE: u32 = WM_APP + 1;
const LOGO_UP: u32 = WM_APP + 2;
const BEGIN: u32 = WM_APP + 3;
const WINDOW_CHANGED: u32 = WM_APP + 4;
const MOVE_WINDOW: u32 = WM_APP + 5;
/// Thread ID of the desktop worker, or zero while it is not running.
static DESKTOP_THREAD: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
const POLL_MS: u32 = 10;
/// Replay to whichever window has focus if Spotlight never reports.
const CAPTURE_LIMIT_MS: u32 = 1500;
/// Release the logo key within this time or the activation is abandoned.
const LOGO_LIMIT_MS: u32 = 3000;
const BUFFER_LIMIT: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Key {
    pub vk: u16,
    pub scan: u16,
    pub up: bool,
    pub extended: bool,
}

#[derive(Clone, Copy, Default)]
pub struct Modifiers {
    pub logo: bool,
    pub other: bool,
    pub shift: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Spotlight,
    MissionControl,
    /// Zero-based workspace index from Win+1 through Win+9.
    Workspace(u8),
    /// Zero-based target for moving the focused window, from Win+Shift+1 through 9.
    MoveTo(u8),
}

#[derive(Debug, PartialEq, Eq)]
pub enum Decision {
    Pass,
    Swallow,
    /// Swallow the key and run the action once the logo key is released.
    Trigger(Action),
}

/// The chords this bridge may take over from Windows.
#[derive(Clone, Copy, Default)]
pub struct Chords {
    pub spotlight: bool,
    pub mission_control: bool,
    pub workspaces: bool,
    /// Win+Shift+1-9 through the opt-in virtual desktop adapter.
    pub moves: bool,
    /// Fullscreen applications get their own desktop, through the same adapter.
    pub fullscreen: bool,
    /// Close surplus empty desktops, keeping one, through the same adapter.
    pub close_empty: bool,
}

#[derive(Default)]
pub struct Machine {
    chords: Chords,
    held: Option<u16>,
    capturing: bool,
    pending: Option<Action>,
    buffer: Vec<Key>,
}

impl Machine {
    pub fn new(chords: Chords) -> Self {
        Self {
            chords,
            ..Self::default()
        }
    }

    pub fn on_key(&mut self, key: Key, modifiers: Modifiers) -> Decision {
        if self.held == Some(key.vk) {
            if key.up {
                self.held = None;
            }
            return Decision::Swallow;
        }
        if key.vk == VK_LWIN || key.vk == VK_RWIN {
            // The system must always observe the logo key itself.
            return Decision::Pass;
        }
        if self.capturing {
            if self.buffer.len() == BUFFER_LIMIT {
                return Decision::Pass;
            }
            self.buffer.push(key);
            return Decision::Swallow;
        }
        if key.up || !modifiers.logo || modifiers.other {
            return Decision::Pass;
        }
        let action = match (key.vk, modifiers.shift) {
            (VK_SPACE, false) if self.chords.spotlight => Action::Spotlight,
            (VK_TAB, false) if self.chords.mission_control => Action::MissionControl,
            (digit @ 0x31..=0x39, false) if self.chords.workspaces => {
                Action::Workspace((digit - 0x31) as u8)
            }
            (digit @ 0x31..=0x39, true) if self.chords.moves => {
                Action::MoveTo((digit - 0x31) as u8)
            }
            _ => return Decision::Pass,
        };
        // A later chord while the logo key stays down replaces an earlier selection.
        self.held = Some(key.vk);
        self.capturing = action == Action::Spotlight;
        self.pending = Some(action);
        Decision::Trigger(action)
    }

    /// The action to run once, after the logo key is up.
    pub fn take_pending(&mut self) -> Option<Action> {
        self.pending.take()
    }

    pub fn awaiting_logo(&self) -> bool {
        self.pending.is_some()
    }

    #[cfg(test)]
    pub fn capturing(&self) -> bool {
        self.capturing
    }

    /// End the capture and return the keys to replay, in order.
    pub fn release(&mut self) -> Vec<Key> {
        self.capturing = false;
        self.pending = None;
        std::mem::take(&mut self.buffer)
    }
}

thread_local! {
    static MACHINE: RefCell<Machine> = RefCell::default();
    static THREAD: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

pub struct SpaceBridge {
    thread_id: u32,
    worker: Option<JoinHandle<()>>,
}

impl SpaceBridge {
    pub fn start(chords: Chords) -> io::Result<Self> {
        let (ready, started) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("monolith-shortcut-bridge".into())
            .spawn(move || unsafe {
                MACHINE.set(Machine::new(chords));
                run(ready)
            })?;
        match started.recv() {
            Ok(Ok(thread_id)) => Ok(Self {
                thread_id,
                worker: Some(worker),
            }),
            Ok(Err(error)) => {
                let _ = worker.join();
                Err(error)
            }
            Err(_) => Err(io::Error::other("The keyboard bridge thread stopped")),
        }
    }

    /// Spotlight has keyboard focus. Replay any keys typed since the chord.
    pub fn release(&self) {
        unsafe { PostThreadMessageW(self.thread_id, RELEASE, 0, 0) };
    }
}

impl Drop for SpaceBridge {
    fn drop(&mut self) {
        unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, 0, 0) };
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

unsafe fn run(ready: mpsc::Sender<io::Result<u32>>) {
    unsafe {
        // Create the message queue before reporting so posted messages are not lost.
        let mut msg = MSG::default();
        PeekMessageW(&mut msg, null_mut(), WM_USER, WM_USER, PM_NOREMOVE);
        let hook = SetWindowsHookExW(
            WH_KEYBOARD_LL,
            Some(capture),
            GetModuleHandleW(std::ptr::null()),
            0,
        );
        if hook.is_null() {
            let _ = ready.send(Err(io::Error::last_os_error()));
            return;
        }
        THREAD.set(GetCurrentThreadId());
        // Desktop moves and fullscreen workspaces run on their own thread, so slow
        // desktop work can never make Windows time out and remove this keyboard hook.
        let (moves, fullscreen, close_empty) = MACHINE.with_borrow(|machine| {
            let chords = machine.chords;
            (chords.moves, chords.fullscreen, chords.close_empty)
        });
        let desktop_worker = (moves || fullscreen || close_empty)
            .then(|| start_desktop_worker(fullscreen, close_empty))
            .flatten();
        let _ = ready.send(Ok(GetCurrentThreadId()));
        let mut logo_timer = 0;
        let mut logo_waited = 0;
        let mut capture_timer = 0;
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            match msg.message {
                BEGIN => {
                    if capture_timer == 0 {
                        capture_timer = SetTimer(null_mut(), 0, CAPTURE_LIMIT_MS, None);
                    }
                    if logo_timer == 0 {
                        logo_waited = 0;
                        logo_timer = SetTimer(null_mut(), 0, POLL_MS, None);
                    }
                }
                LOGO_UP if logo_timer != 0 => {
                    try_run_action(&mut logo_timer, &mut logo_waited);
                }
                WM_TIMER if msg.wParam == logo_timer && logo_timer != 0 => {
                    try_run_action(&mut logo_timer, &mut logo_waited);
                    if logo_timer != 0 && logo_waited >= LOGO_LIMIT_MS {
                        KillTimer(null_mut(), logo_timer);
                        logo_timer = 0;
                        replay(&MACHINE.with_borrow_mut(Machine::release));
                    }
                }
                RELEASE => {
                    finish(&mut capture_timer);
                }
                WM_TIMER if msg.wParam == capture_timer && capture_timer != 0 => {
                    // Prefer delivering late keys to the current window over losing them.
                    if !MACHINE.with_borrow(Machine::awaiting_logo) {
                        finish(&mut capture_timer);
                    }
                }
                _ => {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
        }
        UnhookWindowsHookEx(hook);
        // Stop the desktop worker; it releases the adapter and its hooks itself.
        if let Some(worker) = desktop_worker {
            PostThreadMessageW(DESKTOP_THREAD.load(Ordering::Acquire), WM_QUIT, 0, 0);
            let _ = worker.join();
        }
        if logo_timer != 0 {
            KillTimer(null_mut(), logo_timer);
        }
        finish(&mut capture_timer);
    }
}

/// Start the thread that owns the virtual desktop adapter. None when the adapter's
/// gates fail, in which case moves and fullscreen workspaces stay off.
fn start_desktop_worker(fullscreen: bool, close_empty: bool) -> Option<JoinHandle<()>> {
    let (ready, started) = mpsc::channel();
    let worker = thread::Builder::new()
        .name("monolith-desktops".into())
        .spawn(move || unsafe { run_desktop_worker(fullscreen, close_empty, ready) })
        .ok()?;
    match started.recv() {
        Ok(true) => Some(worker),
        _ => {
            let _ = worker.join();
            None
        }
    }
}

unsafe fn run_desktop_worker(fullscreen: bool, close_empty: bool, ready: mpsc::Sender<bool>) {
    use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};
    unsafe {
        let mut msg = MSG::default();
        PeekMessageW(&mut msg, null_mut(), WM_USER, WM_USER, PM_NOREMOVE);
        let apartment = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
        let Some(mover) = crate::virtual_desktops::Mover::connect() else {
            let _ = ready.send(false);
            if apartment {
                CoUninitialize();
            }
            return;
        };
        DESKTOP_THREAD.store(GetCurrentThreadId(), Ordering::Release);
        // Fullscreen detection follows foreground and window geometry changes.
        let mut hooks = Vec::new();
        let mut events = Vec::new();
        if fullscreen || close_empty {
            events.push(EVENT_SYSTEM_FOREGROUND);
        }
        if fullscreen {
            events.push(EVENT_OBJECT_LOCATIONCHANGE);
        }
        if close_empty {
            events.push(EVENT_OBJECT_DESTROY);
        }
        {
            for event in events {
                let hook = SetWinEventHook(
                    event,
                    event,
                    null_mut(),
                    Some(window_changed),
                    0,
                    0,
                    WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
                );
                if !hook.is_null() {
                    hooks.push(hook);
                }
            }
        }
        let _ = ready.send(true);
        let mut spaces = crate::workspaces::FullscreenSpaces::default();
        if close_empty {
            crate::workspaces::close_empty_desktops(&mover);
        }
        let mut timer = 0;
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            match msg.message {
                WINDOW_CHANGED if timer == 0 => {
                    timer = SetTimer(null_mut(), 0, 300, None);
                }
                WM_TIMER if msg.wParam == timer && timer != 0 => {
                    KillTimer(null_mut(), timer);
                    timer = 0;
                    if fullscreen {
                        spaces.check(&mover);
                    }
                    if close_empty {
                        crate::workspaces::close_empty_desktops(&mover);
                    }
                }
                MOVE_WINDOW => {
                    mover.move_window(
                        windows::Win32::Foundation::HWND(msg.lParam as _),
                        msg.wParam,
                    );
                }
                _ => {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
        }
        for hook in hooks {
            UnhookWinEvent(hook);
        }
        if timer != 0 {
            KillTimer(null_mut(), timer);
        }
        DESKTOP_THREAD.store(0, Ordering::Release);
        drop(spaces);
        drop(mover);
        if apartment {
            CoUninitialize();
        }
    }
}

unsafe fn finish(capture_timer: &mut usize) {
    unsafe {
        if *capture_timer != 0 {
            KillTimer(null_mut(), *capture_timer);
            *capture_timer = 0;
        }
        replay(&MACHINE.with_borrow_mut(Machine::release));
    }
}

unsafe fn try_run_action(timer: &mut usize, waited: &mut u32) {
    unsafe {
        *waited += POLL_MS;
        if pressed(VK_LWIN) || pressed(VK_RWIN) {
            return;
        }
        KillTimer(null_mut(), *timer);
        *timer = 0;
        let Some(action) = MACHINE.with_borrow_mut(Machine::take_pending) else {
            return;
        };
        // Windows owns the registered fallback chords and delivers WM_HOTKEY with
        // activation rights. Desktop switching needs no foreground change.
        let sent = match action {
            Action::Spotlight => workspaces::press(&[VK_LCONTROL, VK_LMENU, VK_SPACE], 1),
            Action::MissionControl => workspaces::press(&[VK_LCONTROL, VK_LMENU, VK_F24], 1),
            Action::Workspace(index) => {
                workspaces::switch_to(usize::from(index));
                true
            }
            Action::MoveTo(index) => {
                // The window that had focus when the chord was pressed keeps it now.
                // The desktop worker performs the move off this hook thread.
                PostThreadMessageW(
                    DESKTOP_THREAD.load(Ordering::Acquire),
                    MOVE_WINDOW,
                    usize::from(index),
                    GetForegroundWindow() as isize,
                );
                true
            }
        };
        if !sent {
            replay(&MACHINE.with_borrow_mut(Machine::release));
        }
    }
}

fn replay(keys: &[Key]) {
    if keys.is_empty() {
        return;
    }
    let events: Vec<_> = keys
        .iter()
        .map(|key| {
            let mut flags = 0;
            if key.up {
                flags |= KEYEVENTF_KEYUP;
            }
            if key.extended {
                flags |= KEYEVENTF_EXTENDEDKEY;
            }
            input(key.vk, key.scan, flags)
        })
        .collect();
    send(&events);
}

fn pressed(key: u16) -> bool {
    unsafe { GetAsyncKeyState(i32::from(key)) < 0 }
}

fn input(vk: u16, scan: u16, flags: u32) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: scan,
                dwFlags: flags,
                dwExtraInfo: INJECTED,
                ..Default::default()
            },
        },
    }
}

fn send(events: &[INPUT]) -> bool {
    let sent = unsafe {
        SendInput(
            events.len() as u32,
            events.as_ptr(),
            size_of::<INPUT>() as i32,
        )
    };
    sent as usize == events.len()
}

/// Foreground and geometry changes schedule a coalesced fullscreen check.
unsafe extern "system" fn window_changed(
    _: windows_sys::Win32::UI::Accessibility::HWINEVENTHOOK,
    _: u32,
    hwnd: windows_sys::Win32::Foundation::HWND,
    object: i32,
    child: i32,
    _: u32,
    _: u32,
) {
    if object == OBJID_WINDOW && child == 0 && !hwnd.is_null() {
        unsafe { PostThreadMessageW(DESKTOP_THREAD.load(Ordering::Acquire), WINDOW_CHANGED, 0, 0) };
    }
}

unsafe extern "system" fn capture(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        if code == HC_ACTION as i32 {
            let event = &*(lp as *const KBDLLHOOKSTRUCT);
            // Ignore our own replay and input injected by lower-integrity processes.
            if event.dwExtraInfo != INJECTED && event.flags & LLKHF_LOWER_IL_INJECTED == 0 {
                let key = Key {
                    vk: event.vkCode as u16,
                    scan: event.scanCode as u16,
                    up: event.flags & LLKHF_UP != 0,
                    extended: event.flags & LLKHF_EXTENDED != 0,
                };
                let modifiers = Modifiers {
                    logo: pressed(VK_LWIN) || pressed(VK_RWIN),
                    other: [VK_CONTROL, VK_MENU].into_iter().any(pressed),
                    shift: pressed(VK_SHIFT),
                };
                let (decision, logo_released) = MACHINE.with_borrow_mut(|machine| {
                    let decision = machine.on_key(key, modifiers);
                    let logo_released = key.up
                        && (key.vk == VK_LWIN || key.vk == VK_RWIN)
                        && machine.awaiting_logo();
                    (decision, logo_released)
                });
                if logo_released {
                    // The key state updates after this callback returns; poll from the loop.
                    PostThreadMessageW(THREAD.get(), LOGO_UP, 0, 0);
                }
                match decision {
                    Decision::Pass => {}
                    Decision::Swallow => return 1,
                    Decision::Trigger(_) => {
                        PostThreadMessageW(THREAD.get(), BEGIN, 0, 0);
                        send(&[
                            input(LOGO_MARKER, 0, 0),
                            input(LOGO_MARKER, 0, KEYEVENTF_KEYUP),
                        ]);
                        return 1;
                    }
                }
            }
        }
        CallNextHookEx(null_mut(), code, wp, lp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(vk: u16, up: bool) -> Key {
        Key {
            vk,
            scan: 0,
            up,
            extended: false,
        }
    }

    const LOGO: Modifiers = Modifiers {
        logo: true,
        other: false,
        shift: false,
    };
    const ALL: Chords = Chords {
        spotlight: true,
        mission_control: true,
        workspaces: true,
        moves: true,
        fullscreen: false,
        close_empty: false,
    };
    const NONE: Modifiers = Modifiers {
        logo: false,
        other: false,
        shift: false,
    };

    #[test]
    fn buffers_keys_typed_before_spotlight_has_focus() {
        let mut machine = Machine::new(ALL);
        assert_eq!(machine.on_key(key(VK_LWIN, false), NONE), Decision::Pass);
        assert_eq!(
            machine.on_key(key(VK_SPACE, false), LOGO),
            Decision::Trigger(Action::Spotlight)
        );
        assert_eq!(
            machine.on_key(key(VK_SPACE, false), LOGO),
            Decision::Swallow
        );
        assert_eq!(machine.on_key(key(VK_LWIN, true), LOGO), Decision::Pass);
        assert_eq!(machine.take_pending(), Some(Action::Spotlight));
        assert_eq!(machine.take_pending(), None);
        let typed = [key(0x43, false), key(VK_SPACE, true), key(0x43, true)];
        for event in typed {
            assert_eq!(machine.on_key(event, NONE), Decision::Swallow);
        }
        // Space up belongs to the activation chord and is not replayed.
        assert_eq!(machine.release(), vec![key(0x43, false), key(0x43, true)]);
        assert!(!machine.capturing());
        assert_eq!(machine.on_key(key(0x43, false), NONE), Decision::Pass);
    }

    #[test]
    fn ignores_space_without_the_logo_key_or_with_other_modifiers() {
        let mut machine = Machine::new(ALL);
        assert_eq!(machine.on_key(key(VK_SPACE, false), NONE), Decision::Pass);
        let shifted = Modifiers {
            logo: true,
            other: true,
            shift: false,
        };
        assert_eq!(
            machine.on_key(key(VK_SPACE, false), shifted),
            Decision::Pass
        );
        assert!(!machine.capturing());
    }

    #[test]
    fn passes_input_once_the_buffer_is_full() {
        let mut machine = Machine::new(ALL);
        machine.on_key(key(VK_SPACE, false), LOGO);
        for _ in 0..BUFFER_LIMIT {
            assert_eq!(machine.on_key(key(0x41, false), NONE), Decision::Swallow);
        }
        assert_eq!(machine.on_key(key(0x41, false), NONE), Decision::Pass);
        assert_eq!(machine.release().len(), BUFFER_LIMIT);
    }

    #[test]
    fn selects_workspaces_and_mission_control_without_buffering() {
        let mut machine = Machine::new(ALL);
        assert_eq!(
            machine.on_key(key(0x32, false), LOGO),
            Decision::Trigger(Action::Workspace(1))
        );
        // A second digit while the logo key is held replaces the selection.
        assert_eq!(machine.on_key(key(0x32, true), LOGO), Decision::Swallow);
        assert_eq!(
            machine.on_key(key(0x34, false), LOGO),
            Decision::Trigger(Action::Workspace(3))
        );
        assert_eq!(machine.on_key(key(0x34, true), LOGO), Decision::Swallow);
        assert_eq!(machine.on_key(key(0x41, false), NONE), Decision::Pass);
        assert_eq!(machine.take_pending(), Some(Action::Workspace(3)));
        assert_eq!(
            machine.on_key(key(VK_TAB, false), LOGO),
            Decision::Trigger(Action::MissionControl)
        );
        assert!(machine.release().is_empty());
    }

    #[test]
    fn leaves_disabled_and_modified_chords_to_windows() {
        let mut machine = Machine::new(Chords {
            spotlight: true,
            ..Chords::default()
        });
        assert_eq!(machine.on_key(key(0x31, false), LOGO), Decision::Pass);
        assert_eq!(machine.on_key(key(VK_TAB, false), LOGO), Decision::Pass);
        let shifted = Modifiers {
            logo: true,
            other: true,
            shift: false,
        };
        let mut all = Machine::new(ALL);
        assert_eq!(all.on_key(key(0x31, false), shifted), Decision::Pass);
    }

    #[test]
    fn moves_with_shift_and_switches_without_it() {
        let mut machine = Machine::new(ALL);
        let shift = Modifiers {
            logo: true,
            other: false,
            shift: true,
        };
        assert_eq!(
            machine.on_key(key(0x33, false), shift),
            Decision::Trigger(Action::MoveTo(2))
        );
        assert_eq!(machine.on_key(key(0x33, true), shift), Decision::Swallow);
        assert_eq!(machine.take_pending(), Some(Action::MoveTo(2)));
        // Shift with Space or Tab stays with Windows; moves need their own opt-in.
        assert_eq!(machine.on_key(key(VK_SPACE, false), shift), Decision::Pass);
        let mut without_moves = Machine::new(Chords {
            moves: false,
            ..ALL
        });
        assert_eq!(
            without_moves.on_key(key(0x33, false), shift),
            Decision::Pass
        );
    }
}
