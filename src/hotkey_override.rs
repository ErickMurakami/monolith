//! Optional interception of the reserved Win+Space chord on the GUI thread.

use std::{cell::Cell, mem::size_of, ptr::null_mut};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, WPARAM},
    System::LibraryLoader::GetModuleHandleW,
    UI::{Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
};

const DELIVER: u32 = WM_APP + 16;
const RETRY: usize = 0x3a01;
const INJECTED: usize = 0x4d4f_4e4f;
// Give the system a harmless key to pair with the logo key instead of opening Start.
const LOGO_MARKER: u16 = 0xff;

#[derive(Clone, Copy, Default)]
struct Pending {
    space_down: bool,
    waiting_for_logo: bool,
    source: Option<HWND>,
    retries: u8,
}

thread_local! {
    static WINDOW: Cell<HWND> = const { Cell::new(null_mut()) };
    static PENDING: Cell<Pending> = Cell::default();
}

pub struct SpaceOverride {
    hook: HHOOK,
    window: HWND,
}

impl SpaceOverride {
    pub fn start(window: HWND) -> Option<Self> {
        let hook = unsafe {
            SetWindowsHookExW(
                WH_KEYBOARD_LL,
                Some(capture),
                GetModuleHandleW(std::ptr::null()),
                0,
            )
        };
        if hook.is_null() {
            return None;
        }
        WINDOW.set(window);
        Some(Self { hook, window })
    }

    pub fn dispatch(&self, message: u32, param: WPARAM) -> bool {
        if message != DELIVER && !(message == WM_TIMER && param == RETRY) {
            return false;
        }
        let mut pending = PENDING.get();
        let logo_held = pressed(VK_LWIN) || pressed(VK_RWIN);
        if logo_held && pending.retries != 0 {
            pending.retries -= 1;
            PENDING.set(pending);
            unsafe { SetTimer(self.window, RETRY, 10, None) };
            return true;
        }
        unsafe { KillTimer(self.window, RETRY) };
        let source = pending.source.take();
        PENDING.set(pending);
        if !logo_held
            && source == Some(unsafe { GetForegroundWindow() })
            && ![VK_CONTROL, VK_MENU, VK_SHIFT, VK_SPACE]
                .into_iter()
                .any(pressed)
        {
            // The registered fallback lets Windows deliver a real WM_HOTKEY with activation rights.
            send(&[
                input(VK_LCONTROL, 0),
                input(VK_LMENU, 0),
                input(VK_SPACE, 0),
                input(VK_SPACE, KEYEVENTF_KEYUP),
                input(VK_LMENU, KEYEVENTF_KEYUP),
                input(VK_LCONTROL, KEYEVENTF_KEYUP),
            ]);
        }
        true
    }
}

impl Drop for SpaceOverride {
    fn drop(&mut self) {
        WINDOW.set(null_mut());
        PENDING.set(Pending::default());
        unsafe {
            KillTimer(self.window, RETRY);
            UnhookWindowsHookEx(self.hook);
        }
    }
}

fn pressed(key: u16) -> bool {
    unsafe { GetAsyncKeyState(i32::from(key)) < 0 }
}

fn input(key: u16, flags: u32) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: key,
                dwFlags: flags,
                dwExtraInfo: INJECTED,
                ..Default::default()
            },
        },
    }
}

fn send(events: &[INPUT]) -> bool {
    let size = size_of::<INPUT>() as i32;
    let sent = unsafe { SendInput(events.len() as u32, events.as_ptr(), size) } as usize;
    if sent != events.len() && sent > 0 {
        let down = events.len() / 2;
        let first_up = (events.len() - sent.min(down)).max(sent);
        let release = &events[first_up..];
        unsafe { SendInput(release.len() as u32, release.as_ptr(), size) };
    }
    sent == events.len()
}

unsafe extern "system" fn capture(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        if code >= 0 {
            let event = &*(lp as *const KBDLLHOOKSTRUCT);
            if event.dwExtraInfo != INJECTED && event.flags & LLKHF_LOWER_IL_INJECTED == 0 {
                let key = event.vkCode as u16;
                let key_up = event.flags & LLKHF_UP != 0;
                let mut pending = PENDING.get();
                if key_up && matches!(key, VK_LWIN | VK_RWIN) && pending.waiting_for_logo {
                    pending.waiting_for_logo = false;
                    pending.retries = 5;
                    PENDING.set(pending);
                    PostMessageW(WINDOW.get(), DELIVER, 0, 0);
                } else if key == VK_SPACE && pending.space_down {
                    if key_up {
                        pending.space_down = false;
                        PENDING.set(pending);
                    }
                    return 1;
                } else if key == VK_SPACE
                    && !key_up
                    && !pressed(VK_SPACE)
                    && (pressed(VK_LWIN) || pressed(VK_RWIN))
                    && ![VK_CONTROL, VK_MENU, VK_SHIFT].into_iter().any(pressed)
                {
                    let source = GetForegroundWindow();
                    if send(&[input(LOGO_MARKER, 0), input(LOGO_MARKER, KEYEVENTF_KEYUP)]) {
                        pending.space_down = true;
                        pending.waiting_for_logo = true;
                        pending.source = Some(source);
                        PENDING.set(pending);
                        return 1;
                    }
                }
            }
        }
        CallNextHookEx(null_mut(), code, wp, lp)
    }
}
