use std::collections::HashSet;
use std::fs::{File, OpenOptions};
use std::io::ErrorKind;
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::thread;
use std::time::Duration;

use super::virtual_pointer::query_layout;
use super::{InputBackend, MouseButton};
use crate::error::InputError;

const UI_DEV_CREATE: u64 = 0x5501;
const UI_DEV_DESTROY: u64 = 0x5502;
const UI_SET_EVBIT: u64 = 0x4004_5564;
const UI_SET_KEYBIT: u64 = 0x4004_5565;
const UI_SET_ABSBIT: u64 = 0x4004_5567;

const EV_SYN: u16 = 0;
const EV_KEY: u16 = 1;
const EV_ABS: u16 = 3;
const ABS_X: u16 = 0;
const ABS_Y: u16 = 1;
const ABS_CNT: usize = 64;

#[repr(C)]
struct UserDev {
    name: [u8; 80],
    bustype: u16,
    vendor: u16,
    product: u16,
    version: u16,
    ff_effects_max: u32,
    absmax: [i32; ABS_CNT],
    absmin: [i32; ABS_CNT],
    absfuzz: [i32; ABS_CNT],
    absflat: [i32; ABS_CNT],
}

pub struct UinputBackend {
    file: Option<File>,
    bounds: Option<(i32, i32, i32, i32)>,
    pressed: HashSet<MouseButton>,
}

fn map_io(err: std::io::Error) -> InputError {
    match err.kind() {
        ErrorKind::PermissionDenied | ErrorKind::NotFound => InputError::UinputPermission,
        _ => InputError::Connection(err.to_string()),
    }
}

fn ioctl(file: &File, request: u64, arg: i32) -> Result<(), InputError> {
    let rc = unsafe { libc::ioctl(file.as_raw_fd(), request as _, arg) };
    if rc < 0 {
        Err(map_io(std::io::Error::last_os_error()))
    } else {
        Ok(())
    }
}

fn write_event(file: &File, kind: u16, code: u16, value: i32) -> Result<(), InputError> {
    let event = libc::input_event {
        time: libc::timeval {
            tv_sec: 0,
            tv_usec: 0,
        },
        type_: kind,
        code,
        value,
    };
    let size = std::mem::size_of::<libc::input_event>();
    let written = unsafe {
        libc::write(
            file.as_raw_fd(),
            (&event as *const libc::input_event).cast(),
            size,
        )
    };
    if written == size as isize {
        Ok(())
    } else {
        Err(InputError::Connection(
            std::io::Error::last_os_error().to_string(),
        ))
    }
}

impl UinputBackend {
    pub fn new() -> Self {
        Self {
            file: None,
            bounds: None,
            pressed: HashSet::new(),
        }
    }

    fn file(&self) -> Result<&File, InputError> {
        self.file.as_ref().ok_or(InputError::NotConnected)
    }

    fn send_button(&mut self, button: MouseButton, value: i32) -> Result<(), InputError> {
        let file = self.file()?;
        write_event(file, EV_KEY, button.evdev_code() as u16, value)?;
        write_event(file, EV_SYN, 0, 0)
    }
}

impl Default for UinputBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl InputBackend for UinputBackend {
    fn name(&self) -> &'static str {
        "uinput"
    }

    fn connect(&mut self) -> Result<(), InputError> {
        if self.file.is_some() {
            return Ok(());
        }
        let bounds = query_layout();
        let file = OpenOptions::new()
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open("/dev/uinput")
            .map_err(map_io)?;

        ioctl(&file, UI_SET_EVBIT, EV_KEY as i32)?;
        for button in MouseButton::ALL {
            ioctl(&file, UI_SET_KEYBIT, button.evdev_code() as i32)?;
        }

        let mut dev = UserDev {
            name: [0; 80],
            bustype: 0x06,
            vendor: 0x1,
            product: 0x1,
            version: 1,
            ff_effects_max: 0,
            absmax: [0; ABS_CNT],
            absmin: [0; ABS_CNT],
            absfuzz: [0; ABS_CNT],
            absflat: [0; ABS_CNT],
        };
        let name = b"taplet virtual pointer";
        dev.name[..name.len()].copy_from_slice(name);

        if let Some((_, _, w, h)) = bounds {
            ioctl(&file, UI_SET_EVBIT, EV_ABS as i32)?;
            ioctl(&file, UI_SET_ABSBIT, ABS_X as i32)?;
            ioctl(&file, UI_SET_ABSBIT, ABS_Y as i32)?;
            dev.absmax[ABS_X as usize] = w - 1;
            dev.absmax[ABS_Y as usize] = h - 1;
        }

        let size = std::mem::size_of::<UserDev>();
        let written =
            unsafe { libc::write(file.as_raw_fd(), (&dev as *const UserDev).cast(), size) };
        if written != size as isize {
            return Err(map_io(std::io::Error::last_os_error()));
        }
        ioctl(&file, UI_DEV_CREATE, 0)?;
        thread::sleep(Duration::from_millis(250));
        self.file = Some(file);
        self.bounds = bounds;
        Ok(())
    }

    fn move_to(&mut self, x: i32, y: i32) -> Result<(), InputError> {
        let (bx, by, w, h) = self
            .bounds
            .ok_or_else(|| InputError::Connection("monitor layout unavailable".to_string()))?;
        let file = self.file()?;
        write_event(file, EV_ABS, ABS_X, (x - bx).clamp(0, w - 1))?;
        write_event(file, EV_ABS, ABS_Y, (y - by).clamp(0, h - 1))?;
        write_event(file, EV_SYN, 0, 0)
    }

    fn press_button(&mut self, button: MouseButton) -> Result<(), InputError> {
        self.pressed.insert(button);
        self.send_button(button, 1)
    }

    fn release_button(&mut self, button: MouseButton) -> Result<(), InputError> {
        self.pressed.remove(&button);
        self.send_button(button, 0)
    }

    fn release_all(&mut self) -> Result<(), InputError> {
        let held: Vec<MouseButton> = self.pressed.drain().collect();
        let mut result = Ok(());
        for button in held {
            if let Err(err) = self.send_button(button, 0) {
                result = Err(err);
            }
        }
        result
    }

    fn disconnect(&mut self) -> Result<(), InputError> {
        let released = if self.file.is_some() {
            self.release_all()
        } else {
            Ok(())
        };
        if let Some(file) = self.file.take() {
            let _ = ioctl(&file, UI_DEV_DESTROY, 0);
        }
        released
    }
}

impl Drop for UinputBackend {
    fn drop(&mut self) {
        let _ = self.disconnect();
    }
}
