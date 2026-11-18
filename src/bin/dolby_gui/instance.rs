use std::fs::{self, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

const IO_TIMEOUT: Duration = Duration::from_secs(3);
const REPLY_TIMEOUT: Duration = Duration::from_secs(5);
const QUIT_FLUSH_TIMEOUT: Duration = Duration::from_millis(500);

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
    pub flushed: mpsc::Receiver<()>,
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
        let _ = fs::remove_file(&self.path);
    }
}

pub fn default_socket_path() -> PathBuf {
    if let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR") {
        if !runtime_dir.trim().is_empty() {
            return PathBuf::from(runtime_dir)
                .join("opendolby")
                .join("gui.sock");
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        if !home.trim().is_empty() {
            return PathBuf::from(home)
                .join(".cache")
                .join("opendolby")
                .join("run")
                .join("gui.sock");
        }
    }
    PathBuf::from(".opendolby-run").join("gui.sock")
}

pub fn is_not_running_error(err: &io::Error) -> bool {
    matches!(
        err.kind(),
        io::ErrorKind::ConnectionRefused | io::ErrorKind::NotFound
    )
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

fn ensure_private_dir(dir: &Path) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

fn bind_private_socket(path: &Path) -> io::Result<UnixListener> {
    let listener = UnixListener::bind(path)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

pub fn bind_or_connect(path: &Path) -> io::Result<BindOutcome> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "socket path has no parent dir")
    })?;
    ensure_private_dir(parent)?;

    let lock_path = parent.join("instance.lock");
    let lock_file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)?;
    fs::set_permissions(&lock_path, fs::Permissions::from_mode(0o600))?;
    lock_file.lock()?;

    match bind_private_socket(path) {
        Ok(listener) => Ok(BindOutcome::Bound(listener)),
        Err(err) if err.kind() == io::ErrorKind::AddrInUse => {
            match send_command(path, IpcCommand::Status) {
                Ok(_) => Ok(BindOutcome::AlreadyRunning),
                Err(conn_err) if is_not_running_error(&conn_err) => {
                    let _ = fs::remove_file(path);
                    bind_private_socket(path).map(BindOutcome::Bound)
                }
                Err(conn_err) => Err(conn_err),
            }
        }
        Err(err) => Err(err),
    }
}

pub fn wait_for_quit_flush(flushed_rx: &mpsc::Receiver<()>) {
    let _ = flushed_rx.recv_timeout(QUIT_FLUSH_TIMEOUT);
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

fn handle_client(mut stream: UnixStream, tx: &async_channel::Sender<IpcRequest>) -> io::Result<()> {
    stream.set_read_timeout(Some(IO_TIMEOUT))?;
    stream.set_write_timeout(Some(IO_TIMEOUT))?;

    let mut line = String::new();
    {
        let mut reader = BufReader::new(&stream);
        if reader.read_line(&mut line)? == 0 {
            return Ok(());
        }
    }

    let (flushed_tx, flushed_rx) = mpsc::sync_channel(1);
    let mut notify_flush: Option<mpsc::SyncSender<()>> = None;

    let response = match IpcCommand::parse(&line) {
        Some(command) => {
            let (reply_tx, reply_rx) = mpsc::sync_channel(1);
            notify_flush = Some(flushed_tx);
            if tx
                .send_blocking(IpcRequest {
                    command,
                    reply: reply_tx,
                    flushed: flushed_rx,
                })
                .is_err()
            {
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
    if let Some(tx) = notify_flush {
        let _ = tx.send(());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn temp_dir(name: &str) -> PathBuf {
        let unique = format!(
            "dolby-gui-test-{}-{}-{}",
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
    fn socket_and_dir_permissions_are_private() {
        let dir = temp_dir("perms");
        let sock = dir.join("gui.sock");

        let listener = match bind_or_connect(&sock).unwrap() {
            BindOutcome::Bound(l) => l,
            BindOutcome::AlreadyRunning => panic!("expected fresh bind"),
        };
        let _guard = SocketGuard::new(sock.clone());

        let dir_mode = fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
        let sock_mode = fs::metadata(&sock).unwrap().permissions().mode() & 0o777;
        assert_eq!(dir_mode, 0o700);
        assert_eq!(sock_mode, 0o600);

        drop(listener);
        drop(_guard);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn stale_socket_is_replaced_and_new_listener_accepts_commands() {
        let dir = temp_dir("stale");
        let sock = dir.join("gui.sock");
        ensure_private_dir(&dir).unwrap();
        {
            let _listener = UnixListener::bind(&sock).unwrap();
        }
        assert!(sock.exists());

        let listener = match bind_or_connect(&sock).unwrap() {
            BindOutcome::Bound(l) => l,
            BindOutcome::AlreadyRunning => panic!("stale socket should have been replaced"),
        };

        let (rx, _guard) = spawn_listener(listener, sock.clone());
        let handle = std::thread::spawn(move || {
            let req = rx.recv_blocking().unwrap();
            assert_eq!(req.command, IpcCommand::Status);
            req.reply
                .send(
                    r#"{"pid":123,"running":false,"profile":"dynamic","window_open":true}"#.into(),
                )
                .unwrap();
        });

        let resp = send_command(&sock, IpcCommand::Status).unwrap();
        assert!(resp.contains(r#""running":false"#));
        handle.join().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn concurrent_bind_or_connect_only_allows_one_winner() {
        let dir = temp_dir("concurrent");
        let sock = Arc::new(dir.join("gui.sock"));

        let barrier = Arc::new(std::sync::Barrier::new(6));
        let mut handles = Vec::new();
        for _ in 0..6 {
            let sock = Arc::clone(&sock);
            let barrier = Arc::clone(&barrier);
            handles.push(std::thread::spawn(move || {
                barrier.wait();
                match bind_or_connect(&sock).unwrap() {
                    BindOutcome::Bound(listener) => {
                        let (rx, guard) = spawn_listener(listener, sock.to_path_buf());
                        // Serve Status probes from peers while they race.
                        for _ in 0..5 {
                            if let Ok(req) = rx.recv_blocking() {
                                let _ = req.reply.send(r#"{"pid":1,"running":false,"profile":"dynamic","window_open":true}"#.into());
                            }
                        }
                        drop(guard);
                        1usize
                    }
                    BindOutcome::AlreadyRunning => 0usize,
                }
            }));
        }

        let winners: usize = handles.into_iter().map(|h| h.join().unwrap()).sum();
        assert_eq!(winners, 1);
        let _ = fs::remove_dir_all(&dir);
    }
}
