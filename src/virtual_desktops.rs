//! Opt-in compatibility adapter for the private Windows virtual desktop interfaces.
//!
//! The public `IVirtualDesktopManager` refuses to move windows that belong to other
//! processes, so moving a window between desktops needs the undocumented
//! `IVirtualDesktopManagerInternal` service of the Explorer immersive shell. The
//! interface IDs and method order come from Ciantic's VirtualDesktopAccessor
//! (MIT, https://github.com/Ciantic/VirtualDesktopAccessor, `src/interfaces.rs` at
//! ceb648849dcc5760a7f4ed223308ffe3de0b51c6), which matches the layout Windhawk's
//! Virtual Desktop Helper uses for build 26100 and later.
//!
//! Three gates protect against a changed layout: the build must be 26100-26999, the
//! services must answer the exact interface IDs, and a self-check must agree with
//! Explorer's registry state (desktop count and current desktop identity) before any
//! window is moved. Only reading desktops and moving one view are used; desktops are
//! never created or removed through this adapter. Runs in Monolith's own process.

use crate::workspaces::{self, DesktopId};
use std::{ffi::c_void, ptr::null_mut};
use windows::{
    Win32::{
        Foundation::HWND,
        System::Com::{CLSCTX_LOCAL_SERVER, CoCreateInstance, IServiceProvider},
        UI::Shell::Common::IObjectArray,
    },
    core::{GUID, HRESULT, IUnknown, Interface},
};

const CLSID_IMMERSIVE_SHELL: GUID = GUID::from_u128(0xC2F03A33_21F5_47FA_B4BB_156362A2F239);
const CLSID_MANAGER_INTERNAL: GUID = GUID::from_u128(0xC5E0CDCA_7B6E_41B2_9FC4_D93975CC467B);
const IID_MANAGER_INTERNAL: GUID = GUID::from_u128(0x53F5CA0B_158F_4124_900C_057158060B27);
const IID_VIEW_COLLECTION: GUID = GUID::from_u128(0x1841C6D7_4F9D_42C0_AF41_8747538F10E5);
const IID_VIRTUAL_DESKTOP: GUID = GUID::from_u128(0x3F07F4BE_B107_441A_AF0F_39D82529072C);

// Vtable slots after IUnknown's three methods.
const MANAGER_GET_COUNT: usize = 3;
const MANAGER_MOVE_VIEW: usize = 4;
const MANAGER_GET_CURRENT: usize = 6;
const MANAGER_GET_DESKTOPS: usize = 7;
const MANAGER_REMOVE_DESKTOP: usize = 13;
const VIEWS_FOR_HWND: usize = 6;
const DESKTOP_GET_ID: usize = 4;

const SUPPORTED_BUILDS: std::ops::Range<u32> = 26_100..27_000;

type GetCount = unsafe extern "system" fn(*mut c_void, *mut u32) -> HRESULT;
type GetObject = unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT;
type MoveView = unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> HRESULT;
type ViewForHwnd = unsafe extern "system" fn(*mut c_void, HWND, *mut *mut c_void) -> HRESULT;
type GetId = unsafe extern "system" fn(*mut c_void, *mut GUID) -> HRESULT;
type RemoveDesktop = unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> HRESULT;

/// Read a vtable slot. The caller guarantees the object implements the interface
/// whose layout defines `index`, which the gates above establish.
unsafe fn slot<F: Copy>(object: *mut c_void, index: usize) -> F {
    unsafe {
        let table = *(object as *const *const usize);
        std::mem::transmute_copy(&*table.add(index))
    }
}

/// Take ownership of a returned COM pointer so it is released when dropped.
unsafe fn owned(raw: *mut c_void) -> Option<IUnknown> {
    (!raw.is_null()).then(|| unsafe { IUnknown::from_raw(raw) })
}

pub struct Mover {
    manager: IUnknown,
    views: IUnknown,
}

impl Mover {
    /// Connect when the build is supported and the self-check agrees with Explorer.
    /// Call on a thread with COM initialised.
    pub fn connect() -> Option<Self> {
        if !SUPPORTED_BUILDS.contains(&build()?) {
            return None;
        }
        let mover = unsafe {
            let shell: IServiceProvider =
                CoCreateInstance(&CLSID_IMMERSIVE_SHELL, None, CLSCTX_LOCAL_SERVER).ok()?;
            // The interface IDs are private, so call QueryService through its vtable.
            let query = Interface::vtable(&shell).QueryService;
            let mut manager = null_mut();
            query(
                shell.as_raw(),
                &CLSID_MANAGER_INTERNAL,
                &IID_MANAGER_INTERNAL,
                &mut manager,
            )
            .ok()
            .ok()?;
            let manager = owned(manager)?;
            let mut views = null_mut();
            query(
                shell.as_raw(),
                &IID_VIEW_COLLECTION,
                &IID_VIEW_COLLECTION,
                &mut views,
            )
            .ok()
            .ok()?;
            Self {
                manager,
                views: owned(views)?,
            }
        };
        mover.agrees_with_explorer().then_some(mover)
    }

    fn agrees_with_explorer(&self) -> bool {
        let Ok(state) = workspaces::read() else {
            return false;
        };
        let count = self.count();
        let current = self.current_id();
        count == Some(state.ids.len())
            && current.is_some()
            && current == state.current.map(|index| state.ids[index])
    }

    fn count(&self) -> Option<usize> {
        let mut count = 0;
        unsafe {
            let call: GetCount = slot(self.manager.as_raw(), MANAGER_GET_COUNT);
            call(self.manager.as_raw(), &mut count).ok().ok()?;
        }
        Some(count as usize)
    }

    fn current_id(&self) -> Option<DesktopId> {
        unsafe {
            let mut desktop = null_mut();
            let call: GetObject = slot(self.manager.as_raw(), MANAGER_GET_CURRENT);
            call(self.manager.as_raw(), &mut desktop).ok().ok()?;
            desktop_id(&owned(desktop)?)
        }
    }

    /// The desktop at `index` in Explorer's order.
    fn desktop(&self, index: usize) -> Option<IUnknown> {
        unsafe {
            let mut array = null_mut();
            let call: GetObject = slot(self.manager.as_raw(), MANAGER_GET_DESKTOPS);
            call(self.manager.as_raw(), &mut array).ok().ok()?;
            let array: IObjectArray = owned(array)?.cast().ok()?;
            if index >= array.GetCount().ok()? as usize {
                return None;
            }
            let mut desktop = null_mut();
            let found: windows::core::Result<()> = (Interface::vtable(&array).GetAt)(
                array.as_raw(),
                index as u32,
                &IID_VIRTUAL_DESKTOP,
                &mut desktop,
            )
            .ok();
            found.ok()?;
            owned(desktop)
        }
    }

    /// Move `window` to the desktop at `index`. Rechecks Explorer's state first, so a
    /// layout change after start-up disables the move instead of misbehaving.
    pub fn move_window(&self, window: HWND, index: usize) -> bool {
        if window.is_invalid() || !self.agrees_with_explorer() {
            return false;
        }
        let Some(desktop) = self.desktop(index) else {
            return false;
        };
        unsafe {
            let mut view = null_mut();
            let lookup: ViewForHwnd = slot(self.views.as_raw(), VIEWS_FOR_HWND);
            if lookup(self.views.as_raw(), window, &mut view).is_err() {
                return false;
            }
            let Some(view) = owned(view) else {
                return false;
            };
            let call: MoveView = slot(self.manager.as_raw(), MANAGER_MOVE_VIEW);
            call(self.manager.as_raw(), view.as_raw(), desktop.as_raw()).is_ok()
        }
    }
}

impl Mover {
    /// Remove the desktop at `index`, moving anything left on it to `fallback`.
    /// Never removes the current desktop. Rechecks Explorer's state first.
    pub fn remove_desktop(&self, index: usize, fallback: usize) -> bool {
        let Ok(state) = workspaces::read() else {
            return false;
        };
        if index == fallback || state.current == Some(index) || !self.agrees_with_explorer() {
            return false;
        }
        let (Some(desktop), Some(fallback)) = (self.desktop(index), self.desktop(fallback)) else {
            return false;
        };
        unsafe {
            let call: RemoveDesktop = slot(self.manager.as_raw(), MANAGER_REMOVE_DESKTOP);
            call(self.manager.as_raw(), desktop.as_raw(), fallback.as_raw()).is_ok()
        }
    }
}

unsafe fn desktop_id(desktop: &IUnknown) -> Option<DesktopId> {
    let mut id = GUID::zeroed();
    unsafe {
        let call: GetId = slot(desktop.as_raw(), DESKTOP_GET_ID);
        call(desktop.as_raw(), &mut id).ok().ok()?;
    }
    // The registry stores GUIDs in their little-endian binary layout.
    let mut bytes = [0_u8; 16];
    bytes[..4].copy_from_slice(&id.data1.to_le_bytes());
    bytes[4..6].copy_from_slice(&id.data2.to_le_bytes());
    bytes[6..8].copy_from_slice(&id.data3.to_le_bytes());
    bytes[8..].copy_from_slice(&id.data4);
    Some(bytes)
}

/// The Windows build number, from the documented registry value.
fn build() -> Option<u32> {
    use windows_sys::Win32::{
        Foundation::ERROR_SUCCESS,
        System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW},
    };
    let key: Vec<u16> = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion"
        .encode_utf16()
        .chain([0])
        .collect();
    let value: Vec<u16> = "CurrentBuildNumber".encode_utf16().chain([0]).collect();
    let mut buffer = [0_u16; 16];
    let mut size = std::mem::size_of_val(&buffer) as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
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
    String::from_utf16_lossy(&buffer[..length])
        .trim()
        .parse()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_support_to_the_verified_build_family() {
        assert!(SUPPORTED_BUILDS.contains(&26_100));
        assert!(SUPPORTED_BUILDS.contains(&26_300));
        assert!(!SUPPORTED_BUILDS.contains(&22_631));
        assert!(!SUPPORTED_BUILDS.contains(&27_000));
        assert!(build().is_some());
    }

    #[test]
    #[ignore = "connects to the interactive Explorer immersive shell"]
    fn connects_only_when_explorer_agrees() {
        use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok().unwrap() };
        let mover = Mover::connect().expect("the adapter did not pass its gates");
        let state = workspaces::read().unwrap();
        assert_eq!(mover.count(), Some(state.ids.len()));
        assert!(mover.desktop(state.ids.len()).is_none());
        assert!(mover.desktop(0).is_some());
    }
}
