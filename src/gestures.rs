//! Opt-in three-finger horizontal swipes on a precision touchpad, to switch desktops.
//!
//! Windows only maps three-finger swipes to desktops through its own setting, which
//! was not available on the founder's machine, so Monolith reads the touchpad itself
//! through documented Raw Input: a message-only window on its own thread registers
//! for HID digitizer touchpads (usage page 0x0D, usage 0x05) with `RIDEV_INPUTSINK`,
//! decodes each contact with `HidP_*` from the device's own report description, and
//! switches desktops through the documented Win+Ctrl+Left/Right shortcuts. No driver
//! or hook is involved. Set the Windows three-finger gesture to "Nothing" to avoid
//! both reacting.

use std::{
    collections::HashMap,
    ffi::c_void,
    io,
    mem::size_of,
    ptr::null_mut,
    sync::mpsc,
    thread::{self, JoinHandle},
};
use windows_sys::Win32::{
    Devices::HumanInterfaceDevice::{
        HIDP_CAPS, HIDP_STATUS_SUCCESS, HIDP_VALUE_CAPS, HidP_GetCaps, HidP_GetUsageValue,
        HidP_GetUsages, HidP_GetValueCaps, HidP_Input,
    },
    Foundation::{HANDLE, HWND, LPARAM, LRESULT, WPARAM},
    System::{LibraryLoader::GetModuleHandleW, Threading::GetCurrentThreadId},
    UI::{
        Input::KeyboardAndMouse::{VK_LCONTROL, VK_LEFT, VK_LWIN, VK_RIGHT},
        Input::{
            GetRawInputData, GetRawInputDeviceInfoW, HRAWINPUT, RAWINPUT, RAWINPUTDEVICE,
            RAWINPUTHEADER, RID_INPUT, RIDEV_INPUTSINK, RIDI_PREPARSEDDATA, RIM_TYPEHID,
            RegisterRawInputDevices,
        },
        WindowsAndMessaging::*,
    },
};

const DIGITIZER: u16 = 0x0D;
const TOUCH_PAD: u16 = 0x05;
const TIP_SWITCH: u16 = 0x42;
const CONTACT_ID: u16 = 0x51;
const CONTACT_COUNT: u16 = 0x54;
const GENERIC_DESKTOP: u16 = 0x01;
const X: u16 = 0x30;
/// Fraction of the touchpad width three fingers must travel to switch desktops.
const TRAVEL: f64 = 0.12;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Swipe {
    /// Fingers moved left: go to the desktop on the right, as on macOS.
    Left,
    Right,
}

/// One finger in a completed touchpad frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Contact {
    pub id: u32,
    pub x: f64,
    pub touching: bool,
}

/// Recognises a single horizontal three-finger swipe per touch.
#[derive(Default)]
pub struct Recogniser {
    start: Option<f64>,
    fired: bool,
}

impl Recogniser {
    /// Feed one frame; `width` is the touchpad's logical X range.
    pub fn frame(&mut self, contacts: &[Contact], width: f64) -> Option<Swipe> {
        let touching: Vec<f64> = contacts
            .iter()
            .filter(|contact| contact.touching)
            .map(|contact| contact.x)
            .collect();
        if touching.len() != 3 || width <= 0.0 {
            // Lifting, or any other finger count, ends the gesture.
            if touching.is_empty() || touching.len() != 3 {
                self.start = None;
                self.fired = false;
            }
            return None;
        }
        let centre = touching.iter().sum::<f64>() / 3.0;
        let start = *self.start.get_or_insert(centre);
        if self.fired || (centre - start).abs() < width * TRAVEL {
            return None;
        }
        self.fired = true;
        Some(if centre < start {
            Swipe::Left
        } else {
            Swipe::Right
        })
    }
}

/// The report layout of one touchpad, from its preparsed HID data.
struct Device {
    preparsed: Vec<u8>,
    /// Link collections that carry an X coordinate: one per contact slot.
    contacts: Vec<u16>,
    width: f64,
    /// Contacts still expected for the frame in progress (hybrid reporting).
    expected: usize,
    frame: HashMap<u32, Contact>,
}

impl Device {
    fn open(handle: HANDLE) -> Option<Self> {
        let mut size = 0_u32;
        unsafe { GetRawInputDeviceInfoW(handle, RIDI_PREPARSEDDATA, null_mut(), &mut size) };
        if size == 0 {
            return None;
        }
        let mut preparsed = vec![0_u8; size as usize];
        if unsafe {
            GetRawInputDeviceInfoW(
                handle,
                RIDI_PREPARSEDDATA,
                preparsed.as_mut_ptr().cast(),
                &mut size,
            )
        } == u32::MAX
        {
            return None;
        }
        let data = preparsed.as_ptr() as isize;
        let mut caps = HIDP_CAPS::default();
        if unsafe { HidP_GetCaps(data, &mut caps) } != HIDP_STATUS_SUCCESS {
            return None;
        }
        let mut count = caps.NumberInputValueCaps;
        let mut values = vec![HIDP_VALUE_CAPS::default(); count as usize];
        if unsafe { HidP_GetValueCaps(HidP_Input, values.as_mut_ptr(), &mut count, data) }
            != HIDP_STATUS_SUCCESS
        {
            return None;
        }
        let mut contacts = Vec::new();
        let mut width = 0.0;
        for value in &values[..count as usize] {
            let usage = if value.IsRange {
                unsafe { value.Anonymous.Range.UsageMin }
            } else {
                unsafe { value.Anonymous.NotRange.Usage }
            };
            if value.UsagePage == GENERIC_DESKTOP && usage == X {
                contacts.push(value.LinkCollection);
                width = f64::from(value.LogicalMax - value.LogicalMin);
            }
        }
        (!contacts.is_empty()).then_some(Self {
            preparsed,
            contacts,
            width,
            expected: 0,
            frame: HashMap::new(),
        })
    }

    fn value(&self, report: &mut [u8], page: u16, link: u16, usage: u16) -> Option<u32> {
        let mut value = 0;
        (unsafe {
            HidP_GetUsageValue(
                HidP_Input,
                page,
                link,
                usage,
                &mut value,
                self.preparsed.as_ptr() as isize,
                report.as_ptr(),
                report.len() as u32,
            )
        } == HIDP_STATUS_SUCCESS)
            .then_some(value)
    }

    fn touching(&self, report: &mut [u8], link: u16) -> bool {
        let mut usages = [0_u16; 16];
        let mut length = usages.len() as u32;
        (unsafe {
            HidP_GetUsages(
                HidP_Input,
                DIGITIZER,
                link,
                usages.as_mut_ptr(),
                &mut length,
                self.preparsed.as_ptr() as isize,
                report.as_mut_ptr(),
                report.len() as u32,
            )
        } == HIDP_STATUS_SUCCESS)
            && usages[..length as usize].contains(&TIP_SWITCH)
    }

    /// Decode one report. Returns the frame once every contact in it has arrived.
    fn report(&mut self, report: &mut [u8]) -> Option<Vec<Contact>> {
        // The first report of a frame carries the contact count; others carry zero.
        if let Some(count) = self.value(report, DIGITIZER, 0, CONTACT_COUNT)
            && count > 0
        {
            self.expected = count as usize;
            self.frame.clear();
        }
        for &link in &self.contacts.clone() {
            let (Some(id), Some(x)) = (
                self.value(report, DIGITIZER, link, CONTACT_ID),
                self.value(report, GENERIC_DESKTOP, link, X),
            ) else {
                continue;
            };
            if self.frame.len() < self.expected {
                let touching = self.touching(report, link);
                self.frame.insert(
                    id,
                    Contact {
                        id,
                        x: f64::from(x),
                        touching,
                    },
                );
            }
        }
        (self.expected > 0 && self.frame.len() >= self.expected).then(|| {
            self.expected = 0;
            self.frame.drain().map(|(_, contact)| contact).collect()
        })
    }
}

thread_local! {
    static STATE: std::cell::RefCell<(HashMap<isize, Option<Device>>, Recogniser)> =
        std::cell::RefCell::new((HashMap::new(), Recogniser::default()));
}

pub struct Gestures {
    thread_id: u32,
    worker: Option<JoinHandle<()>>,
}

impl Gestures {
    pub fn start() -> io::Result<Self> {
        let (ready, started) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("monolith-gestures".into())
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
            Err(_) => Err(io::Error::other("The gesture thread stopped")),
        }
    }
}

impl Drop for Gestures {
    fn drop(&mut self) {
        unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, 0, 0) };
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

unsafe fn run(ready: mpsc::Sender<io::Result<u32>>) {
    unsafe {
        let instance = GetModuleHandleW(std::ptr::null());
        let class: Vec<u16> = "MonolithGestures".encode_utf16().chain([0]).collect();
        let window_class = WNDCLASSW {
            hInstance: instance,
            lpfnWndProc: Some(window_proc),
            lpszClassName: class.as_ptr(),
            ..Default::default()
        };
        RegisterClassW(&window_class);
        let hwnd = CreateWindowExW(
            0,
            class.as_ptr(),
            std::ptr::null(),
            0,
            0,
            0,
            0,
            0,
            HWND_MESSAGE,
            null_mut(),
            instance,
            null_mut(),
        );
        if hwnd.is_null() {
            let _ = ready.send(Err(io::Error::last_os_error()));
            return;
        }
        let device = RAWINPUTDEVICE {
            usUsagePage: DIGITIZER,
            usUsage: TOUCH_PAD,
            dwFlags: RIDEV_INPUTSINK,
            hwndTarget: hwnd,
        };
        if RegisterRawInputDevices(&device, 1, size_of::<RAWINPUTDEVICE>() as u32) == 0 {
            let _ = ready.send(Err(io::Error::last_os_error()));
            DestroyWindow(hwnd);
            return;
        }
        let _ = ready.send(Ok(GetCurrentThreadId()));
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        DestroyWindow(hwnd);
    }
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        if msg == WM_INPUT {
            handle_input(lp as HRAWINPUT);
        }
        DefWindowProcW(hwnd, msg, wp, lp)
    }
}

unsafe fn handle_input(input: HRAWINPUT) {
    unsafe {
        let header = size_of::<RAWINPUTHEADER>() as u32;
        let mut size = 0_u32;
        GetRawInputData(input, RID_INPUT, null_mut(), &mut size, header);
        if size == 0 || size > 64 * 1024 {
            return;
        }
        let mut buffer = vec![0_u64; (size as usize).div_ceil(8)];
        if GetRawInputData(
            input,
            RID_INPUT,
            buffer.as_mut_ptr().cast(),
            &mut size,
            header,
        ) == u32::MAX
        {
            return;
        }
        let raw = &*(buffer.as_ptr() as *const RAWINPUT);
        if raw.header.dwType != RIM_TYPEHID {
            return;
        }
        let hid = &raw.data.hid;
        let report_size = hid.dwSizeHid as usize;
        let reports = hid.dwCount as usize;
        let start = (hid.bRawData.as_ptr() as *const c_void).cast::<u8>();
        let available = size as usize - (start as usize - buffer.as_ptr() as usize);
        if report_size == 0 || report_size * reports > available {
            return;
        }
        let device = raw.header.hDevice;
        let swipe = STATE.with_borrow_mut(|(devices, recogniser)| {
            let device = devices
                .entry(device as isize)
                .or_insert_with(|| Device::open(device))
                .as_mut()?;
            let mut swipe = None;
            for index in 0..reports {
                let mut report =
                    std::slice::from_raw_parts(start.add(index * report_size), report_size)
                        .to_vec();
                if let Some(contacts) = device.report(&mut report) {
                    swipe = swipe.or(recogniser.frame(&contacts, device.width));
                }
            }
            swipe
        });
        if let Some(swipe) = swipe {
            let key = match swipe {
                Swipe::Left => VK_RIGHT,
                Swipe::Right => VK_LEFT,
            };
            crate::workspaces::press(&[VK_LWIN, VK_LCONTROL, key], 1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fingers(xs: &[f64]) -> Vec<Contact> {
        xs.iter()
            .enumerate()
            .map(|(id, &x)| Contact {
                id: id as u32,
                x,
                touching: true,
            })
            .collect()
    }

    #[test]
    fn fires_once_per_three_finger_swipe_in_its_direction() {
        let mut recogniser = Recogniser::default();
        let width = 1000.0;
        assert_eq!(
            recogniser.frame(&fingers(&[500.0, 550.0, 600.0]), width),
            None
        );
        assert_eq!(
            recogniser.frame(&fingers(&[480.0, 530.0, 580.0]), width),
            None
        );
        // 120 units of travel (12% of the width) to the left switches once.
        assert_eq!(
            recogniser.frame(&fingers(&[380.0, 430.0, 480.0]), width),
            Some(Swipe::Left)
        );
        assert_eq!(
            recogniser.frame(&fingers(&[200.0, 250.0, 300.0]), width),
            None
        );
        // Lifting resets; the next swipe can go the other way.
        assert_eq!(recogniser.frame(&[], width), None);
        assert_eq!(
            recogniser.frame(&fingers(&[300.0, 350.0, 400.0]), width),
            None
        );
        assert_eq!(
            recogniser.frame(&fingers(&[430.0, 480.0, 530.0]), width),
            Some(Swipe::Right)
        );
    }

    #[test]
    fn ignores_two_and_four_finger_movement() {
        let mut recogniser = Recogniser::default();
        assert_eq!(recogniser.frame(&fingers(&[500.0, 600.0]), 1000.0), None);
        assert_eq!(recogniser.frame(&fingers(&[100.0, 200.0]), 1000.0), None);
        let four = fingers(&[500.0, 550.0, 600.0, 650.0]);
        assert_eq!(recogniser.frame(&four, 1000.0), None);
        let moved = fingers(&[100.0, 150.0, 200.0, 250.0]);
        assert_eq!(recogniser.frame(&moved, 1000.0), None);
    }

    #[test]
    #[ignore = "reads the interactive machine's touchpad description"]
    fn decodes_the_installed_precision_touchpad() {
        use windows_sys::Win32::UI::Input::{
            GetRawInputDeviceList, RAWINPUTDEVICELIST, RID_DEVICE_INFO, RIDI_DEVICEINFO,
            RIM_TYPEHID,
        };
        let mut count = 0_u32;
        let size = size_of::<RAWINPUTDEVICELIST>() as u32;
        unsafe { GetRawInputDeviceList(null_mut(), &mut count, size) };
        let mut list = vec![RAWINPUTDEVICELIST::default(); count as usize];
        unsafe { GetRawInputDeviceList(list.as_mut_ptr(), &mut count, size) };
        let mut found = 0;
        for device in &list[..count as usize] {
            if device.dwType != RIM_TYPEHID {
                continue;
            }
            let mut info = RID_DEVICE_INFO {
                cbSize: size_of::<RID_DEVICE_INFO>() as u32,
                ..Default::default()
            };
            let mut info_size = info.cbSize;
            unsafe {
                GetRawInputDeviceInfoW(
                    device.hDevice,
                    RIDI_DEVICEINFO,
                    (&raw mut info).cast(),
                    &mut info_size,
                )
            };
            let hid = unsafe { info.Anonymous.hid };
            if hid.usUsagePage == DIGITIZER && hid.usUsage == TOUCH_PAD {
                let decoded = Device::open(device.hDevice).expect("touchpad descriptor");
                println!(
                    "touchpad: {} contact slots, X range {}",
                    decoded.contacts.len(),
                    decoded.width
                );
                found += 1;
            }
        }
        assert!(found > 0, "no precision touchpad found");
    }
}
