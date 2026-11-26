//! Native global shortcut registration for Spotlight.

use crate::config::Shortcut;
use std::io;
use windows_sys::Win32::{
    Foundation::{ERROR_HOTKEY_ALREADY_REGISTERED, HWND},
    UI::Input::KeyboardAndMouse::{
        MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_WIN, RegisterHotKey, UnregisterHotKey, VK_SPACE,
    },
};

pub const ID: i32 = 1;

pub enum Notice {
    Alternate,
    Unavailable(i32),
}

pub struct Hotkey {
    window: HWND,
    registered: bool,
    pub notice: Option<Notice>,
}

impl Hotkey {
    pub fn bind(window: HWND, shortcut: Shortcut) -> Self {
        let mut binding = Self {
            window,
            registered: false,
            notice: None,
        };
        let preferred = match shortcut {
            Shortcut::Disabled => return binding,
            Shortcut::CtrlAltSpace => MOD_CONTROL | MOD_ALT,
            Shortcut::WinSpace => MOD_WIN,
        };
        match register(window, preferred) {
            Ok(()) => binding.registered = true,
            Err(error)
                if preferred == MOD_WIN && error == ERROR_HOTKEY_ALREADY_REGISTERED as i32 =>
            {
                match register(window, MOD_CONTROL | MOD_ALT) {
                    Ok(()) => {
                        binding.registered = true;
                        binding.notice = Some(Notice::Alternate);
                    }
                    Err(error) => binding.notice = Some(Notice::Unavailable(error)),
                }
            }
            Err(error) => binding.notice = Some(Notice::Unavailable(error)),
        }
        binding
    }

    pub fn available(&self) -> bool {
        self.registered
    }
}

impl Drop for Hotkey {
    fn drop(&mut self) {
        if self.registered {
            unsafe { UnregisterHotKey(self.window, ID) };
        }
    }
}

fn register(window: HWND, modifiers: u32) -> Result<(), i32> {
    if unsafe { RegisterHotKey(window, ID, modifiers | MOD_NOREPEAT, VK_SPACE.into()) } != 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error().raw_os_error().unwrap_or(0))
    }
}
