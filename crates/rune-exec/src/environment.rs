//! What the child process sees in its environment.
//!
//! Layering, from weakest to strongest: the parent's environment, then `PATH` with the
//! project's `.bin` directories in front, then rune's own variables, then the script's
//! `env` map.
//!
//! Windows keys are case-insensitive and the operating system enforces that, not the
//! caller. Handing a child both `PATH` and `Path` is legal for a plain map and produces
//! a process where one of the two is picked arbitrarily.

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::ffi::{OsStr, OsString};
use std::path::Path;

use crate::bin_paths;

/// The script rune was asked to run.
pub const SCRIPT_NAME: &str = "RUNE_SCRIPT_NAME";
/// The directory holding the config that defined it.
pub const ROOT: &str = "RUNE_ROOT";
/// The package directory the run started from.
pub const PACKAGE_DIR: &str = "RUNE_PACKAGE_DIR";

const PATH: &str = "PATH";

/// An environment keyed the way the running operating system keys one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChildEnvironment {
  /// Lookup key to the name as first written and the current value. The name is kept so
  /// a child sees `Path` on a machine whose parent environment said `Path`.
  entries: BTreeMap<String, (String, OsString)>,
}

impl ChildEnvironment {
  /// Sets `name`, replacing any earlier value under the same key.
  pub fn set(&mut self, name: &str, value: impl Into<OsString>) {
    let value = value.into();
    match self.entries.entry(lookup_key(name)) {
      Entry::Occupied(mut existing) => existing.get_mut().1 = value,
      Entry::Vacant(empty) => {
        empty.insert((name.to_owned(), value));
      }
    }
  }

  pub fn get(&self, name: &str) -> Option<&OsStr> {
    self.entries.get(&lookup_key(name)).map(|(_, value)| value.as_os_str())
  }

  pub fn len(&self) -> usize {
    self.entries.len()
  }

  pub fn is_empty(&self) -> bool {
    self.entries.is_empty()
  }

  /// Every variable, as `Command::envs` wants them.
  pub fn iter(&self) -> impl Iterator<Item = (&str, &OsStr)> {
    self.entries.values().map(|(name, value)| (name.as_str(), value.as_os_str()))
  }
}

/// Everything about a run that the environment depends on.
pub struct Descriptor<'a> {
  pub script_name: &'a str,
  pub root: &'a Path,
  pub package_dir: &'a Path,
  pub env: &'a BTreeMap<String, String>,
}

/// Builds the child's environment from the parent's.
pub fn build<I>(parent: I, descriptor: &Descriptor<'_>) -> ChildEnvironment
where
  I: IntoIterator<Item = (OsString, OsString)>,
{
  let mut environment = ChildEnvironment::default();
  for (name, value) in parent {
    environment.set(&name.to_string_lossy(), value);
  }

  let inherited = environment.get(PATH).map(OsStr::to_os_string);
  environment.set(PATH, augmented_path(descriptor, inherited.as_deref()));

  environment.set(SCRIPT_NAME, descriptor.script_name);
  environment.set(ROOT, descriptor.root);
  environment.set(PACKAGE_DIR, descriptor.package_dir);

  // Last, so a script that wants a different `PATH` or a different `CI` gets one.
  for (name, value) in descriptor.env {
    environment.set(name, value);
  }

  environment
}

fn augmented_path(descriptor: &Descriptor<'_>, inherited: Option<&OsStr>) -> OsString {
  let mut directories = bin_paths::from_package_to_root(descriptor.package_dir, descriptor.root);
  directories.extend(inherited.into_iter().flat_map(std::env::split_paths));

  std::env::join_paths(directories).unwrap_or_else(|_| {
    // Only reachable when an inherited PATH entry contains the separator itself. Keeping
    // the parent's value is better than handing the child no PATH at all.
    inherited.unwrap_or(OsStr::new("")).to_os_string()
  })
}

fn lookup_key(name: &str) -> String {
  if cfg!(windows) { name.to_uppercase() } else { name.to_owned() }
}

#[cfg(test)]
mod tests {
  use std::collections::BTreeMap;
  use std::ffi::{OsStr, OsString};
  use std::path::{Path, PathBuf};

  #[cfg(unix)]
  use super::ChildEnvironment;
  use super::{Descriptor, PACKAGE_DIR, ROOT, SCRIPT_NAME, build};

  fn root() -> PathBuf {
    if cfg!(windows) { PathBuf::from(r"C:\repo") } else { PathBuf::from("/repo") }
  }

  fn parent(pairs: &[(&str, &str)]) -> Vec<(OsString, OsString)> {
    pairs.iter().map(|(name, value)| (OsString::from(name), OsString::from(value))).collect()
  }

  fn descriptor<'a>(
    root: &'a Path,
    package_dir: &'a Path,
    env: &'a BTreeMap<String, String>,
  ) -> Descriptor<'a> {
    Descriptor { script_name: "test", root, package_dir, env }
  }

  #[test]
  fn runes_own_variables_are_always_present() {
    let root = root();
    let package = root.join("packages/foo");
    let empty = BTreeMap::new();

    let environment = build(parent(&[]), &descriptor(&root, &package, &empty));

    assert_eq!(environment.get(SCRIPT_NAME), Some(OsStr::new("test")));
    assert_eq!(environment.get(ROOT), Some(root.as_os_str()));
    assert_eq!(environment.get(PACKAGE_DIR), Some(package.as_os_str()));
  }

  #[test]
  fn the_scripts_env_beats_an_inherited_value() {
    let root = root();
    let package = root.join("packages/foo");
    let env = BTreeMap::from([("NODE_ENV".to_owned(), "test".to_owned())]);

    let environment =
      build(parent(&[("NODE_ENV", "production")]), &descriptor(&root, &package, &env));

    assert_eq!(environment.get("NODE_ENV"), Some(OsStr::new("test")));
  }

  #[test]
  fn the_bin_directories_come_first_and_the_inherited_path_survives() {
    let root = root();
    let package = root.join("packages/foo");
    let empty = BTreeMap::new();
    let inherited = if cfg!(windows) { r"C:\tools" } else { "/tools" };

    let environment = build(parent(&[("PATH", inherited)]), &descriptor(&root, &package, &empty));

    let path = environment.get("PATH").expect("PATH is set");
    let entries: Vec<PathBuf> = std::env::split_paths(path).collect();

    assert_eq!(entries.first(), Some(&package.join("node_modules/.bin")));
    assert_eq!(entries.last(), Some(&PathBuf::from(inherited)));
  }

  /// Test 3.8 — the Windows case. A plain map overlay hands the child `PATH` *and*
  /// `Path`, after which the operating system picks one of them and the other silently
  /// does nothing.
  #[test]
  #[cfg(windows)]
  fn overlaying_path_onto_uppercase_path_leaves_exactly_one_entry() {
    let root = root();
    let package = root.join("packages/foo");
    let env = BTreeMap::from([("Path".to_owned(), r"C:\only".to_owned())]);

    let environment =
      build(parent(&[("PATH", r"C:\inherited")]), &descriptor(&root, &package, &env));

    let named: Vec<&str> = environment
      .iter()
      .map(|(name, _)| name)
      .filter(|name| name.eq_ignore_ascii_case("path"))
      .collect();

    assert_eq!(named.len(), 1, "{named:?}");
    assert_eq!(environment.get("PATH"), Some(OsStr::new(r"C:\only")));
  }

  #[test]
  #[cfg(unix)]
  fn case_is_significant_away_from_windows() {
    let mut environment = ChildEnvironment::default();
    environment.set("Path", "one");
    environment.set("PATH", "two");

    assert_eq!(environment.len(), 2);
  }
}
