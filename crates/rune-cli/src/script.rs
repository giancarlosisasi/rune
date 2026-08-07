//! What every subcommand does before it can do its own job: find the config, and say
//! something useful when the name the user typed is not in it.
//!
//! `run` and `inspect` share this so the two can never answer "no such script"
//! differently — the point of `inspect` is that it explains what `run` would do.

use std::fmt::Write as _;
use std::path::PathBuf;

use rune_config::env::Environment;
use rune_config::inherit::Scope;
use rune_config::load::{Loaded, load};
use rune_config::suggest::closest;

/// Loads the configs that apply to the directory rune was started in.
pub fn load_here() -> Result<Loaded, String> {
  let working_directory = working_directory()?;

  load(&working_directory, &Environment::from_process()).map_err(|error| error.to_string())
}

pub fn working_directory() -> Result<PathBuf, String> {
  std::env::current_dir().map_err(|error| format!("cannot read the working directory: {error}"))
}

/// The miss, what is available, and the likeliest correction.
///
/// All three matter: the name alone leaves the user guessing, and a suggestion alone
/// hides the rest of the config from someone who is new to it.
pub fn unknown(name: &str, loaded: &Loaded, scope: Scope) -> String {
  let defined = loaded.names(scope);
  let mut message = format!("no script named `{name}`");

  if let Some(closest) = closest(name, defined.iter().copied()) {
    let _ = write!(message, "\n\ndid you mean `{closest}`?");
  }

  if defined.is_empty() {
    message.push_str("\n\nthis config defines no scripts");
    return message;
  }

  message.push_str("\n\nscripts defined here:");
  for name in defined {
    let _ = write!(message, "\n  {name}");
  }

  message
}
