// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The platform layer (spec §5.7, §12.5): the only unsafe code, one module per platform, as module-api's
//! `tree.rs` does it.

#[cfg(windows)]
#[allow(unsafe_code)] // Win32 process creation inside a job, the exit poll, the pid check and the memory query
mod imp {
    use std::ffi::c_void;
    use std::io;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    use futures_util::FutureExt;
    use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
    use windows::Win32::Foundation::{HANDLE, HANDLE_FLAG_INHERIT, STILL_ACTIVE, SetHandleInformation, WAIT_OBJECT_0};
    use windows::Win32::System::JobObjects::{
        CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JobObjectExtendedLimitInformation, SetInformationJobObject, TerminateJobObject,
    };
    use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    use windows::Win32::System::Threading::{
        CREATE_UNICODE_ENVIRONMENT, CreateProcessW, DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT,
        GetExitCodeProcess, InitializeProcThreadAttributeList, LPPROC_THREAD_ATTRIBUTE_LIST, OpenProcess,
        PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROC_THREAD_ATTRIBUTE_JOB_LIST, PROCESS_INFORMATION,
        PROCESS_QUERY_LIMITED_INFORMATION, STARTF_USESHOWWINDOW, STARTF_USESTDHANDLES, STARTUPINFOEXW,
        UpdateProcThreadAttribute, WaitForSingleObject,
    };
    use windows::core::{PCWSTR, PWSTR};

    use crate::launcher::{Command, Exit, Started, TreeControl};

    const SW_HIDE: u16 = 0;

    fn os_error(e: windows::core::Error) -> io::Error {
        io::Error::from_raw_os_error(e.code().0 & 0xFFFF)
    }

    struct JobTree {
        job: OwnedHandle,
    }

    impl TreeControl for JobTree {
        fn terminate(&self) -> io::Result<()> {
            // SAFETY: the job handle is open while `self` lives.
            unsafe { TerminateJobObject(HANDLE(self.job.as_raw_handle()), 1) }.map_err(os_error)
        }

        fn kill(&self) {
            let _ = self.terminate();
        }
    }
    // Closing the job's last handle ends what is left in it (JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE), so the tree
    // also ends with the Director's process

    fn job() -> io::Result<OwnedHandle> {
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
        Ok(job)
    }

    fn inheritable(handle: &impl AsRawHandle) -> io::Result<()> {
        // SAFETY: the handle is open for the call.
        unsafe {
            SetHandleInformation(
                HANDLE(handle.as_raw_handle()),
                HANDLE_FLAG_INHERIT.0,
                HANDLE_FLAG_INHERIT,
            )
        }
        .map_err(os_error)
    }

    /// The parent's end overlapped (read without blocking a thread), the child's end a plain handle.
    fn pipe() -> io::Result<(NamedPipeServer, std::fs::File)> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let name = format!(
            r"\\.\pipe\rustic-isles-director-{}-{}-{:016x}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
            rand::random::<u64>()
        );
        let server = ServerOptions::new()
            .first_pipe_instance(true)
            .access_outbound(false)
            .reject_remote_clients(true)
            .max_instances(1)
            .create(&name)?;
        let client = std::fs::OpenOptions::new().write(true).open(&name)?;
        inheritable(&client)?;
        Ok((server, client))
    }

    /// The server end once connected (a read before that fails), copied through an in-memory pipe.
    fn connected(server: NamedPipeServer) -> crate::launcher::Pipe {
        let (reader, mut writer) = tokio::io::simplex(64 * 1024);
        tokio::spawn(async move {
            let mut server = server;
            if server.connect().await.is_ok() {
                let _ = tokio::io::copy(&mut server, &mut writer).await;
            }
        });
        Box::pin(reader)
    }

    /// `NAME=VALUE\0`... `\0`, sorted by upper-cased name as Windows expects.
    fn environment(env: &[(String, String)]) -> Vec<u16> {
        let mut pairs: Vec<&(String, String)> = env.iter().collect();
        pairs.sort_by_key(|(k, _)| k.to_uppercase());
        let mut block: Vec<u16> = Vec::new();
        for (k, v) in pairs {
            block.extend(format!("{k}={v}").encode_utf16());
            block.push(0);
        }
        if block.is_empty() {
            block.push(0);
        }
        block.push(0);
        block
    }

    pub fn spawn(command: &Command) -> io::Result<Started> {
        let mut line: Vec<u16> = crate::cmdline::command_line(&command.program, &command.args)
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let env = environment(&command.env);
        let job = job()?;
        let (stdout, stdout_child) = pipe()?;
        let (stderr, stderr_child) = pipe()?;
        let null = std::fs::OpenOptions::new().read(true).open("NUL")?;
        inheritable(&null)?;

        let mut size = 0usize;
        // SAFETY: a size query with no list; the call fails with ERROR_INSUFFICIENT_BUFFER and sets `size`.
        let _ = unsafe { InitializeProcThreadAttributeList(None, 2, None, &raw mut size) };
        let mut storage = vec![0u64; size.div_ceil(8).max(1)];
        let list = LPPROC_THREAD_ATTRIBUTE_LIST(storage.as_mut_ptr().cast::<c_void>());
        // SAFETY: `storage` holds at least `size` bytes, aligned for the list, and outlives its use below.
        unsafe { InitializeProcThreadAttributeList(Some(list), 2, None, &raw mut size) }.map_err(os_error)?;
        let handles = [
            HANDLE(null.as_raw_handle()),
            HANDLE(stdout_child.as_raw_handle()),
            HANDLE(stderr_child.as_raw_handle()),
        ];
        let jobs = [HANDLE(job.as_raw_handle())];
        // SAFETY: both arrays outlive CreateProcessW below; the sizes are theirs. Only these three handles are
        // inherited, and the child is in the job from its first instruction.
        let attributes = unsafe {
            UpdateProcThreadAttribute(
                list,
                0,
                PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                Some(handles.as_ptr().cast::<c_void>()),
                size_of_val(&handles),
                None,
                None,
            )
            .and_then(|()| {
                UpdateProcThreadAttribute(
                    list,
                    0,
                    PROC_THREAD_ATTRIBUTE_JOB_LIST as usize,
                    Some(jobs.as_ptr().cast::<c_void>()),
                    size_of_val(&jobs),
                    None,
                    None,
                )
            })
        };
        let mut info = STARTUPINFOEXW::default();
        info.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
        info.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        if command.windows_hide {
            info.StartupInfo.dwFlags |= STARTF_USESHOWWINDOW;
            info.StartupInfo.wShowWindow = SW_HIDE;
        }
        info.StartupInfo.hStdInput = handles[0];
        info.StartupInfo.hStdOutput = handles[1];
        info.StartupInfo.hStdError = handles[2];
        info.lpAttributeList = list;
        let mut process = PROCESS_INFORMATION::default();
        let created = attributes.and_then(|()| {
            // SAFETY: every pointer is to a live local: the NUL-terminated command line (writable, as the API
            // wants), the environment block, the start-up info with its attribute list, the process information.
            unsafe {
                CreateProcessW(
                    PCWSTR::null(),
                    Some(PWSTR(line.as_mut_ptr())),
                    None,
                    None,
                    true,
                    EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT,
                    Some(env.as_ptr().cast::<c_void>()),
                    PCWSTR::null(),
                    &raw const info.StartupInfo,
                    &raw mut process,
                )
            }
        });
        // SAFETY: the list was initialised above and is not used again.
        unsafe { DeleteProcThreadAttributeList(list) };
        created.map_err(os_error)?;
        // SAFETY: both handles were just returned by CreateProcessW and are owned from here on.
        let (child, thread) = unsafe {
            (
                OwnedHandle::from_raw_handle(process.hProcess.0),
                OwnedHandle::from_raw_handle(process.hThread.0),
            )
        };
        drop(thread);
        // Our copies of the child's ends go, so the pipes end when the tree's do
        drop((null, stdout_child, stderr_child));
        let child = Arc::new(child);
        let wait = async move {
            loop {
                // SAFETY: the process handle is open while `child` lives.
                let state = unsafe { WaitForSingleObject(HANDLE(child.as_raw_handle()), 0) };
                if state == WAIT_OBJECT_0 {
                    let mut code = 0u32;
                    // SAFETY: as above; `code` outlives the call.
                    let read = unsafe { GetExitCodeProcess(HANDLE(child.as_raw_handle()), &raw mut code) };
                    return Exit {
                        code: read.is_ok().then_some(code as i32),
                        signal: None,
                    };
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
        .boxed();
        Ok(Started {
            pid: Some(process.dwProcessId),
            stdout: connected(stdout),
            stderr: connected(stderr),
            wait,
            tree: Arc::new(JobTree { job }),
        })
    }

    pub fn pid_alive(pid: u32) -> bool {
        // SAFETY: a query-only handle, closed below by OwnedHandle.
        let Ok(raw) = (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }) else {
            return false;
        };
        // SAFETY: just returned and owned from here on.
        let handle = unsafe { OwnedHandle::from_raw_handle(raw.0) };
        let mut code = 0u32;
        // SAFETY: the handle is open; `code` outlives the call.
        let read = unsafe { GetExitCodeProcess(HANDLE(handle.as_raw_handle()), &raw mut code) };
        read.is_ok() && code == STILL_ACTIVE.0 as u32
    }

    pub fn free_physical_bytes() -> Option<u64> {
        let mut status = MEMORYSTATUSEX {
            dwLength: size_of::<MEMORYSTATUSEX>() as u32,
            ..MEMORYSTATUSEX::default()
        };
        // SAFETY: `status` is sized as the API requires and outlives the call.
        unsafe { GlobalMemoryStatusEx(&raw mut status) }.ok()?;
        (status.ullAvailPhys > 0).then_some(status.ullAvailPhys)
    }
}

#[cfg(unix)]
#[allow(unsafe_code)] // kill(2) on the process group, the parent-death signal and the free-pages query
mod imp {
    use std::io;
    use std::os::unix::process::ExitStatusExt;
    use std::process::Stdio;
    use std::sync::Arc;

    use futures_util::FutureExt;

    use crate::launcher::{Command, Exit, Started, TreeControl};

    /// The group the server leads (`process_group(0)` at spawn).
    struct GroupTree {
        group: libc::pid_t,
    }

    impl TreeControl for GroupTree {
        fn terminate(&self) -> io::Result<()> {
            // SAFETY: kill(2) with a negative pid signals the group; no memory is involved.
            if unsafe { libc::kill(-self.group, libc::SIGTERM) } == 0 {
                Ok(())
            } else {
                Err(io::Error::last_os_error())
            }
        }

        fn kill(&self) {
            // SAFETY: as above.
            let _ = unsafe { libc::kill(-self.group, libc::SIGKILL) };
        }
    }

    impl Drop for GroupTree {
        fn drop(&mut self) {
            self.kill();
        }
    }

    pub fn signal_name(signal: i32) -> String {
        let name = match signal {
            libc::SIGHUP => "SIGHUP",
            libc::SIGINT => "SIGINT",
            libc::SIGQUIT => "SIGQUIT",
            libc::SIGILL => "SIGILL",
            libc::SIGABRT => "SIGABRT",
            libc::SIGBUS => "SIGBUS",
            libc::SIGFPE => "SIGFPE",
            libc::SIGKILL => "SIGKILL",
            libc::SIGSEGV => "SIGSEGV",
            libc::SIGPIPE => "SIGPIPE",
            libc::SIGTERM => "SIGTERM",
            other => return format!("SIG{other}"),
        };
        name.to_owned()
    }

    pub fn spawn(command: &Command) -> io::Result<Started> {
        let mut cmd = tokio::process::Command::new(&command.program);
        cmd.args(&command.args)
            .env_clear()
            .envs(command.env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .kill_on_drop(false);
        #[cfg(target_os = "linux")]
        // SAFETY: prctl(2) is async-signal-safe and touches no memory of the parent.
        unsafe {
            cmd.pre_exec(|| {
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) == -1 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut child = cmd.spawn()?;
        let pid = child.id();
        let group = pid
            .and_then(|p| libc::pid_t::try_from(p).ok())
            .ok_or_else(|| io::Error::other("the game server exited before it could be watched"))?;
        let stdout = child.stdout.take().ok_or_else(|| io::Error::other("no stdout pipe"))?;
        let stderr = child.stderr.take().ok_or_else(|| io::Error::other("no stderr pipe"))?;
        let wait = async move {
            match child.wait().await {
                Ok(status) => Exit {
                    code: status.code(),
                    signal: status.signal().map(signal_name),
                },
                Err(_) => Exit::default(),
            }
        }
        .boxed();
        Ok(Started {
            pid,
            stdout: Box::pin(stdout),
            stderr: Box::pin(stderr),
            wait,
            tree: Arc::new(GroupTree { group }),
        })
    }

    pub fn pid_alive(pid: u32) -> bool {
        let Ok(pid) = libc::pid_t::try_from(pid) else {
            return false;
        };
        // SAFETY: signal 0 only checks the process exists; no memory is involved.
        let sent = unsafe { libc::kill(pid, 0) } == 0;
        sent || io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }

    pub fn free_physical_bytes() -> Option<u64> {
        #[cfg(target_os = "linux")]
        {
            // SAFETY: sysconf(3) reads system values; no memory is involved.
            let (pages, size) = unsafe { (libc::sysconf(libc::_SC_AVPHYS_PAGES), libc::sysconf(libc::_SC_PAGESIZE)) };
            if pages > 0 && size > 0 {
                return u64::try_from(pages).ok()?.checked_mul(u64::try_from(size).ok()?);
            }
        }
        None
    }
}

pub(crate) use imp::spawn;
pub use imp::{free_physical_bytes, pid_alive};
