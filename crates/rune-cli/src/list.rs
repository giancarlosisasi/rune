//! `rune list` and `rune cache clear`.

use rune_config::cache;
use rune_config::discover::discover;
use rune_config::inherit::Scope;
use rune_config::load::Provenance;

use crate::script::{load_here, working_directory};

/// Prints every script, sorted, with its description and where its definition comes from.
///
/// The output carries no absolute paths: run from the repository root or from a package
/// five levels down, the bytes are the same — unless that package has a config of its
/// own, and then the annotations say exactly which entries it changed.
pub fn run() -> Result<(), String> {
  let loaded = load_here()?;

  let mut rows = Vec::new();
  for name in loaded.names(Scope::Nearest) {
    let resolved = loaded
      .resolve(name, Scope::Nearest)
      .map_err(|error| error.to_string())?
      .expect("every listed name comes from a config");

    rows.push((
      name,
      resolved.description.unwrap_or_default(),
      annotation(loaded.provenance(name)),
    ));
  }

  let name_width = rows.iter().map(|(name, ..)| name.len()).max().unwrap_or(0);
  // Descriptions are only padded into a column when something needs to line up beside
  // them, so a repository with no package configs keeps the output it always had.
  let description_width = if rows.iter().any(|(.., annotation)| annotation.is_some()) {
    rows.iter().map(|(_, description, _)| description.len()).max().unwrap_or(0)
  } else {
    0
  };

  for (name, description, annotation) in rows {
    let line = format!(
      "{name:<name_width$}  {description:<description_width$}  {}",
      annotation.unwrap_or_default()
    );

    rune_out::line(line.trim_end());
  }

  Ok(())
}

/// What a package config did to this entry, or nothing when it left it alone.
///
/// Without these a developer reads the same list in two directories and assumes both mean
/// the same thing everywhere, which a package config makes untrue. The word is the rule's
/// own: a package narrows a shared script and can never replace it.
fn annotation(provenance: Provenance) -> Option<&'static str> {
  match provenance {
    Provenance::Root => None,
    Provenance::Narrowed => Some("(narrowed here)"),
    Provenance::Package => Some("(defined here)"),
  }
}

/// Removes the cache directory. Nothing to remove is a success, not a failure.
pub fn clear_cache() -> Result<(), String> {
  let discovered = discover(&working_directory()?).map_err(|error| error.to_string())?;

  cache::clear(&discovered.root).map_err(|error| format!("cannot clear the cache: {error}"))
}

#[cfg(test)]
mod tests {
  use rune_config::load::Provenance;

  use super::annotation;

  /// Test R7.4 — the annotations are the one place a user meets this vocabulary, so the
  /// word the loader refuses cannot come back into them. "Override" is the ordinary word
  /// for replacement, which is exactly what a package may not do.
  #[test]
  fn no_annotation_calls_a_narrowing_an_override() {
    for provenance in [Provenance::Root, Provenance::Narrowed, Provenance::Package] {
      let text = annotation(provenance).unwrap_or_default();

      assert!(!text.contains("overrid"), "`{text}` teaches the rule the loader refuses");
    }
  }
}
