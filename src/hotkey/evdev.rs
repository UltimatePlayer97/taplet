use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use super::{Event, HotkeyBackend, HotkeyStatus};

const EV_KEY: u16 = 1;
const KEY_MAX: u16 = 0x2ff;
const POLL_TIMEOUT_MS: libc::c_int = 200;
const DEBOUNCE: Duration = Duration::from_millis(300);

struct Device {
    file: File,
}

fn eviocgbit(event_type: u16, len: u16) -> libc::c_ulong {
    (2u64 << 30) | ((len as u64) << 16) | ((b'E' as u64) << 8) | (0x20 + event_type as u64)
}

fn eviocgname(len: u16) -> libc::c_ulong {
    (2u64 << 30) | ((len as u64) << 16) | ((b'E' as u64) << 8) | 0x06
}

fn has_key(fd: libc::c_int, code: u16) -> bool {
    let mut types = [0u8; 8];
    let rc = unsafe { libc::ioctl(fd, eviocgbit(0, types.len() as u16), types.as_mut_ptr()) };
    if rc < 0 || types[EV_KEY as usize / 8] & (1 << (EV_KEY % 8)) == 0 {
        return false;
    }

    let len = (KEY_MAX as usize + 8) / 8;
    let mut keys = vec![0u8; len];
    let rc = unsafe { libc::ioctl(fd, eviocgbit(EV_KEY, len as u16), keys.as_mut_ptr()) };
    rc >= 0 && keys[code as usize / 8] & (1 << (code % 8)) != 0
}

fn device_name(fd: libc::c_int) -> Option<String> {
    let mut buf = [0u8; 256];
    let rc = unsafe { libc::ioctl(fd, eviocgname(buf.len() as u16), buf.as_mut_ptr()) };
    if rc < 0 {
        return None;
    }
    let end = buf.iter().position(|byte| *byte == 0).unwrap_or(buf.len());
    Some(String::from_utf8_lossy(&buf[..end]).into_owned())
}

fn scan(code: u16) -> Vec<Device> {
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
            .custom_flags(libc::O_NONBLOCK)
            .open(entry.path())
        else {
            continue;
        };
        let fd = file.as_raw_fd();
        if !has_key(fd, code) {
            continue;
        }
        if device_name(fd).is_some_and(|name| name.starts_with("taplet")) {
            continue;
        }
        devices.push(Device { file });
    }
    devices
}

fn read_activation(file: &File, code: u16) -> std::io::Result<bool> {
    let mut buf = [0u8; 4096];
    let mut activated = false;
    loop {
        let read = unsafe { libc::read(file.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len()) };
        if read < 0 {
            let err = std::io::Error::last_os_error();
            return match err.kind() {
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted => Ok(activated),
                _ => Err(err),
            };
        }
        if read == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "input device closed",
            ));
        }

        let size = std::mem::size_of::<libc::input_event>();
        let bytes = read as usize;
        for chunk in buf[..bytes].chunks_exact(size) {
            let event =
                unsafe { std::ptr::read_unaligned(chunk.as_ptr().cast::<libc::input_event>()) };
            if event.type_ == EV_KEY && event.code == code && event.value == 1 {
                activated = true;
            }
        }
        if bytes < buf.len() {
            return Ok(activated);
        }
    }
}

pub(super) fn run(shortcut: &str, cancel: &AtomicBool, tx: &Sender<Event>) -> Result<(), String> {
    let code = key_code(shortcut).ok_or_else(|| {
        format!("no keyboard mapping for shortcut {shortcut:?}; pick another key")
    })?;

    let mut devices = scan(code);
    if devices.is_empty() {
        return Err(format!(
            "no readable keyboard declares {shortcut} under /dev/input; add your user to the input group or use a portal that supports GlobalShortcuts"
        ));
    }
    if tx
        .send(Event::Status(HotkeyStatus::Active(HotkeyBackend::Uinput)))
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
            devices = scan(code);
            if devices.is_empty() {
                std::thread::sleep(Duration::from_millis(500));
                continue;
            }
        }

        let mut pollfds: Vec<libc::pollfd> = devices
            .iter()
            .map(|device| libc::pollfd {
                fd: device.file.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            })
            .collect();

        let rc = unsafe {
            libc::poll(
                pollfds.as_mut_ptr(),
                pollfds.len() as libc::nfds_t,
                POLL_TIMEOUT_MS,
            )
        };
        if rc < 0 {
            let err = std::io::Error::last_os_error();
            if err.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(format!("poll: {err}"));
        }
        if rc == 0 {
            continue;
        }

        let mut dead = Vec::new();
        let mut fired = false;
        for (index, pollfd) in pollfds.iter().enumerate() {
            if pollfd.revents & libc::POLLIN != 0 {
                match read_activation(&devices[index].file, code) {
                    Ok(true) => fired = true,
                    Ok(false) => {}
                    Err(_) => dead.push(index),
                }
            } else if pollfd.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
                dead.push(index);
            }
        }
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
}
