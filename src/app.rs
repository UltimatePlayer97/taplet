use iced::keyboard::{self, Key};
use iced::{Element, Size, Subscription, Task, Theme};
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Duration;

use crate::config::{
    BackendChoice, ClickLocation, ClickType, Config, MAX_INTERVAL_MS, MIN_INTERVAL_MS, RepeatMode,
    ThemeMode,
};
use crate::engine::{ClickPlan, Command, Engine, Event, StopReason};
use crate::input::{AutoBackend, MouseButton};
use crate::ipc::{self, RemoteCommand};
use crate::ui;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendStatus {
    Connecting,
    Ready(&'static str),
    Unavailable(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Page {
    Main,
    Advanced,
}

#[derive(Debug, Clone)]
pub enum Message {
    IntervalChanged(String),
    ButtonSelected(MouseButton),
    ClickTypeSelected(ClickType),
    RepeatModeSelected(RepeatMode),
    CountChanged(String),
    LocationSelected(ClickLocation),
    FixedXChanged(String),
    FixedYChanged(String),
    ToggleRun,
    OpenAdvanced,
    CloseAdvanced,
    ThemeSelected(ThemeMode),
    CaptureShortcut,
    KeyPressed(Key),
    CopyBind,
    Reconnect,
    BackendSelected(BackendChoice),
    Tick,
    PickPosition,
    PositionPicked(Option<(i32, i32)>),
}

pub struct Taplet {
    pub config: Config,
    pub interval_text: String,
    pub count_text: String,
    pub fixed_x_text: String,
    pub fixed_y_text: String,
    pub backend: BackendStatus,
    pub running: bool,
    pub pending_start: bool,
    pub clicks: u64,
    pub message: Option<String>,
    pub capturing_shortcut: bool,
    page: Page,
    engine: Engine,
    remote: Option<ipc::Listener>,
    backend_choice: Arc<AtomicU8>,
}

impl Taplet {
    pub fn boot() -> Self {
        let mut message = None;
        let config = match Config::load() {
            Ok(config) => config,
            Err(err) => {
                message = Some(format!("Using defaults: {err}"));
                Config::default()
            }
        };
        let remote = match ipc::Listener::bind() {
            Ok(listener) => Some(listener),
            Err(err) => {
                eprintln!("remote control socket unavailable: {err}");
                None
            }
        };
        let backend_choice = Arc::new(AtomicU8::new(config.backend.as_u8()));
        let engine_choice = Arc::clone(&backend_choice);
        Self {
            interval_text: config.interval_ms.to_string(),
            count_text: config.repeat_count.to_string(),
            fixed_x_text: config.fixed_x.to_string(),
            fixed_y_text: config.fixed_y.to_string(),
            config,
            backend: BackendStatus::Connecting,
            running: false,
            pending_start: false,
            clicks: 0,
            message,
            capturing_shortcut: false,
            page: Page::Main,
            engine: Engine::spawn(move || AutoBackend::new(engine_choice)),
            remote,
            backend_choice,
        }
    }

    pub fn interval_valid(&self) -> bool {
        self.interval_text
            .trim()
            .parse::<u64>()
            .is_ok_and(|value| (MIN_INTERVAL_MS..=MAX_INTERVAL_MS).contains(&value))
    }

    pub fn count_valid(&self) -> bool {
        self.config.repeat_mode == RepeatMode::Continuous
            || self
                .count_text
                .trim()
                .parse::<u32>()
                .is_ok_and(|value| value >= 1)
    }

    pub fn location_valid(&self) -> bool {
        self.config.location == ClickLocation::Cursor
            || (self.fixed_x_text.trim().parse::<i32>().is_ok()
                && self.fixed_y_text.trim().parse::<i32>().is_ok())
    }

    pub fn hyprland_bind(&self) -> String {
        format!("bind = , {}, exec, taplet toggle", self.config.shortcut)
    }

    fn persist(&mut self) {
        if let Err(err) = self.config.save() {
            self.message = Some(format!("Could not save settings: {err}"));
        }
    }

    fn start(&mut self) {
        if !self.interval_valid() || !self.count_valid() || !self.location_valid() {
            return;
        }
        self.message = None;
        self.clicks = 0;
        self.pending_start = true;
        self.engine
            .send(Command::Start(ClickPlan::from_config(&self.config)));
    }

    fn stop(&mut self) {
        self.engine.send(Command::Stop);
    }

    fn toggle(&mut self) {
        if self.running || self.pending_start {
            self.stop();
        } else {
            self.start();
        }
    }

    fn handle_event(&mut self, event: Event) {
        match event {
            Event::Backend { name, error } => {
                self.backend = match error {
                    None => BackendStatus::Ready(name),
                    Some(reason) => BackendStatus::Unavailable(reason),
                };
            }
            Event::Started => {
                self.pending_start = false;
                self.running = true;
            }
            Event::Progress(clicks) => self.clicks = clicks,
            Event::Stopped { clicks, reason } => {
                self.pending_start = false;
                self.running = false;
                self.clicks = clicks;
                if let StopReason::Failed(reason) = reason {
                    self.message = Some(reason);
                }
            }
        }
    }

    fn handle_remote(&mut self, command: RemoteCommand) {
        let active = self.running || self.pending_start;
        match command {
            RemoteCommand::Toggle => self.toggle(),
            RemoteCommand::Start if !active => self.start(),
            RemoteCommand::Stop if active => self.stop(),
            _ => {}
        }
    }

    fn key_name(key: &Key) -> Option<String> {
        match key {
            Key::Named(named) => Some(format!("{named:?}")),
            Key::Character(value) => Some(value.to_uppercase()),
            Key::Unidentified => None,
        }
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::PickPosition => return pick_position(),
            Message::PositionPicked(point) => match point {
                Some((x, y)) => {
                    self.config.fixed_x = x;
                    self.config.fixed_y = y;
                    self.fixed_x_text = x.to_string();
                    self.fixed_y_text = y.to_string();
                    self.persist();
                }
                None => {
                    self.message = Some("Pick cancelled, or slurp not installed".to_string());
                }
            },
            Message::IntervalChanged(value) => {
                self.interval_text = value;
                if let Ok(parsed) = self.interval_text.trim().parse::<u64>()
                    && (MIN_INTERVAL_MS..=MAX_INTERVAL_MS).contains(&parsed)
                {
                    self.config.interval_ms = parsed;
                    self.persist();
                }
            }
            Message::ButtonSelected(button) => {
                self.config.button = button;
                self.persist();
            }
            Message::ClickTypeSelected(click_type) => {
                self.config.click_type = click_type;
                self.persist();
            }
            Message::RepeatModeSelected(mode) => {
                self.config.repeat_mode = mode;
                self.persist();
            }
            Message::CountChanged(value) => {
                self.count_text = value;
                if let Ok(parsed) = self.count_text.trim().parse::<u32>()
                    && parsed >= 1
                {
                    self.config.repeat_count = parsed;
                    self.persist();
                }
            }
            Message::LocationSelected(location) => {
                self.config.location = location;
                self.persist();
            }
            Message::FixedXChanged(value) => {
                self.fixed_x_text = value;
                if let Ok(parsed) = self.fixed_x_text.trim().parse::<i32>() {
                    self.config.fixed_x = parsed;
                    self.persist();
                }
            }
            Message::FixedYChanged(value) => {
                self.fixed_y_text = value;
                if let Ok(parsed) = self.fixed_y_text.trim().parse::<i32>() {
                    self.config.fixed_y = parsed;
                    self.persist();
                }
            }
            Message::ToggleRun => self.toggle(),
            Message::OpenAdvanced => self.page = Page::Advanced,
            Message::CloseAdvanced => {
                self.capturing_shortcut = false;
                self.page = Page::Main;
            }
            Message::ThemeSelected(mode) => {
                self.config.theme = mode;
                self.persist();
            }
            Message::CaptureShortcut => self.capturing_shortcut = true,
            Message::KeyPressed(key) => {
                if self.capturing_shortcut {
                    if key == Key::Named(keyboard::key::Named::Escape) {
                        self.capturing_shortcut = false;
                    } else if let Some(name) = Self::key_name(&key) {
                        self.config.shortcut = name;
                        self.capturing_shortcut = false;
                        self.persist();
                    }
                } else if self.page == Page::Main
                    && Self::key_name(&key).as_deref() == Some(self.config.shortcut.as_str())
                {
                    self.toggle();
                }
            }
            Message::CopyBind => return iced::clipboard::write(self.hyprland_bind()),
            Message::Reconnect => {
                self.backend = BackendStatus::Connecting;
                self.engine.send(Command::Reconnect);
            }
            Message::BackendSelected(choice) => {
                self.config.backend = choice;
                self.backend_choice.store(choice.as_u8(), Ordering::Relaxed);
                self.persist();
                self.backend = BackendStatus::Connecting;
                self.engine.send(Command::Reconnect);
            }
            Message::Tick => {
                for event in self.engine.drain_events() {
                    self.handle_event(event);
                }
                let commands: Vec<RemoteCommand> = self
                    .remote
                    .as_ref()
                    .map(|listener| listener.commands.try_iter().collect())
                    .unwrap_or_default();
                for command in commands {
                    self.handle_remote(command);
                }
            }
        }
        Task::none()
    }

    pub fn view(&self) -> Element<'_, Message> {
        match self.page {
            Page::Main => ui::main_view(self),
            Page::Advanced => ui::advanced_view(self),
        }
    }

    pub fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            iced::time::every(Duration::from_millis(40)).map(|_| Message::Tick),
            keyboard::listen().filter_map(|event| match event {
                keyboard::Event::KeyPressed { key, .. } => Some(Message::KeyPressed(key)),
                _ => None,
            }),
        ])
    }

    pub fn theme(&self) -> Theme {
        ui::build_theme(self.config.theme)
    }
}

pub fn run() -> iced::Result {
    iced::application(Taplet::boot, Taplet::update, Taplet::view)
        .title("Taplet")
        .subscription(Taplet::subscription)
        .theme(Taplet::theme)
        .font(iced_fonts::LUCIDE_FONT_BYTES)
        .window(iced::window::Settings {
            size: Size::new(440.0, 740.0),
            min_size: Some(Size::new(440.0, 740.0)),
            max_size: Some(Size::new(440.0, 740.0)),
            resizable: false,
            platform_specific: iced::window::settings::PlatformSpecific {
                application_id: "taplet".to_string(),
                ..Default::default()
            },
            ..Default::default()
        })
        .run()
}

fn pick_position() -> Task<Message> {
    let (tx, rx) = iced::futures::channel::oneshot::channel();
    std::thread::spawn(move || {
        let point = std::process::Command::new("slurp")
            .args(["-p", "-f", "%x %y"])
            .output()
            .ok()
            .filter(|out| out.status.success())
            .and_then(|out| {
                let text = String::from_utf8(out.stdout).ok()?;
                let mut parts = text.split_whitespace();
                let x = parts.next()?.parse::<i32>().ok()?;
                let y = parts.next()?.parse::<i32>().ok()?;
                Some((x, y))
            });
        let _ = tx.send(point);
    });
    Task::perform(
        async move { rx.await.ok().flatten() },
        Message::PositionPicked,
    )
}
