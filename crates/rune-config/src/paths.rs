//! Writing a path the way rune's output writes it.
//!
//! Everything rune prints about a repository is relative to the config root and spelled
//! with forward slashes. Two reasons, and both are about the reader: an absolute path
//! buries the part that identifies the file, and a path that changes shape between
//! Windows and macOS makes the same repository look like two different ones.

use std::fmt::{self, Display};
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// `path` relative to `root`, with forward slashes. Absolute when it lies outside `root`.
pub fn relative_to(root: &Path, path: &Path) -> String {
  let shown = path.strip_prefix(root).unwrap_or(path);
  let shown = shown.to_string_lossy().replace('\\', "/");

  if shown.is_empty() { ".".to_owned() } else { shown }
}

/// A file an error names: the path itself in hand, the repository-relative spelling on
/// screen.
///
/// An error is built where the absolute path is, and read where only the short form is
/// wanted. Carrying both means the shortening happens once, in `Display`, rather than at
/// every site that builds a message.
///
/// One evaluation has one root, and every file it names shares that one value — which is
/// also what keeps an error small enough to return by value.
#[derive(Debug, Clone)]
pub struct Shown {
  root: Rc<Path>,
  path: PathBuf,
}

impl Shown {
  pub fn new(root: &Path, path: &Path) -> Self {
    Self { root: Rc::from(root), path: path.to_path_buf() }
  }

  pub fn as_path(&self) -> &Path {
    &self.path
  }

  pub fn root(&self) -> &Path {
    &self.root
  }

  /// The same repository, a different file in it.
  #[must_use]
  pub fn sibling(&self, path: &Path) -> Self {
    Self { root: Rc::clone(&self.root), path: path.to_path_buf() }
  }
}

impl Display for Shown {
  fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
    formatter.write_str(&relative_to(&self.root, &self.path))
  }
}

#[cfg(test)]
mod tests {
  use std::path::Path;

  use super::relative_to;

  #[test]
  fn a_path_inside_the_root_loses_it_and_gains_forward_slashes() {
    let root = Path::new("/repo");

    assert_eq!(relative_to(root, Path::new("/repo/packages/legacy")), "packages/legacy");
  }

  #[test]
  fn the_root_itself_is_the_current_directory() {
    assert_eq!(relative_to(Path::new("/repo"), Path::new("/repo")), ".");
  }

  #[test]
  fn a_path_outside_the_root_stays_whole() {
    assert_eq!(relative_to(Path::new("/repo"), Path::new("/elsewhere")), "/elsewhere");
  }

  #[test]
  fn a_shown_path_prints_short_and_answers_whole() {
    let shown = super::Shown::new(Path::new("/repo"), Path::new("/repo/scripts/helpers.ts"));

    assert_eq!(shown.to_string(), "scripts/helpers.ts");
    assert_eq!(shown.as_path(), Path::new("/repo/scripts/helpers.ts"));
  }
}
