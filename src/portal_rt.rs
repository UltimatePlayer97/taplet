//! One process-wide Tokio runtime for every `ashpd` call.
//!
//! `ashpd` caches a single session-bus connection in a global. With the `tokio`
//! feature zbus runs that connection's tasks on the runtime that created it. A
//! short-lived runtime therefore kills the connection for every later portal call,
//! and those calls hang forever. Keeping one runtime alive on a parked thread
//! avoids that.

use std::future::Future;
use std::sync::OnceLock;
use std::sync::mpsc;
use std::thread;

use tokio::runtime::{Builder, Handle};

static HANDLE: OnceLock<Option<Handle>> = OnceLock::new();

fn handle() -> Option<&'static Handle> {
    HANDLE
        .get_or_init(|| {
            let (tx, rx) = mpsc::channel();
            thread::Builder::new()
                .name("taplet-portal-rt".to_string())
                .spawn(move || {
                    let Ok(rt) = Builder::new_current_thread().enable_all().build() else {
                        let _ = tx.send(None);
                        return;
                    };
                    // Host apps must register an app ID before any other portal call,
                    // on the same connection ashpd reuses. Failure is not fatal: the
                    // portal may be absent, or the ID may already be registered.
                    // Done before the handle is published so no call can race it.
                    if let Ok(id) = ashpd::AppID::try_from(crate::APP_ID) {
                        let _ = rt.block_on(ashpd::register_host_app(id));
                    }
                    let _ = tx.send(Some(rt.handle().clone()));
                    // Drive spawned tasks forever.
                    rt.block_on(std::future::pending::<()>());
                })
                .ok()?;
            rx.recv().ok().flatten()
        })
        .as_ref()
}

/// Run `future` on the shared runtime and wait for its output. `None` only when
/// the runtime could not start. Callers needing a deadline wrap this call.
pub fn run<T, F>(future: F) -> Option<T>
where
    T: Send + 'static,
    F: Future<Output = T> + Send + 'static,
{
    let handle = handle()?;
    let (tx, rx) = mpsc::channel();
    handle.spawn(async move {
        let _ = tx.send(future.await);
    });
    rx.recv().ok()
}
