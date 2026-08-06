//! Which shell runs a command, and how the command reaches it.
//!
//! Every command goes through a shell. That is what makes `jest --watch | tee log.txt`
//! work without rune parsing a single operator, and it is what npm does, so a command
//! string that worked in `package.json` keeps working here.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Command;

/// npm's own setting. It arrives in the environment whenever rune is called from a
/// package-manager script, and ignoring it would make rune diverge from the npm run
/// happening one line above it.
pub const SHELL_VARIABLE: &str = "npm_config_script_shell";

/// The shell each platform falls back to, matching npm.
const WINDOWS_DEFAULT: &str = "cmd.exe";
const POSIX_DEFAULT: &str = "/bin/sh";

/// Which argument convention a shell speaks. Detected from the program name, because
/// that is all npm and concurrently have to go on either.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellKind {
  Cmd,
  PowerShell,
  Posix,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shell {
  pub program: PathBuf,
  pub kind: ShellKind,
}

impl Shell {
  /// The configured shell, or the platform default when nothing is configured.
  pub fn select(configured: Option<&OsStr>) -> Self {
    let program =
      configured.filter(|value| !value.is_empty()).map_or_else(default_program, PathBuf::from);
    let kind = kind_of(&program);

    Self { program, kind }
  }

  /// The spawnable command, with `command_line` handed to the shell the way that shell
  /// expects to receive it.
  pub fn command(&self, command_line: &str) -> Command {
    let mut command = Command::new(&self.program);
    match self.kind {
      ShellKind::Cmd => push_cmd_arguments(&mut command, command_line),
      ShellKind::PowerShell => {
        command.args(["-NoProfile", "-Command", command_line]);
      }
      ShellKind::Posix => {
        command.args(["-c", command_line]);
      }
    }

    command
  }
}

fn default_program() -> PathBuf {
  PathBuf::from(if cfg!(windows) { WINDOWS_DEFAULT } else { POSIX_DEFAULT })
}

/// Reduces a shell path to the name that decides its calling convention: separators
/// normalized, directories dropped, `.exe` removed, lowercased.
pub fn kind_of(program: &Path) -> ShellKind {
  let text = program.to_string_lossy().replace('\\', "/");
  let name = text.rsplit('/').next().unwrap_or(&text).to_lowercase();
  let name = name.strip_suffix(".exe").unwrap_or(&name);

  match name {
    "cmd" => ShellKind::Cmd,
    "powershell" | "pwsh" => ShellKind::PowerShell,
    _ => ShellKind::Posix,
  }
}

/// `cmd /d /s /c "<command>"`, passed verbatim.
///
/// Verbatim is the whole point: the string is already quoted for cmd.exe, and the Rust
/// runtime's own MSVC-style quoting would escape it a second time, after which cmd
/// mis-parses every quote and metacharacter in it. `/d` skips AutoRun commands from the
/// registry, which are a real source of "works on my machine".
#[cfg(windows)]
fn push_cmd_arguments(command: &mut Command, command_line: &str) {
  use std::os::windows::process::CommandExt as _;

  command.raw_arg("/d").raw_arg("/s").raw_arg("/c").raw_arg(format!("\"{command_line}\""));
}

#[cfg(not(windows))]
fn push_cmd_arguments(command: &mut Command, command_line: &str) {
  // cmd.exe only exists on Windows. The arm stays total so the kind table can be unit
  // tested on every platform.
  command.args(["/d", "/s", "/c", command_line]);
}

/// Finds the shell binary the way a user expects a name typed into a terminal to be found.
///
/// `std::process::Command` on Windows searches `PATH` but only ever appends `.exe`, so a
/// shell installed as `.cmd` or `.bat` is reported missing. Everywhere else the operating
/// system's own lookup is already what the user expects.
pub fn locate(
  program: &Path,
  path: Option<&OsStr>,
  path_extensions: Option<&OsStr>,
) -> Option<PathBuf> {
  if !cfg!(windows) || program.components().count() > 1 {
    return Some(program.to_path_buf());
  }

  let extensions: Vec<OsString> = path_extensions
    .map(|value| std::env::split_paths(value).map(PathBuf::into_os_string).collect())
    .unwrap_or_default();

  for directory in path.into_iter().flat_map(std::env::split_paths) {
    let candidate = directory.join(program);
    if candidate.is_file() {
      return Some(candidate);
    }

    for extension in &extensions {
      let mut with_extension = candidate.clone().into_os_string();
      with_extension.push(extension);
      let with_extension = PathBuf::from(with_extension);
      if with_extension.is_file() {
        return Some(with_extension);
      }
    }
  }

  None
}

#[cfg(test)]
mod tests {
  use std::ffi::OsStr;
  use std::path::Path;

  use super::{Shell, ShellKind, kind_of};

  #[test]
  fn the_kind_survives_directories_extensions_and_case() {
    for program in [r"C:\Windows\System32\CMD.EXE", "cmd", "cmd.exe", r"foo\bar\cmd.exe"] {
      assert_eq!(kind_of(Path::new(program)), ShellKind::Cmd, "{program}");
    }

    for program in ["powershell.exe", "pwsh", "/usr/bin/pwsh"] {
      assert_eq!(kind_of(Path::new(program)), ShellKind::PowerShell, "{program}");
    }

    for program in ["/bin/sh", "bash", "/usr/bin/zsh", r"C:\Program Files\Git\bin\bash.exe"] {
      assert_eq!(kind_of(Path::new(program)), ShellKind::Posix, "{program}");
    }
  }

  #[test]
  fn nothing_configured_falls_back_to_the_platform_default() {
    let shell = Shell::select(None);

    if cfg!(windows) {
      assert_eq!(shell.kind, ShellKind::Cmd);
    } else {
      assert_eq!(shell.kind, ShellKind::Posix);
      assert_eq!(shell.program, Path::new("/bin/sh"));
    }
  }

  #[test]
  fn an_empty_setting_is_the_same_as_no_setting() {
    assert_eq!(Shell::select(Some(OsStr::new(""))), Shell::select(None));
  }

  #[test]
  fn a_configured_shell_replaces_the_default() {
    let shell = Shell::select(Some(OsStr::new("/usr/bin/zsh")));

    assert_eq!(shell.program, Path::new("/usr/bin/zsh"));
    assert_eq!(shell.kind, ShellKind::Posix);
  }
}
