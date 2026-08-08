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
//! Every entry point here answers rather than fails when its target has gone, which is
//! what keeps "a sibling exited a moment before we reached it" out of the error paths.

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

  use crate::lifecycle::{Kill, KillSignal};

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

  /// One child's whole process tree, and the one way to end it.
  ///
  /// A child rune placed in a process group of its own is torn down as that group, which
  /// is what reaches the grandchildren. An interactive child stayed in rune's group, so
  /// signalling the group would signal rune too — it is signalled alone, and that
  /// narrower reach is the documented cost of keeping the terminal.
  #[derive(Debug, Clone)]
  pub struct Tree {
    leader: u32,
    own_group: bool,
    kill: Kill,
  }

  impl Tree {
    pub fn own_group(leader: u32, kill: Kill) -> Self {
      Self { leader, own_group: true, kill }
    }

    pub fn in_runes_group(leader: u32, kill: Kill) -> Self {
      Self { leader, own_group: false, kill }
    }

    /// The process this tree is named by.
    pub fn leader(&self) -> u32 {
      self.leader
    }

    /// How this tree is ended: what it is asked with, and how long the asking lasts.
    pub fn policy(&self) -> Kill {
      self.kill
    }

    /// Asks the tree to end with the signal its script chose, which a child may catch and
    /// act on.
    pub fn terminate(&self) -> io::Result<()> {
      self.deliver(signal_of(self.kill.signal))
    }

    /// Ends the tree whatever it wanted. Nothing can catch `SIGKILL`.
    pub fn kill(&self) -> io::Result<()> {
      self.deliver(Signal::SIGKILL)
    }

    /// Whether anything of the tree is left.
    pub fn is_running(&self) -> bool {
      let Ok(raw) = i32::try_from(self.leader) else {
        return false;
      };

      let asked = if self.own_group {
        signal::killpg(Pid::from_raw(raw), None)
      } else {
        signal::kill(Pid::from_raw(raw), None)
      };

      !matches!(asked, Err(Errno::ESRCH))
    }

    fn deliver(&self, signal: Signal) -> io::Result<()> {
      if self.own_group {
        signal_group(self.leader, signal)
      } else {
        signal_process(self.leader, signal)
      }
    }
  }

  /// The platform signal a configured name stands for.
  fn signal_of(chosen: KillSignal) -> Signal {
    match chosen {
      KillSignal::Hup => Signal::SIGHUP,
      KillSignal::Int => Signal::SIGINT,
      KillSignal::Quit => Signal::SIGQUIT,
      KillSignal::Term => Signal::SIGTERM,
      KillSignal::Kill => Signal::SIGKILL,
    }
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
  use std::os::windows::io::RawHandle;
  use std::rc::Rc;

  use windows_sys::Win32::Foundation::{CloseHandle, FALSE, HANDLE};
  use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject,
  };
  use windows_sys::Win32::System::Threading::{
    GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
  };

  use crate::lifecycle::Kill;

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
    pub fn adopt(&self, process: RawHandle) -> io::Result<()> {
      let assigned = unsafe { AssignProcessToJobObject(self.handle, process.cast()) };

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

  /// What a terminated tree reports as its exit code.
  ///
  /// One is the ordinary "this failed" byte. A job termination has no code of its own to
  /// propagate, and inventing a wide one here would be indistinguishable from a real
  /// status the child chose.
  const TERMINATED: u32 = 1;

  /// One child's whole process tree, and the one way to end it.
  ///
  /// The job holds the whole tree because membership is inherited, so a grandchild is
  /// reached without ever being named. Windows has no gentle form of this: a job is
  /// terminated or it is not, which is why [`Tree::terminate`] and [`Tree::kill`] do the
  /// same thing here and the escalation timer only ever has work to do on POSIX. A script
  /// that configured a signal and a kill timeout gets the unconditional end either way,
  /// and [`Tree::policy`] says so rather than starting a timer with nothing to wait for.
  #[derive(Debug, Clone)]
  pub struct Tree {
    job: Rc<JobObject>,
    leader: u32,
  }

  impl Tree {
    pub fn new(job: Rc<JobObject>, leader: u32) -> Self {
      Self { job, leader }
    }

    /// The process this tree is named by.
    pub fn leader(&self) -> u32 {
      self.leader
    }

    #[expect(
      clippy::unused_self,
      reason = "the shape is the platform interface; a job has no gentler form to configure"
    )]
    pub fn policy(&self) -> Kill {
      Kill::UNCONDITIONAL
    }

    pub fn terminate(&self) -> io::Result<()> {
      self.kill()
    }

    pub fn kill(&self) -> io::Result<()> {
      self.job.terminate(TERMINATED)
    }

    pub fn is_running(&self) -> bool {
      is_running(self.leader)
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
  use super::{Tree, is_running};

  /// A process id no run will ever hand out, for asking what happens to something that
  /// has already gone.
  const GONE: u32 = u32::MAX - 1;

  #[test]
  fn this_process_is_running() {
    assert!(is_running(std::process::id()));
  }

  /// The `canKill` guard exists because this question gets asked about processes that
  /// have already gone. It must answer, not fail.
  #[test]
  fn an_impossible_process_id_is_not_running() {
    assert!(!is_running(GONE));
  }

  /// Test 5c.5, the branch with no clock in it.
  ///
  /// Every teardown asks this of a member that may have exited a moment earlier, on every
  /// path out of a run. Reporting a failure for it would turn an ordinary race into an
  /// error the user is shown.
  #[test]
  fn ending_a_tree_that_has_already_gone_is_not_an_error() {
    for tree in gone_trees() {
      assert!(tree.terminate().is_ok(), "asking a departed tree to end must not fail");
      assert!(tree.kill().is_ok(), "ending a departed tree must not fail");
      assert!(!tree.is_running());
    }
  }

  #[cfg(unix)]
  fn gone_trees() -> Vec<Tree> {
    use crate::lifecycle::Kill;

    vec![Tree::own_group(GONE, Kill::default()), Tree::in_runes_group(GONE, Kill::default())]
  }

  #[cfg(windows)]
  fn gone_trees() -> Vec<Tree> {
    use std::rc::Rc;

    use super::JobObject;

    // An empty job stands for the same thing a departed process group does: there is
    // nothing left in it to end.
    let job = Rc::new(JobObject::new().expect("create a job object"));

    vec![Tree::new(job, GONE)]
  }
}
