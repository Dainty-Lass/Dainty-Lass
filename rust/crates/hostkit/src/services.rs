// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

pub mod tree {
    #[cfg(windows)]
    #[allow(unsafe_code)]
    mod imp {
        use std::ffi::c_void;
        use std::io;
        use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
        use std::process::Child;

        use windows::Win32::Foundation::HANDLE;
        use windows::Win32::System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
            SetInformationJobObject, TerminateJobObject,
        };
        use windows::core::PCWSTR;

        pub struct Tree {
            job: OwnedHandle,
        }

        fn os_error(e: windows::core::Error) -> io::Error {
            io::Error::from_raw_os_error(e.code().0 & 0xFFFF)
        }

        impl Tree {
            pub fn adopt(child: &Child) -> io::Result<Self> {
                let process = child.as_raw_handle();
                // SAFETY: no security attributes and no name; the handle is owned from here on.
                let raw = unsafe { CreateJobObjectW(None, PCWSTR::null()) }.map_err(os_error)?;
                // SAFETY: a valid handle just returned and closed by nothing else.
                let job = unsafe { OwnedHandle::from_raw_handle(raw.0) };
                let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
                limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                // SAFETY: the pointer and size describe `limits`, which outlives the call.
                unsafe {
                    SetInformationJobObject(
                        HANDLE(job.as_raw_handle()),
                        JobObjectExtendedLimitInformation,
                        (&raw const limits).cast::<c_void>(),
                        size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                    )
                }
                .map_err(os_error)?;
                // SAFETY: both handles are open for the call; the child's is held by `child`.
                unsafe { AssignProcessToJobObject(HANDLE(job.as_raw_handle()), HANDLE(process)) }
                    .map_err(os_error)?;
                Ok(Self { job })
            }

            pub fn kill(&self) {
                // SAFETY: the job handle is open while `self` lives.
                _ = unsafe { TerminateJobObject(HANDLE(self.job.as_raw_handle()), 1) };
            }
        }
    }

    #[cfg(unix)]
    #[allow(unsafe_code)]
    mod imp {
        use std::io;
        use std::process::Child;

        pub struct Tree {
            group: libc::pid_t,
        }

        impl Tree {
            pub fn adopt(child: &Child) -> io::Result<Self> {
                let id = child.id();
                let group = libc::pid_t::try_from(id)
                    .map_err(|_| io::Error::other("process id out of range"))?;
                Ok(Self { group })
            }

            pub fn kill(&self) {
                // SAFETY: kill(2) with a negative pid signals the group; no memory is involved.
                _ = unsafe { libc::kill(-self.group, libc::SIGKILL) };
            }
        }

        impl Drop for Tree {
            fn drop(&mut self) {
                self.kill();
            }
        }
    }

    #[cfg(not(any(windows, unix)))]
    mod imp {
        use std::io;
        use std::process::Child;

        pub struct Tree;

        impl Tree {
            pub fn adopt(_child: &Child) -> io::Result<Self> {
                Ok(Self)
            }

            pub fn kill(&self) {}
        }
    }

    pub use imp::Tree;
}

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::envfile::parse_env;
use crate::error::HostError;

pub type Printer = Arc<dyn Fn(&str, &str) + Send + Sync>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ready {
    Http { port: u16, path: String },
    Tcp { host: String, port: u16 },
}

impl Ready {
    #[must_use]
    pub fn http(port: u16, path: impl Into<String>) -> Self {
        Self::Http {
            port,
            path: path.into(),
        }
    }

    #[must_use]
    pub fn tcp(host: impl Into<String>, port: u16) -> Self {
        Self::Tcp {
            host: host.into(),
            port,
        }
    }

    #[must_use]
    pub fn port(&self) -> u16 {
        match self {
            Self::Http { port, .. } | Self::Tcp { port, .. } => *port,
        }
    }

    #[must_use]
    pub fn host(&self) -> &str {
        match self {
            Self::Http { .. } => "127.0.0.1",
            Self::Tcp { host, .. } => host.as_str(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceSpec {
    pub name: String,
    pub cwd: PathBuf,
    pub command: PathBuf,
    pub args: Vec<String>,
    pub env_file: Option<PathBuf>,
    pub extra_env: Vec<(String, String)>,
    pub ready: Option<Ready>,
}

pub trait EnvPair {
    fn into_pair(self) -> (String, String);
}

impl<K: AsRef<str>, V: AsRef<str>> EnvPair for (K, V) {
    fn into_pair(self) -> (String, String) {
        (self.0.as_ref().to_owned(), self.1.as_ref().to_owned())
    }
}

impl<K: AsRef<str>, V: AsRef<str>> EnvPair for &(K, V) {
    fn into_pair(self) -> (String, String) {
        (self.0.as_ref().to_owned(), self.1.as_ref().to_owned())
    }
}

fn upsert_env(vars: &mut Vec<(String, String)>, key: String, val: String) {
    let windows = cfg!(windows);
    if let Some((_, existing)) = vars.iter_mut().find(|(k, _)| {
        if windows {
            k.eq_ignore_ascii_case(&key)
        } else {
            *k == key
        }
    }) {
        *existing = val;
    } else {
        vars.push((key, val));
    }
}

#[must_use]
pub fn service_environment<I1, P1, I2, P2>(
    base: I1,
    from_file: I2,
    extra: &[(String, String)],
) -> Vec<(String, String)>
where
    I1: IntoIterator<Item = P1>,
    P1: EnvPair,
    I2: IntoIterator<Item = P2>,
    P2: EnvPair,
{
    let mut out = Vec::new();
    for item in base {
        let (k, v) = item.into_pair();
        upsert_env(&mut out, k, v);
    }
    for item in from_file {
        let (k, v) = item.into_pair();
        upsert_env(&mut out, k, v);
    }
    for (k, v) in extra {
        upsert_env(&mut out, k.clone(), v.clone());
    }
    out
}

fn tcp_connect(host: &str, port: u16, timeout: Duration) -> Option<TcpStream> {
    let addr_str = format!("{host}:{port}");
    let addrs: Vec<SocketAddr> = addr_str.to_socket_addrs().ok()?.collect();
    for addr in addrs {
        if let Ok(stream) = TcpStream::connect_timeout(&addr, timeout) {
            let _ = stream.set_read_timeout(Some(timeout));
            let _ = stream.set_write_timeout(Some(timeout));
            return Some(stream);
        }
    }
    None
}

#[must_use]
pub fn answers(ready: &Ready, timeout: Duration) -> bool {
    match ready {
        Ready::Tcp { host, port } => tcp_connect(host, *port, timeout).is_some(),
        Ready::Http { port, path } => {
            let Some(mut stream) = tcp_connect("127.0.0.1", *port, timeout) else {
                return false;
            };
            let req = format!(
                "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
            );
            if stream.write_all(req.as_bytes()).is_err() {
                return false;
            }
            let mut buf = [0u8; 64];
            let Ok(n) = stream.read(&mut buf) else {
                return false;
            };
            let head = String::from_utf8_lossy(&buf[..n]);
            let Some(status_line) = head.lines().next() else {
                return false;
            };
            let mut parts = status_line.split_whitespace();
            let _http = parts.next();
            let Some(code_str) = parts.next() else {
                return false;
            };
            code_str
                .parse::<u16>()
                .is_ok_and(|code| (200..500).contains(&code))
        }
    }
}

#[must_use]
pub fn check_ready(ready: &Ready) -> bool {
    answers(ready, Duration::from_millis(100))
}

pub fn wait_for_ready(ready: &Ready, timeout: Duration, cancel: &dyn Fn() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if cancel() {
            return false;
        }
        if check_ready(ready) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    false
}

struct ManagedChild {
    name: String,
    child: Child,
    tree: Option<tree::Tree>,
}

pub struct Supervisor {
    log_dir: PathBuf,
    printer: Printer,
    cancel: Arc<AtomicBool>,
    stopping: Arc<AtomicBool>,
    children: Arc<Mutex<Vec<ManagedChild>>>,
}

impl Supervisor {
    #[must_use]
    pub fn new(log_dir: impl Into<PathBuf>, printer: Printer) -> Self {
        Self {
            log_dir: log_dir.into(),
            printer,
            cancel: Arc::new(AtomicBool::new(false)),
            stopping: Arc::new(AtomicBool::new(false)),
            children: Arc::new(Mutex::new(Vec::new())),
        }
    }

    #[must_use]
    pub fn with_cancel(mut self, cancel: Arc<AtomicBool>) -> Self {
        self.cancel = cancel;
        self
    }

    #[must_use]
    pub fn log_dir(&self) -> &Path {
        &self.log_dir
    }

    fn spawn_one(
        log_dir: &Path,
        printer: &Printer,
        children: &Arc<Mutex<Vec<ManagedChild>>>,
        spec: &ServiceSpec,
    ) -> Result<(Arc<Mutex<std::fs::File>>, u32), HostError> {
        std::fs::create_dir_all(log_dir).map_err(|e| HostError::io(log_dir, e))?;
        let log_path = log_dir.join(format!("{}.log", spec.name));
        let log_file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .map_err(|e| HostError::io(&log_path, e))?;
        let log_shared = Arc::new(Mutex::new(log_file));

        let file_env = match &spec.env_file {
            Some(path) => {
                let text = std::fs::read_to_string(path).map_err(|e| HostError::io(path, e))?;
                parse_env(&text)
            }
            None => Default::default(),
        };
        let env_pairs = service_environment(std::env::vars(), file_env.iter(), &spec.extra_env);

        let mut cmd = Command::new(&spec.command);
        cmd.args(&spec.args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if !spec.cwd.as_os_str().is_empty() {
            cmd.current_dir(&spec.cwd);
        }
        cmd.env_clear();
        for (k, v) in env_pairs {
            cmd.env(k, v);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }

        let mut child = cmd.spawn().map_err(|e| {
            HostError::message(format!(
                "could not start {} ({}): {e}",
                spec.name,
                spec.command.display()
            ))
        })?;
        let pid = child.id();
        let adopted_tree = tree::Tree::adopt(&child).ok();

        let pipe_stream = |stream: Box<dyn Read + Send>,
                           name: String,
                           printer: Printer,
                           log: Arc<Mutex<std::fs::File>>| {
            std::thread::spawn(move || {
                let reader = BufReader::new(stream);
                for line in reader.lines().map_while(Result::ok) {
                    if let Ok(mut f) = log.lock() {
                        let _ = writeln!(f, "{line}");
                    }
                    printer(&name, &line);
                }
            });
        };

        if let Some(stdout) = child.stdout.take() {
            pipe_stream(
                Box::new(stdout),
                spec.name.clone(),
                Arc::clone(printer),
                Arc::clone(&log_shared),
            );
        }
        if let Some(stderr) = child.stderr.take() {
            pipe_stream(
                Box::new(stderr),
                spec.name.clone(),
                Arc::clone(printer),
                Arc::clone(&log_shared),
            );
        }

        if let Ok(mut guard) = children.lock() {
            guard.retain(|c| c.name != spec.name);
            guard.push(ManagedChild {
                name: spec.name.clone(),
                child,
                tree: adopted_tree,
            });
        }

        Ok((log_shared, pid))
    }

    pub fn start(&self, spec: ServiceSpec, ready_timeout: Duration) -> Result<bool, HostError> {
        let (log_shared, _pid) = Self::spawn_one(&self.log_dir, &self.printer, &self.children, &spec)?;

        let ready_ok = if let Some(ready) = &spec.ready {
            let deadline = Instant::now() + ready_timeout;
            let mut ok = false;
            while Instant::now() < deadline {
                if self.cancel.load(Ordering::SeqCst) || self.stopping.load(Ordering::SeqCst) {
                    break;
                }
                if check_ready(ready) {
                    (self.printer)(
                        &spec.name,
                        &format!("ready on {}:{}", ready.host(), ready.port()),
                    );
                    ok = true;
                    break;
                }
                if let Ok(mut guard) = self.children.lock()
                    && let Some(mc) = guard.iter_mut().find(|c| c.name == spec.name)
                    && let Ok(Some(_)) = mc.child.try_wait()
                {
                    break;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            ok
        } else {
            true
        };

        if ready_ok {
            let children = Arc::clone(&self.children);
            let cancel = Arc::clone(&self.cancel);
            let stopping = Arc::clone(&self.stopping);
            let printer = Arc::clone(&self.printer);
            let log_dir = self.log_dir.clone();
            let mut log_file = log_shared;
            std::thread::spawn(move || {
                loop {
                    if cancel.load(Ordering::SeqCst) || stopping.load(Ordering::SeqCst) {
                        return;
                    }
                    let exited = {
                        let Ok(mut guard) = children.lock() else {
                            return;
                        };
                        let Some(mc) = guard.iter_mut().find(|c| c.name == spec.name) else {
                            return;
                        };
                        mc.child.try_wait().ok().flatten()
                    };
                    if let Some(status) = exited {
                        if cancel.load(Ordering::SeqCst) || stopping.load(Ordering::SeqCst) {
                            return;
                        }
                        let code_str = status
                            .code()
                            .map_or_else(|| "signal".to_owned(), |c| c.to_string());
                        if let Ok(mut f) = log_file.lock() {
                            let _ = writeln!(f, "{} exited ({code_str})", spec.name);
                        }
                        printer(
                            &spec.name,
                            &format!("exited ({code_str}); restarting in 4 s"),
                        );
                        let wait_until = Instant::now() + Duration::from_secs(4);
                        while Instant::now() < wait_until {
                            if cancel.load(Ordering::SeqCst) || stopping.load(Ordering::SeqCst) {
                                return;
                            }
                            std::thread::sleep(Duration::from_millis(100));
                        }
                        if cancel.load(Ordering::SeqCst) || stopping.load(Ordering::SeqCst) {
                            return;
                        }
                        if let Ok((new_log, _)) =
                            Self::spawn_one(&log_dir, &printer, &children, &spec)
                        {
                            log_file = new_log;
                        } else {
                            return;
                        }
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            });
        }

        Ok(ready_ok)
    }

    pub fn any_stopped(&self) -> Option<(String, Option<i32>)> {
        let mut guard = self.children.lock().ok()?;
        for mc in guard.iter_mut() {
            if let Ok(Some(status)) = mc.child.try_wait() {
                return Some((mc.name.clone(), status.code()));
            }
        }
        None
    }

    #[must_use]
    pub fn all_stopped(&self) -> bool {
        let Ok(mut guard) = self.children.lock() else {
            return true;
        };
        guard
            .iter_mut()
            .all(|mc| matches!(mc.child.try_wait(), Ok(Some(_))))
    }

    pub fn stop_all(&self) {
        self.stopping.store(true, Ordering::SeqCst);
        let Ok(mut guard) = self.children.lock() else {
            return;
        };
        for mut mc in guard.drain(..) {
            if let Some(t) = &mc.tree {
                t.kill();
            }
            let _ = mc.child.kill();
            let _ = mc.child.wait();
        }
    }
}

impl Drop for Supervisor {
    fn drop(&mut self) {
        self.stop_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_environment_merges_and_overrides_in_order() {
        let base = vec![("A".to_string(), "1".to_string()), ("B".to_string(), "2".to_string())];
        let file = [("B".to_string(), "20".to_string()), ("C".to_string(), "30".to_string())];
        let extra = vec![("C".to_string(), "300".to_string())];
        let merged = service_environment(base, file.iter(), &extra);
        assert_eq!(
            merged,
            vec![
                ("A".to_string(), "1".to_string()),
                ("B".to_string(), "20".to_string()),
                ("C".to_string(), "300".to_string()),
            ]
        );
    }

    #[test]
    fn ready_getters_return_expected_host_and_port() {
        let http = Ready::http(61000, "/status");
        assert_eq!(http.port(), 61000);
        assert_eq!(http.host(), "127.0.0.1");
        let tcp = Ready::tcp("100.101.102.103", 443);
        assert_eq!(tcp.port(), 443);
        assert_eq!(tcp.host(), "100.101.102.103");
    }
}
