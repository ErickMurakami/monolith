//! Built-in display brightness through the documented WMI monitor classes.
//!
//! `WmiMonitorBrightness` reports the internal panel level and
//! `WmiMonitorBrightnessMethods.WmiSetBrightness` changes it. External monitors
//! do not expose these classes, so the control stays hidden for them. All WMI
//! calls run on a worker thread; slider movements are coalesced so only the latest
//! requested level is applied.

use std::{
    io,
    ptr::null_mut,
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, SyncSender},
    },
    thread::{self, JoinHandle},
};
use windows::{
    Win32::System::{
        Com::{
            CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
            CoSetProxyBlanket, CoUninitialize, EOAC_NONE, RPC_C_AUTHN_LEVEL_CALL,
            RPC_C_IMP_LEVEL_IMPERSONATE,
        },
        Variant::{VARENUM, VARIANT, VT_BSTR, VT_I4, VT_UI1, VariantClear},
        Wmi::{
            IWbemClassObject, IWbemLocator, IWbemServices, WBEM_FLAG_FORWARD_ONLY,
            WBEM_FLAG_RETURN_IMMEDIATELY, WBEM_FLAG_RETURN_WBEM_COMPLETE, WBEM_GENERIC_FLAG_TYPE,
            WBEM_INFINITE, WbemLocator,
        },
    },
    core::{BSTR, PCWSTR, w},
};
use windows_sys::Win32::{
    Foundation::HWND,
    UI::WindowsAndMessaging::{PostMessageW, WM_APP},
};

/// Posted with the current level (0-100) in `wParam`, or `UNAVAILABLE`.
pub const READY: u32 = WM_APP + 33;
pub const UNAVAILABLE: usize = usize::MAX;

pub struct Brightness {
    pending: Arc<Mutex<Option<u8>>>,
    wake: Option<SyncSender<()>>,
    worker: Option<JoinHandle<()>>,
}

impl Brightness {
    pub fn start(window: HWND) -> io::Result<Self> {
        let pending = Arc::new(Mutex::new(None));
        let (wake, signal) = mpsc::sync_channel(1);
        let shared = Arc::clone(&pending);
        let target = window as usize;
        let worker = thread::Builder::new()
            .name("monolith-brightness".into())
            .spawn(move || run(signal, shared, target))?;
        Ok(Self {
            pending,
            wake: Some(wake),
            worker: Some(worker),
        })
    }

    /// Request a level; a newer request replaces one not yet applied.
    pub fn set(&self, level: u8) {
        if let Ok(mut pending) = self.pending.lock() {
            *pending = Some(level.min(100));
        }
        if let Some(wake) = &self.wake {
            let _ = wake.try_send(());
        }
    }
}

impl Drop for Brightness {
    fn drop(&mut self) {
        // Closing the channel ends the worker after any call in progress.
        drop(self.wake.take());
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn run(signal: Receiver<()>, pending: Arc<Mutex<Option<u8>>>, target: usize) {
    if unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_err() {
        return;
    }
    let services = connect().ok();
    let post = |level: Option<u8>| unsafe {
        PostMessageW(
            target as HWND,
            READY,
            level.map_or(UNAVAILABLE, usize::from),
            0,
        );
    };
    post(services.as_ref().and_then(read));
    while signal.recv().is_ok() {
        let Some(level) = pending.lock().ok().and_then(|mut pending| pending.take()) else {
            continue;
        };
        if let Some(services) = &services
            && write(services, level).is_ok()
        {
            post(read(services));
        }
    }
    drop(services);
    unsafe { CoUninitialize() };
}

fn connect() -> windows::core::Result<IWbemServices> {
    unsafe {
        let locator: IWbemLocator = CoCreateInstance(&WbemLocator, None, CLSCTX_INPROC_SERVER)?;
        let empty = BSTR::new();
        let services = locator.ConnectServer(
            &BSTR::from(r"ROOT\WMI"),
            &empty,
            &empty,
            &empty,
            0,
            &empty,
            None,
        )?;
        // NTLM authentication service, no authorisation service, caller impersonation.
        CoSetProxyBlanket(
            &services,
            10,
            0,
            PCWSTR::null(),
            RPC_C_AUTHN_LEVEL_CALL,
            RPC_C_IMP_LEVEL_IMPERSONATE,
            None,
            EOAC_NONE,
        )?;
        Ok(services)
    }
}

fn first(services: &IWbemServices, query: &str) -> Option<IWbemClassObject> {
    unsafe {
        let objects = services
            .ExecQuery(
                &BSTR::from("WQL"),
                &BSTR::from(query),
                WBEM_FLAG_FORWARD_ONLY | WBEM_FLAG_RETURN_IMMEDIATELY,
                None,
            )
            .ok()?;
        let mut found = [None];
        let mut returned = 0;
        objects
            .Next(WBEM_INFINITE, &mut found, &mut returned)
            .ok()
            .ok()?;
        found[0].take()
    }
}

fn read(services: &IWbemServices) -> Option<u8> {
    let monitor = first(
        services,
        "SELECT CurrentBrightness FROM WmiMonitorBrightness WHERE Active = TRUE",
    )?;
    let mut value = VARIANT::default();
    unsafe {
        monitor
            .Get(w!("CurrentBrightness"), 0, &mut value, None, None)
            .ok()?;
        let inner = &value.Anonymous.Anonymous;
        let level = (inner.vt == VT_UI1).then(|| inner.Anonymous.bVal);
        let _ = VariantClear(&mut value);
        level.filter(|level| *level <= 100)
    }
}

fn variant(kind: VARENUM, fill: impl FnOnce(&mut VARIANT)) -> VARIANT {
    let mut value = VARIANT::default();
    unsafe {
        (*value.Anonymous.Anonymous).vt = kind;
    }
    fill(&mut value);
    value
}

fn write(services: &IWbemServices, level: u8) -> windows::core::Result<()> {
    let not_found =
        || windows::core::Error::from_hresult(windows::core::HRESULT(0x8004_1002u32 as i32));
    unsafe {
        let instance = first(
            services,
            "SELECT * FROM WmiMonitorBrightnessMethods WHERE Active = TRUE",
        )
        .ok_or_else(not_found)?;
        let mut path = VARIANT::default();
        instance.Get(w!("__PATH"), 0, &mut path, None, None)?;
        let inner = &path.Anonymous.Anonymous;
        if inner.vt != VT_BSTR {
            let _ = VariantClear(&mut path);
            return Err(not_found());
        }
        let object_path = BSTR::clone(&inner.Anonymous.bstrVal);
        let _ = VariantClear(&mut path);

        let mut class = None;
        services.GetObject(
            &BSTR::from("WmiMonitorBrightnessMethods"),
            WBEM_FLAG_RETURN_WBEM_COMPLETE,
            None,
            Some(&mut class),
            None,
        )?;
        let class = class.ok_or_else(not_found)?;
        let mut signature = None;
        class.GetMethod(w!("WmiSetBrightness"), 0, &mut signature, null_mut())?;
        let parameters = signature.ok_or_else(not_found)?.SpawnInstance(0)?;
        let timeout = variant(VT_I4, |value| {
            (*value.Anonymous.Anonymous).Anonymous.lVal = 0;
        });
        parameters.Put(w!("Timeout"), 0, &timeout, 0)?;
        let brightness = variant(VT_UI1, |value| {
            (*value.Anonymous.Anonymous).Anonymous.bVal = level;
        });
        parameters.Put(w!("Brightness"), 0, &brightness, 0)?;
        services.ExecMethod(
            &object_path,
            &BSTR::from("WmiSetBrightness"),
            WBEM_GENERIC_FLAG_TYPE(0),
            None,
            &parameters,
            None,
            None,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "reads the interactive machine's display brightness"]
    fn reads_the_internal_panel_level() {
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok().unwrap() };
        let services = connect().unwrap();
        let level = read(&services).expect("no internal panel brightness");
        assert!(level <= 100);
    }
}
