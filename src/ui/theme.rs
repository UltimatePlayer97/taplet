use iced::theme::Palette;
use iced::{Color, Theme};

use crate::config::{AccentColor, ThemeMode};

pub fn build_theme(mode: ThemeMode, accent: AccentColor, custom: [u8; 3]) -> Theme {
    let primary = accent_color(accent, mode, custom);
    match mode {
        ThemeMode::Dark => Theme::custom(
            "Taplet Dark".to_string(),
            Palette {
                background: Color::from_rgb8(0x14, 0x13, 0x1a),
                text: Color::from_rgb8(0xec, 0xea, 0xf4),
                primary,
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
                primary,
                success: Color::from_rgb8(0x16, 0xa3, 0x4a),
                warning: Color::from_rgb8(0xd9, 0x77, 0x06),
                danger: Color::from_rgb8(0xdc, 0x26, 0x26),
            },
        ),
    }
}

pub fn accent_color(accent: AccentColor, mode: ThemeMode, custom: [u8; 3]) -> Color {
    if accent == AccentColor::Custom {
        return Color::from_rgb8(custom[0], custom[1], custom[2]);
    }
    let rgb = match (accent, mode) {
        (AccentColor::Violet, ThemeMode::Dark) => (0x8b, 0x5c, 0xf6),
        (AccentColor::Violet, ThemeMode::Light) => (0x7c, 0x3a, 0xed),
        (AccentColor::Blue, ThemeMode::Dark) => (0x5b, 0x8c, 0xff),
        (AccentColor::Blue, ThemeMode::Light) => (0x25, 0x63, 0xeb),
        (AccentColor::Teal, ThemeMode::Dark) => (0x2d, 0xd4, 0xbf),
        (AccentColor::Teal, ThemeMode::Light) => (0x0f, 0x76, 0x6e),
        (AccentColor::Green, ThemeMode::Dark) => (0x4a, 0xde, 0x80),
        (AccentColor::Green, ThemeMode::Light) => (0x16, 0xa3, 0x4a),
        (AccentColor::Amber, ThemeMode::Dark) => (0xfb, 0xbf, 0x24),
        (AccentColor::Amber, ThemeMode::Light) => (0xd9, 0x77, 0x06),
        (AccentColor::Rose, ThemeMode::Dark) => (0xfb, 0x71, 0x85),
        (AccentColor::Rose, ThemeMode::Light) => (0xe1, 0x1d, 0x48),
        (AccentColor::Custom, _) => (0x8b, 0x5c, 0xf6),
    };
    Color::from_rgb8(rgb.0, rgb.1, rgb.2)
}

pub fn blend(from: Color, to: Color, amount: f32) -> Color {
    let amount = amount.clamp(0.0, 1.0);
    Color {
        r: from.r + (to.r - from.r) * amount,
        g: from.g + (to.g - from.g) * amount,
        b: from.b + (to.b - from.b) * amount,
        a: from.a + (to.a - from.a) * amount,
    }
}

pub fn surface(theme: &Theme) -> Color {
    let palette = theme.palette();
    let amount = if theme.extended_palette().is_dark {
        0.075
    } else {
        0.035
    };
    blend(palette.background, palette.primary, amount)
}

pub fn field(theme: &Theme) -> Color {
    let palette = theme.palette();
    let amount = if theme.extended_palette().is_dark {
        0.13
    } else {
        0.07
    };
    blend(palette.background, palette.primary, amount)
}

pub fn divider(theme: &Theme) -> Color {
    let palette = theme.palette();
    let amount = if theme.extended_palette().is_dark {
        0.14
    } else {
        0.12
    };
    blend(palette.background, palette.text, amount)
}

pub fn muted_text(theme: &Theme) -> Color {
    let palette = theme.palette();
    blend(palette.text, palette.background, 0.38)
}

pub fn readable_ink(color: Color) -> Color {
    let luminance = 0.2126 * color.r + 0.7152 * color.g + 0.0722 * color.b;
    if luminance > 0.55 {
        Color::BLACK
    } else {
        Color::WHITE
    }
}
