use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::thread;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteCommand {
    Toggle,
    Start,
    Stop,
}

impl RemoteCommand {
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim() {
            "toggle" => Some(Self::Toggle),
            "start" => Some(Self::Start),
            "stop" => Some(Self::Stop),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Toggle => "toggle",
            Self::Start => "start",
            Self::Stop => "stop",
        }
    }
}

pub fn socket_path() -> PathBuf {
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join("taplet.sock")
}

pub fn send(command: RemoteCommand) -> std::io::Result<()> {
    let mut stream = UnixStream::connect(socket_path())?;
    stream.write_all(command.as_str().as_bytes())?;
    stream.write_all(b"\n")?;
    stream.flush()
}

pub fn instance_running() -> bool {
    UnixStream::connect(socket_path()).is_ok()
}

pub struct Listener {
    pub commands: Receiver<RemoteCommand>,
    path: PathBuf,
}

impl Listener {
    pub fn bind() -> std::io::Result<Self> {
        let path = socket_path();
        if path.exists() {
            let _ = std::fs::remove_file(&path);
        }
        let listener = UnixListener::bind(&path)?;
        let (tx, rx) = mpsc::channel();
        thread::Builder::new()
            .name("taplet-ipc".to_string())
            .spawn(move || {
                for stream in listener.incoming().flatten() {
                    let mut line = String::new();
                    if BufReader::new(stream).read_line(&mut line).is_ok()
                        && let Some(command) = RemoteCommand::parse(&line)
                        && tx.send(command).is_err()
                    {
                        break;
                    }
                }
            })?;
        Ok(Self { commands: rx, path })
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}
