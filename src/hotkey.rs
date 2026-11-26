//! Native global shortcut registration for Spotlight and Mission Control.

use crate::{
    config::Shortcut,
    space_bridge::{Chords, SpaceBridge},
};
use std::io;
use windows_sys::Win32::{
    Foundation::{ERROR_HOTKEY_ALREADY_REGISTERED, HWND},
    UI::Input::KeyboardAndMouse::{
        MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_WIN, RegisterHotKey, UnregisterHotKey, VK_F24,
        VK_SPACE,
    },
};

pub const ID: i32 = 1;
/// Ctrl+Alt+F24 has no physical key. The bridge sends it for Win+Tab.
pub const MISSION_ID: i32 = 2;

pub enum Notice {
    Alternate,
    Unavailable(i32),
}

pub struct Hotkey {
    window: HWND,
    registered: bool,
    mission_registered: bool,
    bridge: Option<SpaceBridge>,
    pub notice: Option<Notice>,
}

impl Hotkey {
    /// `win_space_bridge` and `workspace_shortcuts` opt into the experimental keyboard
    /// bridge for chords that Windows reserves.
    pub fn bind(
        window: HWND,
        shortcut: Shortcut,
        win_space_bridge: bool,
        workspace_shortcuts: bool,
    ) -> Self {
        let mut binding = Self {
            window,
            registered: false,
            mission_registered: false,
            bridge: None,
            notice: None,
        };
        let mut chords = Chords {
            workspaces: workspace_shortcuts,
            ..Chords::default()
        };
        let preferred = match shortcut {
            Shortcut::Disabled => None,
            Shortcut::CtrlAltSpace => Some(MOD_CONTROL | MOD_ALT),
            Shortcut::WinSpace => Some(MOD_WIN),
        };
        if let Some(preferred) = preferred {
            match register(window, ID, preferred, VK_SPACE) {
                Ok(()) => binding.registered = true,
                Err(error)
                    if preferred == MOD_WIN && error == ERROR_HOTKEY_ALREADY_REGISTERED as i32 =>
                {
                    match register(window, ID, MOD_CONTROL | MOD_ALT, VK_SPACE) {
                        Ok(()) => {
                            binding.registered = true;
                            chords.spotlight = win_space_bridge;
                            if !win_space_bridge {
                                binding.notice = Some(Notice::Alternate);
                            }
                        }
                        Err(error) => binding.notice = Some(Notice::Unavailable(error)),
                    }
                }
                Err(error) => binding.notice = Some(Notice::Unavailable(error)),
            }
        }
        if workspace_shortcuts {
            binding.mission_registered =
                register(window, MISSION_ID, MOD_CONTROL | MOD_ALT, VK_F24).is_ok();
            chords.mission_control = binding.mission_registered;
        }
        if chords.spotlight || chords.workspaces {
            binding.bridge = SpaceBridge::start(chords).ok();
            if binding.bridge.is_none() && chords.spotlight {
                binding.notice = Some(Notice::Alternate);
            }
        }
        binding
    }

    pub fn available(&self) -> bool {
        self.registered
    }

    /// Spotlight has handled the shortcut. Replay keys typed during a Win+Space bridge.
    pub fn handled(&self) {
        if let Some(bridge) = &self.bridge {
            bridge.release();
        }
    }
}

impl Drop for Hotkey {
    fn drop(&mut self) {
        // Remove the keyboard hook before releasing the chords it sends.
        drop(self.bridge.take());
        if self.registered {
            unsafe { UnregisterHotKey(self.window, ID) };
        }
        if self.mission_registered {
            unsafe { UnregisterHotKey(self.window, MISSION_ID) };
        }
    }
}

fn register(window: HWND, id: i32, modifiers: u32, key: u16) -> Result<(), i32> {
    if unsafe { RegisterHotKey(window, id, modifiers | MOD_NOREPEAT, key.into()) } != 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error().raw_os_error().unwrap_or(0))
    }
}
