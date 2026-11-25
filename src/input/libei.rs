use std::os::unix::net::UnixStream;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

use ashpd::desktop::PersistMode;
use ashpd::desktop::remote_desktop::{
    ConnectToEISOptions, DeviceType, RemoteDesktop, SelectDevicesOptions, StartOptions,
};
use reis::ei::Context;
use reis::ei::handshake::ContextType;
use reis::event::{DeviceCapability, EiEvent, EiEventConverter};
use reis::handshake::EiHandshaker;
use reis::PendingRequestResult;

use super::{InputBackend, InputError, MouseButton};

enum WorkerCmd {
    Click(MouseButton),
    ReleaseAll,
    Shutdown,
}

/// Per-connection EIS state, driven entirely on the worker thread.
///
/// Exactly one button-capable device is used. Compositors may offer several
/// (relative + absolute pointer); sending the click to all would double-click.
struct WorkerState {
    context: Context,
    converter: EiEventConverter,
    connection: reis::event::Connection,
    device: Option<reis::event::Device>,
    button: Option<reis::ei::button::Button>,
    device_ready: bool,
    emulating: bool,
    sequence: u32,
    /// Buttons currently pressed. A click is atomic, so this is only non-empty
    /// if a send failed between press and release.
    held: Vec<MouseButton>,
}

impl WorkerState {
    fn timestamp_us() -> u64 {
        let now = rustix::time::clock_gettime(rustix::time::ClockId::Monotonic);
        (now.tv_sec as u64)
            .wrapping_mul(1_000_000)
            .wrapping_add(now.tv_nsec as u64 / 1_000)
    }

    fn flush(&self) -> Result<(), InputError> {
        self.context
            .flush()
            .map_err(|e| InputError::LibeiConnection(e.to_string()))
    }

    fn send_button(&mut self, button: MouseButton, pressed: bool) {
        if pressed {
            if !self.held.contains(&button) {
                self.held.push(button);
            }
        } else {
            self.held.retain(|b| *b != button);
        }
        let state = if pressed {
            reis::ei::button::ButtonState::Press
        } else {
            reis::ei::button::ButtonState::Released
        };
        if let Some(btn) = &self.button {
            btn.button(button.evdev_code(), state);
        }
    }

    fn frame(&self) -> Result<(), InputError> {
        if let Some(dev) = &self.device {
            dev.device()
                .frame(self.connection.serial(), Self::timestamp_us());
        }
        self.flush()
    }

    fn start_emulating(&mut self) -> Result<(), InputError> {
        if self.emulating {
            return Ok(());
        }
        self.sequence = self.sequence.wrapping_add(1);
        if let Some(dev) = &self.device {
            dev.device()
                .start_emulating(self.connection.serial(), self.sequence);
        }
        self.flush()?;
        self.emulating = true;
        Ok(())
    }

    fn stop_emulating(&mut self) -> Result<(), InputError> {
        if !self.emulating {
            return Ok(());
        }
        if let Some(dev) = &self.device {
            dev.device().stop_emulating(self.connection.serial());
        }
        self.flush()?;
        self.emulating = false;
        Ok(())
    }

    fn click(&mut self, button: MouseButton) -> Result<(), InputError> {
        if !self.device_ready {
            return Err(InputError::LibeiConnection(
                "libei device is paused by the compositor".to_string(),
            ));
        }
        self.start_emulating()?;
        self.send_button(button, true);
        self.frame()?;
        self.send_button(button, false);
        self.frame()
    }

    /// Release any held buttons and stop the emulation sequence. Best effort.
    fn release_all(&mut self) {
        // Only release what is actually held, and never talk to a paused device.
        if self.device_ready && self.emulating && !self.held.is_empty() {
            for button in std::mem::take(&mut self.held) {
                self.send_button(button, false);
            }
            let _ = self.frame();
        }
        self.held.clear();
        let _ = self.stop_emulating();
        self.emulating = false;
    }

    /// Drain any pending EIS events without blocking.
    fn process_events(&mut self) -> Result<(), InputError> {
        match self.context.read() {
            Ok(_) => {}
            Err(err) if err.kind() == std::io::ErrorKind::UnexpectedEof => {
                return Err(InputError::LibeiConnection("EIS disconnected".to_string()));
            }
            Err(err) => return Err(InputError::LibeiConnection(err.to_string())),
        }

        while let Some(result) = self.context.pending_event() {
            let event = match result {
                PendingRequestResult::Request(event) => event,
                PendingRequestResult::ParseError(err) => {
                    return Err(InputError::LibeiConnection(err.to_string()));
                }
                PendingRequestResult::InvalidObject(_) => continue,
            };
            self.converter
                .handle_event(event)
                .map_err(|e| InputError::LibeiConnection(e.to_string()))?;
        }

        while let Some(event) = self.converter.next_event() {
            self.handle_event(event)?;
        }
        // Seat binds and similar replies queued above must reach the server.
        self.flush()
    }

    fn handle_event(&mut self, event: EiEvent) -> Result<(), InputError> {
        match event {
            EiEvent::Disconnected(_) => {
                return Err(InputError::LibeiConnection("EIS disconnected".to_string()));
            }
            EiEvent::SeatAdded(seat_added) => {
                // Capabilities the seat does not advertise map to nothing, so it is
                // safe to ask for both. Absolute pointer is left out: it would add a
                // second button-capable device and only matters for fixed positions.
                let caps = DeviceCapability::Pointer | DeviceCapability::Button;
                seat_added.seat.bind_capabilities(caps);
            }
            EiEvent::DeviceAdded(device_added) => {
                let device = device_added.device;
                if self.device.is_none() && device.has_capability(DeviceCapability::Button) {
                    if let Some(button) = device.interface::<reis::ei::button::Button>() {
                        self.device = Some(device);
                        self.button = Some(button);
                    }
                }
            }
            EiEvent::DeviceRemoved(removed) => {
                if self.device.as_ref() == Some(&removed.device) {
                    self.device = None;
                    self.button = None;
                    self.device_ready = false;
                    self.emulating = false;
                    self.held.clear();
                }
            }
            EiEvent::DeviceResumed(resumed) => {
                if self.device.as_ref() == Some(&resumed.device) {
                    self.device_ready = true;
                }
            }
            EiEvent::DevicePaused(paused) => {
                if self.device.as_ref() == Some(&paused.device) {
                    // The server already stopped emulation; sending now is a protocol error.
                    self.device_ready = false;
                    self.emulating = false;
                    self.held.clear();
                }
            }
            _ => {}
        }
        Ok(())
    }
}

enum WorkerResult {
    Ready { restore_token: Option<String> },
    Failed(InputError),
}

fn portal_err(err: impl std::fmt::Display) -> InputError {
    InputError::LibeiPortal(err.to_string())
}

fn run_worker(
    restore_token: Option<String>,
    cmd_rx: Receiver<WorkerCmd>,
    result_tx: Sender<WorkerResult>,
) {
    // Portal calls run on the shared runtime (see portal_rt): ashpd's cached bus
    // connection must outlive every call, and the session must outlive the EIS fd.
    let setup = crate::portal_rt::run(async move {
        let portal = RemoteDesktop::new().await.map_err(portal_err)?;
        let session = portal
            .create_session(Default::default())
            .await
            .map_err(portal_err)?;

        let mut select_opts = SelectDevicesOptions::default()
            .set_devices(DeviceType::POINTER)
            .set_persist_mode(PersistMode::ExplicitlyRevoked);
        if let Some(token) = restore_token.as_deref() {
            select_opts = select_opts.set_restore_token(token);
        }
        portal
            .select_devices(&session, select_opts)
            .await
            .map_err(portal_err)?;

        let selected = portal
            .start(&session, None, StartOptions::default())
            .await
            .map_err(portal_err)?
            .response()
            .map_err(portal_err)?;
        let new_token = selected.restore_token().map(str::to_string);

        let fd = portal
            .connect_to_eis(&session, ConnectToEISOptions::default())
            .await
            .map_err(portal_err)?;
        Ok::<_, InputError>((portal, session, fd, new_token))
    })
    .unwrap_or_else(|| {
        Err(InputError::LibeiConnection(
            "portal runtime unavailable".to_string(),
        ))
    });

    let (_portal, session, fd, new_token) = match setup {
        Ok(parts) => parts,
        Err(err) => {
            let _ = result_tx.send(WorkerResult::Failed(err));
            return;
        }
    };

    let context = match Context::new(UnixStream::from(fd)) {
        Ok(context) => context,
        Err(err) => {
            let _ = result_tx.send(WorkerResult::Failed(InputError::LibeiConnection(
                err.to_string(),
            )));
            return;
        }
    };

    run_eis(context, new_token, cmd_rx, result_tx);

    // Best effort: tell the portal we are done so the compositor drops the session.
    let _ = crate::portal_rt::run(async move {
        let _keep = _portal;
        session.close().await
    });
}

/// Drive one EIS connection: handshake, wait for a resumed button device, then
/// serve click commands until shutdown or disconnect. Portal-independent, so it
/// can be tested against an in-process EIS server.
fn run_eis(
    context: Context,
    new_token: Option<String>,
    cmd_rx: Receiver<WorkerCmd>,
    result_tx: Sender<WorkerResult>,
) {
    let handshake_resp = match blocking_handshake(&context) {
        Ok(resp) => resp,
        Err(err) => {
            let _ = result_tx.send(WorkerResult::Failed(err));
            return;
        }
    };

    let converter = EiEventConverter::new(&context, handshake_resp);
    let connection = converter.connection().clone();

    let mut state = WorkerState {
        context,
        converter,
        connection,
        device: None,
        button: None,
        device_ready: false,
        emulating: false,
        sequence: 0,
        held: Vec::new(),
    };

    // Do not report Ready until the compositor hands over a resumed button device.
    // Otherwise the first clicks would be silently dropped.
    let deadline = Instant::now() + Duration::from_secs(5);
    while !state.device_ready {
        if let Err(err) = state.process_events() {
            let _ = result_tx.send(WorkerResult::Failed(err));
            return;
        }
        if Instant::now() >= deadline {
            let _ = result_tx.send(WorkerResult::Failed(InputError::LibeiConnection(
                "compositor offered no usable pointer button device".to_string(),
            )));
            return;
        }
        thread::sleep(Duration::from_millis(5));
    }

    let _ = result_tx.send(WorkerResult::Ready {
        restore_token: new_token,
    });

    loop {
        match cmd_rx.recv_timeout(Duration::from_millis(5)) {
            Ok(WorkerCmd::Click(button)) => {
                if let Err(err) = state.click(button) {
                    eprintln!("libei: {err}");
                }
            }
            Ok(WorkerCmd::ReleaseAll) => state.release_all(),
            Ok(WorkerCmd::Shutdown) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                state.release_all();
                break;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }

        if let Err(err) = state.process_events() {
            eprintln!("libei: {err}");
            state.release_all();
            break;
        }
    }
}

fn blocking_handshake(context: &Context) -> Result<reis::handshake::HandshakeResp, InputError> {
    let mut handshaker = EiHandshaker::new("taplet", ContextType::Sender);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if Instant::now() >= deadline {
            return Err(InputError::LibeiHandshake("timed out".to_string()));
        }
        match context.read() {
            Ok(_) => {}
            Err(err) if err.kind() == std::io::ErrorKind::UnexpectedEof => {
                return Err(InputError::LibeiHandshake(
                    "connection closed during handshake".to_string(),
                ));
            }
            Err(err) => return Err(InputError::LibeiConnection(err.to_string())),
        }
        while let Some(result) = context.pending_event() {
            let event = match result {
                PendingRequestResult::Request(event) => event,
                PendingRequestResult::ParseError(err) => {
                    return Err(InputError::LibeiHandshake(err.to_string()));
                }
                PendingRequestResult::InvalidObject(_) => continue,
            };
            if let Some(resp) = handshaker
                .handle_event(event)
                .map_err(|e| InputError::LibeiHandshake(e.to_string()))?
            {
                return Ok(resp);
            }
        }
        thread::sleep(Duration::from_millis(5));
    }
}

pub struct LibeiBackend {
    tx: Option<Sender<WorkerCmd>>,
    worker: Option<thread::JoinHandle<()>>,
    restore_token: Option<String>,
}

impl LibeiBackend {
    pub fn new(restore_token: Option<String>) -> Self {
        Self {
            tx: None,
            worker: None,
            restore_token,
        }
    }

    /// Persist a freshly minted restore token so the next launch skips the prompt.
    fn store_token(&mut self, token: Option<String>) {
        if token == self.restore_token {
            return;
        }
        self.restore_token = token.clone();
        if let Ok(mut config) = crate::config::Config::load() {
            if config.libei_restore_token != token {
                config.libei_restore_token = token;
                let _ = config.save();
            }
        }
    }
}

impl InputBackend for LibeiBackend {
    fn name(&self) -> &'static str {
        "libei"
    }

    fn connect(&mut self) -> Result<(), InputError> {
        // A dead worker (EIS dropped, session closed) must not count as connected.
        if self.worker.as_ref().is_some_and(|w| w.is_finished()) {
            self.disconnect()?;
        }
        if self.tx.is_some() {
            return Ok(());
        }
        check_libei()?;

        let mut token = self.restore_token.clone();
        let mut retried_without_token = false;

        loop {
            let (cmd_tx, cmd_rx) = mpsc::channel();
            let (result_tx, result_rx) = mpsc::channel();
            let worker_token = token.clone();

            let handle = thread::Builder::new()
                .name("taplet-libei".to_string())
                .spawn(move || run_worker(worker_token, cmd_rx, result_tx))
                .map_err(|err| InputError::LibeiConnection(err.to_string()))?;

            self.tx = Some(cmd_tx);
            self.worker = Some(handle);

            match result_rx.recv_timeout(Duration::from_secs(15)) {
                Ok(WorkerResult::Ready { restore_token }) => {
                    self.store_token(restore_token);
                    return Ok(());
                }
                Ok(WorkerResult::Failed(err)) => {
                    self.disconnect()?;
                    let message = err.to_string();
                    let token_related = message.contains("token")
                        || message.contains("restore")
                        || message.contains("permission")
                        || message.contains("denied")
                        || message.contains("rejected");
                    if !retried_without_token && token.is_some() && token_related {
                        // Token may be stale or revoked: retry once, fresh consent.
                        token = None;
                        retried_without_token = true;
                        continue;
                    }
                    return Err(err);
                }
                Err(_) => {
                    self.disconnect()?;
                    return Err(InputError::LibeiConnection(
                        "timed out waiting for the RemoteDesktop portal".to_string(),
                    ));
                }
            }
        }
    }

    fn press_button(&mut self, button: MouseButton) -> Result<(), InputError> {
        let tx = self.tx.as_ref().ok_or(InputError::NotConnected)?;
        tx.send(WorkerCmd::Click(button))
            .map_err(|_| InputError::LibeiConnection("libei worker channel closed".to_string()))
    }

    fn release_button(&mut self, _button: MouseButton) -> Result<(), InputError> {
        // Each click is a complete press/release cycle handled by the worker.
        Ok(())
    }

    fn release_all(&mut self) -> Result<(), InputError> {
        if let Some(tx) = &self.tx {
            let _ = tx.send(WorkerCmd::ReleaseAll);
        }
        Ok(())
    }

    fn disconnect(&mut self) -> Result<(), InputError> {
        if let Some(tx) = self.tx.take() {
            let _ = tx.send(WorkerCmd::Shutdown);
        }
        if let Some(handle) = self.worker.take() {
            let _ = handle.join();
        }
        Ok(())
    }

    fn move_to(&mut self, _x: i32, _y: i32) -> Result<(), InputError> {
        Err(InputError::LibeiConnection(
            "fixed-position clicks are not supported on the libei backend; use cursor mode"
                .to_string(),
        ))
    }
}

impl Drop for LibeiBackend {
    fn drop(&mut self) {
        let _ = self.disconnect();
    }
}

/// Probe whether `zwlr_virtual_pointer_manager_v1` is advertised by the compositor.
/// Creates no portal session.
pub fn probe_wlr() -> bool {
    use wayland_client::Connection;
    use wayland_client::Dispatch;
    use wayland_client::QueueHandle;
    use wayland_client::globals::{GlobalListContents, registry_queue_init};
    use wayland_protocols_wlr::virtual_pointer::v1::client::zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1;

    struct ProbeHandler;

    impl Dispatch<wayland_client::protocol::wl_registry::WlRegistry, GlobalListContents>
        for ProbeHandler
    {
        fn event(
            _state: &mut Self,
            _proxy: &wayland_client::protocol::wl_registry::WlRegistry,
            _event: <wayland_client::protocol::wl_registry::WlRegistry as wayland_client::Proxy>::Event,
            _data: &GlobalListContents,
            _conn: &Connection,
            _qh: &QueueHandle<Self>,
        ) {
        }
    }

    impl Dispatch<ZwlrVirtualPointerManagerV1, ()> for ProbeHandler {
        fn event(
            _state: &mut Self,
            _proxy: &ZwlrVirtualPointerManagerV1,
            _event: <ZwlrVirtualPointerManagerV1 as wayland_client::Proxy>::Event,
            _data: &(),
            _conn: &Connection,
            _qh: &QueueHandle<Self>,
        ) {
        }
    }

    let conn = match Connection::connect_to_env() {
        Ok(conn) => conn,
        Err(_) => return false,
    };
    let (globals, mut queue) = match registry_queue_init::<ProbeHandler>(&conn) {
        Ok(globals) => globals,
        Err(_) => return false,
    };
    let qh = queue.handle();
    let _ = globals.bind::<ZwlrVirtualPointerManagerV1, ProbeHandler, ()>(&qh, 1..=2, ());
    queue.roundtrip(&mut ProbeHandler).is_ok()
}

/// Check the RemoteDesktop portal without creating a session (no permission prompt):
/// the interface must exist and advertise the POINTER device type.
pub fn check_libei() -> Result<(), InputError> {
    crate::portal_rt::run(async {
        let portal = RemoteDesktop::new()
            .await
            .map_err(|_| InputError::LibeiPortalUnavailable)?;
        let types = portal
            .available_device_types()
            .await
            .map_err(|_| InputError::LibeiPortalUnavailable)?;
        if types.contains(DeviceType::POINTER) {
            Ok(())
        } else {
            Err(InputError::LibeiPointerUnsupported)
        }
    })
    .unwrap_or_else(|| Err(InputError::LibeiConnection("portal runtime unavailable".into())))
}

/// Probe whether `/dev/uinput` can be opened for writing. Creates no device.
pub fn probe_uinput() -> bool {
    std::fs::OpenOptions::new()
        .write(true)
        .open("/dev/uinput")
        .is_ok()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeResult {
    Wlr,
    Libei,
    Uinput,
    None,
}

/// Pure selection policy. Order: wlr, libei, uinput.
pub fn select_backend(wlr: bool, libei: bool, uinput: bool) -> ProbeResult {
    if wlr {
        ProbeResult::Wlr
    } else if libei {
        ProbeResult::Libei
    } else if uinput {
        ProbeResult::Uinput
    } else {
        ProbeResult::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reis::eis;
    use reis::request::{DeviceCapability as Cap, EisRequest, EisRequestConverter};
    use std::sync::mpsc::channel;

    #[test]
    fn probe_result_is_copy() {
        let a = ProbeResult::Wlr;
        let b = a;
        assert_eq!(a, b);
    }

    #[test]
    fn selection_prefers_wlr_then_libei_then_uinput() {
        use ProbeResult::*;
        assert_eq!(select_backend(true, true, true), Wlr);
        assert_eq!(select_backend(false, true, true), Libei);
        assert_eq!(select_backend(false, false, true), Uinput);
        assert_eq!(select_backend(false, false, false), None);
        assert_eq!(select_backend(true, false, false), Wlr);
        assert_eq!(select_backend(false, true, false), Libei);
    }

    /// Button event as seen by the fake compositor: (device name, code, pressed).
    type Seen = (String, u32, bool);

    /// Minimal EIS server. Offers a relative AND an absolute pointer, both with
    /// buttons, like real compositors can. Records every button request.
    fn fake_server(
        socket: UnixStream,
        seen: Sender<Seen>,
        pause_after_first_click: bool,
    ) -> thread::JoinHandle<()> {
        thread::spawn(move || {
            let ctx = eis::Context::new(socket).unwrap();
            let mut handshaker = reis::handshake::EisHandshaker::new(&ctx, 0);
            let mut converter: Option<EisRequestConverter> = None;
            let mut devices: Vec<reis::request::Device> = Vec::new();
            let mut clicks = 0;
            let deadline = Instant::now() + Duration::from_secs(10);

            while Instant::now() < deadline {
                if ctx.read().is_err() {
                    return;
                }
                while let Some(result) = ctx.pending_request() {
                    let PendingRequestResult::Request(request) = result else {
                        continue;
                    };
                    if let Some(conv) = converter.as_mut() {
                        conv.handle_request(request).unwrap();
                        while let Some(req) = conv.next_request() {
                            match req {
                                EisRequest::Bind(bind) => {
                                    for (name, cap) in
                                        [("pointer", Cap::Pointer), ("pointer-abs", Cap::PointerAbsolute)]
                                    {
                                        if bind.capabilities.contains(cap)
                                            || (cap == Cap::PointerAbsolute
                                                && bind.capabilities.contains(Cap::Button))
                                        {
                                            let dev = bind.seat.add_device(
                                                Some(name),
                                                eis::device::DeviceType::Virtual,
                                                cap | Cap::Button,
                                                |_| {},
                                            );
                                            dev.resumed();
                                            devices.push(dev);
                                        }
                                    }
                                }
                                EisRequest::Button(b) => {
                                    let name = b.device.name().unwrap_or("").to_string();
                                    let pressed = b.state == eis::button::ButtonState::Press;
                                    let _ = seen.send((name, b.button, pressed));
                                    if !pressed {
                                        clicks += 1;
                                        if pause_after_first_click && clicks == 1 {
                                            for d in &devices {
                                                d.paused();
                                            }
                                        }
                                    }
                                }
                                EisRequest::Disconnect => return,
                                _ => {}
                            }
                        }
                    } else if let Some(resp) = handshaker.handle_request(request).unwrap() {
                        let conv = EisRequestConverter::new(&ctx, resp, 1);
                        let _seat = conv.handle().add_seat(
                            Some("default"),
                            Cap::Pointer | Cap::PointerAbsolute | Cap::Button,
                        );
                        converter = Some(conv);
                    }
                }
                if let Some(conv) = &converter {
                    let _ = conv.handle().flush();
                }
                let _ = ctx.flush();
                thread::sleep(Duration::from_millis(2));
            }
        })
    }

    fn start_client(
        pause_after_first_click: bool,
    ) -> (
        Sender<WorkerCmd>,
        Receiver<WorkerResult>,
        Receiver<Seen>,
        thread::JoinHandle<()>,
    ) {
        let (client_sock, server_sock) = UnixStream::pair().unwrap();
        let (seen_tx, seen_rx) = channel();
        let _server = fake_server(server_sock, seen_tx, pause_after_first_click);
        let (cmd_tx, cmd_rx) = channel();
        let (res_tx, res_rx) = channel();
        let ctx = Context::new(client_sock).unwrap();
        let worker = thread::spawn(move || run_eis(ctx, Some("tok".into()), cmd_rx, res_tx));
        (cmd_tx, res_rx, seen_rx, worker)
    }

    #[test]
    fn click_is_single_press_release_on_one_device() {
        let (cmd, res, seen, worker) = start_client(false);
        match res.recv_timeout(Duration::from_secs(5)).unwrap() {
            WorkerResult::Ready { restore_token } => assert_eq!(restore_token.as_deref(), Some("tok")),
            WorkerResult::Failed(e) => panic!("worker failed: {e}"),
        }
        cmd.send(WorkerCmd::Click(MouseButton::Left)).unwrap();
        cmd.send(WorkerCmd::Click(MouseButton::Right)).unwrap();
        cmd.send(WorkerCmd::Shutdown).unwrap();
        worker.join().unwrap();

        let events: Vec<Seen> = seen.try_iter().collect();
        // Allow the server a moment if events trail the join.
        let events = if events.len() < 4 {
            thread::sleep(Duration::from_millis(200));
            events.into_iter().chain(seen.try_iter()).collect()
        } else {
            events
        };
        assert_eq!(events.len(), 4, "got {events:?}");
        let dev = &events[0].0;
        assert!(events.iter().all(|e| &e.0 == dev), "clicked >1 device: {events:?}");
        assert_eq!(
            events.iter().map(|e| (e.1, e.2)).collect::<Vec<_>>(),
            vec![(0x110, true), (0x110, false), (0x111, true), (0x111, false)]
        );
    }

    #[test]
    fn paused_device_drops_clicks_without_stuck_button() {
        let (cmd, res, seen, worker) = start_client(true);
        assert!(matches!(
            res.recv_timeout(Duration::from_secs(5)).unwrap(),
            WorkerResult::Ready { .. }
        ));
        cmd.send(WorkerCmd::Click(MouseButton::Left)).unwrap();
        thread::sleep(Duration::from_millis(300)); // server pauses after first click
        cmd.send(WorkerCmd::Click(MouseButton::Left)).unwrap(); // must be refused
        thread::sleep(Duration::from_millis(100));
        cmd.send(WorkerCmd::Shutdown).unwrap();
        worker.join().unwrap();
        thread::sleep(Duration::from_millis(100));

        let events: Vec<Seen> = seen.try_iter().collect();
        let presses = events.iter().filter(|e| e.2).count();
        let releases = events.iter().filter(|e| !e.2).count();
        assert_eq!(presses, releases, "unbalanced: {events:?}");
        assert_eq!(presses, 1, "click while paused leaked: {events:?}");
    }
}
