use iced::widget::{
    Space, button, column, container, pick_list, row, rule, scrollable, slider, text, text_input,
};
use iced::{Alignment, Border, Element, Length, Theme};
use iced_fonts::lucide;

use crate::app::{BackendStatus, Message, Taplet};
use crate::config::{BackendChoice, ClickLocation, ClickType, RepeatMode, ThemeMode};
use crate::hotkey::HotkeyStatus;
use crate::input::MouseButton;

fn card<'a>(content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    container(content)
        .padding(14)
        .width(Length::Fill)
        .style(container::rounded_box)
        .into()
}

fn badge<'a>(content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    container(content)
        .padding([3, 10])
        .style(|theme: &Theme| {
            let palette = theme.extended_palette();
            container::Style {
                background: Some(palette.background.strong.color.scale_alpha(0.5).into()),
                border: Border {
                    radius: 6.0.into(),
                    ..Border::default()
                },
                ..container::Style::default()
            }
        })
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
                radius: 6.0.into(),
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
                    radius: 10.0.into(),
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
    .padding(7)
    .on_press(Message::RepeatModeSelected(mode))
    .style(if mode == current {
        button::secondary
    } else {
        button::text
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
    .style(button::text)
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

    let cps_text = if interval_valid {
        format!("{:.0} clicks per second", state.config.cps())
    } else {
        "Enter a whole number of milliseconds".to_string()
    };

    let header = row![
        app_icon(),
        column![
            text("Taplet").size(17),
            text("Wayland-native autoclicker")
                .size(11)
                .style(text::secondary),
        ]
        .spacing(2),
        Space::new().width(Length::Fill),
        status_badge(state),
    ]
    .spacing(12)
    .align_y(Alignment::Center);

    let interval_input = text_input("100", &state.interval_text)
        .on_input(Message::IntervalChanged)
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
                    radius: 8.0.into(),
                },
                ..container::Style::default()
            }
        })
        .into()
    } else {
        Space::new().into()
    };

    let interval_card = card(
        column![
            caption("CLICK INTERVAL"),
            row![
                column![
                    row![interval_input, text("ms").size(20)]
                        .spacing(8)
                        .align_y(Alignment::Center),
                    text(cps_text).size(11).style(if interval_valid {
                        text::secondary
                    } else {
                        text::danger
                    }),
                ]
                .spacing(2),
                Space::new().width(Length::Fill),
                cps_badge,
            ]
            .align_y(Alignment::Center),
            slider(1.0..=1000.0, (1001 - ms) as f32, |v| {
                Message::IntervalChanged(format!("{}", (1001.0 - v).round() as u32))
            })
            .height(14),
            row![
                caption("Slow"),
                Space::new().width(Length::Fill),
                caption("Fast")
            ],
        ]
        .spacing(8),
    );

    let options = row![
        column![
            label("Mouse button"),
            pick_list(
                MouseButton::ALL,
                Some(state.config.button),
                Message::ButtonSelected
            )
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
    .padding(3)
    .width(Length::Fill)
    .style(container::rounded_box);

    let mut repeat = column![label("Repeat"), segmented].spacing(6);

    if mode == RepeatMode::Count {
        repeat = repeat.push(
            row![
                text("Number of clicks").size(12).style(text::secondary),
                Space::new().width(Length::Fill),
                text_input("100", &state.count_text)
                    .on_input(Message::CountChanged)
                    .width(100),
            ]
            .align_y(Alignment::Center),
        );
    }

    let fixed = state.config.location == ClickLocation::Fixed;
    let mut location_card = column![
        location_option(
            lucide::mouse_pointer().size(14).style(text::primary),
            "Follow cursor",
            "Click wherever pointer currently sits",
            !fixed,
            Message::LocationSelected(ClickLocation::Cursor),
        ),
        rule::horizontal(1),
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
                    .width(90),
                text("Y").size(12).style(text::secondary),
                text_input("0", &state.fixed_y_text)
                    .on_input(Message::FixedYChanged)
                    .width(90),
                Space::new().width(Length::Fill),
                button(lucide::pipette().size(15))
                    .padding(8)
                    .style(button::secondary)
                    .on_press(Message::PickPosition),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        );
    }

    let location = column![label("Click location"), card(location_card)].spacing(6);

    let (hotkey_label, hotkey_tone) = hotkey_short(&state.hotkey);
    let shortcut = card(
        column![
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
        .spacing(10),
    );

    let running = state.running || state.pending_start;
    let action_icon = if running {
        lucide::circle_stop().size(15)
    } else {
        lucide::circle_play().size(15)
    };
    let action_label = if running {
        "Stop clicking"
    } else {
        "Start clicking"
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
    .style(if running {
        button::danger
    } else {
        button::primary
    });

    let settings = button(lucide::sliders_horizontal().size(15))
        .padding(12)
        .style(button::secondary)
        .on_press(Message::OpenAdvanced);

    let counter = if running || state.clicks > 0 {
        format!("{} clicks", state.clicks)
    } else {
        String::new()
    };

    let mut content = column![
        header,
        rule::horizontal(1),
        interval_card,
        options,
        repeat,
        location,
        shortcut,
        row![action, settings].spacing(8),
    ]
    .spacing(12);

    if let Some(message) = &state.message {
        content = content.push(text(message.as_str()).size(12).style(text::danger));
    }
    if !counter.is_empty() {
        content = content.push(text(counter).size(11).style(text::secondary));
    }

    container(content).padding(16).width(Length::Fill).into()
}

pub fn advanced_view(state: &Taplet) -> Element<'_, Message> {
    let shortcut_text = if state.capturing_shortcut {
        "Press a key… (Esc to cancel)".to_string()
    } else {
        state.config.shortcut.clone()
    };

    let appearance = card(
        column![
            label("Theme"),
            pick_list(
                ThemeMode::ALL,
                Some(state.config.theme),
                Message::ThemeSelected
            ),
        ]
        .spacing(6),
    );

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
                    .style(button::secondary),
            ]
            .align_y(Alignment::Center),
            text("Works while the Taplet window is focused; the global hotkey uses the same key.")
                .size(11)
                .style(text::secondary),
            rule::horizontal(1),
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
            rule::horizontal(1),
            label("Hyprland fallback"),
            text(bind_line.clone()).size(13).font(iced::Font::MONOSPACE),
            row![
                button(text("Copy line").size(13))
                    .on_press(Message::CopyBind)
                    .style(button::secondary),
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
            .width(Length::Fill),
            status_line(state),
            button(text("Reconnect").size(13))
                .on_press(Message::Reconnect)
                .style(button::secondary),
        ]
        .spacing(8),
    );

    let content = column![
        row![
            button(lucide::arrow_left().size(16))
                .padding(8)
                .style(button::secondary)
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
