//! `rune run <name>`.

use std::fmt::Write as _;

use rune_config::env::{Environment, PLATFORM};
use rune_config::load::load;
use rune_config::schema::{Config, Script};
use rune_exec::{Completion, ExecRequest};

/// Levenshtein distance at which a name stops being a plausible typo and starts being a
/// different word.
const SUGGESTION_LIMIT: usize = 3;

/// Loads the config, finds `name`, and runs it with the child owning the terminal.
///
/// `arguments` are what the user typed after `--`. They go last, after anything the
/// config itself contributes, because the config is the default and the command line is
/// the override — the same order a shell gives precedence in.
pub fn run(name: &str, arguments: &[String]) -> Result<Completion, String> {
  let working_directory = std::env::current_dir()
    .map_err(|error| format!("cannot read the working directory: {error}"))?;

  let loaded = load(&working_directory, &Environment::from_process()).map_err(stringify)?;
  let script = loaded.config.scripts.get(name).ok_or_else(|| unknown(name, &loaded.config))?;

  let Script::Command(command) = script;
  let request = ExecRequest {
    script_name: name,
    // `PLATFORM` is the same constant a config reads as `rune.platform`, so a config
    // branching by hand and a per-OS object cannot disagree about which system this is.
    command: command.command.select(PLATFORM),
    arguments,
    root: &loaded.discovered.root,
    package_dir: &loaded.discovered.package_dir,
    cwd: script.cwd(),
    env: script.env(),
  };

  rune_exec::run(&request).map_err(stringify)
}

/// The miss, what is available, and the likeliest correction.
///
/// All three matter: the name alone leaves the user guessing, and a suggestion alone
/// hides the rest of the config from someone who is new to it.
fn unknown(name: &str, config: &Config) -> String {
  let mut message = format!("no script named `{name}`");

  if let Some(closest) = closest_match(name, config) {
    let _ = write!(message, "\n\ndid you mean `{closest}`?");
  }

  if config.scripts.is_empty() {
    message.push_str("\n\nthis config defines no scripts");
    return message;
  }

  message.push_str("\n\nscripts defined here:");
  for defined in config.scripts.keys() {
    let _ = write!(message, "\n  {defined}");
  }

  message
}

fn closest_match<'a>(name: &str, config: &'a Config) -> Option<&'a str> {
  config
    .scripts
    .keys()
    .map(|defined| (strsim::levenshtein(name, defined), defined.as_str()))
    .filter(|(distance, _)| *distance < SUGGESTION_LIMIT)
    .min_by_key(|(distance, _)| *distance)
    .map(|(_, defined)| defined)
}

fn stringify(error: impl std::fmt::Display) -> String {
  error.to_string()
}

#[cfg(test)]
mod tests {
  use std::collections::BTreeMap;

  use rune_config::schema::{Command, CommandScript, Config, Script};

  use super::{closest_match, unknown};

  fn config(names: &[&str]) -> Config {
    Config {
      scripts: names
        .iter()
        .map(|name| {
          let script = CommandScript {
            command: Command::Everywhere("true".to_owned()),
            description: None,
            cwd: None,
            env: BTreeMap::new(),
          };
          ((*name).to_owned(), Script::Command(script))
        })
        .collect(),
    }
  }

  #[test]
  fn a_one_letter_typo_is_suggested() {
    assert_eq!(closest_match("biuld", &config(&["build", "test"])), Some("build"));
  }

  #[test]
  fn an_unrelated_name_gets_no_suggestion() {
    assert_eq!(closest_match("deploy", &config(&["build", "test"])), None);
  }

  #[test]
  fn the_message_names_the_miss_and_lists_what_exists() {
    let message = unknown("biuld", &config(&["build", "test"]));

    assert!(message.contains("`biuld`"), "{message}");
    assert!(message.contains("`build`"), "{message}");
    assert!(message.contains("test"), "{message}");
  }

  #[test]
  fn an_empty_config_says_so_rather_than_listing_nothing() {
    let message = unknown("build", &config(&[]));

    assert!(message.contains("defines no scripts"), "{message}");
  }
}
