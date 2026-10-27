//! Windows: a game server started hidden (Node's `windowsHide`, STARTF_USESHOWWINDOW with SW_HIDE) and
//! inside a job object. The game is a GUI application and opens a window even with -nullrhi; the
//! standard library can't set the show state on this toolchain (`show_window` is unstable), so the
//! process is created here, suspended, put in its job and only then resumed: nothing it starts can be
//! outside the job. The job has `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, so its tree ends with the last
//! handle to it, the Director's death included.
#![allow(unsafe_code)] // CreateProcessW, pipes, job objects and the process handle

use std::ffi::c_void;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::sync::Arc;

use windows::Win32::Foundation::{HANDLE, HANDLE_FLAG_INHERIT, HANDLE_FLAGS, SetHandleInformation, WAIT_OBJECT_0};
use windows::Win32::Security::SECURITY_ATTRIBUTES;
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_GENERIC_READ, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation, SetInformationJobObject,
    TerminateJobObject,
};
use windows::Win32::System::Pipes::CreatePipe;
use windows::Win32::System::Threading::{
    CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, CreateProcessW, DeleteProcThreadAttributeList,
    EXTENDED_STARTUPINFO_PRESENT, GetExitCodeProcess, INFINITE, InitializeProcThreadAttributeList,
    LPPROC_THREAD_ATTRIBUTE_LIST, PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROCESS_INFORMATION, ResumeThread,
    STARTF_USESHOWWINDOW, STARTF_USESTDHANDLES, STARTUPINFOEXW, STARTUPINFOW, TerminateProcess,
    UpdateProcThreadAttribute, WaitForSingleObject,
};
use windows::core::{PCWSTR, PWSTR};

/// The process and its job, shared by the waiter and whoever stops it.
pub struct Process {
    process: OwnedHandle,
    job: OwnedHandle,
}

// SAFETY: a process handle may be used from any thread; WaitForSingleObject and TerminateProcess
// are thread-safe on the same handle.
unsafe impl Send for Process {}
unsafe impl Sync for Process {}

impl Process {
    fn raw(&self) -> HANDLE {
        HANDLE(self.process.as_raw_handle())
    }

    /// Blocks until the process ends: its exit code.
    pub fn wait(&self) -> std::io::Result<u32> {
        // SAFETY: the handle is open (owned by self) and has SYNCHRONIZE and QUERY rights.
        unsafe {
            if WaitForSingleObject(self.raw(), INFINITE) != WAIT_OBJECT_0 {
                return Err(std::io::Error::last_os_error());
            }
            let mut code = 0u32;
            GetExitCodeProcess(self.raw(), &mut code).map_err(std::io::Error::other)?;
            Ok(code)
        }
    }

    /// Ends every process in the job with exit code 1, as libuv kills a child on Windows.
    fn end_job(&self) {
        // SAFETY: the job handle is open while `self` lives.
        let _ = unsafe { TerminateJobObject(HANDLE(self.job.as_raw_handle()), 1) };
    }
}

impl crate::process::TreeControl for Process {
    // A job object has no polite stop: terminate and kill are the same
    fn terminate(&self) {
        self.end_job();
    }

    fn kill(&self) {
        self.end_job();
    }
}

/// A new job that ends its processes when its last handle closes.
fn new_job() -> std::io::Result<OwnedHandle> {
    // SAFETY: no security attributes and no name; the handle is owned right after.
    let raw = unsafe { CreateJobObjectW(None, PCWSTR::null()) }.map_err(std::io::Error::other)?;
    // SAFETY: a valid handle just returned, closed by nothing else.
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
    .map_err(std::io::Error::other)?;
    Ok(job)
}

/// A PROC_THREAD_ATTRIBUTE_HANDLE_LIST over `handles`, which must outlive it.
struct AttributeList<'a> {
    buffer: Vec<usize>,
    _handles: &'a [HANDLE],
}

impl<'a> AttributeList<'a> {
    fn new(handles: &'a [HANDLE]) -> std::io::Result<Self> {
        let mut size = 0usize;
        // SAFETY: the first call only reports the size (it fails with ERROR_INSUFFICIENT_BUFFER).
        let _ = unsafe { InitializeProcThreadAttributeList(None, 1, None, &mut size) };
        let mut buffer = vec![0usize; size.div_ceil(std::mem::size_of::<usize>())];
        let list = LPPROC_THREAD_ATTRIBUTE_LIST(buffer.as_mut_ptr().cast());
        // SAFETY: the buffer is at least `size` bytes and aligned for pointers; `handles` lives as
        // long as the list (the lifetime), as UpdateProcThreadAttribute requires.
        unsafe {
            InitializeProcThreadAttributeList(Some(list), 1, None, &mut size).map_err(std::io::Error::other)?;
            UpdateProcThreadAttribute(
                list,
                0,
                PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                Some(handles.as_ptr().cast()),
                std::mem::size_of_val(handles),
                None,
                None,
            )
            .map_err(std::io::Error::other)?;
        }
        Ok(Self {
            buffer,
            _handles: handles,
        })
    }

    fn raw(&self) -> LPPROC_THREAD_ATTRIBUTE_LIST {
        LPPROC_THREAD_ATTRIBUTE_LIST(self.buffer.as_ptr().cast_mut().cast())
    }
}

impl Drop for AttributeList<'_> {
    fn drop(&mut self) {
        // SAFETY: the list was initialized in new() and is deleted once.
        unsafe { DeleteProcThreadAttributeList(self.raw()) };
    }
}

pub struct Child {
    pub pid: u32,
    pub process: Arc<Process>,
    pub stdout: std::fs::File,
    pub stderr: std::fs::File,
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn environment_block(env: &[(String, String)]) -> Vec<u16> {
    let mut pairs: Vec<&(String, String)> = env.iter().collect();
    pairs.sort_by_key(|(k, _)| k.to_uppercase());
    let mut block = Vec::new();
    for (k, v) in pairs {
        block.extend(format!("{k}={v}").encode_utf16());
        block.push(0);
    }
    block.push(0);
    block
}

fn pipe(inherit: &SECURITY_ATTRIBUTES) -> std::io::Result<(OwnedHandle, OwnedHandle)> {
    let (mut read, mut write) = (HANDLE::default(), HANDLE::default());
    // SAFETY: both out-pointers are valid; the handles are owned right after.
    unsafe {
        CreatePipe(&mut read, &mut write, Some(inherit), 0).map_err(std::io::Error::other)?;
        let (read, write) = (
            OwnedHandle::from_raw_handle(read.0),
            OwnedHandle::from_raw_handle(write.0),
        );
        // The parent's end must not reach the child, or the pipe never reports its end.
        SetHandleInformation(HANDLE(read.as_raw_handle()), HANDLE_FLAG_INHERIT.0, HANDLE_FLAGS(0))
            .map_err(std::io::Error::other)?;
        Ok((read, write))
    }
}

pub fn spawn(program: &str, args: &[String], env: &[(String, String)], hide: bool) -> std::io::Result<Child> {
    let inherit = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: std::ptr::null_mut(),
        bInheritHandle: true.into(),
    };
    let (stdout_read, stdout_write) = pipe(&inherit)?;
    let (stderr_read, stderr_write) = pipe(&inherit)?;
    let nul = wide("NUL");
    // SAFETY: a NUL-terminated name; the handle is owned right after.
    let stdin = unsafe {
        let handle = CreateFileW(
            PCWSTR(nul.as_ptr()),
            FILE_GENERIC_READ.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            Some(&inherit),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            None,
        )
        .map_err(std::io::Error::other)?;
        OwnedHandle::from_raw_handle(handle.0)
    };
    let mut startup = STARTUPINFOW {
        cb: std::mem::size_of::<STARTUPINFOEXW>() as u32,
        dwFlags: STARTF_USESTDHANDLES,
        hStdInput: HANDLE(stdin.as_raw_handle()),
        hStdOutput: HANDLE(stdout_write.as_raw_handle()),
        hStdError: HANDLE(stderr_write.as_raw_handle()),
        ..Default::default()
    };
    if hide {
        startup.dwFlags |= STARTF_USESHOWWINDOW;
        // SW_HIDE
        startup.wShowWindow = 0;
    }
    let job = new_job()?;
    let mut line = wide(&crate::cmdline::command_line(program, args));
    let block = environment_block(env);
    let mut info = PROCESS_INFORMATION::default();
    // Only these three reach the child. Without the list it would get every inheritable handle of
    // this process, such as the host kit's log pipe, and hold it open after the Director is gone.
    let inherited = [startup.hStdInput, startup.hStdOutput, startup.hStdError];
    let attributes = AttributeList::new(&inherited)?;
    let extended = STARTUPINFOEXW {
        StartupInfo: startup,
        lpAttributeList: attributes.raw(),
    };
    // SAFETY: every pointer is valid for the call: the command line is a mutable NUL-terminated
    // buffer, the environment a double-NUL-terminated UTF-16 block, the structures initialized, and
    // the attribute list (with the handles it names) outlives it.
    unsafe {
        CreateProcessW(
            PCWSTR::null(),
            Some(PWSTR(line.as_mut_ptr())),
            None,
            None,
            true,
            CREATE_SUSPENDED | CREATE_UNICODE_ENVIRONMENT | EXTENDED_STARTUPINFO_PRESENT,
            Some(block.as_ptr().cast::<c_void>()),
            PCWSTR::null(),
            &extended.StartupInfo,
            &mut info,
        )
        .map_err(|e| std::io::Error::from_raw_os_error(e.code().0 & 0xffff))?;
    }
    drop(attributes);
    // SAFETY: both handles are fresh and owned from here on; the thread's is closed after it runs.
    let (process, thread) = unsafe {
        (
            OwnedHandle::from_raw_handle(info.hProcess.0),
            OwnedHandle::from_raw_handle(info.hThread.0),
        )
    };
    // SAFETY: both handles are open for the call.
    let assigned = unsafe { AssignProcessToJobObject(HANDLE(job.as_raw_handle()), HANDLE(process.as_raw_handle())) };
    if let Err(error) = assigned {
        // Outside the job it could outlive the Director: never let it run
        // SAFETY: the process handle is open and has PROCESS_TERMINATE.
        let _ = unsafe { TerminateProcess(HANDLE(process.as_raw_handle()), 1) };
        return Err(std::io::Error::other(error));
    }
    // SAFETY: the thread handle is open and has THREAD_SUSPEND_RESUME (CreateProcessW's own).
    unsafe { ResumeThread(HANDLE(thread.as_raw_handle())) };
    drop(thread);
    // The child's ends close here, so the pipes report the end when the child's copies close.
    drop((stdin, stdout_write, stderr_write));
    let process = Arc::new(Process { process, job });
    Ok(Child {
        pid: info.dwProcessId,
        process,
        stdout: std::fs::File::from(stdout_read),
        stderr: std::fs::File::from(stderr_read),
    })
}
