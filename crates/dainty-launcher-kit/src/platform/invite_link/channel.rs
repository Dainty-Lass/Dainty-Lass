// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT OR Apache-2.0

//! The single-launcher channel: a named pipe on Windows, a Unix socket elsewhere. The first launcher holds it; a
//! second one sends its arguments as one JSON array line and exits.

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use tokio::io::{AsyncBufReadExt as _, AsyncRead, AsyncReadExt as _, BufReader};

use crate::platform::paths::{DEV_PROFILE_VAR, user_data_dir_arg};

/// Longest message line read, newline excluded; the sender is another process, so nothing past this is trusted.
pub const MAX_MESSAGE_BYTES: usize = 64 * 1024;
/// Most arguments one message may carry.
pub const MAX_MESSAGE_ARGS: usize = 64;
/// How long the holder waits for a connected sender's line before dropping it.
pub const READ_TIMEOUT: Duration = Duration::from_secs(5);

const PROFILE_TAIL: usize = 24;
// The sender waits for the holder to close so the line is known to be read, not just buffered.
const DELIVERY_TIMEOUT: Duration = Duration::from_secs(5);

/// Where this user's launcher listens: a pipe name on Windows, a socket path elsewhere.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Channel {
    address: PathBuf,
}

impl Channel {
    /// A channel at an explicit address (tests, or a caller that built the name itself).
    #[must_use]
    pub fn at(address: impl Into<PathBuf>) -> Self {
        Self {
            address: address.into(),
        }
    }

    /// This process's channel for `base_id`, from its arguments, the profile variable and the OS user.
    #[must_use]
    pub fn current(base_id: &str, args: &[OsString]) -> Self {
        let profile = user_data_dir_arg(args).or_else(|| {
            std::env::var_os(DEV_PROFILE_VAR)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        });
        let id = channel_id(base_id, profile.as_deref());
        let var = |name: &str| std::env::var(name).unwrap_or_default();
        if cfg!(windows) {
            Self::at(pipe_name(&id, &var("USERNAME")))
        } else {
            let runtime = std::env::var_os("XDG_RUNTIME_DIR")
                .filter(|v| !v.is_empty())
                .map(PathBuf::from);
            Self::at(socket_path(
                &id,
                runtime.as_deref(),
                &std::env::temp_dir(),
                &var("USER"),
            ))
        }
    }

    /// The pipe name or socket path.
    #[must_use]
    pub fn address(&self) -> &Path {
        &self.address
    }
}

/// `base_id`, plus `-` and the profile path's last 24 ASCII alphanumerics when a profile is in use.
#[must_use]
pub fn channel_id(base_id: &str, profile: Option<&Path>) -> String {
    let Some(profile) = profile else {
        return base_id.to_owned();
    };
    let alnum: Vec<char> = profile
        .to_string_lossy()
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect();
    let tail: String = alnum[alnum.len().saturating_sub(PROFILE_TAIL)..].iter().collect();
    format!("{base_id}-{tail}")
}

/// `\\.\pipe\<id>-<user>`, keeping only the user name's ASCII alphanumerics.
#[must_use]
pub fn pipe_name(id: &str, username: &str) -> String {
    format!(r"\\.\pipe\{id}-{}", alphanumerics(username))
}

/// `<runtime>/<id>.sock` when the runtime folder exists, else `<temp>/<id>-<user>.sock`.
#[must_use]
pub fn socket_path(id: &str, runtime_dir: Option<&Path>, temp: &Path, user: &str) -> PathBuf {
    match runtime_dir.filter(|dir| dir.is_dir()) {
        Some(dir) => dir.join(format!("{id}.sock")),
        // Filtered so a user name can't step out of the temp folder.
        None => temp.join(format!("{id}-{}.sock", alphanumerics(user))),
    }
}

fn alphanumerics(text: &str) -> String {
    text.chars().filter(char::is_ascii_alphanumeric).collect()
}

/// The arguments as one JSON array line.
#[must_use]
pub fn encode_message(args: &[String]) -> Vec<u8> {
    let mut line = serde_json::to_vec(args).unwrap_or_else(|_| b"[]".to_vec());
    line.push(b'\n');
    line
}

/// One received line as arguments; anything unreadable or over the bounds is an empty list.
#[must_use]
pub fn decode_message(line: &[u8]) -> Vec<String> {
    let line = line.strip_suffix(b"\n").unwrap_or(line);
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    if line.len() > MAX_MESSAGE_BYTES {
        return Vec::new();
    }
    match serde_json::from_slice::<Vec<String>>(line) {
        Ok(args) if args.len() <= MAX_MESSAGE_ARGS => args,
        _ => Vec::new(),
    }
}

async fn read_message<R: AsyncRead + Unpin>(stream: R) -> Vec<String> {
    let mut reader = BufReader::new(stream.take(MAX_MESSAGE_BYTES as u64 + 2));
    let mut line = Vec::new();
    match tokio::time::timeout(READ_TIMEOUT, reader.read_until(b'\n', &mut line)).await {
        Ok(Ok(_)) => decode_message(&line),
        _ => Vec::new(),
    }
}

/// The outcome of [`claim`].
#[derive(Debug)]
pub enum Claim {
    /// Another launcher holds the channel and took the arguments: this one exits.
    HandedOver,
    /// This launcher is the first and now holds the channel.
    First(Holder),
}

/// The held end of the channel.
#[derive(Debug)]
pub struct Holder {
    inner: imp::Listener,
}

impl Holder {
    /// Waits for the next launcher and answers its arguments; one connection at a time.
    pub async fn receive(&mut self) -> io::Result<Vec<String>> {
        self.inner.receive().await
    }
}

/// Hands `args` to the launcher holding `channel`, or becomes the holder.
pub async fn claim(channel: &Channel, args: &[String]) -> io::Result<Claim> {
    let line = encode_message(args);
    if imp::send(channel.address(), &line).await {
        return Ok(Claim::HandedOver);
    }
    match imp::Listener::bind(channel.address()) {
        Ok(inner) => Ok(Claim::First(Holder { inner })),
        // Another launcher won the race between our send and bind.
        Err(error) => {
            if imp::send(channel.address(), &line).await {
                Ok(Claim::HandedOver)
            } else {
                Err(error)
            }
        }
    }
}

async fn await_close<S: AsyncRead + Unpin>(stream: &mut S) {
    let mut sink = [0_u8; 64];
    let _ = tokio::time::timeout(DELIVERY_TIMEOUT, async {
        while matches!(stream.read(&mut sink).await, Ok(n) if n > 0) {}
    })
    .await;
}

#[cfg(windows)]
mod imp {
    use std::io;
    use std::path::Path;
    use std::time::Duration;

    use tokio::io::AsyncWriteExt as _;
    use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeServer, ServerOptions};

    use super::{await_close, read_message};

    const ERROR_PIPE_BUSY: i32 = 231;
    const BUSY_RETRIES: u32 = 20;
    const BUSY_WAIT: Duration = Duration::from_millis(50);

    pub(super) async fn send(name: &Path, line: &[u8]) -> bool {
        let mut retries = 0;
        let mut client = loop {
            match ClientOptions::new().open(name) {
                Ok(client) => break client,
                Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) && retries < BUSY_RETRIES => {
                    retries += 1;
                    tokio::time::sleep(BUSY_WAIT).await;
                }
                Err(_) => return false,
            }
        };
        if client.write_all(line).await.is_err() {
            return false;
        }
        await_close(&mut client).await;
        true
    }

    #[derive(Debug)]
    pub(super) struct Listener {
        name: std::ffi::OsString,
        next: NamedPipeServer,
    }

    impl Listener {
        pub(super) fn bind(name: &Path) -> io::Result<Self> {
            // first_pipe_instance fails when another launcher already made the pipe; remote clients are refused.
            let next = ServerOptions::new()
                .first_pipe_instance(true)
                .reject_remote_clients(true)
                .create(name)?;
            Ok(Self {
                name: name.as_os_str().to_owned(),
                next,
            })
        }

        pub(super) async fn receive(&mut self) -> io::Result<Vec<String>> {
            self.next.connect().await?;
            // The next instance exists before this one is read, so a sender never finds no pipe at all.
            let fresh = ServerOptions::new().reject_remote_clients(true).create(&self.name)?;
            let connected = std::mem::replace(&mut self.next, fresh);
            Ok(read_message(connected).await)
        }
    }
}

#[cfg(unix)]
mod imp {
    use std::io;
    use std::os::unix::fs::{FileTypeExt as _, PermissionsExt as _};
    use std::path::Path;

    use tokio::io::AsyncWriteExt as _;
    use tokio::net::{UnixListener, UnixStream};

    use super::{await_close, read_message};

    pub(super) async fn send(path: &Path, line: &[u8]) -> bool {
        let Ok(mut stream) = UnixStream::connect(path).await else {
            return false;
        };
        if stream.write_all(line).await.is_err() {
            return false;
        }
        let _ = stream.shutdown().await;
        await_close(&mut stream).await;
        true
    }

    #[derive(Debug)]
    pub(super) struct Listener {
        listener: UnixListener,
    }

    impl Listener {
        pub(super) fn bind(path: &Path) -> io::Result<Self> {
            // Nobody answered the send, so a socket left here is stale; anything else is not ours to remove.
            if std::fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_socket()) {
                std::fs::remove_file(path)?;
            }
            let listener = UnixListener::bind(path)?;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
            Ok(Self { listener })
        }

        pub(super) async fn receive(&mut self) -> io::Result<Vec<String>> {
            let (stream, _) = self.listener.accept().await?;
            Ok(read_message(stream).await)
        }
    }
}

#[cfg(not(any(windows, unix)))]
mod imp {
    use std::io;
    use std::path::Path;

    pub(super) async fn send(_: &Path, _: &[u8]) -> bool {
        false
    }

    #[derive(Debug)]
    pub(super) struct Listener;

    impl Listener {
        pub(super) fn bind(_: &Path) -> io::Result<Self> {
            Err(io::Error::from(io::ErrorKind::Unsupported))
        }

        pub(super) async fn receive(&mut self) -> io::Result<Vec<String>> {
            Err(io::Error::from(io::ErrorKind::Unsupported))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::*;

    fn unique_channel() -> (Channel, Option<tempfile::TempDir>) {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let id = format!("launcher-kit-test-{}-{n}", std::process::id());
        if cfg!(windows) {
            (Channel::at(pipe_name(&id, "tester")), None)
        } else {
            let dir = tempfile::tempdir().unwrap();
            (Channel::at(dir.path().join("c.sock")), Some(dir))
        }
    }

    fn strings(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn ids_and_names() {
        assert_eq!(channel_id("com.daintylass.server", None), "com.daintylass.server");
        let profile = Path::new("C:/Users/x/profiles/dev-one_two.three/abcdefghijklmnop");
        assert_eq!(channel_id("base", Some(profile)), "base-twothreeabcdefghijklmnop");
        assert_eq!(channel_id("base", Some(Path::new("/a/b"))), "base-ab");
        assert_eq!(channel_id("base", Some(Path::new("/-_/"))), "base-");
        assert_eq!(pipe_name("base", "Jo Doe.2"), r"\\.\pipe\base-JoDoe2");
        assert_eq!(pipe_name("base", ""), r"\\.\pipe\base-");
    }

    #[test]
    fn socket_paths() {
        let runtime = tempfile::tempdir().unwrap();
        let temp = Path::new("/tmp");
        assert_eq!(
            socket_path("id", Some(runtime.path()), temp, "u"),
            runtime.path().join("id.sock")
        );
        let missing = runtime.path().join("missing");
        assert_eq!(socket_path("id", Some(&missing), temp, "jo"), temp.join("id-jo.sock"));
        assert_eq!(socket_path("id", None, temp, "../../etc"), temp.join("id-etc.sock"));
    }

    #[test]
    fn message_round_trip_and_bounds() {
        let args = strings(&["launcher.exe", "dauntless://join?v=1", "ünï \"q\"\n"]);
        let line = encode_message(&args);
        assert_eq!(line.last(), Some(&b'\n'));
        assert_eq!(line.iter().filter(|b| **b == b'\n').count(), 1, "one line");
        assert_eq!(decode_message(&line), args);
        assert_eq!(decode_message(b"[\"a\"]\r\n"), strings(&["a"]));
        assert_eq!(decode_message(b"[\"a\"]"), strings(&["a"]));
        for bad in [
            &b""[..],
            b"\n",
            b"not json\n",
            b"{\"a\":1}\n",
            b"[1]\n",
            b"[\"a\",null]\n",
            b"[\"a\"] x\n",
        ] {
            assert!(decode_message(bad).is_empty(), "{bad:?}");
        }
        let many: Vec<String> = (0..=MAX_MESSAGE_ARGS).map(|i| i.to_string()).collect();
        assert!(decode_message(&encode_message(&many)).is_empty());
        assert_eq!(
            decode_message(&encode_message(&many[..MAX_MESSAGE_ARGS])).len(),
            MAX_MESSAGE_ARGS
        );
        let big = vec!["a".repeat(MAX_MESSAGE_BYTES)];
        assert!(decode_message(&encode_message(&big)).is_empty());
    }

    #[tokio::test]
    async fn reads_are_bounded() {
        let huge = format!("[\"{}\"]\n", "a".repeat(MAX_MESSAGE_BYTES * 2));
        assert!(read_message(huge.as_bytes()).await.is_empty());
        assert_eq!(read_message(&b"[\"x\"]\n[\"y\"]\n"[..]).await, strings(&["x"]));
    }

    #[tokio::test]
    async fn second_launcher_hands_over() {
        let (channel, _dir) = unique_channel();
        let Claim::First(mut holder) = claim(&channel, &strings(&["first"])).await.unwrap() else {
            panic!("the first claim holds the channel");
        };
        let receiver = tokio::spawn(async move {
            let one = holder.receive().await.unwrap();
            let two = holder.receive().await.unwrap();
            (one, two, holder)
        });
        let second = strings(&["launcher.exe", "dauntless://join?v=1&x=1"]);
        assert!(matches!(claim(&channel, &second).await.unwrap(), Claim::HandedOver));
        assert!(matches!(
            claim(&channel, &strings(&["third"])).await.unwrap(),
            Claim::HandedOver
        ));
        let (one, two, _holder) = receiver.await.unwrap();
        assert_eq!(one, second);
        assert_eq!(two, strings(&["third"]));
    }

    #[tokio::test]
    async fn garbage_from_a_sender_is_an_empty_list() {
        let (channel, _dir) = unique_channel();
        let Claim::First(mut holder) = claim(&channel, &[]).await.unwrap() else {
            panic!("first");
        };
        let receiver = tokio::spawn(async move { holder.receive().await.unwrap() });
        assert!(imp::send(channel.address(), b"{\"not\":\"a list\"}\n").await);
        assert!(receiver.await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn nobody_listening_means_not_sent() {
        let (channel, _dir) = unique_channel();
        assert!(!imp::send(channel.address(), b"[]\n").await);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn stale_socket_is_replaced_and_owner_only() {
        use std::os::unix::fs::PermissionsExt as _;
        let (channel, _dir) = unique_channel();
        drop(std::os::unix::net::UnixListener::bind(channel.address()).unwrap());
        let Claim::First(_holder) = claim(&channel, &[]).await.unwrap() else {
            panic!("a stale socket is taken over");
        };
        let mode = std::fs::metadata(channel.address()).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_regular_file_is_not_removed() {
        let (channel, _dir) = unique_channel();
        std::fs::write(channel.address(), b"keep").unwrap();
        assert!(claim(&channel, &[]).await.is_err());
        assert_eq!(std::fs::read(channel.address()).unwrap(), b"keep");
    }
}
