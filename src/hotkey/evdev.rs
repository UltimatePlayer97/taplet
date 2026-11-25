use std::fs::{File, OpenOptions};
use std::os::unix::fs::OpenOptionsExt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use rustix::event::{PollFd, PollFlags, poll};
use rustix::io::Errno;
use rustix::ioctl::{Direction, Opcode, opcode};
use rustix::time::Timespec;

use super::{Event, HotkeyBackend, HotkeyStatus};
use crate::linux;

const EV_KEY: u16 = 1;
const KEY_MAX: u16 = 0x2ff;
const POLL_TIMEOUT: Timespec = Timespec {
    tv_sec: 0,
    tv_nsec: 200_000_000,
};
const DEBOUNCE: Duration = Duration::from_millis(300);

struct Device {
    file: File,
}

fn eviocgbit(event_type: u16, len: u16) -> Opcode {
    opcode::from_components(Direction::Read, b'E', 0x20 + event_type as u8, len as usize)
}

fn eviocgname(len: u16) -> Opcode {
    opcode::from_components(Direction::Read, b'E', 0x06, len as usize)
}

fn has_key(file: &File, codes: &[u16]) -> bool {
    let mut types = [0u8; 8];
    if linux::ioctl_read(file, eviocgbit(0, types.len() as u16), &mut types).is_err()
        || types[EV_KEY as usize / 8] & (1 << (EV_KEY % 8)) == 0
    {
        return false;
    }

    let len = (KEY_MAX as usize + 8) / 8;
    let mut keys = vec![0u8; len];
    if linux::ioctl_read(file, eviocgbit(EV_KEY, len as u16), &mut keys).is_err() {
        return false;
    }
    codes
        .iter()
        .any(|code| keys[*code as usize / 8] & (1 << (*code % 8)) != 0)
}

fn device_name(file: &File) -> Option<String> {
    let mut buf = [0u8; 256];
    if linux::ioctl_read(file, eviocgname(buf.len() as u16), &mut buf).is_err() {
        return None;
    }
    let end = buf.iter().position(|byte| *byte == 0).unwrap_or(buf.len());
    Some(String::from_utf8_lossy(&buf[..end]).into_owned())
}

pub(super) fn input_access_available() -> bool {
    let Ok(entries) = std::fs::read_dir("/dev/input") else {
        return false;
    };
    entries.flatten().any(|entry| {
        if !entry.file_name().to_string_lossy().starts_with("event") {
            return false;
        }
        let Ok(file) = OpenOptions::new().read(true).open(entry.path()) else {
            return false;
        };
        !device_name(&file).is_some_and(|name| name.starts_with("taplet"))
    })
}

fn scan(codes: &[u16]) -> Vec<Device> {
    let Ok(entries) = std::fs::read_dir("/dev/input") else {
        return Vec::new();
    };
    let mut devices = Vec::new();
    for entry in entries.flatten() {
        if !entry.file_name().to_string_lossy().starts_with("event") {
            continue;
        }
        let Ok(file) = OpenOptions::new()
            .read(true)
            .custom_flags(linux::O_NONBLOCK)
            .open(entry.path())
        else {
            continue;
        };
        if !has_key(&file, codes) {
            continue;
        }
        if device_name(&file).is_some_and(|name| name.starts_with("taplet")) {
            continue;
        }
        devices.push(Device { file });
    }
    devices
}

fn read_activation(file: &File, codes: &[u16]) -> std::io::Result<bool> {
    let mut buf = [0u8; 4096];
    let mut activated = false;
    loop {
        let read = match rustix::io::read(file, &mut buf) {
            Ok(read) => read,
            Err(Errno::WOULDBLOCK | Errno::INTR) => return Ok(activated),
            Err(err) => return Err(err.into()),
        };
        if read == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "input device closed",
            ));
        }

        let size = std::mem::size_of::<linux::InputEvent>();
        for chunk in buf[..read].chunks_exact(size) {
            let event =
                unsafe { std::ptr::read_unaligned(chunk.as_ptr().cast::<linux::InputEvent>()) };
            if event.type_ == EV_KEY && codes.contains(&event.code) && event.value == 1 {
                activated = true;
            }
        }
        if read < buf.len() {
            return Ok(activated);
        }
    }
}

pub(super) fn run(shortcut: &str, cancel: &AtomicBool, tx: &Sender<Event>) -> Result<(), String> {
    let codes = key_codes(shortcut).ok_or_else(|| {
        format!("no input mapping for shortcut {shortcut:?}; pick another key or mouse button")
    })?;

    let mut devices = scan(&codes);
    if devices.is_empty() {
        return Err(format!(
            "no readable input device declares {shortcut} under /dev/input; add your user to the input group"
        ));
    }
    if tx
        .send(Event::Status(HotkeyStatus::Active(HotkeyBackend::Evdev)))
        .is_err()
    {
        return Ok(());
    }

    let mut last_fired: Option<Instant> = None;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Ok(());
        }

        if devices.is_empty() {
            devices = scan(&codes);
            if devices.is_empty() {
                std::thread::sleep(Duration::from_millis(500));
                continue;
            }
        }

        let mut pollfds: Vec<PollFd> = devices
            .iter()
            .map(|device| PollFd::new(&device.file, PollFlags::IN))
            .collect();

        match poll(&mut pollfds, Some(&POLL_TIMEOUT)) {
            Ok(0) => continue,
            Ok(_) => {}
            Err(Errno::INTR) => continue,
            Err(err) => return Err(format!("poll: {err}")),
        }

        let mut dead = Vec::new();
        let mut fired = false;
        for (index, pollfd) in pollfds.iter().enumerate() {
            let events = pollfd.revents();
            if events.contains(PollFlags::IN) {
                match read_activation(&devices[index].file, &codes) {
                    Ok(true) => fired = true,
                    Ok(false) => {}
                    Err(_) => dead.push(index),
                }
            } else if events.intersects(PollFlags::ERR | PollFlags::HUP | PollFlags::NVAL) {
                dead.push(index);
            }
        }
        drop(pollfds);
        for index in dead.into_iter().rev() {
            devices.remove(index);
        }

        if fired {
            let debounced = last_fired.is_some_and(|last| last.elapsed() < DEBOUNCE);
            if !debounced {
                last_fired = Some(Instant::now());
                if tx.send(Event::Activated).is_err() {
                    return Ok(());
                }
            }
        }
    }
}

fn letter_code(letter: char) -> Option<u16> {
    const LETTERS: [(char, u16); 26] = [
        ('Q', 16),
        ('W', 17),
        ('E', 18),
        ('R', 19),
        ('T', 20),
        ('Y', 21),
        ('U', 22),
        ('I', 23),
        ('O', 24),
        ('P', 25),
        ('A', 30),
        ('S', 31),
        ('D', 32),
        ('F', 33),
        ('G', 34),
        ('H', 35),
        ('J', 36),
        ('K', 37),
        ('L', 38),
        ('Z', 44),
        ('X', 45),
        ('C', 46),
        ('V', 47),
        ('B', 48),
        ('N', 49),
        ('M', 50),
    ];
    LETTERS
        .iter()
        .find_map(|(name, code)| (*name == letter).then_some(*code))
}

fn key_codes(name: &str) -> Option<Vec<u16>> {
    let name = name.trim();
    if let Some(button) = name.strip_prefix("Mouse") {
        return match button {
            "Right" => Some(vec![0x111]),
            "Middle" => Some(vec![0x112]),
            "Back" => Some(vec![0x113, 0x116]),
            "Forward" => Some(vec![0x114, 0x115]),
            _ => button
                .parse::<u16>()
                .ok()
                .filter(|code| *code <= KEY_MAX)
                .map(|code| vec![code]),
        };
    }
    key_code(name).map(|code| vec![code])
}

fn key_code(name: &str) -> Option<u16> {
    let name = name.trim();

    if let Some(number) = name
        .strip_prefix('F')
        .and_then(|rest| rest.parse::<u8>().ok())
    {
        return match number {
            1..=10 => Some(58 + number as u16),
            11 => Some(87),
            12 => Some(88),
            13..=24 => Some(183 + (number - 13) as u16),
            _ => None,
        };
    }

    match name {
        "Escape" => Some(1),
        "Backspace" => Some(14),
        "Tab" => Some(15),
        "Enter" => Some(28),
        "Space" => Some(57),
        "CapsLock" => Some(58),
        "NumLock" => Some(69),
        "ScrollLock" => Some(70),
        "PrintScreen" => Some(99),
        "Pause" => Some(119),
        "Insert" => Some(110),
        "Delete" => Some(111),
        "Home" => Some(102),
        "End" => Some(107),
        "PageUp" => Some(104),
        "PageDown" => Some(109),
        "ArrowUp" => Some(103),
        "ArrowDown" => Some(108),
        "ArrowLeft" => Some(105),
        "ArrowRight" => Some(106),
        "Shift" => Some(42),
        "Control" => Some(29),
        "Alt" => Some(56),
        "Super" | "Meta" => Some(125),
        "ContextMenu" => Some(139),
        "-" => Some(12),
        "=" => Some(13),
        "[" => Some(26),
        "]" => Some(27),
        "\\" => Some(43),
        ";" => Some(39),
        "'" => Some(40),
        "`" => Some(41),
        "," => Some(51),
        "." => Some(52),
        "/" => Some(53),
        _ => {
            let mut chars = name.chars();
            match (chars.next(), chars.next()) {
                (Some(single), None) => {
                    let upper = single.to_ascii_uppercase();
                    if upper.is_ascii_alphabetic() {
                        letter_code(upper)
                    } else {
                        single
                            .to_digit(10)
                            .map(|digit| if digit == 0 { 11 } else { digit as u16 + 1 })
                    }
                }
                _ => None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_common_keys() {
        assert_eq!(key_code("F6"), Some(64));
        assert_eq!(key_code("F13"), Some(183));
        assert_eq!(key_code("F24"), Some(194));
        assert_eq!(key_code("A"), Some(30));
        assert_eq!(key_code("z"), Some(44));
        assert_eq!(key_code("0"), Some(11));
        assert_eq!(key_code("9"), Some(10));
        assert_eq!(key_code("Space"), Some(57));
        assert_eq!(key_code("ArrowLeft"), Some(105));
    }

    #[test]
    fn rejects_unknown_keys() {
        assert_eq!(key_code("F25"), None);
        assert_eq!(key_code("Nope"), None);
    }

    #[test]
    fn maps_mouse_buttons() {
        assert_eq!(key_codes("MouseRight"), Some(vec![0x111]));
        assert_eq!(key_codes("MouseMiddle"), Some(vec![0x112]));
        assert_eq!(key_codes("MouseBack"), Some(vec![0x113, 0x116]));
        assert_eq!(key_codes("MouseForward"), Some(vec![0x114, 0x115]));
        assert_eq!(key_codes("Mouse999"), None);
    }
}
