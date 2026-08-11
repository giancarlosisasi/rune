//! What every subcommand does before it can do its own job: find the config, and say
//! something useful when the name the user typed is not in it.
//!
//! `run` and `inspect` share this so the two can never answer "no such script"
//! differently — the point of `inspect` is that it explains what `run` would do.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use rune_config::env::Environment;
use rune_config::envfile::Files;
use rune_config::inherit::{Declared, Resolved, Scope};
use rune_config::load::{Loaded, load};
use rune_config::resolve::lexically_normalize;
use rune_config::suggest::closest;
use rune_exec::environment::{Assignment, FileLayer};

/// Loads the configs that apply to the directory rune was started in.
pub fn load_here() -> Result<Loaded, String> {
  let working_directory = working_directory()?;

  load(&working_directory, &Environment::from_process()).map_err(|error| error.to_string())
}

pub fn working_directory() -> Result<PathBuf, String> {
  std::env::current_dir().map_err(|error| format!("cannot read the working directory: {error}"))
}

/// Reads a resolution's dotenv files and hands them over in the shape the layering
/// reads them.
///
/// Shared by `run` and `inspect` so that the explanation and the run cannot disagree
/// about which files took part or in what order — and so that both read a file exactly
/// when a script that declares it is reached. `list` comes nowhere near here: it needs
/// names and descriptions, and it is the command a user runs when something is already
/// broken.
pub fn env_files(
  resolved: &Resolved<'_>,
  script: &str,
  loaded: &Loaded,
  files: &mut Files,
) -> Result<Vec<FileLayer>, String> {
  resolved
    .env_files
    .iter()
    .map(|declared| {
      let file =
        files.read(&loaded.discovered.root, script, declared).map_err(|error| error.to_string())?;

      Ok(FileLayer {
        source: file.source.clone(),
        assignments: file.assignments.iter().map(assignment).collect(),
      })
    })
    .collect()
}

/// The same assignment, spelled for the crate that layers it. rune-config describes what
/// a file said; rune-exec decides what the child sees, and neither depends on the
/// other's vocabulary.
fn assignment(declared: &rune_config::envfile::Assignment) -> Assignment {
  Assignment { name: declared.name.clone(), value: declared.value.clone(), line: declared.line }
}

/// Where a script runs: absolute values as given, relative ones against the config that
/// declared them, and nothing declared against the package the run started from.
///
/// `run` and `inspect` both come here, so an explanation cannot describe a directory the
/// run would not use. Anchoring a relative value on the caller instead would make a
/// definition mean a different place from every package, which is the one thing a shared
/// config exists to prevent.
pub fn directory(cwd: Option<Declared<'_>>, package_dir: &Path) -> PathBuf {
  let Some(declared) = cwd else {
    return package_dir.to_path_buf();
  };

  let path = Path::new(declared.value);
  if path.is_absolute() {
    return path.to_path_buf();
  }

  // Normalised because the result is printed: joining `apps/api` onto a Windows directory
  // otherwise mixes both separators in one path.
  lexically_normalize(&declared.anchor().join(path))
}

/// The miss, what is available, and the likeliest correction.
///
/// All three matter: the name alone leaves the user guessing, and a suggestion alone
/// hides the rest of the config from someone who is new to it.
pub fn unknown(name: &str, loaded: &Loaded, scope: Scope) -> String {
  let defined = loaded.names(scope);
  let mut message = format!("no script named `{name}`");

  if let Some(closest) = closest(name, defined.iter().copied()) {
    let _ = write!(message, "\n\ndid you mean `{closest}`?");
  }

  if defined.is_empty() {
    message.push_str("\n\nthis config defines no scripts");
    return message;
  }

  message.push_str("\n\nscripts defined here:");
  for name in defined {
    let _ = write!(message, "\n  {name}");
  }

  message
}

#[cfg(test)]
mod tests {
  use std::path::{Path, PathBuf};

  use rune_config::inherit::Declared;

  use super::directory;

  /// A repository root spelled the way this operating system spells one.
  fn root() -> PathBuf {
    PathBuf::from(if cfg!(windows) { r"C:\repo" } else { "/repo" })
  }

  /// The root config, which is where every value in these tests was written.
  fn source() -> PathBuf {
    root().join("rune.config.ts")
  }

  #[test]
  fn a_relative_value_anchors_on_the_config_that_declared_it() {
    let source = source();
    let package = root().join("packages").join("ui");
    let cwd = Declared { value: "apps/api", source: &source };

    assert_eq!(directory(Some(cwd), &package), root().join("apps").join("api"));
  }

  #[test]
  fn a_script_with_no_cwd_runs_where_the_run_started() {
    let package = root().join("packages").join("ui");

    assert_eq!(directory(None, &package), package);
  }

  #[test]
  fn an_absolute_value_is_used_as_given() {
    let source = source();
    let elsewhere = if cfg!(windows) { r"C:\elsewhere" } else { "/elsewhere" };
    let cwd = Declared { value: elsewhere, source: &source };

    assert_eq!(directory(Some(cwd), &root()), Path::new(elsewhere));
  }

  /// Test R2.8 — the constructed path is printed, and a config writes `/` whatever the
  /// operating system separates with. Joining it on verbatim produces
  /// `…\packages\ui\apps/api`, which looks like two machines argued over one path.
  #[test]
  fn a_constructed_path_carries_one_separator_kind() {
    let source = source();
    let cwd = Declared { value: "apps/api", source: &source };

    let printed = directory(Some(cwd), &root()).display().to_string();
    let foreign = if cfg!(windows) { '/' } else { '\\' };

    assert!(!printed.contains(foreign), "a printed path mixes separators: {printed}");
  }
}
