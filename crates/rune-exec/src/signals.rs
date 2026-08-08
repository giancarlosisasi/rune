//! What rune does when a signal arrives while a script is running.
//!
//! It records the signal and keeps waiting. It never exits before its child, and it never
//! rewrites the child's answer — a script that handles the interrupt and exits 0 makes
//! rune exit 0, which is what `npm run` does and what years of muscle memory expect.
//!
//! Three disciplines hold the layer together:
//!
//! - The registry is locked across the spawn, so a signal cannot arrive in the gap
//!   between the child existing and rune knowing about it and leave a process behind.
//! - The POSIX handler does one async-signal-safe thing: write a byte down a self-pipe.
//!   Everything else happens on an ordinary thread reading the other end.
//! - A running group learns about the signal by waiting on [`interrupts`] rather than by
//!   having work done for it inside the handler. Tearing several process trees down is
//!   not something to attempt from a signal context, and a group already has a place
//!   that owns its children.

use std::io;
use std::sync::{LazyLock, Mutex, MutexGuard, Once, PoisonError};

use tokio::process::{Child, Command};
use tokio::sync::watch;

/// What rune knows about the run in progress.
#[derive(Debug, Default)]
struct Registry {
  /// Every child rune spawned and has not yet collected. A group has several at once.
  children: Vec<u32>,
  caught: Option<i32>,
}

static REGISTRY: Mutex<Registry> = Mutex::new(Registry { children: Vec::new(), caught: None });
static INSTALLED: Once = Once::new();

/// Carries the first signal to whoever is waiting on one.
///
/// A watch rather than a notification: it holds the value, so a group that subscribes
/// after the signal already arrived still sees it. An edge would simply be missed.
static INTERRUPT: LazyLock<watch::Sender<Option<i32>>> = LazyLock::new(|| watch::channel(None).0);

/// Installs the handlers before anything that depends on them starts.
pub fn arm() {
  INSTALLED.call_once(install);
}

/// A view of the signal that arrived, for a caller that has to react to one.
pub fn interrupts() -> watch::Receiver<Option<i32>> {
  arm();
  INTERRUPT.subscribe()
}

/// Spawns with the registry locked, and binds the platform teardown handle to the child
/// before anything else can observe it.
pub fn spawn<T>(
  command: &mut Command,
  attach: impl FnOnce(&Child) -> io::Result<T>,
) -> io::Result<(Child, T)> {
  arm();

  let mut registry = registry();
  let mut child = command.spawn()?;

  let attached = match attach(&child) {
    Ok(attached) => attached,
    Err(error) => {
      // A child rune cannot supervise is worse than no child at all.
      let _ = child.start_kill();
      return Err(error);
    }
  };

  if let Some(pid) = child.id() {
    registry.children.push(pid);
  }

  Ok((child, attached))
}

/// Drops one child's registration once its status is known.
pub fn forget(pid: u32) {
  registry().children.retain(|registered| *registered != pid);
}

/// The signal that arrived while the children ran, if one did.
pub fn caught() -> Option<i32> {
  registry().caught
}

fn registry() -> MutexGuard<'static, Registry> {
  // A panic elsewhere must not turn every later signal into a second failure.
  REGISTRY.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Records the signal, and forwards it when the operating system did not already.
fn record(signal: i32, forward: bool) {
  let children = {
    let mut registry = registry();
    registry.caught.get_or_insert(signal);

    if forward { registry.children.clone() } else { Vec::new() }
  };

  // Outside the lock: forwarding touches the operating system, and a group waking up on
  // the announcement below will want the registry for itself.
  for pid in children {
    let _ = platform::forward(pid, signal);
  }

  // The first signal is the one reported, matching `caught`. A second Ctrl+C while a
  // teardown is already running must not rewrite what the run is answering for.
  INTERRUPT.send_if_modified(|held| {
    if held.is_some() {
      return false;
    }

    *held = Some(signal);
    true
  });
}

#[cfg(unix)]
use unix as platform;
#[cfg(windows)]
use windows as platform;

fn install() {
  platform::install();
}

#[cfg(unix)]
mod unix {
  use std::io;
  use std::sync::atomic::{AtomicI32, Ordering};

  use nix::fcntl::{FcntlArg, FdFlag, fcntl};
  use nix::libc;
  use nix::sys::signal::{SaFlags, SigAction, SigHandler, SigSet, Signal, sigaction};
  use nix::unistd;

  use crate::teardown;

  /// The write end of the self-pipe, as a raw descriptor because that is all a signal
  /// handler is allowed to touch.
  static NOTIFY: AtomicI32 = AtomicI32::new(-1);

  /// The signals a terminal or an operator sends at a running command.
  const WATCHED: [Signal; 4] = [Signal::SIGINT, Signal::SIGTERM, Signal::SIGHUP, Signal::SIGQUIT];

  pub fn install() {
    // macOS has no `pipe2`, so close-on-exec is set on the two ends afterwards instead of
    // atomically at creation. There is no window to worry about: this runs before rune
    // has spawned anything that could inherit them.
    let Ok((read, write)) = unistd::pipe() else {
      // Without the pipe rune still runs; it just cannot tell a caller it was
      // interrupted. Failing the run over that would be worse.
      return;
    };

    for end in [&read, &write] {
      let _ = fcntl(end, FcntlArg::F_SETFD(FdFlag::FD_CLOEXEC));
    }

    // The write end lives as long as the process, because the handler may run at any
    // moment until rune exits.
    NOTIFY.store(std::os::fd::IntoRawFd::into_raw_fd(write), Ordering::Release);

    for signal in WATCHED {
      let action =
        SigAction::new(SigHandler::Handler(on_signal), SaFlags::SA_RESTART, SigSet::empty());
      // Installing handlers before any thread of rune's own exists.
      let _ = unsafe { sigaction(signal, &action) };
    }

    std::thread::spawn(move || listen(&read));
  }

  /// Async-signal-safe by construction: `errno` saved and restored around one `write`,
  /// and nothing else.
  extern "C" fn on_signal(signal: libc::c_int) {
    let saved = nix::errno::Errno::last_raw();

    let byte = [u8::try_from(signal).unwrap_or(0)];
    let descriptor = NOTIFY.load(Ordering::Acquire);
    let _ = unsafe { libc::write(descriptor, byte.as_ptr().cast(), 1) };

    // A handler that leaves `errno` changed corrupts whatever the interrupted code was
    // about to read from it.
    nix::errno::Errno::set_raw(saved);
  }

  fn listen(read: &std::os::fd::OwnedFd) {
    let mut byte = [0_u8; 1];
    loop {
      match unistd::read(read, &mut byte) {
        Ok(1) => {}
        Err(nix::errno::Errno::EINTR) => continue,
        // A short read means the write end is gone, and any other error means the pipe
        // cannot be read again. Either way there is nothing left to listen for.
        _ => return,
      }

      let signal = i32::from(byte[0]);
      // A terminal delivers SIGINT, SIGHUP and SIGQUIT to the whole foreground process
      // group, and the child is in it, so it already has them. SIGTERM is aimed at rune
      // alone and is the one that has to be passed on.
      super::record(signal, signal == libc::SIGTERM);
    }
  }

  pub fn forward(pid: u32, signal: i32) -> io::Result<()> {
    let signal = Signal::try_from(signal).map_err(io::Error::from)?;
    teardown::signal_process(pid, signal)
  }
}

#[cfg(windows)]
mod windows {
  use std::io;

  use windows_sys::Win32::Foundation::{FALSE, TRUE};
  use windows_sys::Win32::System::Console::{
    CTRL_BREAK_EVENT, CTRL_C_EVENT, SetConsoleCtrlHandler,
  };

  /// What the console handler contract calls a `BOOL`.
  type Handled = i32;

  /// The POSIX numbers, so a caller reads one vocabulary on both platforms.
  const SIGINT: i32 = 2;
  const SIGBREAK: i32 = 21;

  pub fn install() {
    unsafe { SetConsoleCtrlHandler(Some(on_console_event), TRUE) };
  }

  /// Runs on a thread the operating system injects, not in a signal context, so ordinary
  /// code is allowed here.
  ///
  /// Returning `TRUE` claims the event: rune stays alive and goes on waiting. The child
  /// is attached to the same console and received the event too, so it decides how the
  /// run ends. Close, log-off and shutdown events are left to the default handler, which
  /// is the only correct answer to "the machine is going away".
  unsafe extern "system" fn on_console_event(event: u32) -> Handled {
    let signal = match event {
      CTRL_C_EVENT => SIGINT,
      CTRL_BREAK_EVENT => SIGBREAK,
      _ => return FALSE,
    };

    super::record(signal, false);
    TRUE
  }

  /// Console control events reach every process attached to the console, so there is
  /// nothing for rune to pass on. The signature matches the POSIX one so that the caller
  /// stays one piece of code rather than two.
  #[expect(clippy::unnecessary_wraps, reason = "the shape is the platform interface")]
  pub fn forward(_pid: u32, _signal: i32) -> io::Result<()> {
    Ok(())
  }
}
