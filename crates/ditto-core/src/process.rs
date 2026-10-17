//! Direct, bounded helper execution. Callbacks run synchronously and must not block.
use std::{
    io::{self, Read},
    process::{Child, Command, ExitStatus, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    thread,
    time::{Duration, Instant},
};

/// CREATE_NO_WINDOW | CREATE_SUSPENDED: ownership must precede execution.
pub const WINDOWS_HELPER_FLAGS: u32 = 0x08000000 | 0x00000004;

#[cfg(windows)]
fn resume_owned_primary_thread(pid: u32) -> io::Result<()> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, INVALID_HANDLE_VALUE},
        System::{
            Diagnostics::ToolHelp::*,
            Threading::{OpenThread, ResumeThread, THREAD_SUSPEND_RESUME},
        },
    };
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        let mut entry: THREADENTRY32 = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<THREADENTRY32>() as u32;
        let mut present = Thread32First(snapshot, &mut entry);
        let mut result = Err(io::Error::other(
            "Suspended helper primary thread not found",
        ));
        while present != 0 {
            if entry.th32OwnerProcessID == pid {
                let thread = OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID);
                if thread.is_null() {
                    result = Err(io::Error::last_os_error());
                } else {
                    let count = ResumeThread(thread);
                    result = if count == u32::MAX {
                        Err(io::Error::last_os_error())
                    } else if count != 1 {
                        Err(io::Error::other("Unexpected suspended helper thread state"))
                    } else {
                        Ok(())
                    };
                    CloseHandle(thread);
                }
                break;
            }
            present = Thread32Next(snapshot, &mut entry);
        }
        CloseHandle(snapshot);
        result
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ProcessOptions {
    pub timeout: Duration,
    pub max_output: usize,
}
impl Default for ProcessOptions {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(3600),
            max_output: 1024 * 1024,
        }
    }
}
#[derive(Debug)]
pub struct ProcessOutput {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

struct OwnedChild {
    child: Child,
    terminated: bool,
    #[cfg(windows)]
    job: windows_sys::Win32::Foundation::HANDLE,
}
impl OwnedChild {
    fn spawn(command: &mut Command) -> io::Result<Self> {
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(WINDOWS_HELPER_FLAGS);
        }
        let child = command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        #[cfg(windows)]
        {
            use std::{mem, os::windows::io::AsRawHandle, ptr};
            use windows_sys::Win32::{Foundation::CloseHandle, System::JobObjects::*};
            // Assignment failure is fail-closed: never run an unowned helper.
            let job = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
            let mut owned = Self {
                child,
                terminated: false,
                job,
            };
            if job.is_null() {
                return Err(io::Error::last_os_error());
            }
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { mem::zeroed() };
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let configured = unsafe {
                SetInformationJobObject(
                    job,
                    JobObjectExtendedLimitInformation,
                    &info as *const _ as *const _,
                    mem::size_of_val(&info) as u32,
                )
            };
            if configured == 0
                || unsafe { AssignProcessToJobObject(job, owned.child.as_raw_handle() as _) } == 0
            {
                let error = io::Error::last_os_error();
                let _ = owned.child.kill();
                let _ = owned.child.wait();
                unsafe {
                    CloseHandle(job);
                }
                owned.job = ptr::null_mut();
                return Err(error);
            }
            resume_owned_primary_thread(owned.child.id())?;
            Ok(owned)
        }
        #[cfg(not(windows))]
        {
            Ok(Self {
                child,
                terminated: false,
            })
        }
    }
    fn poll(&mut self) -> io::Result<Option<ExitStatus>> {
        #[cfg(unix)]
        unsafe {
            // Observe exit without reaping: the zombie leader keeps its process
            // group id reserved until its descendants have been terminated.
            let mut info: libc::siginfo_t = std::mem::zeroed();
            if libc::waitid(
                libc::P_PID,
                self.child.id() as libc::id_t,
                &mut info,
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            ) != 0
            {
                return Err(io::Error::last_os_error());
            }
            if info.si_pid() == 0 {
                return Ok(None);
            }
            self.terminate_tree();
        }
        self.child.try_wait()
    }
    fn terminate_tree(&mut self) {
        if self.terminated {
            return;
        }
        self.terminated = true;
        #[cfg(unix)]
        unsafe {
            libc::kill(-(self.child.id() as i32), libc::SIGKILL);
        }
        #[cfg(windows)]
        unsafe {
            if !self.job.is_null() {
                windows_sys::Win32::System::JobObjects::TerminateJobObject(self.job, 1);
            }
        }
        let _ = self.child.kill();
    }
}
impl Drop for OwnedChild {
    fn drop(&mut self) {
        self.terminate_tree();
        let _ = self.child.wait();
        #[cfg(windows)]
        unsafe {
            if !self.job.is_null() {
                windows_sys::Win32::Foundation::CloseHandle(self.job);
            }
        }
    }
}

pub fn run(
    command: &mut Command,
    cancel: &Arc<AtomicBool>,
    options: ProcessOptions,
    mut on_stdout: impl FnMut(&[u8]),
) -> io::Result<ProcessOutput> {
    if cancel.load(Ordering::Acquire) {
        return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
    }
    let started = Instant::now();
    let mut owned = OwnedChild::spawn(command)?;
    let (tx, rx) = mpsc::sync_channel(8);
    let readers = [
        (
            true,
            Box::new(owned.child.stdout.take().unwrap()) as Box<dyn Read + Send>,
        ),
        (
            false,
            Box::new(owned.child.stderr.take().unwrap()) as Box<dyn Read + Send>,
        ),
    ]
    .into_iter()
    .map(|(stdout, mut pipe)| {
        let tx = tx.clone();
        thread::spawn(move || {
            let mut bytes = [0; 8192];
            loop {
                match pipe.read(&mut bytes) {
                    Ok(0) => break,
                    Ok(n) => {
                        if tx.send(Ok((stdout, bytes[..n].to_vec()))).is_err() {
                            break;
                        }
                    }
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(error) => {
                        let _ = tx.send(Err(error));
                        break;
                    }
                }
            }
        })
    })
    .collect::<Vec<_>>();
    drop(tx);
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut status = None;
    let mut closed = false;
    let result = loop {
        if cancel.load(Ordering::Acquire) {
            break Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
        }
        if started.elapsed() >= options.timeout {
            break Err(io::Error::new(io::ErrorKind::TimedOut, "helper timeout"));
        }
        if status.is_none() {
            match owned.poll() {
                Ok(Some(exit)) => {
                    status = Some(exit);
                    owned.terminate_tree();
                }
                Ok(None) => {}
                Err(error) => break Err(error),
            }
        }
        if closed && status.is_some() {
            break Ok(());
        }
        match rx.recv_timeout(Duration::from_millis(10)) {
            Ok(Ok((is_stdout, bytes))) => {
                if stdout
                    .len()
                    .saturating_add(stderr.len())
                    .saturating_add(bytes.len())
                    > options.max_output
                {
                    break Err(io::Error::other("process output limit exceeded"));
                }
                if is_stdout {
                    on_stdout(&bytes);
                    stdout.extend(bytes);
                } else {
                    stderr.extend(bytes);
                }
            }
            Ok(Err(error)) => break Err(error),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                closed = true;
                thread::sleep(Duration::from_millis(2));
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    };
    owned.terminate_tree();
    drop(rx);
    // A malicious descendant may detach and retain a pipe. Do not let a reader
    // join bypass the deadline. Normal owned helpers close both pipes on kill.
    for reader in readers {
        if reader.is_finished() {
            let _ = reader.join();
        }
    }
    result?;
    Ok(ProcessOutput {
        status: status.unwrap(),
        stdout,
        stderr,
    })
}
