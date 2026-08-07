//! Finding the configs by walking upward, and knowing when to stop.
//!
//! The boundary is not an optimization. Without it a Rune run in a scratch directory —
//! or a test in a temporary one — walks all the way up and picks up an unrelated config
//! from somewhere else on the machine, and the result depends on whose machine it is.
//!
//! The walk records every config it passes, because two of them matter: the outermost one
//! defines what the repository shares, and the nearest one is how a single package
//! narrows it.

use std::path::{Path, PathBuf};

use thiserror::Error;

/// The config file name. Never `rune.toml`: the config is TypeScript on purpose.
pub const CONFIG_FILE: &str = "rune.config.ts";

/// The entry that ends the upward walk.
const BOUNDARY: &str = ".git";

/// The file that marks a package directory.
const PACKAGE_FILE: &str = "package.json";

#[derive(Debug, Error)]
#[error(
  "no {CONFIG_FILE} found\n\n\
   searched upward from {} and stopped at the repository boundary.\n\n\
   create one at the root of your repository with:\n\n  rune init",
  .started_from.display()
)]
pub struct NotFound {
  pub started_from: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Discovered {
  /// The outermost config found before the boundary — what the repository shares.
  pub root_config: PathBuf,
  /// The directory holding the root config — the monorepo root, in practice.
  pub root: PathBuf,
  /// The nearest config at or above the working directory, when it is not the root
  /// config itself. This is the one that may narrow a shared definition.
  ///
  /// Exactly two configs take part, never more. A config between these two takes no
  /// part: resolution answers "what does the repository say, and what does this package
  /// say about it", and a third opinion in the middle would need an order nobody can
  /// predict from the file they are reading.
  pub package_config: Option<PathBuf>,
  /// The nearest package directory at or above the starting point. Scripts run here.
  pub package_dir: PathBuf,
}

impl Discovered {
  /// Both configs, nearest first — the order resolution reads them in.
  pub fn configs(&self) -> Vec<PathBuf> {
    self.package_config.iter().chain(std::iter::once(&self.root_config)).cloned().collect()
  }
}

/// The nearest `package.json` at or above `start`, within the repository boundary.
///
/// The same walk `discover` makes, so `rune init` seeds from the package the loader
/// would later call this script's home.
pub fn nearest_package_json(start: &Path) -> Option<PathBuf> {
  for directory in start.ancestors() {
    let candidate = directory.join(PACKAGE_FILE);
    if candidate.is_file() {
      return Some(candidate);
    }

    if directory.join(BOUNDARY).exists() {
      break;
    }
  }

  None
}

/// Walks up from `start` looking for configs, stopping at a `.git` or the filesystem root.
pub fn discover(start: &Path) -> Result<Discovered, NotFound> {
  let started_from = start.to_path_buf();
  let mut package_dir = None;
  let mut configs = Vec::new();

  for directory in start.ancestors() {
    if package_dir.is_none() && directory.join(PACKAGE_FILE).is_file() {
      package_dir = Some(directory.to_path_buf());
    }

    let candidate = directory.join(CONFIG_FILE);
    if candidate.is_file() {
      configs.push(candidate);
    }

    // Checked after the config, because a repository root normally holds both.
    if directory.join(BOUNDARY).exists() {
      break;
    }
  }

  let Some(root_config) = configs.last().cloned() else {
    return Err(NotFound { started_from });
  };

  let root = root_config.parent().unwrap_or(Path::new("")).to_path_buf();
  // `configs` holds at least the root config, so a second entry is a genuinely nearer one.
  let package_config = (configs.len() > 1).then(|| configs.swap_remove(0));

  Ok(Discovered {
    root_config,
    root,
    package_config,
    package_dir: package_dir.unwrap_or(started_from),
  })
}

#[cfg(test)]
mod tests {
  use std::fs;
  use std::path::Path;

  use tempfile::TempDir;

  use super::{CONFIG_FILE, discover};

  fn fixture(files: &[&str]) -> TempDir {
    let dir = tempfile::tempdir().expect("create tempdir");
    for relative in files {
      let path = dir.path().join(relative);
      fs::create_dir_all(path.parent().expect("has a parent")).expect("create parents");
      fs::write(&path, "").expect("write fixture");
    }
    dir
  }

  /// Test 2.21 — the whole scenario in one place: found at the root, package directory
  /// recorded, and the walk stopped where the repository does.
  #[test]
  fn discovery_from_a_nested_package_finds_the_root_config() {
    let dir = fixture(&[".git/HEAD", CONFIG_FILE, "package.json", "packages/foo/package.json"]);
    let start = dir.path().join("packages/foo");

    let found = discover(&start).expect("finds the root config");

    assert_eq!(found.root_config, dir.path().join(CONFIG_FILE));
    assert_eq!(found.root, dir.path());
    assert_eq!(found.package_dir, start);
    assert_eq!(found.package_config, None, "no package config means nothing to narrow");
  }

  /// The override case: both configs are recorded, and the root stays the repository
  /// root rather than becoming the package the walk found first.
  #[test]
  fn a_package_config_and_a_root_config_are_both_recorded() {
    let dir = fixture(&[
      ".git/HEAD",
      CONFIG_FILE,
      "package.json",
      "packages/legacy/package.json",
      "packages/legacy/rune.config.ts",
    ]);
    let start = dir.path().join("packages/legacy");

    let found = discover(&start).expect("finds both configs");

    assert_eq!(found.root_config, dir.path().join(CONFIG_FILE));
    assert_eq!(found.root, dir.path());
    assert_eq!(found.package_config.as_deref(), Some(start.join(CONFIG_FILE).as_path()));
    assert_eq!(found.package_dir, start);
    assert_eq!(found.configs(), vec![start.join(CONFIG_FILE), dir.path().join(CONFIG_FILE)]);
  }

  /// A package with a config but no root config above it: that config *is* the root, and
  /// there is nothing for it to narrow.
  #[test]
  fn the_only_config_is_the_root_config_wherever_it_sits() {
    let dir = fixture(&[".git/HEAD", "packages/solo/package.json", "packages/solo/rune.config.ts"]);
    let start = dir.path().join("packages/solo");

    let found = discover(&start).expect("finds the config");

    assert_eq!(found.root, start);
    assert_eq!(found.package_config, None);
  }

  #[test]
  fn the_walk_stops_at_a_repository_boundary() {
    // The config sits *above* the boundary, so a walk that ignored `.git` would find it.
    let dir = fixture(&[CONFIG_FILE, "repo/.git/HEAD", "repo/packages/foo/package.json"]);

    let error = discover(&dir.path().join("repo/packages/foo")).unwrap_err();

    assert_eq!(error.started_from, dir.path().join("repo/packages/foo"));
  }

  #[test]
  fn a_config_at_the_boundary_directory_is_still_found() {
    let dir = fixture(&[".git/HEAD", CONFIG_FILE]);

    let found = discover(dir.path()).expect("the repository root holds both");

    assert_eq!(found.root, dir.path());
  }

  /// Test 2.22's half that does not need the binary: the message names where it looked.
  #[test]
  fn no_config_names_the_directory_the_search_started_from() {
    let dir = fixture(&[".git/HEAD"]);

    let error = discover(dir.path()).unwrap_err().to_string();

    assert!(error.contains(&dir.path().display().to_string()), "{error}");
    assert!(error.contains("rune init"), "{error}");
  }

  #[test]
  fn the_package_directory_defaults_to_where_the_search_started() {
    let dir = fixture(&[".git/HEAD", CONFIG_FILE]);
    let start = dir.path().join("nested");
    std::fs::create_dir_all(&start).expect("create nested");

    let found = discover(&start).expect("finds the config");

    assert_eq!(found.package_dir, start);
    assert_eq!(found.root, dir.path());
    assert_ne!(found.root, Path::new(""));
  }
}
