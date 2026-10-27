//! The real launcher: a game server as a child process leading its own process tree, as the host
//! kit's services do (`services/tree.rs` there).
//!
//! - Windows: the process is created hidden and suspended (`windowsHide`, STARTF_USESHOWWINDOW with
//!   SW_HIDE), put in a job object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, then resumed, so nothing
//!   it starts is ever outside the job and the tree also ends when the Director's process does, a hard
//!   kill included. Stop is `TerminateJobObject`.
//! - Linux and macOS: a new process group the server leads; stop is SIGTERM to the group, then SIGKILL.
//!   On Linux the server also gets SIGTERM if the thread that started it goes away.

use std::io;

use crate::process::{Command, Exit, Launcher, Started};

/// Starts real processes.
#[derive(Debug, Clone, Copy, Default)]
pub struct OsLauncher;

impl Launcher for OsLauncher {
    fn start(&self, command: &Command) -> io::Result<Started> {
        imp::start(command)
    }
}

#[cfg(unix)]
#[allow(unsafe_code)] // kill(2) on the process group, prctl(2)
mod imp {
    use std::io;
    use std::process::Stdio;
    use std::sync::Arc;

    use futures_util::FutureExt;

    use super::{Command, Exit, Started};
    use crate::process::TreeControl;

    struct Group(libc::pid_t);

    impl Group {
        fn signal(&self, signal: libc::c_int) {
            // SAFETY: kill(2) with a negative pid signals the group; no memory is involved. The group may be
            // gone already (ESRCH), which is the point of stopping it.
            unsafe {
                libc::kill(-self.0, signal);
            }
        }
    }

    // What the server left behind goes with the last handle to the tree, as the Windows job does
    impl Drop for Group {
        fn drop(&mut self) {
            self.signal(libc::SIGKILL);
        }
    }

    impl TreeControl for Group {
        fn terminate(&self) {
            self.signal(libc::SIGTERM);
        }

        fn kill(&self) {
            self.signal(libc::SIGKILL);
        }
    }

    pub fn start(command: &Command) -> io::Result<Started> {
        let mut process = tokio::process::Command::new(&command.program);
        process
            .args(&command.args)
            .env_clear()
            .envs(command.env.iter().map(|(k, v)| (k, v)))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0);
        #[cfg(target_os = "linux")]
        // SAFETY: prctl is async-signal-safe and touches only the new process.
        unsafe {
            process.pre_exec(|| {
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) == -1 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut child = process.spawn()?;
        let pid = child.id();
        let stdout = child.stdout.take().ok_or_else(|| io::Error::other("no stdout pipe"))?;
        let stderr = child.stderr.take().ok_or_else(|| io::Error::other("no stderr pipe"))?;
        let group = pid
            .and_then(|p| libc::pid_t::try_from(p).ok())
            .ok_or_else(|| io::Error::other("no process id"))?;
        Ok(Started {
            pid,
            stdout: Box::new(stdout),
            stderr: Box::new(stderr),
            wait: async move {
                match child.wait().await {
                    Ok(status) => exit_of(status),
                    Err(_) => Exit::unknown(),
                }
            }
            .boxed(),
            tree: Arc::new(Group(group)),
        })
    }

    fn exit_of(status: std::process::ExitStatus) -> Exit {
        use std::os::unix::process::ExitStatusExt;
        Exit {
            code: status.code(),
            signal: status.signal().map(signal_name),
        }
    }

    fn signal_name(signal: i32) -> String {
        match signal {
            libc::SIGHUP => "SIGHUP".into(),
            libc::SIGINT => "SIGINT".into(),
            libc::SIGQUIT => "SIGQUIT".into(),
            libc::SIGILL => "SIGILL".into(),
            libc::SIGABRT => "SIGABRT".into(),
            libc::SIGBUS => "SIGBUS".into(),
            libc::SIGFPE => "SIGFPE".into(),
            libc::SIGKILL => "SIGKILL".into(),
            libc::SIGSEGV => "SIGSEGV".into(),
            libc::SIGPIPE => "SIGPIPE".into(),
            libc::SIGTERM => "SIGTERM".into(),
            other => format!("SIG{other}"),
        }
    }
}

#[cfg(windows)]
mod imp {
    use futures_util::FutureExt;
    use std::io;

    use super::{Command, Exit, Started};

    pub fn start(command: &Command) -> io::Result<Started> {
        let child = crate::win_spawn::spawn(&command.program, &command.args, &command.env, command.windows_hide)?;
        let (waiter, tree) = (child.process.clone(), child.process);
        Ok(Started {
            pid: Some(child.pid),
            stdout: Box::new(child.stdout),
            stderr: Box::new(child.stderr),
            wait: async move {
                let code = loop {
                    if let Some(code) = waiter.exit_code() {
                        break code;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                };
                Exit {
                    // The exit code is a u32 (NTSTATUS for a crash); Node reports it as the signed value libuv gives
                    code: Some(code as i32),
                    signal: None,
                }
            }
            .boxed(),
            tree,
        })
    }
}
