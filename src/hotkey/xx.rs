//! Compositor-side global hotkeys via `xx_hotkey_v1` (experimental Wayland protocol).
//!
//! The client commits its own trigger and the compositor accepts or denies it, so no
//! compositor config, portal or `/dev/input` access is needed. Hyprland advertises it.
//! The protocol is experimental: anything unexpected returns `Err` so the caller falls
//! through to the portal or evdev.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use rustix::event::{PollFd, PollFlags, poll};
use rustix::time::Timespec;
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::{Connection, Dispatch, QueueHandle};

use super::{Event, HotkeyBackend, HotkeyStatus};

mod protocol {
    #![allow(dead_code, non_camel_case_types, unused_unsafe, unused_variables)]
    #![allow(non_upper_case_globals, non_snake_case, unused_imports)]
    #![allow(missing_docs, clippy::all)]

    pub mod client {
        use wayland_client;
        use wayland_client::protocol::*;

        pub mod __interfaces {
            use wayland_client::protocol::__interfaces::*;
            wayland_scanner::generate_interfaces!("./protocols/xx-hotkey-v1.xml");
        }
        use self::__interfaces::*;

        wayland_scanner::generate_client_code!("./protocols/xx-hotkey-v1.xml");
    }
}

use protocol::client::xx_hotkey_manager_v1::{Modifiers, XxHotkeyManagerV1};
use protocol::client::xx_hotkey_v1::{self, DenyReason, XxHotkeyV1};

const INTERFACE: &str = "xx_hotkey_manager_v1";
const POLL_TIMEOUT: Timespec = Timespec {
    tv_sec: 0,
    tv_nsec: 100_000_000,
};
/// The compositor answers each commit with exactly one `bound` or `denied`.
const COMMIT_WAIT: Duration = Duration::from_secs(5);

enum Trigger {
    Key(u32),
    Button(u32),
}

/// Taplet key name to an X11 keysym (unshifted) or a pointer button code.
fn trigger_for(name: &str) -> Option<Trigger> {
    let name = name.trim();
    if let Some(button) = name.strip_prefix("Mouse") {
        return match button {
            "Right" => Some(Trigger::Button(0x111)),
            "Middle" => Some(Trigger::Button(0x112)),
            "Back" => Some(Trigger::Button(0x113)),
            "Forward" => Some(Trigger::Button(0x114)),
            _ => button
                .parse::<u32>()
                .ok()
                .filter(|code| (0x110..0x160).contains(code))
                .map(Trigger::Button),
        };
    }
    keysym_for(name).map(Trigger::Key)
}

fn keysym_for(name: &str) -> Option<u32> {
    if let Some(number) = name
        .strip_prefix('F')
        .and_then(|rest| rest.parse::<u32>().ok())
    {
        // XK_F1 = 0xffbe, consecutive up to F35.
        return (1..=35).contains(&number).then(|| 0xffbe + number - 1);
    }

    let named = match name {
        "Space" => 0x0020,
        "Backspace" => 0xff08,
        "Tab" => 0xff09,
        "Enter" => 0xff0d,
        "Pause" => 0xff13,
        "ScrollLock" => 0xff14,
        "Escape" => 0xff1b,
        "Home" => 0xff50,
        "ArrowLeft" => 0xff51,
        "ArrowUp" => 0xff52,
        "ArrowRight" => 0xff53,
        "ArrowDown" => 0xff54,
        "PageUp" => 0xff55,
        "PageDown" => 0xff56,
        "End" => 0xff57,
        "PrintScreen" => 0xff61,
        "Insert" => 0xff63,
        "ContextMenu" => 0xff67,
        "NumLock" => 0xff7f,
        "Shift" => 0xffe1,
        "Control" => 0xffe3,
        "CapsLock" => 0xffe5,
        "Alt" => 0xffe9,
        "Super" | "Meta" => 0xffeb,
        "Delete" => 0xffff,
        _ => 0,
    };
    if named != 0 {
        return Some(named);
    }

    // Latin-1 keysyms equal the code point; the protocol wants the unshifted form.
    let mut chars = name.chars();
    match (chars.next(), chars.next()) {
        (Some(single), None) if single.is_ascii_graphic() => {
            Some(single.to_ascii_lowercase() as u32)
        }
        _ => None,
    }
}

fn deny_text(reason: wayland_client::WEnum<DenyReason>, message: &str) -> String {
    let why = match reason {
        wayland_client::WEnum::Value(DenyReason::AlreadyBound) => "already bound by something else",
        wayland_client::WEnum::Value(DenyReason::NotPermitted) => {
            "not permitted by compositor policy (plain letters and digits are usually refused; try an F-key)"
        }
        _ => "denied",
    };
    if message.is_empty() {
        format!("compositor denied the hotkey: {why}")
    } else {
        format!("compositor denied the hotkey: {why} ({message})")
    }
}

#[derive(Default)]
struct State {
    bound: bool,
    failure: Option<String>,
    fired: u32,
}

impl Dispatch<WlRegistry, GlobalListContents> for State {
    fn event(
        _state: &mut Self,
        _proxy: &WlRegistry,
        _event: <WlRegistry as wayland_client::Proxy>::Event,
        _data: &GlobalListContents,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<XxHotkeyManagerV1, ()> for State {
    fn event(
        _state: &mut Self,
        _proxy: &XxHotkeyManagerV1,
        _event: <XxHotkeyManagerV1 as wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<XxHotkeyV1, ()> for State {
    fn event(
        state: &mut Self,
        _proxy: &XxHotkeyV1,
        event: xx_hotkey_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            xx_hotkey_v1::Event::Bound => state.bound = true,
            xx_hotkey_v1::Event::Denied { reason, message } => {
                state.failure = Some(deny_text(reason, &message));
            }
            xx_hotkey_v1::Event::Revoked { message } => {
                state.bound = false;
                state.failure = Some(if message.is_empty() {
                    "compositor revoked the hotkey".to_string()
                } else {
                    format!("compositor revoked the hotkey ({message})")
                });
            }
            xx_hotkey_v1::Event::Triggered { .. } => state.fired += 1,
            _ => {}
        }
    }
}

/// True when the compositor advertises `xx_hotkey_manager_v1`. Binds nothing.
pub(super) fn available() -> bool {
    let Ok(conn) = Connection::connect_to_env() else {
        return false;
    };
    let Ok((globals, _queue)) = registry_queue_init::<State>(&conn) else {
        return false;
    };
    globals
        .contents()
        .with_list(|list| list.iter().any(|global| global.interface == INTERFACE))
}

pub(super) fn run(shortcut: &str, cancel: &AtomicBool, tx: &Sender<Event>) -> Result<(), String> {
    let conn = Connection::connect_to_env().map_err(|e| format!("wayland connect: {e}"))?;
    run_on(&conn, shortcut, cancel, tx)
}

fn run_on(
    conn: &Connection,
    shortcut: &str,
    cancel: &AtomicBool,
    tx: &Sender<Event>,
) -> Result<(), String> {
    let trigger = trigger_for(shortcut)
        .ok_or_else(|| format!("no xx_hotkey trigger for shortcut {shortcut:?}"))?;

    let (globals, mut queue) =
        registry_queue_init::<State>(conn).map_err(|e| format!("wayland registry: {e}"))?;
    let qh = queue.handle();
    let manager: XxHotkeyManagerV1 = globals
        .bind(&qh, 1..=1, ())
        .map_err(|_| format!("compositor does not advertise {INTERFACE}"))?;

    // Optional, but must precede the first commit.
    manager.set_app_id(crate::APP_ID.to_string());
    let hotkey = manager.create_hotkey(&qh, ());
    hotkey.set_description("Start/stop clicking".to_string());
    match trigger {
        Trigger::Key(keysym) => hotkey.set_key_trigger(keysym, Modifiers::empty()),
        Trigger::Button(button) => hotkey.set_button_trigger(button, Modifiers::empty()),
    }
    hotkey.commit();
    conn.flush().map_err(|e| format!("wayland flush: {e}"))?;

    let mut state = State::default();
    let mut announced = false;
    let deadline = Instant::now() + COMMIT_WAIT;

    let result = loop {
        if cancel.load(Ordering::Relaxed) {
            break Ok(());
        }

        queue
            .dispatch_pending(&mut state)
            .map_err(|e| format!("wayland dispatch: {e}"))?;

        if let Some(failure) = state.failure.take() {
            break Err(failure);
        }
        if state.bound && !announced {
            announced = true;
            if tx
                .send(Event::Status(HotkeyStatus::Active(HotkeyBackend::Xx)))
                .is_err()
            {
                break Ok(());
            }
        }
        while state.fired > 0 {
            state.fired -= 1;
            if tx.send(Event::Activated).is_err() {
                break;
            }
        }
        if !announced && Instant::now() >= deadline {
            break Err("compositor did not answer the hotkey commit".to_string());
        }

        conn.flush().map_err(|e| format!("wayland flush: {e}"))?;
        // `None` means events are already queued: loop and dispatch them.
        if let Some(guard) = queue.prepare_read() {
            let fd = guard.connection_fd();
            let mut fds = [PollFd::new(&fd, PollFlags::IN)];
            let ready = poll(&mut fds, Some(&POLL_TIMEOUT))
                .map(|n| n > 0)
                .unwrap_or(false);
            if ready {
                guard
                    .read()
                    .map_err(|e| format!("compositor connection lost: {e}"))?;
            }
        }
    };

    hotkey.destroy();
    manager.destroy();
    let _ = conn.flush();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(name: &str) -> Option<u32> {
        match trigger_for(name) {
            Some(Trigger::Key(sym)) => Some(sym),
            _ => None,
        }
    }

    #[test]
    fn maps_function_keys() {
        assert_eq!(key("F1"), Some(0xffbe));
        assert_eq!(key("F6"), Some(0xffc3));
        assert_eq!(key("F12"), Some(0xffc9));
        assert_eq!(key("F36"), None);
        assert_eq!(key("F0"), None);
    }

    #[test]
    fn maps_named_and_single_keys() {
        assert_eq!(key("Space"), Some(0x20));
        assert_eq!(key("Escape"), Some(0xff1b));
        assert_eq!(key("A"), Some(0x61));
        assert_eq!(key("a"), Some(0x61));
        assert_eq!(key("7"), Some(0x37));
        assert_eq!(key("-"), Some(0x2d));
        assert_eq!(key(""), None);
        assert_eq!(key("Nonsense"), None);
    }

    #[test]
    fn maps_mouse_buttons() {
        assert!(matches!(trigger_for("MouseMiddle"), Some(Trigger::Button(0x112))));
        assert!(matches!(trigger_for("MouseBack"), Some(Trigger::Button(0x113))));
        assert!(trigger_for("Mouse1").is_none());
    }

    // ---- fake compositor, built from the same XML ----------------------------------

    mod server_proto {
        #![allow(dead_code, non_camel_case_types, unused_unsafe, unused_variables)]
        #![allow(non_upper_case_globals, non_snake_case, unused_imports)]
        #![allow(missing_docs, clippy::all)]

        pub mod server {
            use wayland_server;
            use wayland_server::protocol::*;

            pub mod __interfaces {
                use wayland_server::protocol::__interfaces::*;
                wayland_scanner::generate_interfaces!("./protocols/xx-hotkey-v1.xml");
            }
            use self::__interfaces::*;

            wayland_scanner::generate_server_code!("./protocols/xx-hotkey-v1.xml");
        }
    }

    use server_proto::server::xx_hotkey_manager_v1::{self as srv_manager, XxHotkeyManagerV1 as SrvManager};
    use server_proto::server::xx_hotkey_v1::{self as srv_hotkey, DenyReason as SrvDeny, XxHotkeyV1 as SrvHotkey};
    use std::os::unix::net::UnixStream;
    use std::sync::{Arc, Mutex};
    use std::sync::mpsc::channel;
    use std::thread;
    use wayland_server::{Client, DataInit, Display, DisplayHandle, GlobalDispatch, New};

    #[derive(Default)]
    struct Seen {
        app_id: Option<String>,
        keysym: Option<u32>,
        button: Option<u32>,
        modifiers: Option<u32>,
        commits: u32,
        destroyed: bool,
    }

    struct Fake {
        seen: Arc<Mutex<Seen>>,
        deny: bool,
    }

    struct NoData;
    impl wayland_server::backend::ClientData for NoData {}

    impl GlobalDispatch<SrvManager, ()> for Fake {
        fn bind(
            _s: &mut Self,
            _h: &DisplayHandle,
            _c: &Client,
            resource: New<SrvManager>,
            _d: &(),
            init: &mut DataInit<'_, Self>,
        ) {
            init.init(resource, ());
        }
    }

    impl wayland_server::Dispatch<SrvManager, ()> for Fake {
        fn request(
            state: &mut Self,
            _c: &Client,
            _r: &SrvManager,
            request: srv_manager::Request,
            _d: &(),
            _h: &DisplayHandle,
            init: &mut DataInit<'_, Self>,
        ) {
            match request {
                srv_manager::Request::SetAppId { app_id } => {
                    state.seen.lock().unwrap().app_id = Some(app_id);
                }
                srv_manager::Request::CreateHotkey { id } => {
                    init.init(id, ());
                }
                _ => {}
            }
        }
    }

    impl wayland_server::Dispatch<SrvHotkey, ()> for Fake {
        fn request(
            state: &mut Self,
            _c: &Client,
            resource: &SrvHotkey,
            request: srv_hotkey::Request,
            _d: &(),
            _h: &DisplayHandle,
            _i: &mut DataInit<'_, Self>,
        ) {
            match request {
                srv_hotkey::Request::SetKeyTrigger { keysym, modifiers } => {
                    let mut seen = state.seen.lock().unwrap();
                    seen.keysym = Some(keysym);
                    seen.modifiers = Some(match modifiers {
                        wayland_server::WEnum::Value(m) => m.bits(),
                        wayland_server::WEnum::Unknown(raw) => raw,
                    });
                }
                srv_hotkey::Request::SetButtonTrigger { button, modifiers: _ } => {
                    state.seen.lock().unwrap().button = Some(button);
                }
                srv_hotkey::Request::Commit => {
                    state.seen.lock().unwrap().commits += 1;
                    if state.deny {
                        resource.denied(SrvDeny::NotPermitted, "policy says no".to_string());
                    } else {
                        resource.bound();
                        resource.triggered(7, 1000);
                        resource.released(8, 1010);
                        resource.triggered(9, 2000);
                    }
                }
                srv_hotkey::Request::Destroy => {
                    state.seen.lock().unwrap().destroyed = true;
                }
                _ => {}
            }
        }
    }

    /// Run the client against the fake compositor on a socketpair.
    fn drive(
        shortcut: &str,
        deny: bool,
        collect: impl FnOnce(&std::sync::mpsc::Receiver<Event>, &AtomicBool),
    ) -> (Result<(), String>, Arc<Mutex<Seen>>) {
        let seen = Arc::new(Mutex::new(Seen::default()));
        let (client_sock, server_sock) = UnixStream::pair().unwrap();
        let stop = Arc::new(AtomicBool::new(false));

        let server_seen = Arc::clone(&seen);
        let server_stop = Arc::clone(&stop);
        let server = thread::spawn(move || {
            let mut display: Display<Fake> = Display::new().unwrap();
            display.handle().create_global::<Fake, SrvManager, ()>(1, ());
            display
                .handle()
                .insert_client(server_sock, Arc::new(NoData))
                .unwrap();
            let mut fake = Fake { seen: server_seen, deny };
            while !server_stop.load(Ordering::Relaxed) {
                display.dispatch_clients(&mut fake).unwrap();
                display.flush_clients().unwrap();
                thread::sleep(Duration::from_millis(2));
            }
        });

        let conn = Connection::from_socket(client_sock).unwrap();
        let (tx, rx) = channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        let shortcut = shortcut.to_string();
        let worker = thread::spawn(move || run_on(&conn, &shortcut, &worker_cancel, &tx));

        collect(&rx, &cancel);
        cancel.store(true, Ordering::Relaxed);
        let result = worker.join().unwrap();
        thread::sleep(Duration::from_millis(50));
        stop.store(true, Ordering::Relaxed);
        server.join().unwrap();
        (result, seen)
    }

    #[test]
    fn binds_key_and_reports_activations() {
        let mut events = Vec::new();
        let (result, seen) = drive("F6", false, |rx, _| {
            let deadline = Instant::now() + Duration::from_secs(5);
            while events.len() < 3 && Instant::now() < deadline {
                if let Ok(event) = rx.recv_timeout(Duration::from_millis(100)) {
                    events.push(event);
                }
            }
        });
        assert_eq!(result, Ok(()));
        assert_eq!(
            events,
            vec![
                Event::Status(HotkeyStatus::Active(HotkeyBackend::Xx)),
                Event::Activated,
                Event::Activated,
            ]
        );
        let seen = seen.lock().unwrap();
        assert_eq!(seen.app_id.as_deref(), Some(crate::APP_ID));
        assert_eq!(seen.keysym, Some(0xffc3));
        assert_eq!(seen.modifiers, Some(0));
        assert_eq!(seen.commits, 1);
        assert!(seen.destroyed, "hotkey must be destroyed on exit");
    }

    #[test]
    fn binds_mouse_button() {
        let (result, seen) = drive("MouseMiddle", false, |rx, _| {
            let _ = rx.recv_timeout(Duration::from_secs(5));
        });
        assert_eq!(result, Ok(()));
        assert_eq!(seen.lock().unwrap().button, Some(0x112));
    }

    #[test]
    fn denied_commit_is_reported_with_reason() {
        let (result, _) = drive("a", true, |_, _| {
            thread::sleep(Duration::from_millis(500));
        });
        let err = result.unwrap_err();
        assert!(err.contains("not permitted"), "{err}");
        assert!(err.contains("policy says no"), "{err}");
    }
}
