use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use ksni::blocking::TrayMethods;
use ksni::menu::{CheckmarkItem, MenuItem, StandardItem, SubMenu};
use ksni::{Category, Icon, Status, ToolTip};

const CPS_PRESETS: &[(u64, &str)] = &[
    (1000, "1000 ms - 1 cps"),
    (500, "500 ms - 2 cps"),
    (200, "200 ms - 5 cps"),
    (100, "100 ms - 10 cps"),
    (50, "50 ms - 20 cps"),
    (25, "25 ms - 40 cps"),
    (10, "10 ms - 100 cps"),
];

pub enum TrayEvent {
    Toggle,
    ToggleWindow,
    SetInterval(u64),
    Quit,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct TrayShared {
    pub running: bool,
    pub interval_ms: u64,
    pub window_visible: bool,
}

fn shared(guard: &Arc<Mutex<TrayShared>>) -> TrayShared {
    guard.lock().map(|value| *value).unwrap_or(TrayShared {
        running: false,
        interval_ms: crate::config::DEFAULT_INTERVAL_MS,
        window_visible: true,
    })
}

fn speed_label(interval_ms: u64) -> String {
    let cps = 1000.0 / interval_ms as f64;
    format!("{interval_ms} ms ({cps:.1} cps)")
}

fn tray_description(shared: TrayShared) -> String {
    if shared.running {
        format!("Clicking at {}", speed_label(shared.interval_ms))
    } else {
        "Idle".to_string()
    }
}

fn load_icons() -> Vec<Icon> {
    let Ok(image) = image::load_from_memory(include_bytes!("../icon.png")) else {
        return Vec::new();
    };
    let small = image.resize_exact(64, 64, image::imageops::FilterType::Lanczos3);
    let rgba = small.to_rgba8();
    let width = rgba.width() as i32;
    let height = rgba.height() as i32;
    let mut data = rgba.into_raw();
    for pixel in data.as_chunks_mut::<4>().0 {
        pixel.rotate_right(1);
    }
    vec![Icon {
        width,
        height,
        data,
    }]
}

pub struct TapletTray {
    shared: Arc<Mutex<TrayShared>>,
    events: Sender<TrayEvent>,
    icons: Vec<Icon>,
}

impl ksni::Tray for TapletTray {
    fn id(&self) -> String {
        "taplet".to_string()
    }

    fn title(&self) -> String {
        "Taplet".to_string()
    }

    fn category(&self) -> Category {
        Category::ApplicationStatus
    }

    fn status(&self) -> Status {
        // Hosts hide Passive items, so an idle autoclicker would vanish from the tray.
        Status::Active
    }

    fn icon_name(&self) -> String {
        "taplet".to_string()
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        self.icons.clone()
    }

    fn tool_tip(&self) -> ToolTip {
        let state = shared(&self.shared);
        ToolTip {
            icon_name: "taplet".to_string(),
            icon_pixmap: Vec::new(),
            title: "Taplet".to_string(),
            description: tray_description(state),
        }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        let _ = self.events.send(TrayEvent::ToggleWindow);
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let state = shared(&self.shared);
        let window_item = StandardItem {
            label: if state.window_visible {
                "Hide window".to_string()
            } else {
                "Show window".to_string()
            },
            activate: Box::new(|tray: &mut Self| {
                let _ = tray.events.send(TrayEvent::ToggleWindow);
            }),
            ..Default::default()
        };

        let start_stop = StandardItem {
            label: if state.running {
                "Stop clicking".to_string()
            } else {
                "Start clicking".to_string()
            },
            icon_name: if state.running {
                "media-playback-stop".to_string()
            } else {
                "media-playback-start".to_string()
            },
            activate: Box::new(|tray: &mut Self| {
                let _ = tray.events.send(TrayEvent::Toggle);
            }),
            ..Default::default()
        };

        let speed_items: Vec<MenuItem<Self>> = CPS_PRESETS
            .iter()
            .map(|&(interval_ms, label)| {
                CheckmarkItem {
                    label: label.to_string(),
                    checked: state.interval_ms == interval_ms,
                    activate: Box::new(move |tray: &mut Self| {
                        let _ = tray.events.send(TrayEvent::SetInterval(interval_ms));
                    }),
                    ..Default::default()
                }
                .into()
            })
            .collect();
        let speed = SubMenu {
            label: format!("Speed ({})", speed_label(state.interval_ms)),
            icon_name: "preferences-other".to_string(),
            submenu: speed_items,
            ..Default::default()
        };

        let quit = StandardItem {
            label: "Quit Taplet".to_string(),
            icon_name: "application-exit".to_string(),
            activate: Box::new(|tray: &mut Self| {
                let _ = tray.events.send(TrayEvent::Quit);
            }),
            ..Default::default()
        };

        vec![
            window_item.into(),
            start_stop.into(),
            speed.into(),
            MenuItem::Separator,
            quit.into(),
        ]
    }
}

pub fn spawn(
    shared: Arc<Mutex<TrayShared>>,
    events: Sender<TrayEvent>,
) -> Option<ksni::blocking::Handle<TapletTray>> {
    let tray = TapletTray {
        shared,
        events,
        icons: load_icons(),
    };
    match tray.spawn() {
        Ok(handle) => Some(handle),
        Err(err) => {
            eprintln!("system tray unavailable: {err}");
            None
        }
    }
}
