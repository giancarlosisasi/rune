//! Running what a script name stands for.
//!
//! A single script gets rune's own standard input, output and error, untouched. Colors,
//! progress bars, watch-mode interfaces and interactive prompts therefore behave exactly
//! as they do without rune, because rune is not between the child and the terminal — it
//! is beside it, waiting.
//!
//! A group of scripts running at once cannot have that, because two children writing to
//! one terminal produce output nobody can attribute. Those children are piped and their
//! bytes are labelled on the way through. Which of the two a child gets is decided in
//! exactly one place — [`spawn::Wiring`] — and never guessed at again.
//!
//! Internally this is an async runtime waiting on many children and many pipes at once.
//! Nothing about that reaches a caller: every entry point here is an ordinary blocking
//! function that answers with a [`Completion`].

pub mod bin_paths;
pub mod environment;
mod group;
pub mod quote;
pub mod shell;
mod signals;
mod spawn;
pub mod teardown;

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{ExitCode, ExitStatus};

use thiserror::Error;

use crate::environment::FileLayer;

pub use crate::group::{Member, SuccessPolicy, Task};

/// A script, resolved down to everything spawning it needs.
pub struct ExecRequest<'a> {
  pub script_name: &'a str,
  pub command: &'a str,
  /// Appended to `command`, in order, each quoted for the shell that will read them.
  /// The caller owns the order: whatever configuration contributes comes first, and what
  /// the user typed on the command line comes last.
  pub arguments: &'a [String],
  /// The directory holding the config that defined the script.
  pub root: &'a Path,
  /// The package the run started from. The default working directory, and the deepest
  /// `node_modules/.bin` on the child's `PATH`.
  pub package_dir: &'a Path,
  /// A relative value resolves against `package_dir`; an absolute one is used as given.
  pub cwd: Option<&'a Path>,
  pub env: &'a BTreeMap<String, String>,
  /// The script's dotenv files, nearest to the script first.
  pub env_files: &'a [FileLayer<'a>],
  /// The script owns the terminal even inside a group, and its siblings stay piped.
  ///
  /// Outside a group this changes nothing — a lone script already has the terminal. It is
  /// how a watch interface survives being one member of a `dev` group.
  pub interactive: bool,
}

#[derive(Debug, Error)]
pub enum ExecError {
  #[error("could not find the shell `{shell}`")]
  ShellNotFound { shell: String },

  #[error("could not run the shell `{shell}`: {source}")]
  ShellNotRunnable { shell: String, source: io::Error },

  #[error("cannot run `{script}` in {}: {source}", .directory.display())]
  WorkingDirectory { script: String, directory: PathBuf, source: io::Error },

  #[error("could not wait for `{script}`: {source}")]
  Wait { script: String, source: io::Error },

  #[error("could not start the runner: {source}")]
  Runtime { source: io::Error },
}

/// How a run finished.
///
/// The exit code and the interruption travel together on purpose. A caller reaching for
/// one has the other right beside it, so "was I interrupted?" cannot be silently skipped
/// the way it can when the answer lives in a global.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Completion {
  /// What rune must exit with: the child's own code, or `128 + n` when a POSIX child
  /// died from signal `n`.
  pub code: u32,
  /// The signal rune caught while the child ran, if one arrived.
  pub caught_signal: Option<i32>,
}

impl Completion {
  /// The value `main` hands back to the operating system.
  ///
  /// A POSIX status is always a byte. A Windows status uses all 32 bits — a Ctrl+C death
  /// reports `0xC000013A` — and `ExitCode` carries only a byte, so the wide ones have to
  /// leave through `process::exit`. Truncating them instead would turn "the user pressed
  /// Ctrl+C" into "the script failed with 58", which a CI job would then act on.
  pub fn exit_code(self) -> ExitCode {
    match u8::try_from(self.code) {
      Ok(code) => ExitCode::from(code),
      Err(_) => wide_exit(self.code),
    }
  }
}

#[cfg(windows)]
#[expect(clippy::exit, reason = "the only way to report a status wider than a byte")]
fn wide_exit(code: u32) -> ExitCode {
  std::process::exit(code.cast_signed())
}

#[cfg(not(windows))]
fn wide_exit(_code: u32) -> ExitCode {
  // Unreachable: a POSIX status never exceeds a byte.
  ExitCode::FAILURE
}

/// Runs `request` to completion, with the child owning the terminal throughout.
pub fn run(request: &ExecRequest<'_>) -> Result<Completion, ExecError> {
  runtime()?.block_on(group::alone(request))
}

/// Runs everything `task` stands for, however many children that turns out to be.
pub fn run_task(task: &Task<'_>) -> Result<Completion, ExecError> {
  runtime()?.block_on(group::execute(task))
}

/// One thread, and only the drivers rune uses.
///
/// Current-thread on purpose: every future here borrows the run's own data, which a work
/// stealing runtime could not accept without making all of it `'static` and `Send` first.
fn runtime() -> Result<tokio::runtime::Runtime, ExecError> {
  tokio::runtime::Builder::new_current_thread()
    .enable_all()
    .build()
    .map_err(|source| ExecError::Runtime { source })
}

#[cfg(unix)]
pub(crate) fn code_of(status: ExitStatus) -> u32 {
  use std::os::unix::process::ExitStatusExt as _;

  status.code().map_or_else(
    || status.signal().map_or(1, |signal| 128_u32.saturating_add(signal.unsigned_abs())),
    |code| code.cast_unsigned() & 0xff,
  )
}

#[cfg(windows)]
pub(crate) fn code_of(status: ExitStatus) -> u32 {
  status.code().unwrap_or(1).cast_unsigned()
}
