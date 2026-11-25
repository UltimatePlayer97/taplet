mod evdev;
mod portal;
mod xx;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyBackend {
    /// Compositor-native `xx_hotkey_v1`: client picks the key, no config needed.
    Xx,
    Portal,
    Evdev,
}

impl HotkeyBackend {
    pub fn label(self) -> &'static str {
        match self {
            HotkeyBackend::Xx => "compositor hotkey (xx_hotkey_v1)",
            HotkeyBackend::Portal => "XDG desktop portal",
            HotkeyBackend::Evdev => "evdev (reading /dev/input)",
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

/// Order: compositor-native `xx_hotkey_v1`, XDG portal, then raw evdev. Each failure is
/// logged so the reason survives even when a later backend works.
fn listen(shortcut: String, prefer_uinput: bool, cancel: Arc<AtomicBool>, tx: Sender<Event>) {
    let mut notes: Vec<String> = Vec::new();

    if !prefer_uinput {
        if xx::available() {
            match xx::run(&shortcut, &cancel, &tx) {
                Ok(()) => return,
                Err(err) => {
                    if cancel.load(Ordering::Relaxed) {
                        return;
                    }
                    eprintln!("taplet: xx_hotkey_v1 failed, trying the portal: {err}");
                    notes.push(format!("xx_hotkey: {err}"));
                }
            }
        }

        match portal::run(&shortcut, &cancel, &tx) {
            Ok(()) => return,
            Err(err) => {
                if cancel.load(Ordering::Relaxed) {
                    return;
                }
                // Without this the reason is lost whenever the evdev fallback works.
                eprintln!("taplet: GlobalShortcuts portal failed, falling back to evdev: {err}");
                notes.push(format!("portal: {err}"));
            }
        }
    }

    match evdev::run(&shortcut, &cancel, &tx) {
        Ok(()) => {}
        Err(err) => {
            if cancel.load(Ordering::Relaxed) {
                return;
            }
            notes.push(format!("uinput: {err}"));
            let _ = tx.send(Event::Status(HotkeyStatus::Unavailable(notes.join("; "))));
        }
    }
}

/// True when the compositor advertises `xx_hotkey_manager_v1`.
pub fn xx_available() -> bool {
    xx::available()
}

type RunFn = fn(&str, &AtomicBool, &Sender<Event>) -> Result<(), String>;

/// Bind a throwaway shortcut through the portal and report the exact outcome.
/// Used by `taplet doctor --hotkey`. May show a permission dialog on some desktops.
pub fn probe_portal(shortcut: &str, wait: std::time::Duration) -> Result<(), String> {
    probe(shortcut, wait, portal::run)
}

/// Same for the compositor-native `xx_hotkey_v1` protocol.
pub fn probe_xx(shortcut: &str, wait: std::time::Duration) -> Result<(), String> {
    probe(shortcut, wait, xx::run)
}

fn probe(shortcut: &str, wait: std::time::Duration, run: RunFn) -> Result<(), String> {
    let cancel = Arc::new(AtomicBool::new(false));
    let (tx, rx) = std::sync::mpsc::channel();
    let worker_cancel = Arc::clone(&cancel);
    let worker_shortcut = shortcut.to_string();
    let handle = std::thread::spawn(move || run(&worker_shortcut, &worker_cancel, &tx));

    let deadline = std::time::Instant::now() + wait;
    let outcome = loop {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        match rx.recv_timeout(left) {
            Ok(Event::Status(HotkeyStatus::Active(_))) => break Ok(()),
            Ok(_) => {}
            // Sender dropped: the backend already returned, its error is the answer.
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                break match handle.join() {
                    Ok(Err(err)) => Err(err),
                    Ok(Ok(())) => Err("session ended immediately".to_string()),
                    Err(_) => Err("hotkey thread panicked".to_string()),
                };
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                break Err(format!("no answer within {}s", wait.as_secs()));
            }
        }
    };
    cancel.store(true, Ordering::Relaxed);
    outcome
}
