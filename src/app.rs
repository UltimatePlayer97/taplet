use iced::futures::StreamExt;
use iced::keyboard::{self, Key};
use iced::{Element, Size, Subscription, Task, Theme};
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::{Duration, Instant};

use crate::config::{
    AccentColor, BackendChoice, ClickLocation, ClickType, Config, MAX_INTERVAL_MS, MIN_INTERVAL_MS,
    RepeatMode, ThemeMode,
};
use crate::engine::{ClickPlan, Command, Engine, Event, StopReason};
use crate::hotkey::{Event as HotkeyEvent, HotkeyStatus, Hotkeys};
use crate::input::{AutoBackend, MouseButton};
use crate::ipc::{self, RemoteCommand};
use crate::ui;

const DEFAULT_WINDOW_WIDTH: f32 = 720.0;
const DEFAULT_WINDOW_HEIGHT: f32 = 480.0;
const MIN_WINDOW_WIDTH: f32 = 280.0;
const MIN_WINDOW_HEIGHT: f32 = 200.0;
const HOTKEY_CAPTURE_GRACE: Duration = Duration::from_millis(750);

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
    AccentSelected(AccentColor),
    CustomAccentChanged([u8; 3]),
    CustomHexChanged(String),
    CaptureShortcut,
    KeyPressed(Key),
    CopyBind,
    Reconnect,
    BackendSelected(BackendChoice),
    WindowResized(Size),
    EnableResize,
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
    pub hotkey: HotkeyStatus,
    pub running: bool,
    pub pending_start: bool,
    pub clicks: u64,
    pub message: Option<String>,
    pub capturing_shortcut: bool,
    suppress_hotkey_until: Option<Instant>,
    pub custom_hex_text: String,
    pub window_size: Size,
    page: Page,
    engine: Engine,
    remote: Option<ipc::Listener>,
    backend_choice: Arc<AtomicU8>,
    hotkeys: Hotkeys,
}

impl Taplet {
    pub fn boot() -> (Self, Task<Message>) {
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
        let custom_hex_text = hex_string(config.custom_accent);
        let mut hotkeys = Hotkeys::new();
        hotkeys.register(&config.shortcut, config.backend == BackendChoice::Uinput);
        let state = Self {
            interval_text: config.interval_ms.to_string(),
            count_text: config.repeat_count.to_string(),
            fixed_x_text: config.fixed_x.to_string(),
            fixed_y_text: config.fixed_y.to_string(),
            config,
            backend: BackendStatus::Connecting,
            hotkey: HotkeyStatus::Registering,
            running: false,
            pending_start: false,
            clicks: 0,
            message,
            capturing_shortcut: false,
            suppress_hotkey_until: None,
            custom_hex_text,
            window_size: Size::new(DEFAULT_WINDOW_WIDTH, DEFAULT_WINDOW_HEIGHT),
            page: Page::Main,
            engine: Engine::spawn(move || AutoBackend::new(engine_choice)),
            remote,
            backend_choice,
            hotkeys,
        };
        (state, enable_resize_later())
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

    pub fn custom_hex_valid(&self) -> bool {
        parse_hex(&self.custom_hex_text).is_some()
    }

    pub fn hyprland_bind(&self) -> String {
        format!("bind = , {}, exec, taplet toggle", self.config.shortcut)
    }

    fn persist(&mut self) {
        if let Err(err) = self.config.save() {
            self.message = Some(format!("Could not save settings: {err}"));
        }
    }

    fn refresh_hotkeys(&mut self) {
        let prefer_uinput = self.config.backend == BackendChoice::Uinput;
        if self.hotkeys.register(&self.config.shortcut, prefer_uinput) {
            self.hotkey = HotkeyStatus::Registering;
        }
    }

    fn handle_hotkey(&mut self, event: HotkeyEvent) {
        match event {
            HotkeyEvent::Status(status) => self.hotkey = status,
            HotkeyEvent::Activated => {
                let suppressed = self.capturing_shortcut
                    || self
                        .suppress_hotkey_until
                        .is_some_and(|until| Instant::now() < until);
                if !suppressed {
                    self.toggle();
                }
            }
        }
    }

    fn start(&mut self) {
        if self.capturing_shortcut
            || !self.interval_valid()
            || !self.count_valid()
            || !self.location_valid()
        {
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
                if self.capturing_shortcut {
                    self.suppress_hotkey_until = Some(Instant::now() + HOTKEY_CAPTURE_GRACE);
                }
                self.capturing_shortcut = false;
                self.page = Page::Main;
            }
            Message::ThemeSelected(mode) => {
                self.config.theme = mode;
                self.persist();
            }
            Message::AccentSelected(accent) => {
                self.config.accent = accent;
                if accent == AccentColor::Custom {
                    self.custom_hex_text = hex_string(self.config.custom_accent);
                }
                self.persist();
            }
            Message::CustomAccentChanged(rgb) => {
                self.config.custom_accent = rgb;
                self.config.accent = AccentColor::Custom;
                self.custom_hex_text = hex_string(rgb);
                self.persist();
            }
            Message::CustomHexChanged(value) => {
                self.custom_hex_text = value;
                if let Some(rgb) = parse_hex(&self.custom_hex_text) {
                    self.config.custom_accent = rgb;
                    self.config.accent = AccentColor::Custom;
                    self.persist();
                }
            }
            Message::CaptureShortcut => {
                self.capturing_shortcut = true;
                self.suppress_hotkey_until = Some(Instant::now() + HOTKEY_CAPTURE_GRACE);
            }
            Message::KeyPressed(key) => {
                if self.capturing_shortcut {
                    if key == Key::Named(keyboard::key::Named::Escape) {
                        self.capturing_shortcut = false;
                        self.suppress_hotkey_until =
                            Some(Instant::now() + HOTKEY_CAPTURE_GRACE);
                    } else if let Some(name) = Self::key_name(&key) {
                        self.config.shortcut = name;
                        self.capturing_shortcut = false;
                        self.suppress_hotkey_until =
                            Some(Instant::now() + HOTKEY_CAPTURE_GRACE);
                        self.persist();
                        self.refresh_hotkeys();
                    }
                } else if self.page == Page::Main
                    // The global listener already delivers this key; toggling twice cancels out.
                    && !matches!(self.hotkey, HotkeyStatus::Active(_))
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
                self.refresh_hotkeys();
                self.backend = BackendStatus::Connecting;
                self.engine.send(Command::Reconnect);
            }
            Message::WindowResized(size) => {
                self.window_size = size;
            }
            Message::EnableResize => {
                std::thread::spawn(|| {
                    std::thread::sleep(Duration::from_millis(250));
                    request_floating();
                });
                return iced::window::latest().and_then(|id| {
                    iced::window::set_resizable(id, true)
                        .chain(iced::window::set_min_size(
                            id,
                            Some(Size::new(MIN_WINDOW_WIDTH, MIN_WINDOW_HEIGHT)),
                        ))
                        .chain(iced::window::set_max_size(id, None))
                });
            }
            Message::Tick => {
                for event in self.engine.drain_events() {
                    self.handle_event(event);
                }
                for event in self.hotkeys.poll() {
                    self.handle_hotkey(event);
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
            ticker(),
            iced::window::resize_events().map(|(_, size)| Message::WindowResized(size)),
            keyboard::listen().filter_map(|event| match event {
                keyboard::Event::KeyPressed { key, .. } => Some(Message::KeyPressed(key)),
                _ => None,
            }),
        ])
    }

    pub fn theme(&self) -> Theme {
        ui::build_theme(
            self.config.theme,
            self.config.accent,
            self.config.custom_accent,
        )
    }
}

fn ticker() -> Subscription<Message> {
    Subscription::run_with("taplet-tick", |_| {
        let (sender, receiver) = iced::futures::channel::mpsc::unbounded();
        let _ = std::thread::Builder::new()
            .name("taplet-tick".to_string())
            .spawn(move || {
                loop {
                    std::thread::sleep(Duration::from_millis(40));
                    if sender.unbounded_send(()).is_err() {
                        break;
                    }
                }
            });
        receiver.map(|_| Message::Tick)
    })
}

fn enable_resize_later() -> Task<Message> {
    let (tx, rx) = iced::futures::channel::oneshot::channel();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(500));
        let _ = tx.send(());
    });
    Task::perform(
        async move {
            let _ = rx.await;
        },
        |_| Message::EnableResize,
    )
}

fn request_floating() {
    let hyprland = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some();
    let sway = std::env::var_os("SWAYSOCK").is_some();
    if !hyprland && !sway {
        return;
    }

    let _ = std::thread::Builder::new()
        .name("taplet-float".to_string())
        .spawn(move || {
            if hyprland {
                for _ in 0..40 {
                    if hyprland_has_window() && hyprland_set_floating() {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            } else {
                for _ in 0..8 {
                    let _ = std::process::Command::new("swaymsg")
                        .arg("[app_id=\"taplet\"] floating enable")
                        .output();
                    std::thread::sleep(Duration::from_millis(250));
                }
            }
        });
}

fn hyprland_has_window() -> bool {
    let Ok(output) = std::process::Command::new("hyprctl")
        .arg("clients")
        .output()
    else {
        return false;
    };
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .any(|line| line.trim() == "class: taplet")
}

fn hyprland_set_floating() -> bool {
    let Ok(output) = std::process::Command::new("hyprctl")
        .args(["dispatch", "setfloating", "class:taplet"])
        .output()
    else {
        return false;
    };
    let mut text = String::from_utf8_lossy(&output.stdout).to_lowercase();
    text.push_str(&String::from_utf8_lossy(&output.stderr).to_lowercase());
    if output.status.success() && text.trim() == "ok" {
        return true;
    }
    if text.contains("invalid") || text.contains("unknown") {
        let Ok(fallback) = std::process::Command::new("hyprctl")
            .args(["dispatch", "togglefloating", "class:taplet"])
            .output()
        else {
            return false;
        };
        let mut fallback_text = String::from_utf8_lossy(&fallback.stdout).to_lowercase();
        fallback_text.push_str(&String::from_utf8_lossy(&fallback.stderr).to_lowercase());
        return fallback.status.success() && fallback_text.trim() == "ok";
    }
    false
}

pub fn run() -> iced::Result {
    request_floating();
    iced::application(Taplet::boot, Taplet::update, Taplet::view)
        .title("Taplet")
        .subscription(Taplet::subscription)
        .theme(Taplet::theme)
        .font(iced_fonts::LUCIDE_FONT_BYTES)
        .window(iced::window::Settings {
            size: Size::new(DEFAULT_WINDOW_WIDTH, DEFAULT_WINDOW_HEIGHT),
            min_size: Some(Size::new(DEFAULT_WINDOW_WIDTH, DEFAULT_WINDOW_HEIGHT)),
            max_size: Some(Size::new(DEFAULT_WINDOW_WIDTH, DEFAULT_WINDOW_HEIGHT)),
            resizable: false,
            position: iced::window::Position::Centered,
            platform_specific: iced::window::settings::PlatformSpecific {
                application_id: "taplet".to_string(),
                ..Default::default()
            },
            ..Default::default()
        })
        .run()
}

fn hex_string(rgb: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2])
}

fn parse_hex(text: &str) -> Option<[u8; 3]> {
    let value = text.trim();
    let value = value
        .strip_prefix('#')
        .or_else(|| value.strip_prefix("0x"))
        .or_else(|| value.strip_prefix("0X"))
        .unwrap_or(value);
    let digits = value
        .chars()
        .map(|character| character.to_digit(16).map(|digit| digit as u8))
        .collect::<Option<Vec<u8>>>()?;
    match digits.as_slice() {
        [r, g, b] => Some([r * 17, g * 17, b * 17]),
        [r1, r2, g1, g2, b1, b2] => Some([r1 * 16 + r2, g1 * 16 + g2, b1 * 16 + b2]),
        _ => None,
    }
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
