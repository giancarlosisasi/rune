use std::ffi::OsString;
use std::process::ExitCode;

use clap::{Arg, CommandFactory, Parser, Subcommand};
use rune_config::inherit::Scope;

mod init;
mod inspect;
mod list;
mod run;
mod script;
mod version;

#[derive(Parser)]
#[command(
  name = "rune",
  // Without this, usage is spelled from the file on disk and Windows users are told to
  // run `rune.exe`. Nobody types that, and it would make the help text differ per
  // operating system.
  bin_name = "rune",
  version = version::VERSION,
  about = "Centralized script runner for JS/TS monorepos"
)]
struct Cli {
  #[command(subcommand)]
  command: Command,
}

#[derive(Subcommand)]
enum Command {
  /// Run a script by name
  ///
  /// rune's own options come before the script name. Everything after the name is
  /// appended to the script's command, whether or not a `--` separates it.
  Run {
    /// The script to run
    name: String,
    /// Resolve against the root config only, ignoring this package's definitions
    #[arg(long)]
    root: bool,
    /// Arguments appended to the script's command
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    args: Vec<String>,
  },
  /// List all available scripts
  List,
  /// Show how a script resolves and where it comes from
  Inspect {
    /// The script to explain
    name: String,
    /// Resolve against the root config only, ignoring this package's definitions
    #[arg(long)]
    root: bool,
  },
  /// Write a starter rune.config.ts beside the nearest package.json
  ///
  /// The nearest package.json at or above this directory decides where the config goes,
  /// so running from a subfolder still writes it where the rest of rune can see it. With
  /// no package.json above, it is written here.
  Init {
    /// Seed the starter with the scripts from that same package.json
    #[arg(long)]
    from_package_json: bool,
  },
  /// Manage the resolved-config cache
  Cache {
    #[command(subcommand)]
    command: CacheCommand,
  },
}

/// "Resolve as if you were standing at the root" is the whole of what `--root` means.
fn scope(root_only: bool) -> Scope {
  if root_only { Scope::Root } else { Scope::Nearest }
}

#[derive(Subcommand)]
enum CacheCommand {
  /// Remove every cached config result
  Clear,
}

fn main() -> ExitCode {
  let cli = Cli::parse_from(with_the_boundary_marked(std::env::args_os().collect()));

  match cli.command {
    // The child's exit code is the product of this subcommand, so it does not go through
    // the success-or-diagnostic path the others share.
    Command::Run { name, root, args } => {
      note_options_bound_for_the_command(&name, &args);

      match run::run(&name, &args, scope(root)) {
        Ok(completion) => completion.exit_code(),
        Err(message) => fail(&message),
      }
    }
    Command::List => report(list::run()),
    Command::Init { from_package_json } => report(init::run(from_package_json)),
    Command::Cache { command: CacheCommand::Clear } => report(list::clear_cache()),
    Command::Inspect { name, root } => report(inspect::run(&name, scope(root))),
  }
}

/// Writes the boundary between rune's options and the command's arguments into `raw`, as
/// the `--` a user is no longer required to type.
///
/// The rule is positional: rune's options come before the script name and everything
/// after it belongs to the command. A parser cannot express that on its own — it knows
/// `--root` and reads it wherever it appears, which is how `rune run build --root` would
/// silently resolve against the root config instead of passing the flag on. Marking the
/// boundary before parsing is what makes the position decide.
fn with_the_boundary_marked(mut raw: Vec<OsString>) -> Vec<OsString> {
  let Some(subcommand) = first_word(&raw, 1) else { return raw };
  if raw[subcommand] != *"run" {
    return raw;
  }

  let Some(script) = first_word(&raw, subcommand + 1) else { return raw };

  let boundary = script + 1;
  if raw.get(boundary).is_none_or(|token| token != "--") {
    raw.insert(boundary, OsString::from("--"));
  }

  raw
}

/// Where the first token that is not option-shaped sits, at or after `from`.
///
/// Every option `run` accepts is a flag carrying no value of its own, so skipping the
/// option-shaped tokens lands on a word the grammar wants. `run_options_are_all_flags`
/// is what fails if that ever stops being true.
fn first_word(raw: &[OsString], from: usize) -> Option<usize> {
  raw
    .iter()
    .enumerate()
    .skip(from)
    .find(|(_, token)| !token.to_str().is_some_and(|token| token.starts_with('-')))
    .map(|(at, _)| at)
}

/// Says so when a token after the script name spells one of rune's own options.
///
/// The boundary is the script name, so such a token is the command's. That is the one
/// case the rule can surprise somebody, and a line on stderr costs less than a user
/// wondering why the option did nothing.
fn note_options_bound_for_the_command(name: &str, args: &[String]) {
  let taken = options_rune_would_have_read(args);
  if taken.is_empty() {
    return;
  }

  let quoted: Vec<String> = taken.iter().map(|option| format!("`{option}`")).collect();
  rune_out::diagnostic(&format!(
    "warning: {} went to the command, not to rune\n\n\
     rune's own options come before the script name:\n  rune run {} {name}",
    quoted.join(", "),
    taken.join(" ")
  ));
}

/// Which of `args` the parser would have read as rune's own, had they come first.
///
/// Read off the parser rather than listed by hand, so a new option cannot arrive without
/// this warning learning about it.
fn options_rune_would_have_read(args: &[String]) -> Vec<&str> {
  if !args.iter().any(|argument| argument.starts_with('-')) {
    return Vec::new();
  }

  let cli = Cli::command();
  let run = cli.find_subcommand("run").expect("`run` is one of rune's subcommands");
  let spellings: Vec<String> = run.get_arguments().flat_map(spellings_of).collect();

  args
    .iter()
    .map(String::as_str)
    .filter(|argument| spellings.iter().any(|spelling| spelling == *argument))
    .collect()
}

fn spellings_of(argument: &Arg) -> Vec<String> {
  argument
    .get_long()
    .map(|long| format!("--{long}"))
    .into_iter()
    .chain(argument.get_short().map(|short| format!("-{short}")))
    .collect()
}

fn report(outcome: Result<(), String>) -> ExitCode {
  match outcome {
    Ok(()) => ExitCode::SUCCESS,
    Err(message) => fail(&message),
  }
}

fn fail(message: &str) -> ExitCode {
  // Never stdout: a diagnostic there would be indistinguishable from a script's own
  // output, and stdout belongs to the child.
  rune_out::diagnostic(message);
  ExitCode::FAILURE
}

#[cfg(test)]
mod tests {
  use super::{Cli, with_the_boundary_marked};
  use clap::CommandFactory;

  #[test]
  fn cli_grammar_is_valid() {
    Cli::command().debug_assert();
  }

  #[test]
  fn the_version_clap_reports_is_the_one_embedded_from_the_version_file() {
    let cmd = Cli::command();

    assert_eq!(cmd.get_version(), Some(crate::version::VERSION));
  }

  /// The boundary scanner walks past rune's own options to find the script name, which
  /// only works while none of them takes a value of its own. The day one does, the scanner
  /// has to learn where that value ends — and this is what says so.
  #[test]
  fn run_options_are_all_flags() {
    let cli = Cli::command();
    let run = cli.find_subcommand("run").expect("`run` is one of rune's subcommands");

    let options = run.get_arguments().filter(|argument| argument.get_long().is_some());

    for option in options {
      assert!(
        !option.get_action().takes_values(),
        "`--{}` takes a value, so `first_word` can no longer skip it blindly",
        option.get_long().expect("filtered to the options")
      );
    }
  }

  fn marked(raw: &[&str]) -> Vec<String> {
    with_the_boundary_marked(raw.iter().map(std::ffi::OsString::from).collect())
      .iter()
      .map(|token| token.to_string_lossy().into_owned())
      .collect()
  }

  #[test]
  fn the_boundary_goes_after_the_script_name() {
    assert_eq!(marked(&["rune", "run", "build"]), ["rune", "run", "build", "--"]);
    assert_eq!(
      marked(&["rune", "run", "build", "--root"]),
      ["rune", "run", "build", "--", "--root"]
    );
    assert_eq!(
      marked(&["rune", "run", "--root", "build", "-w"]),
      ["rune", "run", "--root", "build", "--", "-w"]
    );
  }

  /// A separator the user typed is the boundary. Adding a second one would turn it into
  /// an argument the command never asked for.
  #[test]
  fn a_separator_already_there_is_left_alone() {
    assert_eq!(marked(&["rune", "run", "build", "--", "-w"]), ["rune", "run", "build", "--", "-w"]);
    assert_eq!(
      marked(&["rune", "run", "build", "--", "--", "-w"]),
      ["rune", "run", "build", "--", "--", "-w"]
    );
  }

  /// Nothing to mark, and nothing to guess at: the parser reports the missing name and
  /// prints the help itself.
  #[test]
  fn a_run_with_no_script_name_is_left_for_the_parser() {
    assert_eq!(marked(&["rune", "run"]), ["rune", "run"]);
    assert_eq!(marked(&["rune", "run", "--help"]), ["rune", "run", "--help"]);
  }

  #[test]
  fn other_subcommands_are_untouched() {
    assert_eq!(marked(&["rune", "inspect", "build"]), ["rune", "inspect", "build"]);
    assert_eq!(marked(&["rune", "--version"]), ["rune", "--version"]);
  }
}
