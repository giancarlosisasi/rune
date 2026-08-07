use std::process::ExitCode;

use clap::{Parser, Subcommand};
use rune_config::inherit::Scope;

mod inspect;
mod list;
mod run;
mod script;

#[derive(Parser)]
#[command(name = "rune", version, about = "Centralized script runner for JS/TS monorepos")]
struct Cli {
  #[command(subcommand)]
  command: Command,
}

#[derive(Subcommand)]
enum Command {
  /// Run a script by name
  Run {
    name: String,
    /// Resolve against the root config only, ignoring this package's definitions
    #[arg(long)]
    root: bool,
    /// Arguments appended to the script's command, after `--`
    #[arg(last = true)]
    args: Vec<String>,
  },
  /// List all available scripts
  List,
  /// Show how a script resolves and where it comes from
  Inspect {
    name: String,
    /// Resolve against the root config only, ignoring this package's definitions
    #[arg(long)]
    root: bool,
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
  let cli = Cli::parse();

  match cli.command {
    // The child's exit code is the product of this subcommand, so it does not go through
    // the success-or-diagnostic path the others share.
    Command::Run { name, root, args } => match run::run(&name, &args, scope(root)) {
      Ok(completion) => completion.exit_code(),
      Err(message) => fail(&message),
    },
    Command::List => report(list::run()),
    Command::Cache { command: CacheCommand::Clear } => report(list::clear_cache()),
    Command::Inspect { name, root } => report(inspect::run(&name, scope(root))),
  }
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
  use super::Cli;
  use clap::CommandFactory;

  #[test]
  fn cli_grammar_is_valid() {
    Cli::command().debug_assert();
  }

  #[test]
  fn version_is_three_numbers() {
    let cmd = Cli::command();
    let version = cmd.get_version().expect("version is set in #[command]");

    let parts: Vec<&str> = version.split('.').collect();
    assert_eq!(parts.len(), 3, "expected MAJOR.MINOR.PATCH, got {version}");

    for part in parts {
      assert!(part.parse::<u32>().is_ok(), "segment `{part}` is not a number");
    }
  }
}
