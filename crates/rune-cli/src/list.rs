//! `rune list` and `rune cache clear`.

use std::path::PathBuf;

use rune_config::cache;
use rune_config::discover::discover;
use rune_config::env::Environment;
use rune_config::load::load;

/// Prints every script, sorted, with its description.
///
/// The output is the deliverable here, so it carries no absolute paths: run from the
/// repository root or from a package five levels down, the bytes are the same.
pub fn run() -> Result<(), String> {
  let loaded = load(&working_directory()?, &Environment::from_process()).map_err(stringify)?;

  let width = loaded.config.scripts.keys().map(String::len).max().unwrap_or(0);

  for (name, script) in &loaded.config.scripts {
    match script.description() {
      Some(description) => rune_out::line(&format!("{name:<width$}  {description}")),
      None => rune_out::line(name),
    }
  }

  Ok(())
}

/// Removes the cache directory. Nothing to remove is a success, not a failure.
pub fn clear_cache() -> Result<(), String> {
  let discovered = discover(&working_directory()?).map_err(stringify)?;
  cache::clear(&discovered.root).map_err(|error| format!("cannot clear the cache: {error}"))
}

fn working_directory() -> Result<PathBuf, String> {
  std::env::current_dir().map_err(|error| format!("cannot read the working directory: {error}"))
}

fn stringify(error: impl std::fmt::Display) -> String {
  error.to_string()
}
