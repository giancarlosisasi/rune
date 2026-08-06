//! Where a bare tool name in a command is found.
//!
//! `"command": "jest --watch"` only works because npm puts every `node_modules/.bin`
//! between the package and the repository root on `PATH` first. Without this a bare tool
//! name resolves to whatever is installed globally, or to nothing at all.

use std::path::{Path, PathBuf};

/// The directory a package manager links executables into.
const BIN_DIRECTORY: &str = "node_modules/.bin";

/// Every `.bin` directory from `package_dir` up to and including `root`, most specific
/// first.
///
/// Order is the part with teeth: a package's own copy of a tool has to win over the
/// root's, or a package that pinned an older version silently runs the newer one.
///
/// Existence is not checked, matching npm. A directory that appears later still lands in
/// the right position.
pub fn from_package_to_root(package_dir: &Path, root: &Path) -> Vec<PathBuf> {
  let mut directories = Vec::new();

  for ancestor in package_dir.ancestors() {
    directories.push(ancestor.join(BIN_DIRECTORY));
    if ancestor == root {
      return directories;
    }
  }

  // The package directory was not inside the root — discovery cannot produce that, but a
  // silently truncated `PATH` would be a miserable thing to debug if it ever did.
  directories.push(root.join(BIN_DIRECTORY));
  directories
}

#[cfg(test)]
mod tests {
  use std::path::{Path, PathBuf};

  use super::from_package_to_root;

  fn root() -> PathBuf {
    if cfg!(windows) { PathBuf::from(r"C:\repo") } else { PathBuf::from("/repo") }
  }

  /// Test 3.4 — package before root, and every level in between.
  #[test]
  fn directories_run_from_the_package_up_to_the_root() {
    let root = root();
    let package = root.join("packages").join("foo");

    let directories = from_package_to_root(&package, &root);

    assert_eq!(
      directories,
      vec![
        package.join("node_modules/.bin"),
        root.join("packages/node_modules/.bin"),
        root.join("node_modules/.bin"),
      ]
    );
  }

  #[test]
  fn running_from_the_root_yields_one_directory() {
    let root = root();

    assert_eq!(from_package_to_root(&root, &root), vec![root.join("node_modules/.bin")]);
  }

  /// The separator is the operating system's, because these are joined into `PATH`.
  #[test]
  fn the_platform_separator_joins_them() {
    let root = root();
    let directories = from_package_to_root(&root.join("packages/foo"), &root);
    let joined = std::env::join_paths(&directories).expect("no path contains a separator");

    let expected = if cfg!(windows) { ';' } else { ':' };
    assert_eq!(joined.to_string_lossy().matches(expected).count(), directories.len() - 1);
  }

  #[test]
  fn a_package_outside_the_root_still_gets_the_roots_directory() {
    let elsewhere =
      if cfg!(windows) { Path::new(r"C:\elsewhere") } else { Path::new("/elsewhere") };

    let directories = from_package_to_root(elsewhere, &root());

    assert!(directories.contains(&root().join("node_modules/.bin")), "{directories:?}");
  }
}
