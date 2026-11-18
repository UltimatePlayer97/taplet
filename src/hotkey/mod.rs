mod evdev;
mod portal;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyBackend {
    Portal,
    Uinput,
}

impl HotkeyBackend {
    pub fn label(self) -> &'static str {
        match self {
            HotkeyBackend::Portal => "XDG desktop portal",
            HotkeyBackend::Uinput => "uinput (reading /dev/input)",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HotkeyStatus {
    Registering,
    Active(HotkeyBackend),
    Unavailable(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Status(HotkeyStatus),
    Activated,
}

pub fn input_access_available() -> bool {
    evdev::input_access_available()
}

pub struct Hotkeys {
    events: Receiver<Event>,
    events_tx: Sender<Event>,
    cancel: Option<Arc<AtomicBool>>,
    current: Option<(String, bool)>,
}

impl Hotkeys {
    pub fn new() -> Self {
        let (events_tx, events) = mpsc::channel();
        Self {
            events,
            events_tx,
            cancel: None,
            current: None,
        }
    }

    /// Starts (or restarts) the global shortcut listener.
    ///
    /// Returns `true` when the request changed and a new listener was started.
    pub fn register(&mut self, shortcut: &str, prefer_uinput: bool) -> bool {
        let wanted = (shortcut.to_string(), prefer_uinput);
        if self.current.as_ref() == Some(&wanted) {
            return false;
        }
        if let Some(cancel) = self.cancel.take() {
            cancel.store(true, Ordering::Relaxed);
        }

        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel = Some(Arc::clone(&cancel));
        self.current = Some(wanted);

        let tx = self.events_tx.clone();
        let shortcut = shortcut.to_string();
        let spawned = thread::Builder::new()
            .name("taplet-hotkey".to_string())
            .spawn(move || listen(shortcut, prefer_uinput, cancel, tx));
        if spawned.is_err() {
            self.current = None;
            let _ = self.events_tx.send(Event::Status(HotkeyStatus::Unavailable(
                "could not start the shortcut listener thread".to_string(),
            )));
        }
        true
    }

    pub fn poll(&self) -> Vec<Event> {
        self.events.try_iter().collect()
    }
}

impl Drop for Hotkeys {
    fn drop(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            cancel.store(true, Ordering::Relaxed);
        }
    }
}

fn listen(shortcut: String, prefer_uinput: bool, cancel: Arc<AtomicBool>, tx: Sender<Event>) {
    let portal_error = if prefer_uinput {
        None
    } else {
        match portal::run(&shortcut, &cancel, &tx) {
            Ok(()) => return,
            Err(err) => {
                if cancel.load(Ordering::Relaxed) {
                    return;
                }
                Some(err)
            }
        }
    };

    match evdev::run(&shortcut, &cancel, &tx) {
        Ok(()) => {}
        Err(err) => {
            if cancel.load(Ordering::Relaxed) {
                return;
            }
            let reason = match portal_error {
                Some(portal) => format!("portal: {portal}; uinput: {err}"),
                None => format!("uinput: {err}"),
            };
            let _ = tx.send(Event::Status(HotkeyStatus::Unavailable(reason)));
        }
    }
}
