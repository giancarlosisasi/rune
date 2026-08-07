//! Writing a path the way rune's output writes it.
//!
//! Everything rune prints about a repository is relative to the config root and spelled
//! with forward slashes. Two reasons, and both are about the reader: an absolute path
//! buries the part that identifies the file, and a path that changes shape between
//! Windows and macOS makes the same repository look like two different ones.

use std::path::Path;

/// `path` relative to `root`, with forward slashes. Absolute when it lies outside `root`.
pub fn relative_to(root: &Path, path: &Path) -> String {
  let shown = path.strip_prefix(root).unwrap_or(path);
  let shown = shown.to_string_lossy().replace('\\', "/");

  if shown.is_empty() { ".".to_owned() } else { shown }
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
}
