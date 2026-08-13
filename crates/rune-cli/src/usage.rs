//! The message for an argument a command does not accept.
//!
//! The parser decides what is valid; it is only the wording that moves here. A parser is
//! asked about one subcommand at a time, which is why it can say `'--roo' is close to
//! '--root'` inside `run` and cannot say `'--root' works on rune run` from inside `init`.
//! Rune can ask about all of them, so it writes this one message itself.
//!
//! Everything on screen is read off the grammar. A sixth command, or a second flag, cannot
//! arrive without this message covering it.

use std::ffi::OsString;
use std::fmt::Write as _;

use clap::error::{ContextKind, ContextValue};
use clap::{Arg, Command};

/// Rune's words for an argument `raw` typed at a command that does not take it.
///
/// `None` when the failure carries no argument to name, which leaves it to the parser.
pub fn unknown_argument(root: &Command, raw: &[OsString], error: &clap::Error) -> Option<String> {
  let refused = context(error, ContextKind::InvalidArg)?;
  let (command, path) = refusing(root, raw);

  let mut message = format!("`{refused}` is not an option of `{path}`");

  if let Some(similar) = context(error, ContextKind::SuggestedArg) {
    let _ = write!(message, "\n\ndid you mean `{similar}`?");
  }

  let options = options_of(command);
  if options.is_empty() {
    let _ = write!(message, "\n\n`{path}` takes no options of its own.");
  } else {
    let _ = write!(message, "\n\n`{path}` accepts:");
    let width = options.iter().map(|(spelling, _)| spelling.len()).max().unwrap_or(0);
    for (spelling, description) in &options {
      let _ = write!(message, "\n  {spelling:<width$}  {description}");
    }
  }

  let elsewhere = commands_accepting(root, &refused);
  if !elsewhere.is_empty() {
    let _ = write!(message, "\n\n`{refused}` works on:");
    for command in elsewhere {
      let _ = write!(message, "\n  {command}");
    }
  }

  Some(message)
}

/// One piece of what the parser recorded about the failure, when it is a single string.
fn context(error: &clap::Error, kind: ContextKind) -> Option<String> {
  match error.get(kind)? {
    ContextValue::String(value) => Some(value.clone()),
    ContextValue::Strings(values) => values.first().cloned(),
    _ => None,
  }
}

/// The command the parser was refusing for, found by walking the grammar rather than by
/// guessing which token was the subcommand.
fn refusing<'a>(root: &'a Command, raw: &[OsString]) -> (&'a Command, String) {
  let mut current = root;
  let mut path = root.get_name().to_owned();

  for token in raw.iter().skip(1) {
    let Some(word) = token.to_str() else { break };
    let Some(next) = current.find_subcommand(word) else { break };

    current = next;
    path.push(' ');
    path.push_str(word);
  }

  (current, path)
}

/// The options a command declares, with what its own help says they do.
///
/// `--help` and `--version` are the parser's, offered by every command alike, so listing
/// them would say nothing about the one that just refused an argument.
fn options_of(command: &Command) -> Vec<(String, String)> {
  command
    .get_arguments()
    .filter(|argument| !matches!(argument.get_id().as_str(), "help" | "version"))
    .filter_map(|argument| {
      let spelling = format!("--{}", argument.get_long()?);
      let description = argument.get_help().map(ToString::to_string).unwrap_or_default();

      Some((spelling, description))
    })
    .collect()
}

/// Every command whose grammar defines an option of that spelling, named the way a user
/// types it.
pub fn commands_accepting(root: &Command, spelling: &str) -> Vec<String> {
  let mut found = Vec::new();
  gather(root, root.get_name(), spelling, &mut found);

  found
}

fn gather(command: &Command, path: &str, spelling: &str, found: &mut Vec<String>) {
  if command.get_arguments().any(|argument| spells(argument, spelling)) {
    found.push(path.to_owned());
  }

  for subcommand in command.get_subcommands() {
    gather(subcommand, &format!("{path} {}", subcommand.get_name()), spelling, found);
  }
}

fn spells(argument: &Arg, spelling: &str) -> bool {
  let long = argument.get_long().is_some_and(|long| format!("--{long}") == spelling);
  let short = argument.get_short().is_some_and(|short| format!("-{short}") == spelling);

  long || short
}

#[cfg(test)]
mod tests {
  use clap::{Arg, ArgAction, Command};

  use super::commands_accepting;

  /// A grammar with the shape rune's has: one option two commands share, one only a
  /// nested command carries, and a spelling nothing defines.
  fn grammar() -> Command {
    let shared = || Arg::new("root").long("root").action(ArgAction::SetTrue);

    Command::new("rune")
      .subcommand(Command::new("run").arg(shared()))
      .subcommand(Command::new("inspect").arg(shared()))
      .subcommand(Command::new("list"))
      .subcommand(Command::new("cache").subcommand(
        Command::new("clear").arg(Arg::new("all").long("all").action(ArgAction::SetTrue)),
      ))
  }

  /// Test R14.8 — the answer comes off the grammar, so an option added to a sixth command
  /// is covered without this code being told about it.
  #[test]
  fn the_commands_accepting_an_option_are_read_from_the_grammar() {
    assert_eq!(commands_accepting(&grammar(), "--root"), ["rune run", "rune inspect"]);
  }

  #[test]
  fn a_nested_command_is_named_by_its_whole_path() {
    assert_eq!(commands_accepting(&grammar(), "--all"), ["rune cache clear"]);
  }

  /// Nothing to say rather than something wrong: a spelling no command defines produces
  /// no "works on" block at all.
  #[test]
  fn a_spelling_nothing_defines_finds_nothing() {
    assert!(commands_accepting(&grammar(), "--nonsense").is_empty());
  }
}
