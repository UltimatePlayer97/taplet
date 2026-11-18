use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::time::Duration;

use ashpd::desktop::CreateSessionOptions;
use ashpd::desktop::global_shortcuts::{BindShortcutsOptions, GlobalShortcuts, NewShortcut};
use futures_util::StreamExt;

use super::{Event, HotkeyBackend, HotkeyStatus};

const SHORTCUT_ID: &str = "taplet-toggle";
const SHORTCUT_DESCRIPTION: &str = "Start or stop Taplet clicking";
const CANCEL_POLL: Duration = Duration::from_millis(200);

pub(super) fn run(shortcut: &str, cancel: &AtomicBool, tx: &Sender<Event>) -> Result<(), String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|err| format!("async runtime: {err}"))?;
    runtime.block_on(run_async(shortcut, cancel, tx))
}

async fn run_async(shortcut: &str, cancel: &AtomicBool, tx: &Sender<Event>) -> Result<(), String> {
    let portal = GlobalShortcuts::new()
        .await
        .map_err(|err| format!("portal unavailable: {err}"))?;
    let session = portal
        .create_session(CreateSessionOptions::default())
        .await
        .map_err(|err| format!("session: {err}"))?;

    let mut activations = portal
        .receive_activated()
        .await
        .map_err(|err| format!("activation stream: {err}"))?;

    let trigger = preferred_trigger(shortcut);
    let mut new_shortcut = NewShortcut::new(SHORTCUT_ID, SHORTCUT_DESCRIPTION);
    if let Some(trigger) = trigger.as_deref() {
        new_shortcut = new_shortcut.preferred_trigger(trigger);
    }

    let bound = portal
        .bind_shortcuts(
            &session,
            &[new_shortcut],
            None,
            BindShortcutsOptions::default(),
        )
        .await
        .map_err(|err| format!("bind: {err}"))?
        .response()
        .map_err(|err| format!("bind rejected: {err}"))?;
    if !bound
        .shortcuts()
        .iter()
        .any(|shortcut| shortcut.id() == SHORTCUT_ID)
    {
        return Err("portal did not bind the shortcut".to_string());
    }

    if tx
        .send(Event::Status(HotkeyStatus::Active(HotkeyBackend::Portal)))
        .is_err()
    {
        return Ok(());
    }

    loop {
        tokio::select! {
            activation = activations.next() => {
                let Some(activation) = activation else {
                    return Err("portal closed the shortcut stream".to_string());
                };
                if activation.shortcut_id() == SHORTCUT_ID
                    && tx.send(Event::Activated).is_err()
                {
                    return Ok(());
                }
            }
            _ = tokio::time::sleep(CANCEL_POLL) => {
                if cancel.load(Ordering::Relaxed) {
                    return Ok(());
                }
            }
        }
    }
}

/// Best-effort conversion from a Taplet key name to the XDG shortcut syntax.
fn preferred_trigger(shortcut: &str) -> Option<String> {
    let shortcut = shortcut.trim();
    if shortcut.is_empty() {
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
        (Some(single), None) if single.is_alphanumeric() => Some(single.to_lowercase().to_string()),
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
        assert_eq!(preferred_trigger("-"), None);
        assert_eq!(preferred_trigger("  "), None);
    }
}
