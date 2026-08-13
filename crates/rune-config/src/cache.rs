//! A content-addressed cache for the resolved config.
//!
//! No TTL, no mtime comparison, no invalidation rules: content in, hash out, hit or
//! miss. Restoring a file's original bytes therefore reuses the original entry, which
//! is the property mtime-based caching cannot deliver.
//!
//! **Every failure degrades to full evaluation, silently.** Unreadable file, corrupt
//! entry, undeletable directory, a file sitting where the directory belongs — none of
//! them is a user-facing error. A caching layer that can break a build is worse than no
//! caching layer.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::env::{Environment, PLATFORM};
use crate::eval::Evaluated;
use crate::resolve::resolve;
use crate::strip::strip_types;

/// Where the cache lives, relative to the config root.
const CACHE_PATH: [&str; 3] = ["node_modules", ".cache", "rune"];

/// A stored evaluation.
///
/// The environment is recorded rather than hashed into the key because the variables a
/// config reads are only known *after* it runs. Hashing the whole environment instead
/// would miss on every unrelated change and make the cache worthless; recording what
/// was read and re-checking it on lookup gives the same guarantee at no cost.
#[derive(Debug, Serialize, Deserialize)]
struct Entry {
  /// One value per config that took part, in the order they were evaluated.
  values: Vec<serde_json::Value>,
  env: BTreeMap<String, Option<String>>,
  /// Set when a config asked for the whole environment. The names it saw are all in
  /// `env`, so a change to any of them is already caught; what this adds is the one
  /// change those names cannot describe, which is a variable arriving that was not
  /// there when the config was built.
  enumerated: bool,
}

pub struct Cache {
  root: PathBuf,
  directory: PathBuf,
}

impl Cache {
  pub fn for_root(root: &Path) -> Self {
    Self {
      root: root.to_path_buf(),
      directory: CACHE_PATH.iter().fold(root.to_path_buf(), |path, part| path.join(part)),
    }
  }

  pub fn directory(&self) -> &Path {
    &self.directory
  }

  fn entry_path(&self, key: &str) -> PathBuf {
    self.directory.join(format!("config-{key}.json"))
  }
}

/// Returns the cached results when every file and variable they depended on is
/// unchanged. Any doubt at all is a miss.
///
/// `entries` are the configs that take part in resolution. One key covers all of them, so
/// editing any one of them — or anything any one of them imports — misses.
pub fn lookup(
  cache: &Cache,
  entries: &[PathBuf],
  environment: &Environment,
) -> Option<Vec<serde_json::Value>> {
  let key = key_for(&cache.root, entries)?;
  let stored = std::fs::read_to_string(cache.entry_path(&key)).ok()?;
  let stored: Entry = serde_json::from_str(&stored).ok()?;

  if stored.values.len() != entries.len() {
    return None;
  }

  // The files are already proven identical by the key. The variables are not.
  let unchanged =
    stored.env.iter().all(|(name, seen)| environment.get(name).map(str::to_owned) == *seen);
  let complete =
    !stored.enumerated || environment.names().all(|name| stored.env.contains_key(name));

  (unchanged && complete).then_some(stored.values)
}

/// Best-effort write. A failure here costs a re-evaluation next time and nothing else.
pub fn store(cache: &Cache, entries: &[PathBuf], evaluated: &[Evaluated]) {
  let Some(key) = key_for(&cache.root, entries) else {
    return;
  };
  if std::fs::create_dir_all(&cache.directory).is_err() {
    return;
  }

  // Every config saw the same environment, so a variable read by both was read with the
  // same value; the union is what the next lookup has to re-check.
  let mut env = BTreeMap::new();
  for one in evaluated {
    env.extend(one.observed.values.iter().map(|(name, value)| (name.clone(), value.clone())));
  }

  let stored = Entry {
    values: evaluated.iter().map(|one| one.value.clone()).collect(),
    env,
    enumerated: evaluated.iter().any(|one| one.observed.enumerated),
  };
  if let Ok(serialized) = serde_json::to_string(&stored) {
    let _ = std::fs::write(cache.entry_path(&key), serialized);
  }
}

/// Removes the cache directory. Succeeds when there is nothing to remove.
pub fn clear(root: &Path) -> std::io::Result<()> {
  let directory = Cache::for_root(root).directory;
  match std::fs::remove_dir_all(&directory) {
    Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
    other => other,
  }
}

/// Hashes everything that decides what an evaluation produces, except the environment.
///
/// The binary version is in the key because a rune upgrade may change how a config is
/// interpreted; the platform is, because a config can branch on `rune.platform`.
///
/// Every config's import closure goes into the one key. A key covering only the root
/// config would serve a stale result the moment a package config changed, which is the
/// worst failure a cache can produce: correct-looking output from the wrong definition.
fn key_for(root: &Path, entries: &[PathBuf]) -> Option<String> {
  let mut hasher = blake3::Hasher::new();
  hasher.update(env!("CARGO_PKG_VERSION").as_bytes());
  hasher.update(PLATFORM.as_bytes());

  for entry in entries {
    for file in import_closure(root, entry)? {
      let bytes = std::fs::read(&file).ok()?;
      // The path is hashed too: moving a helper changes what the config means, even when
      // its bytes do not.
      hasher.update(file.to_string_lossy().as_bytes());
      hasher.update(&bytes);
    }
  }

  Some(hasher.finalize().to_hex().to_string())
}

/// Every file the config transitively imports, entry first, in a stable order.
///
/// Resolution goes through [`crate::resolve::resolve`] — the same function the module
/// loader uses. Two resolvers could disagree, and then this hash would describe a file
/// set that is not the one that ran.
fn import_closure(root: &Path, entry: &Path) -> Option<BTreeSet<PathBuf>> {
  let mut found = BTreeSet::new();
  let mut queue = vec![entry.to_path_buf()];

  while let Some(file) = queue.pop() {
    if !found.insert(file.clone()) {
      continue;
    }

    let source = std::fs::read_to_string(&file).ok()?;
    let stripped = strip_types(&source, &file).ok()?;
    for specifier in &stripped.imports {
      queue.push(resolve(root, &file, specifier).ok()?);
    }
  }

  Some(found)
}
