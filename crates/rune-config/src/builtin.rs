//! The one module rune's config language supplies itself.
//!
//! A config imports `defineConfig` from the package rune publishes, and the embedded
//! engine has no npm resolution to satisfy that import from `node_modules`. The binary
//! answers it instead, which is what lets one file typecheck in an editor and load here.

/// The specifier a config imports `defineConfig` from.
pub const MODULE: &str = "@giancarlosio/rune";

/// What that import evaluates to.
///
/// `defineConfig` carries types and nothing else, so it hands back what it was given. It
/// has to agree with `npm/rune/index.js`, which is what every other tool reading the same
/// config — an editor, a bundler, a test that imports it — resolves the import to.
pub const SOURCE: &str = "export function defineConfig(config) {\n  return config;\n}\n";

/// Whether `specifier` names a module the binary supplies rather than a file on disk.
pub fn is_builtin(specifier: &str) -> bool {
  specifier == MODULE
}

#[cfg(test)]
mod tests {
  use super::is_builtin;

  #[test]
  fn only_the_exact_specifier_is_supplied() {
    assert!(is_builtin("@giancarlosio/rune"));
    assert!(!is_builtin("@giancarlosio/rune-linux-x64"));
    assert!(!is_builtin("@giancarlosio/rune/extra"));
    assert!(!is_builtin("rune"));
  }
}
