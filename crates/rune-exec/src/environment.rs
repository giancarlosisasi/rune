//! What the child process sees in its environment.
//!
//! Layering, from weakest to strongest: the parent's environment, then `PATH` with the
//! project's `.bin` directories in front, then the script's dotenv files, then the
//! script's `env` map, then rune's own variables.
//!
//! Dotenv files are the layer with a rule of its own: they fill gaps and never override.
//! A committed file that could beat the real environment is a foot-gun with a long fuse —
//! CI exports `CI=true`, a file in the repository says `CI=false`, and the build behaves
//! like a laptop while its tests still pass. Every assignment that does not reach the
//! child for that reason is reported rather than dropped in silence.
//!
//! Windows keys are case-insensitive and the operating system enforces that, not the
//! caller. Handing a child both `PATH` and `Path` is legal for a plain map and produces
//! a process where one of the two is picked arbitrarily.

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::path::Path;

use crate::bin_paths;

/// The script rune was asked to run.
pub const SCRIPT_NAME: &str = "RUNE_SCRIPT_NAME";
/// The directory holding the config that defined it.
pub const ROOT: &str = "RUNE_ROOT";
/// The package directory the run started from.
pub const PACKAGE_DIR: &str = "RUNE_PACKAGE_DIR";

/// The names rune keeps for itself.
///
/// The whole prefix rather than the three names above, so that a fourth variable later is
/// not a breaking change. A config able to set these could tell a script it is running in
/// a package it is not, and every tool downstream reading them would be lied to.
pub const RESERVED_PREFIX: &str = "RUNE_";

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

/// One dotenv file's contribution, already read.
pub struct FileLayer<'a> {
  /// The file, named the way rune's output names paths. A label rather than a path,
  /// because every warning prints it and an absolute path would bury the name.
  pub source: &'a str,
  pub assignments: &'a [(String, String)],
}

/// Where an assignment was written, or where a value already in place came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
  /// The environment rune itself was started with.
  Process,
  /// A dotenv file the script declared.
  File(String),
  /// The script's own `env` map.
  ScriptEnv,
}

/// Why an assignment never reached the child.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reason {
  AlreadySet(Origin),
  Reserved,
}

/// One assignment a config asked for that the child will not see.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ignored {
  pub name: String,
  pub source: Origin,
  pub reason: Reason,
}

impl fmt::Display for Origin {
  fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
    match self {
      Self::Process => write!(formatter, "the process environment"),
      Self::File(source) => write!(formatter, "`{source}`"),
      Self::ScriptEnv => write!(formatter, "this script's `env`"),
    }
  }
}

impl fmt::Display for Reason {
  fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
    match self {
      Self::AlreadySet(origin) => write!(formatter, "{origin} already sets it"),
      Self::Reserved => {
        write!(formatter, "`{RESERVED_PREFIX}` is reserved for rune's own variables")
      }
    }
  }
}

impl fmt::Display for Ignored {
  fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
    write!(formatter, "`{}` from {} was ignored: {}", self.name, self.source, self.reason)
  }
}

/// The environment a child will run with, and what the config asked for that it will not.
pub struct Layering {
  pub environment: ChildEnvironment,
  /// What the config contributed and the child does see, by name. The rest of the
  /// child's environment is inherited, so listing it would say nothing.
  pub applied: BTreeMap<String, String>,
  pub ignored: Vec<Ignored>,
}

/// Everything about a run that the environment depends on.
pub struct Descriptor<'a> {
  pub script_name: &'a str,
  pub root: &'a Path,
  pub package_dir: &'a Path,
  pub env: &'a BTreeMap<String, String>,
  /// Nearest to the script first, which is the order they get to fill a gap in.
  pub env_files: &'a [FileLayer<'a>],
}

/// Builds the child's environment from the parent's.
pub fn build<I>(parent: I, descriptor: &Descriptor<'_>) -> Layering
where
  I: IntoIterator<Item = (OsString, OsString)>,
{
  let mut environment = ChildEnvironment::default();
  for (name, value) in parent {
    environment.set(&name.to_string_lossy(), value);
  }

  let inherited = environment.get(PATH).map(OsStr::to_os_string);
  environment.set(PATH, augmented_path(descriptor, inherited.as_deref()));

  let mut owner = owners(&environment, descriptor);
  let mut applied = BTreeMap::new();
  let mut ignored = Vec::new();

  for file in descriptor.env_files {
    for (name, value) in file.assignments {
      let source = Origin::File(file.source.to_owned());

      if is_reserved(name) {
        ignored.push(Ignored { name: name.clone(), source, reason: Reason::Reserved });
        continue;
      }

      if let Some(holder) = owner.get(&lookup_key(name)) {
        let reason = Reason::AlreadySet(holder.clone());
        ignored.push(Ignored { name: name.clone(), source, reason });
        continue;
      }

      environment.set(name, value.as_str());
      applied.insert(name.clone(), value.clone());
      owner.insert(lookup_key(name), source);
    }
  }

  // After the files, so a script that wants a different `PATH` or a different `CI` gets
  // one whatever any file said.
  for (name, value) in descriptor.env {
    if is_reserved(name) {
      let source = Origin::ScriptEnv;
      ignored.push(Ignored { name: name.clone(), source, reason: Reason::Reserved });
      continue;
    }

    environment.set(name, value.as_str());
    applied.insert(name.clone(), value.clone());
  }

  // Last of all, so nothing a config wrote can tell a script it is running elsewhere.
  environment.set(SCRIPT_NAME, descriptor.script_name);
  environment.set(ROOT, descriptor.root);
  environment.set(PACKAGE_DIR, descriptor.package_dir);

  Layering { environment, applied, ignored }
}

/// Who holds each name before any file is read.
///
/// A file assignment applies only where nothing holds the name yet, and this is the map
/// that answers that. The `env` map is entered up front rather than when it is applied,
/// because a file must lose to it too — otherwise the file's value would reach the child
/// for a moment and the warning explaining the loss would never be written.
fn owners(environment: &ChildEnvironment, descriptor: &Descriptor<'_>) -> BTreeMap<String, Origin> {
  let inherited = environment.iter().map(|(name, _)| (lookup_key(name), Origin::Process));
  let declared = descriptor
    .env
    .keys()
    .filter(|name| !is_reserved(name))
    .map(|name| (lookup_key(name), Origin::ScriptEnv));

  inherited.chain(declared).collect()
}

fn is_reserved(name: &str) -> bool {
  lookup_key(name).starts_with(RESERVED_PREFIX)
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

/// Looks a variable up in a parent environment the way the operating system would.
///
/// Windows stores the search path as `Path`, not `PATH`. A case-sensitive scan for
/// `PATH` therefore finds nothing at all, and whatever depended on it behaves as if the
/// variable were unset — which is how a missing shell gets reported for a shell that is
/// sitting in `System32`.
pub fn find<'a>(parent: &'a [(OsString, OsString)], name: &str) -> Option<&'a OsStr> {
  let wanted = lookup_key(name);

  parent
    .iter()
    .find(|(key, _)| lookup_key(&key.to_string_lossy()) == wanted)
    .map(|(_, value)| value.as_os_str())
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
  use super::{Descriptor, FileLayer, Origin, PACKAGE_DIR, ROOT, Reason, SCRIPT_NAME, build};

  fn root() -> PathBuf {
    if cfg!(windows) { PathBuf::from(r"C:\repo") } else { PathBuf::from("/repo") }
  }

  fn parent(pairs: &[(&str, &str)]) -> Vec<(OsString, OsString)> {
    pairs.iter().map(|(name, value)| (OsString::from(name), OsString::from(value))).collect()
  }

  fn assignments(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs.iter().map(|(name, value)| ((*name).to_owned(), (*value).to_owned())).collect()
  }

  fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs.iter().map(|(name, value)| ((*name).to_owned(), (*value).to_owned())).collect()
  }

  fn descriptor<'a>(
    root: &'a Path,
    package_dir: &'a Path,
    env: &'a BTreeMap<String, String>,
  ) -> Descriptor<'a> {
    Descriptor { script_name: "test", root, package_dir, env, env_files: &[] }
  }

  #[test]
  fn runes_own_variables_are_always_present() {
    let root = root();
    let package = root.join("packages/foo");
    let empty = BTreeMap::new();

    let environment = build(parent(&[]), &descriptor(&root, &package, &empty)).environment;

    assert_eq!(environment.get(SCRIPT_NAME), Some(OsStr::new("test")));
    assert_eq!(environment.get(ROOT), Some(root.as_os_str()));
    assert_eq!(environment.get(PACKAGE_DIR), Some(package.as_os_str()));
  }

  #[test]
  fn the_scripts_env_beats_an_inherited_value() {
    let root = root();
    let package = root.join("packages/foo");
    let env = map(&[("NODE_ENV", "test")]);

    let environment =
      build(parent(&[("NODE_ENV", "production")]), &descriptor(&root, &package, &env)).environment;

    assert_eq!(environment.get("NODE_ENV"), Some(OsStr::new("test")));
  }

  #[test]
  fn the_bin_directories_come_first_and_the_inherited_path_survives() {
    let root = root();
    let package = root.join("packages/foo");
    let empty = BTreeMap::new();
    let inherited = if cfg!(windows) { r"C:\tools" } else { "/tools" };

    let environment =
      build(parent(&[("PATH", inherited)]), &descriptor(&root, &package, &empty)).environment;

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
    let env = map(&[("Path", r"C:\only")]);

    let environment =
      build(parent(&[("PATH", r"C:\inherited")]), &descriptor(&root, &package, &env)).environment;

    let named: Vec<&str> = environment
      .iter()
      .map(|(name, _)| name)
      .filter(|name| name.eq_ignore_ascii_case("path"))
      .collect();

    assert_eq!(named.len(), 1, "{named:?}");
    assert_eq!(environment.get("PATH"), Some(OsStr::new(r"C:\only")));
  }

  /// A file fills what the environment left empty and loses everything else, and the
  /// assignment it lost is recorded rather than dropped.
  #[test]
  fn a_file_fills_a_gap_and_never_takes_a_name_the_environment_already_holds() {
    let root = root();
    let package = root.join("packages/foo");
    let empty = BTreeMap::new();
    let assignments = assignments(&[("API_URL", "https://example.test"), ("CI", "false")]);
    let files = [FileLayer { source: ".env", assignments: &assignments }];

    let layered = build(
      parent(&[("CI", "true")]),
      &Descriptor { env_files: &files, ..descriptor(&root, &package, &empty) },
    );

    assert_eq!(layered.environment.get("API_URL"), Some(OsStr::new("https://example.test")));
    assert_eq!(layered.environment.get("CI"), Some(OsStr::new("true")));

    assert_eq!(layered.ignored.len(), 1, "{:?}", layered.ignored);
    assert_eq!(layered.ignored[0].name, "CI");
    assert_eq!(layered.ignored[0].source, Origin::File(".env".to_owned()));
    assert_eq!(layered.ignored[0].reason, Reason::AlreadySet(Origin::Process));
  }

  /// Rune's own variables answer "where am I running", so a config able to write them
  /// could tell a script it is somewhere it is not. Both places a config can try are
  /// refused the same way.
  #[test]
  fn the_reserved_prefix_is_refused_from_a_file_and_from_the_env_map() {
    let root = root();
    let package = root.join("packages/foo");
    let env = map(&[("RUNE_PACKAGE_DIR", "elsewhere")]);
    let assignments = assignments(&[("RUNE_ROOT", "elsewhere")]);
    let files = [FileLayer { source: ".env", assignments: &assignments }];

    let layered =
      build(parent(&[]), &Descriptor { env_files: &files, ..descriptor(&root, &package, &env) });

    assert_eq!(layered.environment.get(ROOT), Some(root.as_os_str()));
    assert_eq!(layered.environment.get(PACKAGE_DIR), Some(package.as_os_str()));

    let refused: Vec<(&str, &Origin)> =
      layered.ignored.iter().map(|one| (one.name.as_str(), &one.source)).collect();
    assert_eq!(
      refused,
      [("RUNE_ROOT", &Origin::File(".env".to_owned())), ("RUNE_PACKAGE_DIR", &Origin::ScriptEnv),]
    );
    assert!(layered.ignored.iter().all(|one| one.reason == Reason::Reserved));
  }

  /// The lookup that finds the shell. Windows hands a process `Path`, so a scan that
  /// only matches `PATH` finds nothing and every bare shell name is reported missing.
  #[test]
  fn a_parent_variable_is_found_the_way_the_operating_system_names_it() {
    let stored = parent(&[("Path", r"C:\Windows\System32"), ("PATHEXT", ".COM;.EXE")]);

    assert_eq!(super::find(&stored, "PATHEXT"), Some(OsStr::new(".COM;.EXE")));
    assert_eq!(super::find(&stored, "NOT_SET"), None);

    let found = super::find(&stored, "PATH");
    if cfg!(windows) {
      assert_eq!(found, Some(OsStr::new(r"C:\Windows\System32")), "`Path` must answer `PATH`");
    } else {
      assert_eq!(found, None, "case is significant away from Windows");
    }
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
