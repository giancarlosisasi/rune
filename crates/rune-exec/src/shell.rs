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

/// What a command can begin with that means it does not begin with a plain program name.
const OPERATOR: &[char] = &['&', '|', '<', '>', '(', ')', '^', '%', '!'];

/// The program a command starts with, when it starts with one.
///
/// Nothing else about the command is read. rune does not learn what `&&`, a pipe or a
/// redirection mean — the shell keeps that job. It reads the first word, because
/// identifying the child is the only way to know how many readers its arguments will pass
/// through.
///
/// A command beginning with an operator, a variable expansion or nothing at all yields
/// `None`, and `None` means "behave exactly as if this had never been asked".
pub fn leading_program(command: &str) -> Option<&str> {
  let command = command.trim_start();

  if let Some(quoted) = command.strip_prefix('"') {
    return quoted.split('"').next().filter(|token| !token.is_empty());
  }

  let token = command.split_whitespace().next()?;
  (!token.contains(OPERATOR)).then_some(token)
}

/// Finds a program on `PATH` the way Windows finds one, for callers that need to know
/// which file a bare name stands for.
///
/// `std::process::Command` on Windows searches `PATH` but only ever appends `.exe`, so a
/// program installed as `.cmd` or `.bat` is reported missing. Everywhere else the
/// operating system's own lookup is already what the user expects.
///
/// `PATHEXT` is tried before the name as written, which is the order Windows itself uses:
/// a package manager writes both `biome` and `biome.CMD` into `node_modules/.bin`, the
/// first is a shell script no Windows process can start, and the second is the one that
/// runs. Preferring the exact name would answer with the file nothing executes.
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

    for extension in &extensions {
      let mut with_extension = candidate.clone().into_os_string();
      with_extension.push(extension);
      let with_extension = PathBuf::from(with_extension);
      if with_extension.is_file() {
        return Some(with_extension);
      }
    }

    if candidate.is_file() {
      return Some(candidate);
    }
  }

  None
}

#[cfg(test)]
mod tests {
  use std::ffi::OsStr;
  use std::path::Path;

  use super::{Shell, ShellKind, kind_of, leading_program};

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

  /// The order that decides which of two files with the same stem is the one that runs.
  #[cfg(windows)]
  #[test]
  fn a_path_extension_wins_over_the_name_as_written() {
    let directory = tempfile::tempdir().expect("create tempdir");
    for name in ["biome", "biome.CMD"] {
      std::fs::write(directory.path().join(name), "").expect("write the fixture");
    }

    let located = super::locate(
      Path::new("biome"),
      Some(directory.path().as_os_str()),
      Some(OsStr::new(".EXE;.CMD")),
    );

    assert_eq!(located, Some(directory.path().join("biome.CMD")));
  }

  /// Test R3.7 — the first word, and nothing more.
  ///
  /// Everything that is not a plain program name answers `None`, which is what keeps this
  /// from growing into a command parser: there is no case where reading further would
  /// change an answer.
  #[test]
  fn the_leading_program_is_read_and_nothing_else() {
    let table = [
      ("biome lint .", Some("biome")),
      ("  vitest --run  ", Some("vitest")),
      ("tsc", Some("tsc")),
      (r#""C:\Program Files\tool\biome.cmd" lint ."#, Some(r"C:\Program Files\tool\biome.cmd")),
      ("&& echo late", None),
      ("| tee log.txt", None),
      ("%NPM_TOOL% lint", None),
      ("(echo grouped)", None),
      ("", None),
      ("   ", None),
      (r#""" lint"#, None),
    ];

    for (command, expected) in table {
      assert_eq!(leading_program(command), expected, "{command:?}");
    }
  }
}
