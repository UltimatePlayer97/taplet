use std::collections::HashSet;

use rustix::time::{ClockId, clock_gettime};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_output::WlOutput;
use wayland_client::protocol::wl_pointer::ButtonState;
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle};
use wayland_protocols::xdg::xdg_output::zv1::client::zxdg_output_manager_v1::ZxdgOutputManagerV1;
use wayland_protocols::xdg::xdg_output::zv1::client::zxdg_output_v1::{self, ZxdgOutputV1};
use wayland_protocols_wlr::virtual_pointer::v1::client::zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1;
use wayland_protocols_wlr::virtual_pointer::v1::client::zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1;

use super::{InputBackend, MouseButton};
use crate::error::InputError;

#[derive(Debug, Clone, Copy, Default)]
struct OutputRect {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

#[derive(Default)]
struct Handler {
    outputs: Vec<OutputRect>,
}

impl Handler {
    fn bounds(&self) -> Option<OutputRect> {
        let mut known = self.outputs.iter().filter(|rect| rect.w > 0 && rect.h > 0);
        let first = known.next()?;
        let mut min_x = first.x;
        let mut min_y = first.y;
        let mut max_x = first.x + first.w;
        let mut max_y = first.y + first.h;
        for rect in known {
            min_x = min_x.min(rect.x);
            min_y = min_y.min(rect.y);
            max_x = max_x.max(rect.x + rect.w);
            max_y = max_y.max(rect.y + rect.h);
        }
        Some(OutputRect {
            x: min_x,
            y: min_y,
            w: max_x - min_x,
            h: max_y - min_y,
        })
    }
}

impl Dispatch<WlRegistry, GlobalListContents> for Handler {
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

impl Dispatch<ZwlrVirtualPointerManagerV1, ()> for Handler {
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

impl Dispatch<ZwlrVirtualPointerV1, ()> for Handler {
    fn event(
        _state: &mut Self,
        _proxy: &ZwlrVirtualPointerV1,
        _event: <ZwlrVirtualPointerV1 as wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlOutput, ()> for Handler {
    fn event(
        _state: &mut Self,
        _proxy: &WlOutput,
        _event: <WlOutput as wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZxdgOutputManagerV1, ()> for Handler {
    fn event(
        _state: &mut Self,
        _proxy: &ZxdgOutputManagerV1,
        _event: <ZxdgOutputManagerV1 as wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZxdgOutputV1, usize> for Handler {
    fn event(
        state: &mut Self,
        _proxy: &ZxdgOutputV1,
        event: zxdg_output_v1::Event,
        index: &usize,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        let Some(rect) = state.outputs.get_mut(*index) else {
            return;
        };
        match event {
            zxdg_output_v1::Event::LogicalPosition { x, y } => {
                rect.x = x;
                rect.y = y;
            }
            zxdg_output_v1::Event::LogicalSize { width, height } => {
                rect.w = width;
                rect.h = height;
            }
            _ => {}
        }
    }
}

struct Session {
    conn: Connection,
    queue: EventQueue<Handler>,
    manager: ZwlrVirtualPointerManagerV1,
    pointer: ZwlrVirtualPointerV1,
    bounds: Option<OutputRect>,
}

pub struct VirtualPointerBackend {
    session: Option<Session>,
    pressed: HashSet<MouseButton>,
}

impl VirtualPointerBackend {
    pub fn new() -> Self {
        Self {
            session: None,
            pressed: HashSet::new(),
        }
    }

    fn timestamp_ms() -> u32 {
        let now = clock_gettime(ClockId::Monotonic);
        (now.tv_sec as u64)
            .wrapping_mul(1000)
            .wrapping_add(now.tv_nsec as u64 / 1_000_000) as u32
    }

    fn session(&self) -> Result<&Session, InputError> {
        self.session.as_ref().ok_or(InputError::NotConnected)
    }

    fn send_button(&mut self, button: MouseButton, state: ButtonState) -> Result<(), InputError> {
        let session = self.session()?;
        session
            .pointer
            .button(Self::timestamp_ms(), button.evdev_code(), state);
        session.pointer.frame();
        session
            .conn
            .flush()
            .map_err(|err| InputError::Connection(err.to_string()))
    }
}

impl Default for VirtualPointerBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl InputBackend for VirtualPointerBackend {
    fn name(&self) -> &'static str {
        "wlr-virtual-pointer"
    }

    fn connect(&mut self) -> Result<(), InputError> {
        if self.session.is_some() {
            return Ok(());
        }
        let conn =
            Connection::connect_to_env().map_err(|err| InputError::NoDisplay(err.to_string()))?;
        let (globals, mut queue) = registry_queue_init::<Handler>(&conn)
            .map_err(|err| InputError::Connection(err.to_string()))?;
        let qh = queue.handle();
        let manager = globals
            .bind::<ZwlrVirtualPointerManagerV1, Handler, ()>(&qh, 1..=2, ())
            .map_err(|_| InputError::ProtocolUnsupported)?;
        let pointer = manager.create_virtual_pointer(None, &qh, ());

        let mut handler = Handler::default();
        if let Ok(xdg_manager) = globals.bind::<ZxdgOutputManagerV1, Handler, ()>(&qh, 1..=3, ()) {
            for global in globals.contents().clone_list() {
                if global.interface == "wl_output" {
                    let output = globals.registry().bind::<WlOutput, _, _>(
                        global.name,
                        global.version.min(4),
                        &qh,
                        (),
                    );
                    let index = handler.outputs.len();
                    handler.outputs.push(OutputRect::default());
                    xdg_manager.get_xdg_output(&output, &qh, index);
                }
            }
        }

        queue
            .roundtrip(&mut handler)
            .map_err(|err| InputError::Connection(err.to_string()))?;
        queue
            .roundtrip(&mut handler)
            .map_err(|err| InputError::Connection(err.to_string()))?;

        self.session = Some(Session {
            conn,
            queue,
            manager,
            pointer,
            bounds: handler.bounds(),
        });
        Ok(())
    }

    fn move_to(&mut self, x: i32, y: i32) -> Result<(), InputError> {
        let session = self.session()?;
        let bounds = session.bounds.ok_or_else(|| {
            InputError::Connection("monitor layout unavailable (xdg-output missing)".to_string())
        })?;
        let abs_x = (x - bounds.x).clamp(0, bounds.w - 1) as u32;
        let abs_y = (y - bounds.y).clamp(0, bounds.h - 1) as u32;
        session.pointer.motion_absolute(
            Self::timestamp_ms(),
            abs_x,
            abs_y,
            bounds.w as u32,
            bounds.h as u32,
        );
        session.pointer.frame();
        session
            .conn
            .flush()
            .map_err(|err| InputError::Connection(err.to_string()))
    }

    fn press_button(&mut self, button: MouseButton) -> Result<(), InputError> {
        self.pressed.insert(button);
        self.send_button(button, ButtonState::Pressed)
    }

    fn release_button(&mut self, button: MouseButton) -> Result<(), InputError> {
        self.pressed.remove(&button);
        self.send_button(button, ButtonState::Released)
    }

    fn release_all(&mut self) -> Result<(), InputError> {
        let held: Vec<MouseButton> = self.pressed.drain().collect();
        let mut result = Ok(());
        for button in held {
            if let Err(err) = self.send_button(button, ButtonState::Released) {
                result = Err(err);
            }
        }
        result
    }

    fn disconnect(&mut self) -> Result<(), InputError> {
        let released = if self.session.is_some() {
            self.release_all()
        } else {
            Ok(())
        };
        if let Some(mut session) = self.session.take() {
            session.pointer.destroy();
            session.manager.destroy();
            let _ = session.queue.roundtrip(&mut Handler::default());
        }
        released
    }
}

impl Drop for VirtualPointerBackend {
    fn drop(&mut self) {
        let _ = self.disconnect();
    }
}

pub(super) fn query_layout() -> Option<(i32, i32, i32, i32)> {
    let conn = Connection::connect_to_env().ok()?;
    let (globals, mut queue) = registry_queue_init::<Handler>(&conn).ok()?;
    let qh = queue.handle();
    let xdg_manager = globals
        .bind::<ZxdgOutputManagerV1, Handler, ()>(&qh, 1..=3, ())
        .ok()?;
    let mut handler = Handler::default();
    for global in globals.contents().clone_list() {
        if global.interface == "wl_output" {
            let output = globals.registry().bind::<WlOutput, _, _>(
                global.name,
                global.version.min(4),
                &qh,
                (),
            );
            let index = handler.outputs.len();
            handler.outputs.push(OutputRect::default());
            xdg_manager.get_xdg_output(&output, &qh, index);
        }
    }
    queue.roundtrip(&mut handler).ok()?;
    queue.roundtrip(&mut handler).ok()?;
    let bounds = handler.bounds()?;
    Some((bounds.x, bounds.y, bounds.w, bounds.h))
}
