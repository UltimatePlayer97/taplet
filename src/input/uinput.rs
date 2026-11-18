use std::collections::HashSet;
use std::fs::{File, OpenOptions};
use std::io::ErrorKind;
use std::os::unix::fs::OpenOptionsExt;
use std::thread;
use std::time::Duration;

use rustix::ioctl::Opcode;
use rustix::ioctl::opcode;

use super::virtual_pointer::query_layout;
use super::{InputBackend, MouseButton};
use crate::error::InputError;
use crate::linux;

const UI_DEV_CREATE: Opcode = opcode::none(b'U', 1);
const UI_DEV_DESTROY: Opcode = opcode::none(b'U', 2);
const UI_SET_EVBIT: Opcode = opcode::write::<i32>(b'U', 100);
const UI_SET_KEYBIT: Opcode = opcode::write::<i32>(b'U', 101);
const UI_SET_ABSBIT: Opcode = opcode::write::<i32>(b'U', 103);

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

fn ioctl(file: &File, request: Opcode, argument: i32) -> Result<(), InputError> {
    linux::ioctl_value(file, request, argument).map_err(map_io)
}

fn write_event(file: &File, kind: u16, code: u16, value: i32) -> Result<(), InputError> {
    let event = linux::InputEvent {
        time: [0; 2],
        type_: kind,
        code,
        value,
    };
    linux::write_struct(file, &event).map_err(map_io)
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
            .custom_flags(linux::O_NONBLOCK)
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

        linux::write_struct(&file, &dev).map_err(map_io)?;
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
