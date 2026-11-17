use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

const IO_TIMEOUT: Duration = Duration::from_secs(3);
const REPLY_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IpcCommand {
    Activate,
    Start,
    Stop,
    Status,
    Quit,
}

impl IpcCommand {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Activate => "ACTIVATE",
            Self::Start => "START",
            Self::Stop => "STOP",
            Self::Status => "STATUS",
            Self::Quit => "QUIT",
        }
    }

    pub fn parse(line: &str) -> Option<Self> {
        match line.trim() {
            "ACTIVATE" => Some(Self::Activate),
            "START" => Some(Self::Start),
            "STOP" => Some(Self::Stop),
            "STATUS" => Some(Self::Status),
            "QUIT" => Some(Self::Quit),
            _ => None,
        }
    }
}

pub struct IpcRequest {
    pub command: IpcCommand,
    pub reply: mpsc::SyncSender<String>,
}

pub enum BindOutcome {
    Bound(UnixListener),
    AlreadyRunning,
}

pub struct SocketGuard {
    path: PathBuf,
}

impl SocketGuard {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

impl Drop for SocketGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

pub fn default_socket_path() -> PathBuf {
    if let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR") {
        if !runtime_dir.trim().is_empty() {
            return PathBuf::from(runtime_dir).join("dolby-gui.sock");
        }
    }
    std::env::temp_dir().join("dolby-gui.sock")
}

pub fn send_command(path: &Path, command: IpcCommand) -> io::Result<String> {
    let mut stream = UnixStream::connect(path)?;
    stream.set_read_timeout(Some(IO_TIMEOUT))?;
    stream.set_write_timeout(Some(IO_TIMEOUT))?;

    writeln!(stream, "{}", command.as_str())?;
    stream.flush()?;

    let mut reader = BufReader::new(stream);
    let mut response = String::new();
    let n = reader.read_line(&mut response)?;
    if n == 0 {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "IPC server closed connection without a response",
        ));
    }
    Ok(response.trim_end_matches(['\r', '\n']).to_string())
}

pub fn bind_or_connect(path: &Path) -> io::Result<BindOutcome> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    for attempt in 0..3 {
        match UnixListener::bind(path) {
            Ok(listener) => return Ok(BindOutcome::Bound(listener)),
            Err(err) if err.kind() == io::ErrorKind::AddrInUse => {
                match send_command(path, IpcCommand::Status) {
                    Ok(_) => return Ok(BindOutcome::AlreadyRunning),
                    Err(conn_err)
                        if matches!(
                            conn_err.kind(),
                            io::ErrorKind::ConnectionRefused | io::ErrorKind::NotFound
                        ) =>
                    {
                        let _ = std::fs::remove_file(path);
                        if attempt == 2 {
                            return UnixListener::bind(path).map(BindOutcome::Bound);
                        }
                    }
                    Err(conn_err) => return Err(conn_err),
                }
            }
            Err(err) => return Err(err),
        }
    }

    Err(io::Error::new(
        io::ErrorKind::AddrInUse,
        "failed to bind or connect to dolby-gui IPC socket",
    ))
}

pub fn spawn_listener(
    listener: UnixListener,
    socket_path: PathBuf,
) -> (async_channel::Receiver<IpcRequest>, SocketGuard) {
    let (tx, rx) = async_channel::unbounded::<IpcRequest>();
    let guard = SocketGuard::new(socket_path);

    std::thread::Builder::new()
        .name("dolby-gui-ipc".into())
        .spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else {
                    break;
                };
                if handle_client(stream, &tx).is_err() {
                    // Client disconnected or timed out; keep listening.
                }
            }
        })
        .expect("failed to spawn dolby-gui IPC listener thread");

    (rx, guard)
}

fn handle_client(
    mut stream: UnixStream,
    tx: &async_channel::Sender<IpcRequest>,
) -> io::Result<()> {
    stream.set_read_timeout(Some(IO_TIMEOUT))?;
    stream.set_write_timeout(Some(IO_TIMEOUT))?;

    let mut line = String::new();
    {
        let mut reader = BufReader::new(&stream);
        if reader.read_line(&mut line)? == 0 {
            return Ok(());
        }
    }

    let response = match IpcCommand::parse(&line) {
        Some(command) => {
            let (reply_tx, reply_rx) = mpsc::sync_channel(1);
            if tx.send_blocking(IpcRequest { command, reply: reply_tx }).is_err() {
                r#"{"ok":false,"error":"gui event loop closed"}"#.to_string()
            } else {
                reply_rx.recv_timeout(REPLY_TIMEOUT).unwrap_or_else(|_| {
                    r#"{"ok":false,"error":"gui event loop timeout"}"#.to_string()
                })
            }
        }
        None => r#"{"ok":false,"error":"unknown command"}"#.to_string(),
    };

    writeln!(stream, "{response}")?;
    stream.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_sock(name: &str) -> PathBuf {
        let unique = format!(
            "dolby-gui-test-{}-{}-{}.sock",
            name,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        std::env::temp_dir().join(unique)
    }

    #[test]
    fn stale_socket_is_replaced_and_new_listener_accepts_commands() {
        let sock = temp_sock("stale");
        // Create a stale socket by binding and dropping the listener without removing the file.
        {
            let _listener = UnixListener::bind(&sock).unwrap();
        }
        assert!(sock.exists());

        let outcome = bind_or_connect(&sock).unwrap();
        let listener = match outcome {
            BindOutcome::Bound(l) => l,
            BindOutcome::AlreadyRunning => panic!("stale socket should have been replaced"),
        };

        let (rx, _guard) = spawn_listener(listener, sock.clone());
        let handle = std::thread::spawn(move || {
            let req = rx.recv_blocking().unwrap();
            assert_eq!(req.command, IpcCommand::Status);
            req.reply
                .send(r#"{"pid":123,"running":false,"profile":"dynamic","window_open":true}"#.into())
                .unwrap();
        });

        let resp = send_command(&sock, IpcCommand::Status).unwrap();
        assert!(resp.contains(r#""running":false"#));
        handle.join().unwrap();
    }

    #[test]
    fn active_socket_is_preserved_and_reports_already_running() {
        let sock = temp_sock("active");
        let listener = match bind_or_connect(&sock).unwrap() {
            BindOutcome::Bound(l) => l,
            BindOutcome::AlreadyRunning => panic!("expected fresh bind"),
        };
        let (rx, _guard) = spawn_listener(listener, sock.clone());

        let server = std::thread::spawn(move || {
            for _ in 0..2 {
                let req = rx.recv_blocking().unwrap();
                let resp = match req.command {
                    IpcCommand::Status => r#"{"pid":42,"running":true,"profile":"movie","window_open":false}"#,
                    IpcCommand::Activate => r#"{"ok":true,"window_open":true}"#,
                    _ => r#"{"ok":true}"#,
                };
                req.reply.send(resp.into()).unwrap();
            }
        });

        match bind_or_connect(&sock).unwrap() {
            BindOutcome::AlreadyRunning => {}
            BindOutcome::Bound(_) => panic!("active socket must not be overwritten"),
        }

        let resp = send_command(&sock, IpcCommand::Activate).unwrap();
        assert!(resp.contains(r#""window_open":true"#));
        server.join().unwrap();
    }
}
