//! Turning a resolved script into a running child.
//!
//! Everything a child needs decided before it exists is decided here: which shell reads
//! the command, which directory it starts in, what its environment holds, and whether it
//! gets rune's own terminal or a pair of pipes. One script and one member of a group take
//! the same path through this module, so the two can never drift apart on quoting, on
//! `PATH`, or on which variables a child sees.

use std::io;
use std::path::PathBuf;

use rune_out::color::{COLOR_VARIABLE, ColorLevel, forced_color};
use tokio::process::{Child, Command};

use crate::environment::Descriptor;
use crate::lifecycle::Kill;
use crate::shell::{SHELL_VARIABLE, Shell};
use crate::teardown::Tree;
use crate::{ExecError, ExecRequest, environment, quote, shell, signals};

/// How a child is wired to the terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wiring {
  /// Mode A: rune's own standard input, output and error, untouched. The child owns the
  /// terminal, so colors, progress bars and prompts behave as they do without rune.
  Inherited,
  /// Mode B: pipes rune reads, so the bytes can be attributed to the script that wrote
  /// them before they reach the terminal.
  ///
  /// The level travels with the wiring because a piped child sees no terminal and turns
  /// its own color off. Telling it what rune's output supports is what keeps a tool's
  /// colors alive underneath a prefix.
  Piped(ColorLevel),
}

/// A running child, with the handle that ends its whole tree.
pub struct Spawned {
  pub child: Child,
  pub pid: u32,
  pub tree: Tree,
}

/// Starts `request`, wired as `wiring` says.
pub fn start(request: &ExecRequest<'_>, wiring: Wiring) -> Result<Spawned, ExecError> {
  let parent: Vec<(std::ffi::OsString, std::ffi::OsString)> = std::env::vars_os().collect();
  let lookup = |name: &str| environment::find(&parent, name);

  let shell = resolve_shell(lookup(SHELL_VARIABLE), lookup("PATH"), lookup("PATHEXT"))?;
  let directory = working_directory(request)?;

  let mut layering = environment::build(
    parent.iter().cloned(),
    &Descriptor {
      script_name: request.script_name,
      root: request.root,
      package_dir: request.package_dir,
      env: request.env,
      env_files: request.env_files,
    },
  );

  if let Wiring::Piped(level) = wiring {
    // The value itself is never read — a level the user already chose is theirs, whatever
    // it says, and rune only fills the gap when nobody has.
    let chosen = layering.environment.get(COLOR_VARIABLE).is_some().then_some("");
    if let Some(indicator) = forced_color(level, chosen) {
      layering.environment.set(COLOR_VARIABLE, indicator);
    }
  }

  // Before the child starts, and on stderr: an assignment that silently did nothing is
  // how a wrong environment survives a week, and stdout belongs to the child.
  for ignored in &layering.ignored {
    rune_out::diagnostic(&format!("warning: {ignored}"));
  }

  // Quoting can only happen once the shell is known: the same argument needs three
  // different spellings depending on which of them is about to read the line.
  let command_line = quote::command_line(request.command, request.arguments, shell.kind);

  // A piped child is torn down as a unit because its siblings may fail; a timeout-bearing
  // child is torn down as a unit because it promised to end within a budget. Same need,
  // same placement.
  let own_group = matches!(wiring, Wiring::Piped(_)) || request.wants_own_group();

  let mut command = Command::from(shell.command(&command_line));
  command.current_dir(&directory).env_clear().envs(layering.environment.iter());
  wire(&mut command, wiring, own_group);

  let kill = request.lifecycle.kill;
  let (child, tree) = signals::spawn(&mut command, |child| attach(child, own_group, kill))
    .map_err(|error| {
      let shell = shell.program.display().to_string();
      if error.kind() == io::ErrorKind::NotFound {
        ExecError::ShellNotFound { shell }
      } else {
        ExecError::ShellNotRunnable { shell, source: error }
      }
    })?;

  let pid = child.id().unwrap_or_default();

  Ok(Spawned { child, pid, tree })
}

#[cfg(unix)]
fn wire(command: &mut Command, wiring: Wiring, own_group: bool) {
  if matches!(wiring, Wiring::Piped(_)) {
    pipe_stdio(command);
  }

  // A process group of its own, so the whole tree can be torn down as a unit. The cost is
  // paid by whoever asked for it: a process outside the terminal's foreground group is
  // stopped by `SIGTTIN` the moment it reads the terminal, which is why a script keeping
  // the terminal never lands here.
  if own_group {
    crate::teardown::into_own_process_group(command.as_std_mut());
  }
}

#[cfg(windows)]
fn wire(command: &mut Command, wiring: Wiring, _own_group: bool) {
  // Job objects reach the whole tree whatever the stdio, so there is no placement to
  // decide here.
  if matches!(wiring, Wiring::Piped(_)) {
    pipe_stdio(command);
  }
}

/// Stdin is closed rather than shared: exactly one process may own the terminal's input,
/// and a piped member is not it. Routing input to a chosen member is a separate feature.
fn pipe_stdio(command: &mut Command) {
  use std::process::Stdio;

  command.stdout(Stdio::piped()).stderr(Stdio::piped()).stdin(Stdio::null());
}

/// Binds the platform teardown handle to a child that has just been spawned.
///
/// The job handle is held for the lifetime of the run inside the [`Tree`] it returns:
/// closing it kills the tree, so rune exiting for any reason takes the children with it.
#[cfg(windows)]
fn attach(child: &Child, _own_group: bool, _kill: Kill) -> io::Result<Tree> {
  use std::rc::Rc;

  use crate::teardown::JobObject;

  let job = Rc::new(JobObject::new()?);
  let handle = child.raw_handle().ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
  job.adopt(handle)?;

  Ok(Tree::new(job, child.id().unwrap_or_default()))
}

#[cfg(unix)]
fn attach(child: &Child, own_group: bool, kill: Kill) -> io::Result<Tree> {
  let pid = child.id().unwrap_or_default();

  Ok(if own_group {
    Tree::own_group(pid, kill)
  } else {
    // A child that stayed in rune's group has the terminal's own signal delivery, and
    // reading the terminal cannot stop it. Signalling that group would signal rune too, so
    // such a child is only ever reached alone.
    Tree::in_runes_group(pid, kill)
  })
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
