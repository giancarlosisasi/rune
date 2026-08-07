//! Running one script.
//!
//! The child gets rune's own standard input, output and error, untouched. Colors,
//! progress bars, watch-mode interfaces and interactive prompts therefore behave exactly
//! as they do without rune, because rune is not between the child and the terminal — it
//! is beside it, waiting.

pub mod bin_paths;
pub mod environment;
pub mod quote;
pub mod shell;
mod signals;
pub mod teardown;

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, ExitCode, ExitStatus};

use thiserror::Error;

use crate::environment::Descriptor;
use crate::shell::{SHELL_VARIABLE, Shell};

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
  let parent: Vec<(std::ffi::OsString, std::ffi::OsString)> = std::env::vars_os().collect();
  let lookup =
    |name: &str| parent.iter().find(|(key, _)| key == name).map(|(_, value)| value.as_os_str());

  let shell = resolve_shell(lookup(SHELL_VARIABLE), lookup("PATH"), lookup("PATHEXT"))?;
  let directory = working_directory(request)?;

  let child_environment = environment::build(
    parent.iter().cloned(),
    &Descriptor {
      script_name: request.script_name,
      root: request.root,
      package_dir: request.package_dir,
      env: request.env,
    },
  );

  // Quoting can only happen once the shell is known: the same argument needs three
  // different spellings depending on which of them is about to read the line.
  let command_line = quote::command_line(request.command, request.arguments, shell.kind);

  let mut command = shell.command(&command_line);
  command.current_dir(&directory).env_clear().envs(child_environment.iter());

  let (mut child, _teardown) = signals::spawn(&mut command, attach).map_err(|error| {
    let shell = shell.program.display().to_string();
    if error.kind() == io::ErrorKind::NotFound {
      ExecError::ShellNotFound { shell }
    } else {
      ExecError::ShellNotRunnable { shell, source: error }
    }
  })?;

  let status = wait(&mut child, request.script_name);
  signals::forget();

  Ok(Completion { code: code_of(status?), caught_signal: signals::caught() })
}

fn wait(child: &mut Child, script: &str) -> Result<ExitStatus, ExecError> {
  child.wait().map_err(|source| ExecError::Wait { script: script.to_owned(), source })
}

fn resolve_shell(
  configured: Option<&std::ffi::OsStr>,
  path: Option<&std::ffi::OsStr>,
  path_extensions: Option<&std::ffi::OsStr>,
) -> Result<Shell, ExecError> {
  let shell = Shell::select(configured);
  let Some(program) = shell::locate(&shell.program, path, path_extensions) else {
    return Err(ExecError::ShellNotFound { shell: shell.program.display().to_string() });
  };

  Ok(Shell { program, kind: shell.kind })
}

/// Checked before spawning, because a missing directory otherwise surfaces as the same
/// "not found" the operating system reports for a missing shell.
fn working_directory(request: &ExecRequest<'_>) -> Result<PathBuf, ExecError> {
  let directory = match request.cwd {
    Some(cwd) if cwd.is_absolute() => cwd.to_path_buf(),
    Some(cwd) => request.package_dir.join(cwd),
    None => request.package_dir.to_path_buf(),
  };

  if directory.is_dir() {
    return Ok(directory);
  }

  Err(ExecError::WorkingDirectory {
    script: request.script_name.to_owned(),
    directory,
    source: io::Error::from(io::ErrorKind::NotFound),
  })
}

/// Binds the platform teardown handle to a child that has just been spawned.
///
/// The returned value is held for the lifetime of the run: on Windows it is the job
/// handle whose closing kills the tree.
#[cfg(windows)]
fn attach(child: &Child) -> io::Result<Option<teardown::JobObject>> {
  let job = teardown::JobObject::new()?;
  job.adopt(child)?;
  Ok(Some(job))
}

#[cfg(unix)]
fn attach(_child: &Child) -> io::Result<Option<()>> {
  // An interactive child stays in rune's process group, so the terminal's own signal
  // delivery reaches it and reading the terminal cannot stop it. Teardown for piped and
  // kill-capable children is built in `teardown` and used by parallel runs.
  Ok(None)
}

#[cfg(unix)]
fn code_of(status: ExitStatus) -> u32 {
  use std::os::unix::process::ExitStatusExt as _;

  status.code().map_or_else(
    || status.signal().map_or(1, |signal| 128_u32.saturating_add(signal.unsigned_abs())),
    |code| code.cast_unsigned() & 0xff,
  )
}

#[cfg(windows)]
fn code_of(status: ExitStatus) -> u32 {
  status.code().unwrap_or(1).cast_unsigned()
}
