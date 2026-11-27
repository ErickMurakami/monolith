//! Wi-Fi and Bluetooth radio state and switching through `Windows.Devices.Radios`.
//!
//! The worker requests radio access once, mirrors each radio's state through its
//! `StateChanged` event and applies toggle requests. Windows enforces airplane mode
//! and policy: a radio it reports as disabled is shown as unavailable.

use std::{
    io,
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, SyncSender},
    },
    thread::{self, JoinHandle},
};
use windows::{
    Devices::Radios::{Radio, RadioAccessStatus, RadioKind, RadioState},
    Foundation::TypedEventHandler,
    Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize},
};
use windows_sys::Win32::{
    Foundation::HWND,
    UI::WindowsAndMessaging::{PostMessageW, WM_APP},
};

/// Posted to the panel whenever a radio state changes.
pub const READY: u32 = WM_APP + 34;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    WiFi,
    Bluetooth,
}

/// `None` when the radio is absent, disabled by Windows or access is denied.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct States {
    pub wifi: Option<bool>,
    pub bluetooth: Option<bool>,
}

impl States {
    pub fn get(self, kind: Kind) -> Option<bool> {
        match kind {
            Kind::WiFi => self.wifi,
            Kind::Bluetooth => self.bluetooth,
        }
    }

    fn set(&mut self, kind: Kind, state: Option<bool>) {
        match kind {
            Kind::WiFi => self.wifi = state,
            Kind::Bluetooth => self.bluetooth = state,
        }
    }
}

pub struct Radios {
    states: Arc<Mutex<States>>,
    commands: Option<SyncSender<Kind>>,
    worker: Option<JoinHandle<()>>,
}

impl Radios {
    pub fn start(window: HWND) -> io::Result<Self> {
        let states = Arc::new(Mutex::new(States::default()));
        let (commands, requests) = mpsc::sync_channel(2);
        let shared = Arc::clone(&states);
        let target = window as usize;
        let worker = thread::Builder::new()
            .name("monolith-radios".into())
            .spawn(move || run(requests, shared, target))?;
        Ok(Self {
            states,
            commands: Some(commands),
            worker: Some(worker),
        })
    }

    pub fn states(&self) -> States {
        self.states.lock().map(|states| *states).unwrap_or_default()
    }

    /// Switch a radio that Windows reports as available.
    pub fn toggle(&self, kind: Kind) {
        if let Some(commands) = &self.commands {
            let _ = commands.try_send(kind);
        }
    }
}

impl Drop for Radios {
    fn drop(&mut self) {
        // Closing the channel ends the worker, which removes its event handlers.
        drop(self.commands.take());
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn kind_of(radio: &Radio) -> Option<Kind> {
    match radio.Kind().ok()? {
        RadioKind::WiFi => Some(Kind::WiFi),
        RadioKind::Bluetooth => Some(Kind::Bluetooth),
        _ => None,
    }
}

fn state_of(radio: &Radio) -> Option<bool> {
    match radio.State().ok()? {
        RadioState::On => Some(true),
        RadioState::Off => Some(false),
        _ => None,
    }
}

fn run(requests: Receiver<Kind>, states: Arc<Mutex<States>>, target: usize) {
    if unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_err() {
        return;
    }
    let notify = move || unsafe {
        PostMessageW(target as HWND, READY, 0, 0);
    };
    let allowed = Radio::RequestAccessAsync()
        .and_then(|operation| operation.join())
        .ok()
        == Some(RadioAccessStatus::Allowed);
    let radios: Vec<(Kind, Radio)> = if allowed {
        Radio::GetRadiosAsync()
            .and_then(|operation| operation.join())
            .map(|list| {
                list.into_iter()
                    .filter_map(|radio| Some((kind_of(&radio)?, radio)))
                    .collect()
            })
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let mut tokens = Vec::with_capacity(radios.len());
    for (kind, radio) in &radios {
        if let Ok(mut shared) = states.lock() {
            shared.set(*kind, state_of(radio));
        }
        let shared = Arc::clone(&states);
        let kind = *kind;
        let handler = TypedEventHandler::new(move |radio: windows::core::Ref<Radio>, _| {
            if let Some(radio) = radio.as_ref()
                && let Ok(mut states) = shared.lock()
            {
                states.set(kind, state_of(radio));
            }
            notify();
            Ok(())
        });
        if let Ok(token) = radio.StateChanged(&handler) {
            tokens.push((radio.clone(), token));
        }
    }
    notify();
    while let Ok(kind) = requests.recv() {
        let Some((_, radio)) = radios.iter().find(|(candidate, _)| *candidate == kind) else {
            continue;
        };
        let target_state = match state_of(radio) {
            Some(true) => RadioState::Off,
            Some(false) => RadioState::On,
            None => continue,
        };
        if let Ok(operation) = radio.SetStateAsync(target_state) {
            let _ = operation.join();
        }
    }
    for (radio, token) in tokens {
        let _ = radio.RemoveStateChanged(token);
    }
    drop(radios);
    unsafe { CoUninitialize() };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_each_kind_independently() {
        let mut states = States::default();
        states.set(Kind::WiFi, Some(true));
        assert_eq!(states.get(Kind::WiFi), Some(true));
        assert_eq!(states.get(Kind::Bluetooth), None);
        states.set(Kind::Bluetooth, Some(false));
        assert_eq!(states.get(Kind::Bluetooth), Some(false));
    }
}
