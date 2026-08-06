//! Making a process tree die, and knowing when there is nothing left to kill.
//!
//! Every command runs through a shell, so the process rune holds is the shell, not the
//! tool. Killing that process alone leaves whatever it started behind. Each platform has
//! exactly one mechanism that solves this properly, and they are not alike:
//!
//! - Windows: a Job Object set to terminate when its last handle closes. Membership is
//!   inherited, so a grandchild cannot escape by being spawned normally. Job Objects are
//!   unrelated to console control events, so this costs nothing in Ctrl+C behavior.
//! - POSIX: a process group plus `killpg`. This is deliberately *not* used for an
//!   interactive single script — a process outside the terminal's foreground group is
//!   stopped by `SIGTTIN` the moment it reads the terminal, which freezes exactly the
//!   watch modes and prompts that inherited stdio exists to preserve.
//!
//! One rule spans both: a process that has already exited is never an error to kill.

#[cfg(unix)]
pub use self::unix::*;
#[cfg(windows)]
pub use self::windows::*;

#[cfg(unix)]
mod unix {
  use std::io;
  use std::process::Command;

  use nix::errno::Errno;
  use nix::sys::signal::{self, Signal};
  use nix::unistd::Pid;

  /// Whether a process id still names a live process.
  pub fn is_running(pid: u32) -> bool {
    let Ok(raw) = i32::try_from(pid) else {
      return false;
    };

    // Signal 0 asks the question without delivering anything.
    !matches!(signal::kill(Pid::from_raw(raw), None), Err(Errno::ESRCH))
  }

  /// Signals one process. A process that has already exited counts as done, not failed.
  pub fn signal_process(pid: u32, signal: Signal) -> io::Result<()> {
    let raw = i32::try_from(pid).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    forgive_missing(signal::kill(Pid::from_raw(raw), signal))
  }

  /// Signals a whole process group, for children that were placed in one.
  pub fn signal_group(group: u32, signal: Signal) -> io::Result<()> {
    let raw = i32::try_from(group).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    forgive_missing(signal::killpg(Pid::from_raw(raw), signal))
  }

  /// Puts the child in a process group of its own, so it can be torn down as a unit.
  ///
  /// For piped and kill-capable children only. An interactive child must stay in rune's
  /// group; see the module documentation.
  pub fn into_own_process_group(command: &mut Command) {
    use std::os::unix::process::CommandExt as _;

    command.process_group(0);
  }

  fn forgive_missing(result: Result<(), Errno>) -> io::Result<()> {
    match result {
      Ok(()) | Err(Errno::ESRCH) => Ok(()),
      Err(error) => Err(io::Error::from(error)),
    }
  }
}

#[cfg(windows)]
mod windows {
  use std::io;
  use std::mem;
  use std::os::windows::io::AsRawHandle as _;
  use std::process::Child;

  use windows_sys::Win32::Foundation::{CloseHandle, FALSE, HANDLE};
  use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject,
  };
  use windows_sys::Win32::System::Threading::{
    GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
  };

  /// `GetExitCodeProcess` reports this while the process is alive.
  const STILL_ACTIVE: u32 = 259;

  /// A job that terminates every process in it once the last handle to it closes.
  ///
  /// Holding the handle for the lifetime of the run is what makes the guarantee: rune
  /// exiting for any reason, including being killed outright, takes the tree with it.
  #[derive(Debug)]
  pub struct JobObject {
    handle: HANDLE,
  }

  // The handle is owned by this value and only ever used through it.
  unsafe impl Send for JobObject {}

  impl JobObject {
    pub fn new() -> io::Result<Self> {
      let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
      if handle.is_null() {
        return Err(io::Error::last_os_error());
      }

      let job = Self { handle };
      job.limit_to_kill_on_close()?;
      Ok(job)
    }

    fn limit_to_kill_on_close(&self) -> io::Result<()> {
      let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { mem::zeroed() };
      limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;

      let size = u32::try_from(mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>())
        .map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
      let assigned = unsafe {
        SetInformationJobObject(
          self.handle,
          JobObjectExtendedLimitInformation,
          std::ptr::from_mut(&mut limits).cast(),
          size,
        )
      };

      if assigned == FALSE { Err(io::Error::last_os_error()) } else { Ok(()) }
    }

    /// Adds a freshly spawned child. Its own children join automatically.
    pub fn adopt(&self, child: &Child) -> io::Result<()> {
      let assigned = unsafe { AssignProcessToJobObject(self.handle, child.as_raw_handle().cast()) };

      if assigned == FALSE { Err(io::Error::last_os_error()) } else { Ok(()) }
    }

    /// Kills everything in the job at once.
    pub fn terminate(&self, code: u32) -> io::Result<()> {
      let terminated = unsafe { TerminateJobObject(self.handle, code) };

      if terminated == FALSE { Err(io::Error::last_os_error()) } else { Ok(()) }
    }
  }

  impl Drop for JobObject {
    fn drop(&mut self) {
      unsafe { CloseHandle(self.handle) };
    }
  }

  /// Whether a process id still names a live process.
  pub fn is_running(pid: u32) -> bool {
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid) };
    if handle.is_null() {
      return false;
    }

    let mut code = 0_u32;
    let read = unsafe { GetExitCodeProcess(handle, &raw mut code) };
    unsafe { CloseHandle(handle) };

    read != FALSE && code == STILL_ACTIVE
  }
}

#[cfg(test)]
mod tests {
  use super::is_running;

  #[test]
  fn this_process_is_running() {
    assert!(is_running(std::process::id()));
  }

  /// The `canKill` guard exists because this question gets asked about processes that
  /// have already gone. It must answer, not fail.
  #[test]
  fn an_impossible_process_id_is_not_running() {
    assert!(!is_running(u32::MAX - 1));
  }
}
