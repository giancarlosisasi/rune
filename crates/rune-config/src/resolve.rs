//! The one place where an import specifier becomes a path on disk.
//!
//! Both the module loader and the cache-key walk call [`resolve`]. Two implementations
//! could disagree — one accepting `./dir` as `./dir/index.ts` while the other did not —
//! and then the cache key would stop describing what actually ran. Rune would serve a
//! stale result with no symptom. One function, two call sites.

use std::path::{Component, Path, PathBuf};

use thiserror::Error;

use crate::paths::Shown;

/// The file extension a config and its helpers are written in.
const EXTENSION: &str = "ts";

/// The file a directory specifier resolves to.
const DIRECTORY_ENTRY: &str = "index.ts";

#[derive(Debug, Error)]
pub enum ResolveError {
  #[error(
    "cannot import `{specifier}` from {importer}\n\n\
     rune evaluates this config with an embedded JavaScript engine, so npm packages and \
     Node built-in modules do not exist at runtime.\n\n\
     what does work:\n  \
     - `import type {{ ... }} from \"{specifier}\"` — type-only imports are erased before evaluation\n  \
     - a relative import of a file in this repository, such as `./scripts/helpers.ts`\n  \
     - `import {{ defineConfig }} from \"{0}\"` — the one package rune supplies itself\n  \
     - `import {{ rune }} from \"{0}\"` — then `rune.env`, `rune.platform`, `rune.isCI`",
    crate::builtin::MODULE
  )]
  NotRelative { specifier: String, importer: Shown },

  #[error("cannot find `{specifier}` imported from {importer}\n\ntried:\n{}", format_candidates(.tried))]
  NotFound { specifier: String, importer: Shown, tried: Vec<Shown> },

  #[error("cannot read {path}: {source}")]
  Unreadable { path: Shown, source: std::io::Error },
}

fn format_candidates(candidates: &[Shown]) -> String {
  candidates.iter().map(|candidate| format!("  {candidate}")).collect::<Vec<_>>().join("\n")
}

/// Whether a specifier points at a file in this repository rather than at a package.
///
/// A config author on Windows may write `./scripts\helpers`, so both separators count.
pub fn is_relative(specifier: &str) -> bool {
  matches!(specifier.get(..2), Some("./" | ".\\"))
    || matches!(specifier.get(..3), Some("../" | "..\\"))
}

/// Turns `specifier`, as written inside `importer`, into one canonical absolute path.
///
/// Accepts the extensionless form, the explicit `.ts` form and a directory holding
/// `index.ts`, written with either separator. The returned path is canonical, so the
/// same file always produces the same key no matter which form named it.
///
/// `root` never decides what resolves; it decides how a failure spells the files it names.
pub fn resolve(root: &Path, importer: &Path, specifier: &str) -> Result<PathBuf, ResolveError> {
  if !is_relative(specifier) {
    return Err(ResolveError::NotRelative {
      specifier: specifier.to_owned(),
      importer: Shown::new(root, importer),
    });
  }

  // `\` and `/` must name the same file, or the loader and the hasher would key the
  // same helper twice and the cache would split.
  let normalized = specifier.replace('\\', "/");
  let base = importer.parent().unwrap_or_else(|| Path::new("."));
  let joined = lexically_normalize(&base.join(&normalized));

  // A file always wins over a directory of the same name, which is what a reader of
  // `./h` expects when both `h.ts` and `h/` exist.
  let candidates = [joined.clone(), append_extension(&joined), joined.join(DIRECTORY_ENTRY)];

  for candidate in &candidates {
    if candidate.is_file() {
      return canonical(root, candidate);
    }
  }

  Err(ResolveError::NotFound {
    specifier: specifier.to_owned(),
    importer: Shown::new(root, importer),
    tried: candidates.iter().map(|candidate| Shown::new(root, candidate)).collect(),
  })
}

/// Appends `.ts` rather than replacing whatever follows the last dot. `Path::with_extension`
/// would turn `./release.config` into `release.ts` and never find `release.config.ts`.
fn append_extension(path: &Path) -> PathBuf {
  let Some(name) = path.file_name() else {
    return path.to_owned();
  };

  let mut with_extension = name.to_os_string();
  with_extension.push(".");
  with_extension.push(EXTENSION);
  path.with_file_name(with_extension)
}

/// Collapses `.` and `..` without touching the filesystem, so a candidate that does not
/// exist is still named as a path the user can go and look for.
pub fn lexically_normalize(path: &Path) -> PathBuf {
  let mut normalized = PathBuf::new();
  for component in path.components() {
    match component {
      Component::CurDir => {}
      Component::ParentDir => {
        normalized.pop();
      }
      other => normalized.push(other),
    }
  }
  normalized
}

/// Canonicalizes without the `\\?\` prefix `std::fs::canonicalize` adds on Windows.
/// That prefix is correct but it leaks into every error message and module name.
pub fn canonical(root: &Path, path: &Path) -> Result<PathBuf, ResolveError> {
  dunce::canonicalize(path)
    .map_err(|source| ResolveError::Unreadable { path: Shown::new(root, path), source })
}

#[cfg(test)]
mod tests {
  use std::fs;

  use tempfile::TempDir;

  use super::{is_relative, resolve};

  fn fixture(files: &[&str]) -> TempDir {
    let dir = tempfile::tempdir().expect("create tempdir");
    for relative in files {
      let path = dir.path().join(relative);
      fs::create_dir_all(path.parent().expect("has a parent")).expect("create parents");
      fs::write(&path, "export const X = 1;\n").expect("write fixture");
    }
    dir
  }

  #[test]
  fn both_separators_count_as_relative() {
    assert!(is_relative("./h"));
    assert!(is_relative(".\\h"));
    assert!(is_relative("../h"));
    assert!(is_relative("..\\h"));
  }

  #[test]
  fn packages_and_builtins_are_not_relative() {
    assert!(!is_relative("lodash"));
    assert!(!is_relative("node:fs"));
    assert!(!is_relative("@scope/pkg"));
    assert!(!is_relative("/absolute"));
  }

  #[test]
  fn every_specifier_form_reaches_the_same_file() {
    let dir = fixture(&["h.ts", "pkg/index.ts"]);
    let importer = dir.path().join("rune.config.ts");
    let expected = dunce::canonicalize(dir.path().join("h.ts")).expect("canonicalize");

    for specifier in ["./h", "./h.ts", ".\\h", "./././h"] {
      let resolved = resolve(dir.path(), &importer, specifier).expect(specifier);
      assert_eq!(resolved, expected, "specifier `{specifier}` resolved elsewhere");
    }

    let directory = resolve(dir.path(), &importer, "./pkg").expect("./pkg");
    assert_eq!(
      directory,
      dunce::canonicalize(dir.path().join("pkg/index.ts")).expect("canonicalize")
    );
  }

  #[test]
  fn a_dot_in_the_file_name_is_not_an_extension() {
    let dir = fixture(&["release.config.ts"]);
    let importer = dir.path().join("rune.config.ts");
    let resolved = resolve(dir.path(), &importer, "./release.config").expect("./release.config");

    assert_eq!(resolved, dunce::canonicalize(dir.path().join("release.config.ts")).unwrap());
  }

  #[test]
  fn a_nested_import_walks_back_out() {
    let dir = fixture(&["h.ts", "scripts/inner.ts"]);
    let importer = dir.path().join("scripts/inner.ts");
    let resolved = resolve(dir.path(), &importer, "../h").expect("../h");

    assert_eq!(resolved, dunce::canonicalize(dir.path().join("h.ts")).expect("canonicalize"));
  }

  #[test]
  fn a_missing_file_names_the_specifier_and_the_importer() {
    let dir = fixture(&[]);
    let importer = dir.path().join("rune.config.ts");
    let error = resolve(dir.path(), &importer, "./nope.ts").unwrap_err().to_string();

    assert!(error.contains("./nope.ts"), "{error}");
    assert!(error.contains("rune.config.ts"), "{error}");
  }

  /// Everything a failed resolution names sits inside the repository, and the reader is
  /// already standing in it.
  #[test]
  fn a_failure_names_every_file_from_the_repository_root() {
    let dir = fixture(&[]);
    let importer = dir.path().join("scripts").join("helpers.ts");
    let error = resolve(dir.path(), &importer, "./nope.ts").unwrap_err().to_string();

    assert!(error.contains("scripts/helpers.ts"), "{error}");
    assert!(
      !error.contains(&dir.path().to_string_lossy().into_owned()),
      "an absolute path survived:\n{error}"
    );
  }
}
