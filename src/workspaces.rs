//! Native Windows virtual desktops as ordered Monolith workspaces.
//!
//! Windows publishes no API for desktop order or the current desktop. Explorer
//! persists both under the user's registry hive, which PowerToys FancyZones also
//! reads. Monolith only reads these values and switches through the documented
//! Win+Ctrl+Left/Right shortcuts, so Explorer remains the owner of desktop state.

use crate::space_bridge::INJECTED;
use std::{io, mem::size_of, ptr::null_mut};
use windows_sys::Win32::{
    Foundation::ERROR_SUCCESS,
    System::{
        Registry::{HKEY_CURRENT_USER, RRF_RT_REG_BINARY, RegGetValueW},
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
    #[ignore = "reads the interactive user's Explorer state"]
    fn reads_the_live_desktop_order() {
        let workspaces = read().unwrap();
        assert!(!workspaces.ids.is_empty());
        assert!(workspaces.current.is_some());
    }
}
