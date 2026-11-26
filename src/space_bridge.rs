//! Opt-in Win+Space activation for Spotlight on builds where Windows reserves the chord.
//!
//! A low-level keyboard hook on a dedicated thread swallows Space while the logo key
//! is held and pairs the logo key with a harmless marker so Start does not open. After
//! the logo key is released, it sends the registered Ctrl+Alt+Space chord so Windows
//! delivers a real `WM_HOTKEY` with foreground rights. Keys typed between the chord
//! and Spotlight taking focus are held in a small buffer and replayed as native input
//! once Spotlight reports focus, so dead keys and modifiers keep their layout meaning.
//! The buffer exists only during that window and is never stored or logged.

use std::{
    cell::RefCell,
    io,
    mem::size_of,
    ptr::null_mut,
    sync::mpsc,
    thread::{self, JoinHandle},
};
use windows_sys::Win32::{
    Foundation::{LPARAM, LRESULT, WPARAM},
    System::{LibraryLoader::GetModuleHandleW, Threading::GetCurrentThreadId},
    UI::{Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
};

/// Tags Monolith's own synthetic input so the hook passes it through.
const INJECTED: usize = 0x4d4f_4e4f;
// Give the system a harmless key to pair with the logo key instead of opening Start.
const LOGO_MARKER: u16 = 0xff;
const RELEASE: u32 = WM_APP + 1;
const LOGO_UP: u32 = WM_APP + 2;
const BEGIN: u32 = WM_APP + 3;
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
}

#[derive(Debug, PartialEq, Eq)]
pub enum Decision {
    Pass,
    Swallow,
    /// Swallow Space and begin an activation.
    Trigger,
}

#[derive(Default)]
pub struct Machine {
    space_held: bool,
    capturing: bool,
    awaiting_logo: bool,
    buffer: Vec<Key>,
}

impl Machine {
    pub fn on_key(&mut self, key: Key, modifiers: Modifiers) -> Decision {
        if key.vk == VK_SPACE && self.space_held {
            if key.up {
                self.space_held = false;
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
        if key.vk == VK_SPACE && !key.up && modifiers.logo && !modifiers.other {
            self.space_held = true;
            self.capturing = true;
            self.awaiting_logo = true;
            return Decision::Trigger;
        }
        Decision::Pass
    }

    /// The bridge chord should be sent once, after the logo key is up.
    pub fn take_logo_release(&mut self) -> bool {
        std::mem::take(&mut self.awaiting_logo)
    }

    pub fn awaiting_logo(&self) -> bool {
        self.awaiting_logo
    }

    #[cfg(test)]
    pub fn capturing(&self) -> bool {
        self.capturing
    }

    /// End the capture and return the keys to replay, in order.
    pub fn release(&mut self) -> Vec<Key> {
        self.capturing = false;
        self.awaiting_logo = false;
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
    pub fn start() -> io::Result<Self> {
        let (ready, started) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("monolith-space-bridge".into())
            .spawn(move || unsafe { run(ready) })?;
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
                    try_send_chord(&mut logo_timer, &mut logo_waited);
                }
                WM_TIMER if msg.wParam == logo_timer && logo_timer != 0 => {
                    try_send_chord(&mut logo_timer, &mut logo_waited);
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
        if logo_timer != 0 {
            KillTimer(null_mut(), logo_timer);
        }
        finish(&mut capture_timer);
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

unsafe fn try_send_chord(timer: &mut usize, waited: &mut u32) {
    unsafe {
        *waited += POLL_MS;
        if pressed(VK_LWIN) || pressed(VK_RWIN) {
            return;
        }
        KillTimer(null_mut(), *timer);
        *timer = 0;
        if !MACHINE.with_borrow_mut(Machine::take_logo_release) {
            return;
        }
        // Windows owns the registered chord and delivers WM_HOTKEY with activation rights.
        let chord = [
            input(VK_LCONTROL, 0, 0),
            input(VK_LMENU, 0, 0),
            input(VK_SPACE, 0, 0),
            input(VK_SPACE, 0, KEYEVENTF_KEYUP),
            input(VK_LMENU, 0, KEYEVENTF_KEYUP),
            input(VK_LCONTROL, 0, KEYEVENTF_KEYUP),
        ];
        if !send(&chord) {
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
                    other: [VK_CONTROL, VK_MENU, VK_SHIFT].into_iter().any(pressed),
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
                    Decision::Trigger => {
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
    };
    const NONE: Modifiers = Modifiers {
        logo: false,
        other: false,
    };

    #[test]
    fn buffers_keys_typed_before_spotlight_has_focus() {
        let mut machine = Machine::default();
        assert_eq!(machine.on_key(key(VK_LWIN, false), NONE), Decision::Pass);
        assert_eq!(
            machine.on_key(key(VK_SPACE, false), LOGO),
            Decision::Trigger
        );
        assert_eq!(
            machine.on_key(key(VK_SPACE, false), LOGO),
            Decision::Swallow
        );
        assert_eq!(machine.on_key(key(VK_LWIN, true), LOGO), Decision::Pass);
        assert!(machine.take_logo_release());
        assert!(!machine.take_logo_release());
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
        let mut machine = Machine::default();
        assert_eq!(machine.on_key(key(VK_SPACE, false), NONE), Decision::Pass);
        let shifted = Modifiers {
            logo: true,
            other: true,
        };
        assert_eq!(
            machine.on_key(key(VK_SPACE, false), shifted),
            Decision::Pass
        );
        assert!(!machine.capturing());
    }

    #[test]
    fn passes_input_once_the_buffer_is_full() {
        let mut machine = Machine::default();
        machine.on_key(key(VK_SPACE, false), LOGO);
        for _ in 0..BUFFER_LIMIT {
            assert_eq!(machine.on_key(key(0x41, false), NONE), Decision::Swallow);
        }
        assert_eq!(machine.on_key(key(0x41, false), NONE), Decision::Pass);
        assert_eq!(machine.release().len(), BUFFER_LIMIT);
    }
}
