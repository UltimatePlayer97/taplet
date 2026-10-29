use thiserror::Error;

#[derive(Debug, Error)]
pub enum InputError {
    #[error("no Wayland display found (is WAYLAND_DISPLAY set?): {0}")]
    NoDisplay(String),
    #[error(
        "compositor does not support zwlr_virtual_pointer_manager_v1; this backend needs a wlroots-style compositor such as Hyprland or Sway"
    )]
    ProtocolUnsupported,
    #[error("Wayland connection failed: {0}")]
    Connection(String),
    #[error("backend is not connected")]
    NotConnected,

    #[error("no permission to open /dev/uinput")]
    UinputPermission,
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("could not determine the configuration directory")]
    NoConfigDir,
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid configuration: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("could not serialise configuration: {0}")]
    Serialize(#[from] toml::ser::Error),
}
