use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteCommand {
    Toggle,
    Start,
    Stop,
    Status,
}

impl RemoteCommand {
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim() {
            "toggle" => Some(Self::Toggle),
            "start" => Some(Self::Start),
            "stop" => Some(Self::Stop),
            "status" => Some(Self::Status),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Toggle => "toggle",
            Self::Start => "start",
            Self::Stop => "stop",
            Self::Status => "status",
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

pub fn send_with_reply(command: RemoteCommand, timeout: Duration) -> std::io::Result<String> {
    let mut stream = UnixStream::connect(socket_path())?;
    stream.write_all(command.as_str().as_bytes())?;
    stream.write_all(b"\n")?;
    stream.flush()?;
    stream.set_read_timeout(Some(timeout))?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    Ok(line.trim().to_string())
}

pub fn instance_running() -> bool {
    UnixStream::connect(socket_path()).is_ok()
}

pub struct Listener {
    pub commands: Receiver<(RemoteCommand, Option<Sender<String>>)>,
    path: PathBuf,
}

impl Listener {
    pub fn bind() -> std::io::Result<Self> {
        let path = socket_path();
        if path.exists() {
            let _ = std::fs::remove_file(&path);
        }
        let listener = UnixListener::bind(&path)?;
        let (tx, rx): (Sender<(RemoteCommand, Option<Sender<String>>)>, Receiver<(RemoteCommand, Option<Sender<String>>)>) = mpsc::channel();
        thread::Builder::new()
            .name("taplet-ipc".to_string())
            .spawn(move || {
                for stream in listener.incoming().flatten() {
                    let mut reader = BufReader::new(&stream);
                    let mut line = String::new();
                    if reader.read_line(&mut line).is_ok()
                        && let Some(command) = RemoteCommand::parse(&line)
                    {
                        let (reply_tx, reply_rx) = if command == RemoteCommand::Status {
                            let (tx, rx) = mpsc::channel();
                            (Some(tx), Some(rx))
                        } else {
                            (None, None)
                        };
                        if tx.send((command, reply_tx)).is_err() {
                            break;
                        }
                        if let Some(rx) = reply_rx
                            && let Ok(reply) = rx.recv_timeout(Duration::from_secs(2))
                        {
                            let mut stream = stream;
                            let _ = stream.write_all(reply.as_bytes());
                            let _ = stream.write_all(b"\n");
                            let _ = stream.flush();
                        }
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
