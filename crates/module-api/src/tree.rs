//! An external module's whole process tree, so an unload leaves nothing running (DD5): a job object on
//! Windows that ends every process in it when it is closed, the child's own process group elsewhere.
//! A process the child starts before it is adopted escapes on Windows; a module does nothing before its
//! hello, so the window is the time between the spawn and the next line.

#[cfg(windows)]
#[allow(unsafe_code, reason = "the Win32 job object calls are FFI")]
mod imp {
    use std::ffi::c_void;
    use std::io;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};

    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation, SetInformationJobObject,
        TerminateJobObject,
    };
    use windows::core::PCWSTR;

    pub struct Tree {
        job: OwnedHandle,
    }

    fn os_error(e: windows::core::Error) -> io::Error {
        io::Error::from_raw_os_error(e.code().0 & 0xFFFF)
    }

    impl Tree {
        /// A job that holds the child, and every process it starts from now on.
        pub fn adopt(child: &tokio::process::Child) -> io::Result<Self> {
            let process = child
                .raw_handle()
                .ok_or_else(|| io::Error::other("the module exited before it could be adopted"))?;
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
            unsafe { AssignProcessToJobObject(HANDLE(job.as_raw_handle()), HANDLE(process)) }.map_err(os_error)?;
            Ok(Self { job })
        }

        pub fn kill(&self) {
            // SAFETY: the job handle is open while `self` lives.
            _ = unsafe { TerminateJobObject(HANDLE(self.job.as_raw_handle()), 1) };
        }
    }
    // Closing the job's last handle ends what is left in it (JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE)
}

#[cfg(unix)]
#[allow(unsafe_code, reason = "kill(2) on the process group is a libc call")]
mod imp {
    use std::io;

    /// The group the child leads (`process_group(0)` at spawn).
    pub struct Tree {
        group: libc::pid_t,
    }

    impl Tree {
        pub fn adopt(child: &tokio::process::Child) -> io::Result<Self> {
            let id = child
                .id()
                .ok_or_else(|| io::Error::other("the module exited before it could be adopted"))?;
            let group = libc::pid_t::try_from(id).map_err(|_| io::Error::other("process id out of range"))?;
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

pub(crate) use imp::Tree;
