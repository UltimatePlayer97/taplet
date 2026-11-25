use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use dbus::arg::{PropMap, RefArg, Variant};
use dbus::blocking::Connection;
use dbus::message::MatchRule;
use dbus::{Message, Path};

use super::{Event, HotkeyBackend, HotkeyStatus};

const SHORTCUT_ID: &str = "taplet-toggle";
const SHORTCUT_DESCRIPTION: &str = "Start or stop Taplet clicking";
const PORTAL_SERVICE: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const PORTAL_INTERFACE: &str = "org.freedesktop.portal.GlobalShortcuts";
const REQUEST_INTERFACE: &str = "org.freedesktop.portal.Request";
const SESSION_INTERFACE: &str = "org.freedesktop.portal.Session";
const CALL_TIMEOUT: Duration = Duration::from_secs(5);
const POLL_TIMEOUT: Duration = Duration::from_millis(100);
/// CreateSession never prompts, so a silent portal is broken.
const SESSION_WAIT: Duration = Duration::from_secs(10);
/// BindShortcuts may show a permission dialog on some desktops.
const BIND_WAIT: Duration = Duration::from_secs(120);
static TOKEN_COUNTER: AtomicU64 = AtomicU64::new(0);

enum Signal {
    Response {
        path: String,
        code: u32,
        results: PropMap,
    },
    Activated {
        session: String,
        shortcut_id: String,
    },
}

/// Register this process as `APP_ID` with the portal. Required for host apps, whose
/// ID portals cannot derive. Skipped inside Flatpak, where the ID is known.
fn register_host_app(connection: &Connection) -> Result<(), String> {
    if std::path::Path::new("/.flatpak-info").exists() {
        return Ok(());
    }
    // The Registry interface sits on the main portal object, same as the others.
    let proxy = connection.with_proxy(PORTAL_SERVICE, PORTAL_PATH, CALL_TIMEOUT);
    proxy
        .method_call::<(), _, _, _>(
            "org.freedesktop.host.portal.Registry",
            "Register",
            (crate::APP_ID, PropMap::new()),
        )
        .map_err(|err| format!("host registry: {err}"))
}

pub(super) fn run(
    shortcut: &str,
    cancel: &AtomicBool,
    tx: &Sender<Event>,
) -> Result<(), String> {
    let connection = Connection::new_session()
        .map_err(|err| format!("portal session bus: {err}"))?;
    let (signal_tx, signal_rx) = mpsc::channel();
    // Not fatal here: old portals lack the Registry. Kept to explain a later failure.
    let registry_note = register_host_app(&connection).err();
    let with_note = |err: String| match &registry_note {
        Some(note) => format!("{err} ({note})"),
        None => err,
    };

    let response_tx = signal_tx.clone();
    connection
        .add_match(
            MatchRule::new_signal(REQUEST_INTERFACE, "Response"),
            move |(code, results): (u32, PropMap), _: &Connection, message: &Message| {
                let path = message
                    .path()
                    .map(|path| path.to_string())
                    .unwrap_or_default();
                let _ = response_tx.send(Signal::Response {
                    path,
                    code,
                    results,
                });
                true
            },
        )
        .map_err(|err| format!("response match: {err}"))?;

    connection
        .add_match(
            MatchRule::new_signal(PORTAL_INTERFACE, "Activated"),
            move |(session, shortcut_id, _timestamp, _options): (
                Path<'static>,
                String,
                u64,
                PropMap,
            ),
                  _: &Connection,
                  _: &Message| {
                let _ = signal_tx.send(Signal::Activated {
                    session: session.to_string(),
                    shortcut_id,
                });
                true
            },
        )
        .map_err(|err| format!("activation match: {err}"))?;

    let proxy = connection.with_proxy(PORTAL_SERVICE, PORTAL_PATH, CALL_TIMEOUT);

    let session_options = session_options(&token("session"));
    let (create_request,): (Path<'static>,) = proxy
        .method_call(PORTAL_INTERFACE, "CreateSession", (session_options,))
        .map_err(|err| with_note(format!("create session: {err}")))?;
    let create_results =
        wait_response(
            &connection,
            &signal_rx,
            &create_request.to_string(),
            cancel,
            SESSION_WAIT,
        )?;
    let session_handle = create_results
        .get("session_handle")
        .and_then(|value| {
            dbus::arg::cast::<String>(&value.0)
                .cloned()
                .or_else(|| {
                    dbus::arg::cast::<Path<'static>>(&value.0).map(|path| path.to_string())
                })
        })
        .ok_or_else(|| "portal returned no session handle".to_string())?;

    let mut shortcut_properties: PropMap = PropMap::new();
    shortcut_properties.insert("description".to_string(), variant(SHORTCUT_DESCRIPTION));
    if let Some(trigger) = preferred_trigger(shortcut) {
        shortcut_properties.insert("preferred_trigger".to_string(), variant(trigger));
    }

    let shortcuts: Vec<(String, PropMap)> =
        vec![(SHORTCUT_ID.to_string(), shortcut_properties)];
    let bind_options = request_options(&token("bind"));
    let session_path =
        Path::new(session_handle.clone()).map_err(|err| format!("session handle: {err}"))?;
    let (bind_request,): (Path<'static>,) = proxy
        .method_call(
            PORTAL_INTERFACE,
            "BindShortcuts",
            (
                session_path,
                shortcuts,
                String::new(),
                bind_options,
            ),
        )
        .map_err(|err| format!("bind: {err}"))?;
    wait_response(
        &connection,
        &signal_rx,
        &bind_request.to_string(),
        cancel,
        BIND_WAIT,
    )?;

    if tx
        .send(Event::Status(HotkeyStatus::Active(HotkeyBackend::Portal)))
        .is_err()
    {
        return Ok(());
    }

    loop {
        if cancel.load(Ordering::Relaxed) {
            break;
        }

        connection
            .process(POLL_TIMEOUT)
            .map_err(|err| format!("dbus poll: {err}"))?;

        while let Ok(signal) = signal_rx.try_recv() {
            if let Signal::Activated {
                session,
                shortcut_id,
            } = signal
                && session == session_handle
                && shortcut_id == SHORTCUT_ID
                && tx.send(Event::Activated).is_err()
            {
                return Ok(());
            }
        }
    }

    close_session(&connection, &session_handle);
    Ok(())
}

fn wait_response(
    connection: &Connection,
    signals: &Receiver<Signal>,
    request_path: &str,
    cancel: &AtomicBool,
    limit: Duration,
) -> Result<PropMap, String> {
    let deadline = Instant::now() + limit;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err("portal request cancelled".to_string());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "portal sent no Response within {}s (backend not answering)",
                limit.as_secs()
            ));
        }

        connection
            .process(POLL_TIMEOUT)
            .map_err(|err| format!("dbus poll: {err}"))?;

        while let Ok(signal) = signals.try_recv() {
            if let Signal::Response {
                path,
                code,
                results,
            } = signal
                && path == request_path
            {
                if code != 0 {
                    return Err(format!("portal request rejected (code {code})"));
                }
                return Ok(results);
            }
        }
    }
}

fn close_session(connection: &Connection, session_handle: &str) {
    let Ok(session_path) = Path::new(session_handle.to_string()) else {
        return;
    };
    let proxy = connection.with_proxy(PORTAL_SERVICE, session_path, Duration::from_secs(1));
    let _: Result<(), dbus::Error> = proxy.method_call(SESSION_INTERFACE, "Close", ());
}

fn variant(value: impl Into<String>) -> Variant<Box<dyn RefArg>> {
    Variant(Box::new(value.into()))
}

fn request_options(handle_token: &str) -> PropMap {
    let mut options = PropMap::new();
    options.insert(
        "handle_token".to_string(),
        variant(handle_token.to_string()),
    );
    options
}

fn session_options(session_token: &str) -> PropMap {
    let mut options = request_options(session_token);
    options.insert(
        "session_handle_token".to_string(),
        variant(session_token.to_string()),
    );
    options
}

fn token(prefix: &str) -> String {
    let counter = TOKEN_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("taplet_{prefix}_{}_{counter}", std::process::id())
}

/// Best-effort conversion from a Taplet key name to the XDG shortcut syntax.
fn preferred_trigger(shortcut: &str) -> Option<String> {
    let shortcut = shortcut.trim();
    if shortcut.is_empty() || shortcut.starts_with("Mouse") {
        return None;
    }

    let named = match shortcut {
        "Space" => "space",
        "Enter" => "Return",
        "Escape" => "Escape",
        "Tab" => "Tab",
        "Backspace" => "BackSpace",
        "Delete" => "Delete",
        "Insert" => "Insert",
        "Home" => "Home",
        "End" => "End",
        "PageUp" => "Page_Up",
        "PageDown" => "Page_Down",
        "ArrowUp" => "Up",
        "ArrowDown" => "Down",
        "ArrowLeft" => "Left",
        "ArrowRight" => "Right",
        "PrintScreen" => "Print",
        "Pause" => "Pause",
        "Shift" => "Shift_L",
        "Control" => "Control_L",
        "Alt" => "Alt_L",
        "Super" | "Meta" => "Super_L",
        "ContextMenu" => "Menu",
        _ => "",
    };
    if !named.is_empty() {
        return Some(named.to_string());
    }

    let mut chars = shortcut.chars();
    match (chars.next(), chars.next()) {
        (Some(single), None) if single.is_alphanumeric() => {
            Some(single.to_lowercase().to_string())
        }
        (Some(_), None) => None,
        _ => Some(shortcut.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::preferred_trigger;

    #[test]
    fn converts_key_names_to_portal_triggers() {
        assert_eq!(preferred_trigger("F6").as_deref(), Some("F6"));
        assert_eq!(preferred_trigger("A").as_deref(), Some("a"));
        assert_eq!(preferred_trigger("Space").as_deref(), Some("space"));
        assert_eq!(preferred_trigger("ArrowLeft").as_deref(), Some("Left"));
        assert_eq!(preferred_trigger("MouseBack"), None);
        assert_eq!(preferred_trigger("-"), None);
        assert_eq!(preferred_trigger("  "), None);
    }
}
