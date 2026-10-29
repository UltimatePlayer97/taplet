use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::ConfigError;
use crate::input::MouseButton;

pub const MIN_INTERVAL_MS: u64 = 1;
pub const MAX_INTERVAL_MS: u64 = 3_600_000;
pub const DEFAULT_INTERVAL_MS: u64 = 100;
pub const DEFAULT_COUNT: u32 = 100;
pub const DEFAULT_SHORTCUT: &str = "F6";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClickType {
    Single,
    Double,
}

impl ClickType {
    pub const ALL: [ClickType; 2] = [ClickType::Single, ClickType::Double];
}

impl std::fmt::Display for ClickType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClickType::Single => f.write_str("Single"),
            ClickType::Double => f.write_str("Double"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RepeatMode {
    Continuous,
    Count,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThemeMode {
    Dark,
    Light,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClickLocation {
    Cursor,
    Fixed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BackendChoice {
    Auto,
    Wlr,
    Uinput,
}

impl BackendChoice {
    pub const ALL: [BackendChoice; 3] = [
        BackendChoice::Auto,
        BackendChoice::Wlr,
        BackendChoice::Uinput,
    ];

    pub fn as_u8(self) -> u8 {
        match self {
            BackendChoice::Auto => 0,
            BackendChoice::Wlr => 1,
            BackendChoice::Uinput => 2,
        }
    }

    pub fn from_u8(value: u8) -> Self {
        match value {
            1 => BackendChoice::Wlr,
            2 => BackendChoice::Uinput,
            _ => BackendChoice::Auto,
        }
    }
}

impl std::fmt::Display for BackendChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BackendChoice::Auto => f.write_str("Auto"),
            BackendChoice::Wlr => f.write_str("wlr-virtual-pointer"),
            BackendChoice::Uinput => f.write_str("uinput"),
        }
    }
}

impl ThemeMode {
    pub const ALL: [ThemeMode; 2] = [ThemeMode::Dark, ThemeMode::Light];
}

impl std::fmt::Display for ThemeMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ThemeMode::Dark => f.write_str("Dark"),
            ThemeMode::Light => f.write_str("Light"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub interval_ms: u64,
    pub button: MouseButton,
    pub click_type: ClickType,
    pub repeat_mode: RepeatMode,
    pub repeat_count: u32,
    pub theme: ThemeMode,
    pub shortcut: String,
    pub location: ClickLocation,
    pub fixed_x: i32,
    pub fixed_y: i32,
    pub backend: BackendChoice,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            interval_ms: DEFAULT_INTERVAL_MS,
            button: MouseButton::Left,
            click_type: ClickType::Single,
            repeat_mode: RepeatMode::Continuous,
            repeat_count: DEFAULT_COUNT,
            theme: ThemeMode::Dark,
            shortcut: DEFAULT_SHORTCUT.to_string(),
            location: ClickLocation::Cursor,
            fixed_x: 0,
            fixed_y: 0,
            backend: BackendChoice::Auto,
        }
    }
}

impl Config {
    pub fn path() -> Result<PathBuf, ConfigError> {
        let base = dirs::config_dir().ok_or(ConfigError::NoConfigDir)?;
        Ok(base.join("taplet").join("config.toml"))
    }

    pub fn load() -> Result<Self, ConfigError> {
        let path = Self::path()?;
        match std::fs::read_to_string(&path) {
            Ok(text) => Ok(toml::from_str::<Config>(&text)?.sanitised()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(err) => Err(err.into()),
        }
    }

    pub fn save(&self) -> Result<(), ConfigError> {
        let path = Self::path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = toml::to_string_pretty(self)?;
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }

    pub fn sanitised(mut self) -> Self {
        self.interval_ms = self.interval_ms.clamp(MIN_INTERVAL_MS, MAX_INTERVAL_MS);
        self.repeat_count = self.repeat_count.max(1);
        if self.shortcut.trim().is_empty() {
            self.shortcut = DEFAULT_SHORTCUT.to_string();
        }
        self
    }

    pub fn interval(&self) -> Duration {
        Duration::from_millis(self.interval_ms)
    }

    pub fn cps(&self) -> f64 {
        let per_cycle = match self.click_type {
            ClickType::Single => 1.0,
            ClickType::Double => 2.0,
        };
        per_cycle * 1000.0 / self.interval_ms as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_toml() {
        let config = Config {
            interval_ms: 25,
            button: MouseButton::Right,
            click_type: ClickType::Double,
            repeat_mode: RepeatMode::Count,
            repeat_count: 7,
            theme: ThemeMode::Light,
            shortcut: "F8".to_string(),
            location: ClickLocation::Fixed,
            fixed_x: 100,
            fixed_y: -20,
            backend: BackendChoice::Uinput,
        };
        let text = toml::to_string_pretty(&config).unwrap();
        let parsed: Config = toml::from_str(&text).unwrap();
        assert_eq!(parsed, config);
    }

    #[test]
    fn missing_fields_fall_back_to_defaults() {
        let parsed: Config = toml::from_str("interval_ms = 40").unwrap();
        assert_eq!(parsed.interval_ms, 40);
        assert_eq!(parsed.button, MouseButton::Left);
    }

    #[test]
    fn sanitise_clamps_out_of_range_values() {
        let config = Config {
            interval_ms: 0,
            repeat_count: 0,
            shortcut: "  ".to_string(),
            ..Config::default()
        }
        .sanitised();
        assert_eq!(config.interval_ms, MIN_INTERVAL_MS);
        assert_eq!(config.repeat_count, 1);
        assert_eq!(config.shortcut, DEFAULT_SHORTCUT);
    }

    #[test]
    fn cps_accounts_for_double_clicks() {
        let single = Config {
            interval_ms: 100,
            ..Config::default()
        };
        let double = Config {
            interval_ms: 100,
            click_type: ClickType::Double,
            ..Config::default()
        };
        assert!((single.cps() - 10.0).abs() < f64::EPSILON);
        assert!((double.cps() - 20.0).abs() < f64::EPSILON);
    }
}
