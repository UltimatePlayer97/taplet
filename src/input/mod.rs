mod uinput;
mod virtual_pointer;

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use crate::config::BackendChoice;
pub use uinput::UinputBackend;
pub use virtual_pointer::VirtualPointerBackend;

use crate::error::InputError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
}

impl MouseButton {
    pub const ALL: [MouseButton; 3] = [MouseButton::Left, MouseButton::Middle, MouseButton::Right];

    pub fn evdev_code(self) -> u32 {
        match self {
            MouseButton::Left => 0x110,
            MouseButton::Right => 0x111,
            MouseButton::Middle => 0x112,
        }
    }
}

impl std::fmt::Display for MouseButton {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MouseButton::Left => f.write_str("Left"),
            MouseButton::Middle => f.write_str("Middle"),
            MouseButton::Right => f.write_str("Right"),
        }
    }
}

pub trait InputBackend {
    fn name(&self) -> &'static str;
    fn connect(&mut self) -> Result<(), InputError>;
    fn press_button(&mut self, button: MouseButton) -> Result<(), InputError>;
    fn release_button(&mut self, button: MouseButton) -> Result<(), InputError>;
    fn release_all(&mut self) -> Result<(), InputError>;
    fn disconnect(&mut self) -> Result<(), InputError>;
    fn move_to(&mut self, _x: i32, _y: i32) -> Result<(), InputError> {
        Ok(())
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Active {
    Wlr,
    Uinput,
}

pub struct AutoBackend {
    wlr: VirtualPointerBackend,
    uinput: UinputBackend,
    active: Active,
    choice: Arc<AtomicU8>,
}

impl AutoBackend {
    pub fn new(choice: Arc<AtomicU8>) -> Self {
        Self {
            wlr: VirtualPointerBackend::new(),
            uinput: UinputBackend::new(),
            active: Active::Wlr,
            choice,
        }
    }

    fn current(&mut self) -> &mut dyn InputBackend {
        match self.active {
            Active::Wlr => &mut self.wlr,
            Active::Uinput => &mut self.uinput,
        }
    }

    fn connect_auto(&mut self) -> Result<(), InputError> {
        let wlr_err = match self.wlr.connect() {
            Ok(()) => {
                self.active = Active::Wlr;
                return Ok(());
            }
            Err(err) => err,
        };
        match self.uinput.connect() {
            Ok(()) => {
                self.active = Active::Uinput;
                Ok(())
            }
            Err(InputError::UinputPermission) => {
                self.active = Active::Uinput;
                Err(InputError::UinputPermission)
            }
            Err(_) => {
                self.active = Active::Wlr;
                Err(wlr_err)
            }
        }
    }
}

impl InputBackend for AutoBackend {
    fn name(&self) -> &'static str {
        match self.active {
            Active::Wlr => self.wlr.name(),
            Active::Uinput => self.uinput.name(),
        }
    }

    fn connect(&mut self) -> Result<(), InputError> {
        match BackendChoice::from_u8(self.choice.load(Ordering::Relaxed)) {
            BackendChoice::Wlr => {
                self.active = Active::Wlr;
                self.wlr.connect()
            }
            BackendChoice::Uinput => {
                self.active = Active::Uinput;
                self.uinput.connect()
            }
            BackendChoice::Auto => self.connect_auto(),
        }
    }

    fn move_to(&mut self, x: i32, y: i32) -> Result<(), InputError> {
        self.current().move_to(x, y)
    }

    fn press_button(&mut self, button: MouseButton) -> Result<(), InputError> {
        self.current().press_button(button)
    }

    fn release_button(&mut self, button: MouseButton) -> Result<(), InputError> {
        self.current().release_button(button)
    }

    fn release_all(&mut self) -> Result<(), InputError> {
        self.current().release_all()
    }

    fn disconnect(&mut self) -> Result<(), InputError> {
        let first = self.wlr.disconnect();
        let second = self.uinput.disconnect();
        first.and(second)
    }
}
