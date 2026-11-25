//! Bounded Core Audio work that runs only while a Control Center is open.

use std::{
    io,
    ptr::null,
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, SyncSender},
    },
    thread,
};
use windows::{
    Win32::{
        Foundation::PROPERTYKEY,
        Media::Audio::{
            AUDIO_VOLUME_NOTIFICATION_DATA, DEVICE_STATE, EDataFlow, ERole,
            Endpoints::{
                IAudioEndpointVolume, IAudioEndpointVolumeCallback,
                IAudioEndpointVolumeCallback_Impl,
            },
            IMMDevice, IMMDeviceEnumerator, IMMNotificationClient, IMMNotificationClient_Impl,
            MMDeviceEnumerator, eMultimedia, eRender,
        },
        System::Com::{
            CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
            CoTaskMemFree, CoUninitialize,
        },
    },
    core::{PCWSTR, implement},
};
use windows_sys::Win32::{
    Foundation::HWND,
    UI::WindowsAndMessaging::{PostMessageW, WM_APP},
};

pub const READY: u32 = WM_APP + 8;

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Volume {
    pub endpoint: u64,
    pub level: u8,
    pub muted: bool,
}

#[derive(Default)]
struct State {
    volume: Option<Volume>,
    posted: bool,
    level: Option<(u64, u8)>,
    mute: Option<(u64, bool)>,
    closed: bool,
}

pub struct Audio {
    wake: SyncSender<()>,
    state: Arc<Mutex<State>>,
}

impl Audio {
    pub fn start(window: HWND) -> io::Result<Self> {
        let window = window as usize;
        let (wake, messages) = mpsc::sync_channel(1);
        let state = Arc::new(Mutex::new(State::default()));
        let worker_state = Arc::clone(&state);
        let notifications = wake.clone();
        let _ = wake.try_send(());
        thread::Builder::new()
            .name("monolith-audio".into())
            .spawn(move || {
                if serve(window as HWND, &worker_state, notifications, messages).is_err() {
                    publish(window as HWND, &worker_state, None);
                }
            })?;
        Ok(Self { wake, state })
    }

    pub fn take_update(&self) -> Option<Volume> {
        let mut shared = self.state.lock().unwrap();
        shared.posted = false;
        shared.volume
    }

    pub fn set_level(&self, endpoint: u64, level: u8) {
        self.state.lock().unwrap().level = Some((endpoint, level));
        let _ = self.wake.try_send(());
    }

    pub fn set_mute(&self, endpoint: u64, mute: bool) {
        self.state.lock().unwrap().mute = Some((endpoint, mute));
        let _ = self.wake.try_send(());
    }
}

impl Drop for Audio {
    fn drop(&mut self) {
        self.state.lock().unwrap().closed = true;
        let _ = self.wake.try_send(());
    }
}

fn publish(window: HWND, state: &Mutex<State>, volume: Option<Volume>) {
    let mut shared = state.lock().unwrap();
    if !shared.closed {
        shared.volume = volume;
        if !shared.posted {
            shared.posted = unsafe { PostMessageW(window, READY, 0, 0) } != 0;
        }
    }
}

struct ComApartment;

impl Drop for ComApartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

#[implement(IAudioEndpointVolumeCallback, IMMNotificationClient)]
struct Notify(SyncSender<()>);

impl IAudioEndpointVolumeCallback_Impl for Notify_Impl {
    fn OnNotify(&self, _: *mut AUDIO_VOLUME_NOTIFICATION_DATA) -> windows::core::Result<()> {
        let _ = self.0.try_send(());
        Ok(())
    }
}

impl IMMNotificationClient_Impl for Notify_Impl {
    fn OnDefaultDeviceChanged(
        &self,
        flow: EDataFlow,
        role: ERole,
        _: &PCWSTR,
    ) -> windows::core::Result<()> {
        if flow == eRender && role == eMultimedia {
            let _ = self.0.try_send(());
        }
        Ok(())
    }

    fn OnDeviceStateChanged(&self, _: &PCWSTR, _: DEVICE_STATE) -> windows::core::Result<()> {
        let _ = self.0.try_send(());
        Ok(())
    }

    fn OnDeviceAdded(&self, _: &PCWSTR) -> windows::core::Result<()> {
        Ok(())
    }
    fn OnDeviceRemoved(&self, _: &PCWSTR) -> windows::core::Result<()> {
        Ok(())
    }
    fn OnPropertyValueChanged(&self, _: &PCWSTR, _: &PROPERTYKEY) -> windows::core::Result<()> {
        Ok(())
    }
}

struct Endpoint {
    id: Vec<u16>,
    volume: IAudioEndpointVolume,
    notify: IAudioEndpointVolumeCallback,
}

impl Endpoint {
    fn open(
        device: IMMDevice,
        id: Vec<u16>,
        notify: IAudioEndpointVolumeCallback,
    ) -> windows::core::Result<Self> {
        unsafe {
            let volume: IAudioEndpointVolume = device.Activate(CLSCTX_INPROC_SERVER, None)?;
            volume.RegisterControlChangeNotify(&notify)?;
            Ok(Self { id, volume, notify })
        }
    }

    fn read(&self, endpoint: u64) -> windows::core::Result<Volume> {
        unsafe {
            let scalar = self.volume.GetMasterVolumeLevelScalar()?;
            let muted = self.volume.GetMute()?.as_bool();
            Ok(Volume {
                endpoint,
                level: (scalar.clamp(0.0, 1.0) * 100.0).round() as u8,
                muted,
            })
        }
    }
}

impl Drop for Endpoint {
    fn drop(&mut self) {
        let _ = unsafe { self.volume.UnregisterControlChangeNotify(&self.notify) };
    }
}

fn output(enumerator: &IMMDeviceEnumerator) -> windows::core::Result<(IMMDevice, Vec<u16>)> {
    unsafe {
        let device = enumerator.GetDefaultAudioEndpoint(eRender, eMultimedia)?;
        let id = device.GetId()?;
        let copy = id.as_wide().to_vec();
        CoTaskMemFree(Some(id.0.cast()));
        Ok((device, copy))
    }
}

fn serve(
    window: HWND,
    state: &Mutex<State>,
    wake: SyncSender<()>,
    messages: Receiver<()>,
) -> windows::core::Result<()> {
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok()? };
    let _apartment = ComApartment;
    let enumerator: IMMDeviceEnumerator =
        unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_INPROC_SERVER)? };
    let notify: IMMNotificationClient = Notify(wake.clone()).into();
    let volume_notify: IAudioEndpointVolumeCallback = Notify(wake).into();
    unsafe { enumerator.RegisterEndpointNotificationCallback(&notify)? };
    let mut endpoint: Option<Endpoint> = None;
    let mut generation = 0;
    while messages.recv().is_ok() {
        let (level, mute) = {
            let mut shared = state.lock().unwrap();
            if shared.closed {
                break;
            }
            (shared.level.take(), shared.mute.take())
        };
        match output(&enumerator) {
            Ok((device, id)) if endpoint.as_ref().is_none_or(|active| active.id != id) => {
                generation += 1;
                endpoint = Endpoint::open(device, id, volume_notify.clone()).ok();
            }
            Err(_) => endpoint = None,
            _ => {}
        }
        let volume = endpoint.as_ref().and_then(|active| {
            unsafe {
                if let Some((_, value)) = level.filter(|(id, _)| *id == generation) {
                    let _ = active
                        .volume
                        .SetMasterVolumeLevelScalar(f32::from(value.min(100)) / 100.0, null());
                }
                if let Some((_, value)) = mute.filter(|(id, _)| *id == generation) {
                    let _ = active.volume.SetMute(value, null());
                }
            }
            active.read(generation).ok()
        });
        publish(window, state, volume);
    }
    drop(endpoint);
    let _ = unsafe { enumerator.UnregisterEndpointNotificationCallback(&notify) };
    Ok(())
}
