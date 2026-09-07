//! Own subprocess lifetime, including cancellation by dropping the run future.
//!
//! Signalling a process tree is only safe while the leader is still unreaped:
//! a zombie keeps its PID, and therefore its group id, reserved. Once
//! `wait()` collects it the operating system may hand that id to an unrelated
//! process, so the caller kills the tree first and disarms afterwards.
use anyhow::Result;
use tokio::process::{Child, Command};

pub(crate) fn prepare(command: &mut Command) {
    // Applied between fork and exec, so the leader is already the group leader
    // before it can run any code: there is no window for an escaping child.
    #[cfg(unix)]
    command.process_group(0);
    #[cfg(not(unix))]
    let _ = command;
}

#[cfg(unix)]
pub(crate) struct ProcessTree {
    group: i32,
    armed: bool,
}

#[cfg(unix)]
impl ProcessTree {
    pub(crate) fn attach(child: &Child) -> Result<Self> {
        let pid = child
            .id()
            .ok_or_else(|| anyhow::anyhow!("Missing child process ID"))?;
        Ok(Self {
            group: i32::try_from(pid)?,
            armed: true,
        })
    }

    /// Kill the whole tree. Must be called before the leader is reaped.
    pub(crate) fn kill(&self) {
        if !self.armed {
            return;
        }
        // SAFETY: prepare() created a dedicated group with this positive child
        // PID. A negative kill target signals only that worker's process group.
        unsafe {
            libc::kill(-self.group, libc::SIGKILL);
        }
    }

    /// Give up the right to signal, because the leader has now been reaped.
    pub(crate) fn disarm(&mut self) {
        self.armed = false;
    }
}

#[cfg(unix)]
impl Drop for ProcessTree {
    fn drop(&mut self) {
        // Reached when the run future is dropped mid-flight, where the leader
        // is necessarily still unreaped.
        self.kill();
    }
}

#[cfg(windows)]
pub(crate) struct ProcessTree {
    job: std::os::windows::io::OwnedHandle,
    armed: bool,
}

#[cfg(windows)]
impl ProcessTree {
    /// The job is assigned after `CreateProcess` returns, because resuming a
    /// `CREATE_SUSPENDED` process needs its primary thread handle and neither
    /// `std` nor `tokio` exposes one; recovering it by enumerating system
    /// threads would be less trustworthy than the window it closes. The window
    /// is instead narrowed by ordering and magnitude: the job is assigned as
    /// soon as `CreateProcess` returns and before the prompt is written, while
    /// Codex's own configuration and authentication startup runs first, so its
    /// earliest possible child creation is far later than the assignment.
    pub(crate) fn attach(child: &Child) -> Result<Self> {
        use std::os::windows::io::FromRawHandle;
        use windows_sys::Win32::System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
            SetInformationJobObject,
        };
        let process = child
            .raw_handle()
            .ok_or_else(|| anyhow::anyhow!("Missing child handle"))?;
        // SAFETY: null attributes/name create a private job. All FFI buffers
        // have the documented layout and lifetime; OwnedHandle closes once.
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job.is_null() {
                return Err(std::io::Error::last_os_error().into());
            }
            let owner = std::os::windows::io::OwnedHandle::from_raw_handle(job);
            let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                std::ptr::from_ref(&limits).cast(),
                u32::try_from(std::mem::size_of_val(&limits))?,
            ) == 0
                || AssignProcessToJobObject(job, process) == 0
            {
                return Err(std::io::Error::last_os_error().into());
            }
            Ok(Self {
                job: owner,
                armed: true,
            })
        }
    }

    /// A job object names its members directly, so unlike a Unix group id it
    /// can never be aimed at a recycled process; the arming flag exists so both
    /// platforms expose one lifecycle.
    pub(crate) fn kill(&self) {
        use std::os::windows::io::AsRawHandle;
        if !self.armed {
            return;
        }
        // SAFETY: this job is owned and remains valid until OwnedHandle drops.
        unsafe {
            windows_sys::Win32::System::JobObjects::TerminateJobObject(self.job.as_raw_handle(), 1);
        }
    }

    /// Suppresses the explicit terminate only: `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`
    /// still reaps stragglers when the handle closes, which stays correct here
    /// precisely because job membership cannot outlive the processes it names.
    pub(crate) fn disarm(&mut self) {
        self.armed = false;
    }
}

#[cfg(windows)]
impl Drop for ProcessTree {
    fn drop(&mut self) {
        self.kill();
    }
}
