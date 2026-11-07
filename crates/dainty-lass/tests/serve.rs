//! The MIT core serves the admin API with no library at all, and runs a service `[services.<name>]` pins as a
//! library beside it (RD27).
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dainty_hostkit::hostconfig::HostConfig;
use dainty_hostkit::paths::Paths;

const HOST_TOML: &str = "version = 1\nmode = \"private\"\nname = \"Core Test\"\nhost = \"100.101.102.103\"\n\
    game_address = \"100.101.102.103\"\nregistration = \"INVITECODE\"\nmax_hunt_servers = 1\n\
    [mongo]\nuri = \"mongodb://127.0.0.1:27017\"\nmanage = false\n";

/// Builds the controller's echo service once, into a target folder of its own (controller's tests load theirs).
fn echo_library() -> PathBuf {
    static BUILT: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    BUILT
        .get_or_init(|| {
            let here = Path::new(env!("CARGO_MANIFEST_DIR"));
            let target = here.join("../../target/plugin-fixtures-server");
            let status = Command::new(env!("CARGO"))
                .args(["build", "--quiet", "--manifest-path"])
                .arg(here.join("../../crates/dainty-controller/tests/echo-service/Cargo.toml"))
                .arg("--target-dir")
                .arg(&target)
                .status()
                .expect("cargo runs");
            assert!(status.success(), "the echo service builds");
            let name = format!(
                "{}echo_service{}",
                std::env::consts::DLL_PREFIX,
                std::env::consts::DLL_SUFFIX
            );
            target.join("debug").join(name)
        })
        .clone()
}

fn slashes(path: &Path) -> String {
    path.display().to_string().replace('\\', "/")
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

/// A server folder with setup's files, the admin API on a port of its own.
fn server_folder(extra_toml: &str) -> (tempfile::TempDir, Paths, u16) {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::new(dir.path());
    std::fs::create_dir_all(&paths.config).unwrap();
    let text = format!("{HOST_TOML}{extra_toml}");
    std::fs::write(&paths.host_toml, &text).unwrap();
    dainty_hostkit::serve::ensure_server_files(&paths, &HostConfig::from_toml(&text).unwrap()).unwrap();
    let port = free_port();
    let admin = paths.config.join("admin.env");
    let env: String = std::fs::read_to_string(&admin)
        .unwrap()
        .lines()
        .map(|line| {
            if line.starts_with("PORT=") {
                format!("PORT={port}\n")
            } else {
                format!("{line}\n")
            }
        })
        .collect();
    std::fs::write(&admin, env).unwrap();
    (dir, paths, port)
}

struct Server {
    child: Child,
    output: Arc<Mutex<String>>,
}

impl Server {
    fn start(root: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_dainty-lass"))
            .arg("--root")
            .arg(root)
            .env("REFORGED_CONTROL", "stdin")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let output = Arc::new(Mutex::new(String::new()));
        let (ready_tx, ready_rx) = mpsc::channel();
        for stream in [
            Box::new(child.stdout.take().unwrap()) as Box<dyn std::io::Read + Send>,
            Box::new(child.stderr.take().unwrap()),
        ] {
            let (output, ready_tx) = (output.clone(), ready_tx.clone());
            std::thread::spawn(move || {
                for line in BufReader::new(stream).lines().map_while(Result::ok) {
                    if line.contains("REFORGED_SERVER_READY") {
                        _ = ready_tx.send(());
                    }
                    output.lock().unwrap().push_str(&format!("{line}\n"));
                }
            });
        }
        let server = Self { child, output };
        if ready_rx.recv_timeout(Duration::from_secs(90)).is_err() {
            panic!("the server never said it was ready:\n{}", server.output());
        }
        server
    }

    fn output(&self) -> String {
        self.output.lock().unwrap().clone()
    }

    fn stop(mut self) {
        self.child.stdin.take().unwrap().write_all(b"stop\n").unwrap();
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert!(status.success(), "it stopped with {status}:\n{}", self.output());
                return;
            }
            assert!(Instant::now() < deadline, "it never stopped:\n{}", self.output());
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        _ = self.child.kill();
    }
}

fn get(port: u16, path: &str) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut answer = String::new();
    stream.read_to_string(&mut answer).unwrap();
    answer
}

#[test]
fn with_no_library_it_serves_the_admin_api() {
    let (dir, _paths, port) = server_folder("");
    let server = Server::start(dir.path());
    let health = get(port, "/health");
    assert!(health.starts_with("HTTP/1.1 200"), "{health}\n{}", server.output());
    server.stop();
}

#[test]
fn a_service_host_toml_pins_runs_as_a_library() {
    let library = echo_library();
    let sha256 = dainty_hostkit::selfupdate::sha256_file(&library).unwrap();
    let toml = format!(
        "\n[services.echo]\nlibrary = \"{}\"\nsha256 = \"{sha256}\"\nserves = [\"content_files\"]\n",
        slashes(&library)
    );
    let (dir, paths, port) = server_folder(&toml);
    let echo_file = dir.path().join("echo.txt");
    std::fs::write(
        paths.config.join("echo.env"),
        format!("ECHO_FILE={}\nECHO_GREETING=hi\n", slashes(&echo_file)),
    )
    .unwrap();

    let server = Server::start(dir.path());
    let started = std::fs::read_to_string(&echo_file).unwrap_or_default();
    assert!(
        started.lines().any(|l| l == "start"),
        "{started:?}\n{}",
        server.output()
    );
    assert!(get(port, "/health").starts_with("HTTP/1.1 200"));
    server.stop();
    let ended = std::fs::read_to_string(&echo_file).unwrap();
    assert_eq!(ended.lines().last(), Some("stop"), "{ended:?}");

    // A library that isn't the one pinned never runs
    let wrong = toml.replace(&sha256, &"00".repeat(32));
    let (dir, paths, _) = server_folder(&wrong);
    std::fs::write(paths.config.join("echo.env"), "").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_dainty-lass"))
        .arg("--root")
        .arg(dir.path())
        .env("REFORGED_CONTROL", "stdin")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let said = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!out.status.success() && said.contains("host.toml pins"), "{said}");
}
