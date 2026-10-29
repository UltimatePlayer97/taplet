use iced::theme::Palette;
use iced::{Color, Theme};

use crate::config::ThemeMode;

pub fn build_theme(mode: ThemeMode) -> Theme {
    match mode {
        ThemeMode::Dark => Theme::custom(
            "Taplet Dark".to_string(),
            Palette {
                background: Color::from_rgb8(0x14, 0x13, 0x1a),
                text: Color::from_rgb8(0xec, 0xea, 0xf4),
                primary: Color::from_rgb8(0x8b, 0x5c, 0xf6),
                success: Color::from_rgb8(0x4a, 0xde, 0x80),
                warning: Color::from_rgb8(0xfb, 0xbf, 0x24),
                danger: Color::from_rgb8(0xf8, 0x71, 0x71),
            },
        ),
        ThemeMode::Light => Theme::custom(
            "Taplet Light".to_string(),
            Palette {
                background: Color::from_rgb8(0xf6, 0xf5, 0xfb),
                text: Color::from_rgb8(0x1b, 0x18, 0x2b),
                primary: Color::from_rgb8(0x7c, 0x3a, 0xed),
                success: Color::from_rgb8(0x16, 0xa3, 0x4a),
                warning: Color::from_rgb8(0xd9, 0x77, 0x06),
                danger: Color::from_rgb8(0xdc, 0x26, 0x26),
            },
        ),
    }
}
