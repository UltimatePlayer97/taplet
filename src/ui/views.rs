use iced::overlay::menu;
use iced::widget::{
    Space, button, column, container, pick_list, row, rule, scrollable, slider, text, text_input,
};
use iced::{Alignment, Border, Color, Element, Length, Shadow, Theme};
use iced_fonts::lucide;

use super::theme::{accent_color, blend, divider, field, muted_text, readable_ink, surface};
use crate::app::{BackendStatus, Message, Taplet};
use crate::config::{AccentColor, BackendChoice, ClickLocation, ClickType, RepeatMode, ThemeMode};
use crate::hotkey::HotkeyStatus;
use crate::input::MouseButton;

const SHORTCUT_CARD_HEIGHT: f32 = 116.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ButtonKind {
    Primary,
    Secondary,
    Text,
    Selected,
    Danger,
    Row(bool),
}

fn button_style(theme: &Theme, status: button::Status, kind: ButtonKind) -> button::Style {
    let palette = theme.extended_palette();
    let accent = palette.primary.base.color;
    let background = palette.background.base.color;
    let text = palette.background.base.text;

    let (fill, mut text_color, border_color, border_width) = match kind {
        ButtonKind::Primary | ButtonKind::Danger => {
            let base = if kind == ButtonKind::Primary {
                accent
            } else {
                palette.danger.base.color
            };
            let fill = match status {
                button::Status::Hovered => blend(base, Color::WHITE, 0.12),
                button::Status::Pressed => blend(base, Color::BLACK, 0.08),
                button::Status::Disabled => blend(base, background, 0.55),
                button::Status::Active => base,
            };
            let ink = if kind == ButtonKind::Primary {
                palette.primary.base.text
            } else {
                palette.danger.base.text
            };
            (Some(fill), ink, fill, 0.0)
        }
        ButtonKind::Secondary => {
            let fill = match status {
                button::Status::Hovered => blend(field(theme), accent, 0.05),
                button::Status::Pressed => surface(theme),
                _ => field(theme),
            };
            (Some(fill), text, divider(theme), 1.0)
        }
        ButtonKind::Selected => {
            let fill = blend(background, accent, 0.22);
            (Some(fill), accent, blend(accent, background, 0.35), 1.0)
        }
        ButtonKind::Text => {
            let fill = match status {
                button::Status::Hovered => Some(field(theme)),
                button::Status::Pressed => Some(surface(theme)),
                _ => None,
            };
            (fill, text, Color::TRANSPARENT, 0.0)
        }
        ButtonKind::Row(selected) => {
            let fill = if selected {
                Some(blend(background, accent, 0.12))
            } else {
                match status {
                    button::Status::Hovered => Some(field(theme)),
                    button::Status::Pressed => Some(surface(theme)),
                    _ => None,
                }
            };
            (fill, text, Color::TRANSPARENT, 0.0)
        }
    };

    if matches!(status, button::Status::Disabled) {
        text_color = muted_text(theme);
    }

    button::Style {
        background: fill.map(Into::into),
        text_color,
        border: Border {
            color: border_color,
            width: border_width,
            radius: 5.0.into(),
        },
        shadow: Shadow::default(),
        snap: true,
    }
}

fn card_style(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();
    container::Style {
        background: Some(surface(theme).into()),
        border: Border {
            color: divider(theme),
            width: 1.0,
            radius: 8.0.into(),
        },
        text_color: Some(palette.background.base.text),
        ..container::Style::default()
    }
}

fn badge_style(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(field(theme).into()),
        border: Border {
            color: divider(theme),
            width: 1.0,
            radius: 5.0.into(),
        },
        ..container::Style::default()
    }
}

fn segment_track_style(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(field(theme).into()),
        border: Border {
            color: divider(theme),
            width: 1.0,
            radius: 7.0.into(),
        },
        ..container::Style::default()
    }
}

fn pick_list_style(theme: &Theme, status: pick_list::Status) -> pick_list::Style {
    let palette = theme.extended_palette();
    let open = matches!(status, pick_list::Status::Opened { .. });
    let fill = match status {
        pick_list::Status::Hovered | pick_list::Status::Opened { .. } => {
            blend(field(theme), palette.primary.base.color, 0.04)
        }
        pick_list::Status::Active => field(theme),
    };
    pick_list::Style {
        text_color: palette.background.base.text,
        placeholder_color: muted_text(theme),
        handle_color: muted_text(theme),
        background: fill.into(),
        border: Border {
            color: if open {
                palette.primary.base.color
            } else {
                divider(theme)
            },
            width: 1.0,
            radius: 5.0.into(),
        },
    }
}

fn menu_style(theme: &Theme) -> menu::Style {
    let palette = theme.extended_palette();
    let shadow = if palette.is_dark {
        Color::from_rgba(0.0, 0.0, 0.0, 0.35)
    } else {
        Color::from_rgba(0.0, 0.0, 0.0, 0.15)
    };
    menu::Style {
        background: field(theme).into(),
        border: Border {
            color: divider(theme),
            width: 1.0,
            radius: 6.0.into(),
        },
        text_color: palette.background.base.text,
        selected_text_color: palette.primary.base.color,
        selected_background: blend(
            palette.background.base.color,
            palette.primary.base.color,
            0.18,
        )
        .into(),
        shadow: Shadow {
            color: shadow,
            offset: iced::Vector::new(0.0, 4.0),
            blur_radius: 14.0,
        },
    }
}

fn text_input_style(theme: &Theme, status: text_input::Status) -> text_input::Style {
    let palette = theme.extended_palette();
    let focused = matches!(status, text_input::Status::Focused { .. });
    let hovered = matches!(
        status,
        text_input::Status::Hovered | text_input::Status::Focused { is_hovered: true }
    );
    let fill = if hovered || focused {
        blend(field(theme), palette.primary.base.color, 0.04)
    } else {
        field(theme)
    };
    text_input::Style {
        background: fill.into(),
        border: Border {
            color: if focused {
                palette.primary.base.color
            } else {
                divider(theme)
            },
            width: 1.0,
            radius: 5.0.into(),
        },
        icon: muted_text(theme),
        placeholder: muted_text(theme),
        value: palette.background.base.text,
        selection: palette.primary.base.color.scale_alpha(0.4),
    }
}

fn rule_style(theme: &Theme) -> rule::Style {
    rule::Style {
        color: divider(theme),
        radius: 1.0.into(),
        fill_mode: rule::FillMode::Full,
        snap: true,
    }
}

fn slider_style(theme: &Theme, _status: slider::Status) -> slider::Style {
    let palette = theme.extended_palette();
    let accent = palette.primary.base.color;
    slider::Style {
        rail: slider::Rail {
            backgrounds: (accent.into(), field(theme).into()),
            width: 4.0,
            border: Border {
                color: Color::TRANSPARENT,
                width: 0.0,
                radius: 2.0.into(),
            },
        },
        handle: slider::Handle {
            shape: slider::HandleShape::Circle { radius: 6.0 },
            background: accent.into(),
            border_width: 2.0,
            border_color: palette.background.base.color,
        },
    }
}

fn accent_swatch<'a>(
    accent: AccentColor,
    current: AccentColor,
    mode: ThemeMode,
    custom: [u8; 3],
) -> Element<'a, Message> {
    let color = accent_color(accent, mode, custom);
    let selected = accent == current;
    button(
        container(if selected {
            lucide::check().size(12)
        } else if accent == AccentColor::Custom {
            lucide::palette().size(12)
        } else {
            lucide::circle().size(8)
        })
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(Alignment::Center)
        .align_y(Alignment::Center),
    )
    .width(24)
    .height(24)
    .padding(0)
    .style(move |theme, status| {
        let fill = match status {
            button::Status::Hovered => blend(color, Color::WHITE, 0.15),
            button::Status::Pressed => blend(color, Color::BLACK, 0.12),
            _ => color,
        };
        let ink = readable_ink(fill);
        button::Style {
            background: Some(fill.into()),
            text_color: ink,
            border: Border {
                color: if selected { ink } else { divider(theme) },
                width: if selected { 2.0 } else { 1.0 },
                radius: 5.0.into(),
            },
            shadow: Shadow::default(),
            snap: true,
        }
    })
    .on_press(Message::AccentSelected(accent))
    .into()
}

fn color_preview<'a>(color: Color) -> Element<'a, Message> {
    container(Space::new())
        .width(24)
        .height(24)
        .style(move |theme: &Theme| container::Style {
            background: Some(color.into()),
            border: Border {
                color: divider(theme),
                width: 1.0,
                radius: 5.0.into(),
            },
            ..container::Style::default()
        })
        .into()
}

fn custom_channel<'a>(name: &'a str, channel: usize, rgb: [u8; 3]) -> Element<'a, Message> {
    row![
        text(name).size(11).style(text::secondary).width(14),
        slider(0.0..=255.0, rgb[channel] as f32, move |value| {
            let mut next = rgb;
            next[channel] = value.round() as u8;
            Message::CustomAccentChanged(next)
        })
        .style(slider_style),
        text(format!("{:3}", rgb[channel]))
            .size(11)
            .font(iced::Font::MONOSPACE)
            .style(text::secondary),
    ]
    .spacing(8)
    .align_y(Alignment::Center)
    .into()
}

fn card<'a>(content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    container(content)
        .padding(14)
        .width(Length::Fill)
        .height(Length::Shrink)
        .style(card_style)
        .into()
}

fn card_fill<'a>(content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    container(content)
        .padding(14)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(card_style)
        .into()
}

fn card_fixed<'a>(
    height: f32,
    content: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    container(content)
        .padding(14)
        .width(Length::Fill)
        .height(Length::Fixed(height))
        .style(card_style)
        .into()
}

fn badge<'a>(content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    container(content)
        .padding([3, 10])
        .style(badge_style)
        .into()
}

fn status_badge<'a>(state: &'a Taplet) -> Element<'a, Message> {
    let (label, ok) = match &state.backend {
        BackendStatus::Connecting => ("Connecting…".to_string(), false),
        BackendStatus::Ready(_) => ("Connected".to_string(), true),
        BackendStatus::Unavailable(reason) => (reason.clone(), false),
    };
    container(
        text(label)
            .size(11)
            .style(if ok { text::success } else { text::danger }),
    )
    .padding([3, 10])
    .style(move |theme: &Theme| {
        let palette = theme.extended_palette();
        let base = if ok {
            palette.success.base.color
        } else {
            palette.danger.base.color
        };
        container::Style {
            background: Some(base.scale_alpha(0.15).into()),
            border: Border {
                color: base.scale_alpha(0.4),
                width: 1.0,
                radius: 5.0.into(),
            },
            ..container::Style::default()
        }
    })
    .into()
}

fn app_icon<'a>() -> Element<'a, Message> {
    container(lucide::mouse_pointer_click().size(18))
        .padding(10)
        .style(|theme: &Theme| {
            let palette = theme.extended_palette();
            container::Style {
                background: Some(palette.primary.base.color.into()),
                text_color: Some(palette.primary.base.text),
                border: Border {
                    radius: 8.0.into(),
                    ..Border::default()
                },
                ..container::Style::default()
            }
        })
        .into()
}

fn label<'a>(value: &'a str) -> Element<'a, Message> {
    text(value).size(13).style(text::secondary).into()
}

fn caption<'a>(value: &'a str) -> Element<'a, Message> {
    text(value).size(10).style(text::secondary).into()
}

fn status_line<'a>(state: &'a Taplet) -> Element<'a, Message> {
    match &state.backend {
        BackendStatus::Connecting => text("Connecting…").size(12).into(),
        BackendStatus::Ready(_) => text("Connected").size(12).style(text::success).into(),
        BackendStatus::Unavailable(reason) => {
            text(reason.as_str()).size(12).style(text::danger).into()
        }
    }
}

fn backend_name(state: &Taplet) -> String {
    match &state.backend {
        BackendStatus::Ready(name) => name.to_string(),
        BackendStatus::Connecting => "Connecting…".to_string(),
        BackendStatus::Unavailable(_) => "Unavailable".to_string(),
    }
}

fn hotkey_line(status: &HotkeyStatus) -> (String, Option<bool>) {
    match status {
        HotkeyStatus::Registering => ("Registering…".to_string(), None),
        HotkeyStatus::Active(backend) => (format!("Active via {}", backend.label()), Some(true)),
        HotkeyStatus::Unavailable(reason) => (reason.clone(), Some(false)),
    }
}

fn hotkey_short(status: &HotkeyStatus) -> (&'static str, Option<bool>) {
    match status {
        HotkeyStatus::Registering => ("Registering…", None),
        HotkeyStatus::Active(_) => ("Active", Some(true)),
        HotkeyStatus::Unavailable(_) => ("Unavailable", Some(false)),
    }
}

fn status_text_style(tone: Option<bool>) -> fn(&Theme) -> text::Style {
    match tone {
        Some(true) => text::success,
        Some(false) => text::danger,
        None => text::secondary,
    }
}

fn segment<'a>(title: &'a str, mode: RepeatMode, current: RepeatMode) -> Element<'a, Message> {
    button(
        text(title)
            .size(13)
            .width(Length::Fill)
            .align_x(Alignment::Center),
    )
    .width(Length::Fill)
    .height(30)
    .padding(7)
    .on_press(Message::RepeatModeSelected(mode))
    .style(move |theme, status| {
        button_style(
            theme,
            status,
            if mode == current {
                ButtonKind::Selected
            } else {
                ButtonKind::Text
            },
        )
    })
    .into()
}

fn location_option<'a>(
    icon: impl Into<Element<'a, Message>>,
    title: &'a str,
    subtitle: &'a str,
    selected: bool,
    message: Message,
) -> Element<'a, Message> {
    let dot = if selected {
        lucide::circle_dot().size(14).style(text::primary)
    } else {
        lucide::circle().size(14).style(text::secondary)
    };
    button(
        row![
            icon.into(),
            column![
                text(title).size(13),
                text(subtitle).size(11).style(text::secondary),
            ]
            .spacing(2),
            Space::new().width(Length::Fill),
            dot,
        ]
        .spacing(12)
        .align_y(Alignment::Center),
    )
    .padding(6)
    .width(Length::Fill)
    .style(move |theme, status| button_style(theme, status, ButtonKind::Row(selected)))
    .on_press(message)
    .into()
}

pub fn main_view(state: &Taplet) -> Element<'_, Message> {
    let interval_valid = state.interval_valid();
    let ms = state
        .interval_text
        .trim()
        .parse::<u32>()
        .unwrap_or(100)
        .clamp(1, 1000);

    let width = state.window_size.width;
    let height = state.window_size.height;
    let show_header = height >= 320.0;
    let show_subtitle = width >= 400.0 && height >= 500.0;
    let show_slider = height >= 340.0;
    let wide = width >= 640.0 && height >= 480.0;
    let show_options = wide || (height >= 430.0 && width >= 300.0);
    let show_repeat = wide || (height >= 540.0 && width >= 320.0);
    let show_details = wide || (height >= 700.0 && width >= 340.0);

    let counter = if state.running || state.pending_start || state.clicks > 0 {
        format!("{} clicks", state.clicks)
    } else {
        String::new()
    };

    let cps_text = if interval_valid {
        format!("{:.0} clicks per second", state.config.cps())
    } else {
        "Enter a whole number of milliseconds".to_string()
    };

    let settings = button(lucide::settings().size(15))
        .padding(8)
        .style(|theme, status| button_style(theme, status, ButtonKind::Secondary))
        .on_press(Message::OpenAdvanced);

    let mut title = column![text("Taplet").size(17)].spacing(2);
    if show_subtitle {
        title = title.push(
            text("Wayland-native autoclicker")
                .size(11)
                .style(text::secondary),
        );
    }
    let mut header = row![
        app_icon(),
        title,
        Space::new().width(Length::Fill),
    ];
    if wide && !counter.is_empty() {
        header = header.push(
            text(counter.clone())
                .size(11)
                .style(text::secondary),
        );
    }
    header = header.push(status_badge(state)).push(settings);
    let header = header.spacing(10).align_y(Alignment::Center);

    let interval_input = text_input("100", &state.interval_text)
        .on_input(Message::IntervalChanged)
        .style(text_input_style)
        .size(26)
        .padding(4)
        .width(110);

    let cps_badge: Element<'_, Message> = if interval_valid {
        container(
            column![
                lucide::zap().size(16).style(text::primary),
                text(format!("{:.0} CPS", state.config.cps()))
                    .size(10)
                    .style(text::primary),
            ]
            .spacing(2)
            .align_x(Alignment::Center),
        )
        .padding([8, 12])
        .style(|theme: &Theme| {
            let palette = theme.extended_palette();
            container::Style {
                background: Some(palette.primary.base.color.scale_alpha(0.15).into()),
                border: Border {
                    color: palette.primary.base.color.scale_alpha(0.4),
                    width: 1.0,
                    radius: 6.0.into(),
                },
                ..container::Style::default()
            }
        })
        .into()
    } else {
        Space::new().into()
    };

    let mut interval_left = column![
        row![interval_input, text("ms").size(20)]
            .spacing(8)
            .align_y(Alignment::Center)
    ]
    .spacing(2);
    if height >= 300.0 {
        interval_left = interval_left.push(text(cps_text).size(11).style(if interval_valid {
            text::secondary
        } else {
            text::danger
        }));
    }

    let mut interval_body = column![].spacing(8);
    if height >= 300.0 {
        interval_body = interval_body.push(caption("CLICK INTERVAL"));
    }
    interval_body = interval_body.push(
        row![interval_left, Space::new().width(Length::Fill), cps_badge,]
            .align_y(Alignment::Center),
    );
    if show_slider {
        interval_body = interval_body.push(
            slider(1.0..=1000.0, (1001 - ms) as f32, |v| {
                Message::IntervalChanged(format!("{}", (1001.0 - v).round() as u32))
            })
            .style(slider_style)
            .height(14),
        );
        interval_body = interval_body.push(row![
            caption("Slow"),
            Space::new().width(Length::Fill),
            caption("Fast")
        ]);
    }
    let interval_card = card(interval_body);

    let options = row![
        column![
            label("Mouse button"),
            pick_list(
                MouseButton::ALL,
                Some(state.config.button),
                Message::ButtonSelected
            )
            .style(pick_list_style)
            .menu_style(menu_style)
            .width(Length::Fill),
        ]
        .spacing(6)
        .width(Length::Fill),
        column![
            label("Click type"),
            pick_list(
                ClickType::ALL,
                Some(state.config.click_type),
                Message::ClickTypeSelected
            )
            .style(pick_list_style)
            .menu_style(menu_style)
            .width(Length::Fill),
        ]
        .spacing(6)
        .width(Length::Fill),
    ]
    .spacing(12);

    let mode = state.config.repeat_mode;
    let segmented = container(
        row![
            segment("Until stopped", RepeatMode::Continuous, mode),
            segment("Fixed count", RepeatMode::Count, mode),
        ]
        .spacing(4),
    )
    .padding(2)
    .width(Length::Fill)
    .style(segment_track_style);

    let mut repeat_controls = row![segmented]
        .spacing(8)
        .width(Length::Fill)
        .align_y(Alignment::Center);

    if mode == RepeatMode::Count {
        repeat_controls = repeat_controls.push(
            text_input("100", &state.count_text)
                .on_input(Message::CountChanged)
                .style(text_input_style)
                .width(70),
        );
    }

    let repeat = column![label("Repeat"), repeat_controls]
        .spacing(4)
        .width(Length::Fill);

    let fixed = state.config.location == ClickLocation::Fixed;
    let mut location_card = column![
        location_option(
            lucide::mouse_pointer().size(14).style(text::primary),
            "Follow cursor",
            "Click wherever pointer currently sits",
            !fixed,
            Message::LocationSelected(ClickLocation::Cursor),
        ),
        rule::horizontal(1).style(rule_style),
        location_option(
            lucide::crosshair().size(14).style(text::primary),
            "Fixed position",
            "Choose screen coordinates",
            fixed,
            Message::LocationSelected(ClickLocation::Fixed),
        ),
    ]
    .spacing(6);

    if fixed {
        location_card = location_card.push(
            row![
                text("X").size(12).style(text::secondary),
                text_input("0", &state.fixed_x_text)
                    .on_input(Message::FixedXChanged)
                    .style(text_input_style)
                    .width(90),
                text("Y").size(12).style(text::secondary),
                text_input("0", &state.fixed_y_text)
                    .on_input(Message::FixedYChanged)
                    .style(text_input_style)
                    .width(90),
                Space::new().width(Length::Fill),
                button(lucide::pipette().size(15))
                    .padding(8)
                    .style(|theme, status| { button_style(theme, status, ButtonKind::Secondary) })
                    .on_press(Message::PickPosition),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        );
    }

    let location = card(location_card);

    let (hotkey_label, hotkey_tone) = hotkey_short(&state.hotkey);
    let shortcut_body: Element<'_, Message> = column![
        row![
            lucide::keyboard().size(13).style(text::secondary),
            text("Start / stop shortcut").size(13),
            Space::new().width(Length::Fill),
            badge(text(state.config.shortcut.clone()).size(11)),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
        row![
            lucide::globe().size(13).style(text::secondary),
            text("Global hotkey").size(11).style(text::secondary),
            Space::new().width(Length::Fill),
            text(hotkey_label)
                .size(11)
                .style(status_text_style(hotkey_tone)),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
        row![
            lucide::monitor().size(13).style(text::secondary),
            text("Input backend").size(11).style(text::secondary),
            Space::new().width(Length::Fill),
            text(backend_name(state)).size(11).style(text::primary),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
    ]
    .spacing(10)
    .into();
    let shortcut = if wide {
        card_fill(shortcut_body)
    } else {
        card_fixed(SHORTCUT_CARD_HEIGHT, shortcut_body)
    };

    let running = state.running || state.pending_start;
    let action_icon = if running {
        lucide::circle_stop().size(15)
    } else {
        lucide::circle_play().size(15)
    };
    let compact_label = width < 380.0;
    let action_label = if running {
        if compact_label {
            "Stop"
        } else {
            "Stop clicking"
        }
    } else if compact_label {
        "Start"
    } else {
        "Start clicking"
    };

    let action_kind = if running {
        ButtonKind::Danger
    } else {
        ButtonKind::Primary
    };

    let action = button(
        container(
            row![action_icon, text(action_label).size(14)]
                .spacing(8)
                .align_y(Alignment::Center),
        )
        .width(Length::Fill)
        .align_x(Alignment::Center),
    )
    .padding(12)
    .width(Length::Fill)
    .on_press_maybe(
        (running || (interval_valid && state.count_valid() && state.location_valid()))
            .then_some(Message::ToggleRun),
    )
    .style(move |theme, status| button_style(theme, status, action_kind));

    let mut body = column![].spacing(12);
    if show_header {
        body = body.push(header);
        body = body.push(rule::horizontal(1).style(rule_style));
    }

    if wide {
        let left = column![
            interval_card,
            options,
            Space::new().height(Length::Fill),
            card(repeat)
        ]
        .spacing(12)
        .height(Length::Fill);
        let right = column![location, shortcut].spacing(12).height(Length::Fill);
        body = body.push(
            row![left.width(Length::Fill), right.width(Length::Fill)]
                .spacing(12)
                .height(Length::Fill),
        );
    } else {
        body = body.push(interval_card);
        if show_options {
            body = body.push(options);
        }
        if show_repeat {
            body = body.push(repeat);
        }
        if show_details {
            body = body.push(location);
            body = body.push(shortcut);
        }
    }

    if let Some(message) = &state.message {
        body = body.push(text(message.as_str()).size(12).style(text::danger));
    }
    if !wide && !counter.is_empty() {
        body = body.push(text(counter).size(11).style(text::secondary));
    }

    let padding = if height < 400.0 { 12 } else { 16 };
    let content: Element<'_, Message> = if wide {
        column![body.height(Length::Fill), action]
            .spacing(12)
            .height(Length::Fill)
            .into()
    } else {
        column![scrollable(body).height(Length::Fill), action]
            .spacing(12)
            .height(Length::Fill)
            .into()
    };

    container(content)
        .padding(padding)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

pub fn advanced_view(state: &Taplet) -> Element<'_, Message> {
    let shortcut_text = if state.capturing_shortcut {
        "Press a key… (Esc to cancel)".to_string()
    } else {
        state.config.shortcut.clone()
    };

    let accent = state.config.accent;
    let mode = state.config.theme;
    let custom = state.config.custom_accent;
    let mut swatches = row![accent_swatch(AccentColor::ALL[0], accent, mode, custom)]
        .spacing(5)
        .width(Length::Fill)
        .align_y(Alignment::Center);
    for swatch in &AccentColor::ALL[1..] {
        swatches = swatches.push(accent_swatch(*swatch, accent, mode, custom));
    }
    if state.window_size.width >= 380.0 {
        swatches = swatches
            .push(Space::new().width(Length::Fill))
            .push(text(accent.to_string()).size(11).style(text::secondary));
    }

    let mut appearance_body = column![
        label("Appearance"),
        column![
            text("Theme").size(11).style(text::secondary),
            pick_list(ThemeMode::ALL, Some(mode), Message::ThemeSelected)
                .style(pick_list_style)
                .menu_style(menu_style),
        ]
        .spacing(6),
        column![text("Accent").size(11).style(text::secondary), swatches,].spacing(6),
    ]
    .spacing(10);

    if accent == AccentColor::Custom {
        let valid = state.custom_hex_valid();
        let empty = state.custom_hex_text.trim().is_empty();
        let narrow = state.window_size.width < 360.0;
        let hint = if valid || empty {
            "Preview"
        } else {
            "Invalid hex"
        };
        let mut hex_row = row![
            text("Hex").size(11).style(text::secondary).width(16),
            text_input("#8b5cf6", &state.custom_hex_text)
                .on_input(Message::CustomHexChanged)
                .style(text_input_style)
                .width(if narrow { 96 } else { 110 }),
            color_preview(accent_color(AccentColor::Custom, mode, custom)),
        ]
        .spacing(8)
        .align_y(Alignment::Center);
        if !narrow {
            hex_row =
                hex_row
                    .push(Space::new().width(Length::Fill))
                    .push(text(hint).size(10).style(if valid || empty {
                        text::secondary
                    } else {
                        text::danger
                    }));
        }

        let mut custom_section = column![hex_row];
        if narrow && !valid && !empty {
            custom_section = custom_section.push(text("Invalid hex").size(10).style(text::danger));
        }
        custom_section = custom_section
            .push(custom_channel("R", 0, custom))
            .push(custom_channel("G", 1, custom))
            .push(custom_channel("B", 2, custom));
        appearance_body = appearance_body.push(custom_section.spacing(6));
    }

    let appearance = card(appearance_body);

    let (hotkey_text, hotkey_tone) = hotkey_line(&state.hotkey);
    let (hotkey_label, hotkey_badge_tone) = hotkey_short(&state.hotkey);
    let bind_line = state.hyprland_bind();
    let shortcut = card(
        column![
            label("Keyboard shortcut"),
            row![
                text(shortcut_text).size(18),
                Space::new().width(Length::Fill),
                button(text("Change").size(13))
                    .on_press(Message::CaptureShortcut)
                    .style(|theme, status| {
                        button_style(theme, status, ButtonKind::Secondary)
                    }),
            ]
            .align_y(Alignment::Center),
            text("Works while the Taplet window is focused; the global hotkey uses the same key.")
                .size(11)
                .style(text::secondary),
            rule::horizontal(1).style(rule_style),
            row![
                text("Global hotkey").size(13),
                Space::new().width(Length::Fill),
                text(hotkey_label)
                    .size(12)
                    .style(status_text_style(hotkey_badge_tone)),
            ]
            .align_y(Alignment::Center),
            text(hotkey_text)
                .size(11)
                .style(status_text_style(hotkey_tone)),
            text("Tries the XDG GlobalShortcuts portal first, then falls back to reading /dev/input (input group required).")
                .size(11)
                .style(text::secondary),
            rule::horizontal(1).style(rule_style),
            label("Hyprland fallback"),
            text(bind_line.clone()).size(13).font(iced::Font::MONOSPACE),
            row![
                button(text("Copy line").size(13))
                    .on_press(Message::CopyBind)
                    .style(|theme, status| {
                        button_style(theme, status, ButtonKind::Secondary)
                    }),
                text("Only needed if global registration fails.")
                    .size(11)
                    .style(text::secondary),
            ]
            .spacing(10)
            .align_y(Alignment::Center),
        ]
        .spacing(8),
    );

    let backend = card(
        column![
            label("Input backend"),
            pick_list(
                BackendChoice::ALL,
                Some(state.config.backend),
                Message::BackendSelected
            )
            .style(pick_list_style)
            .menu_style(menu_style)
            .width(Length::Fill),
            status_line(state),
            button(text("Reconnect").size(13))
                .on_press(Message::Reconnect)
                .style(|theme, status| { button_style(theme, status, ButtonKind::Secondary) }),
        ]
        .spacing(8),
    );

    let content = column![
        row![
            button(lucide::arrow_left().size(16))
                .padding(8)
                .style(|theme, status| { button_style(theme, status, ButtonKind::Secondary) })
                .on_press(Message::CloseAdvanced),
            text("Advanced").size(22),
        ]
        .spacing(12)
        .align_y(Alignment::Center),
        appearance,
        shortcut,
        backend,
    ]
    .spacing(14);

    container(scrollable(content))
        .padding(20)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}
