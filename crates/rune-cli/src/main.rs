use std::process::ExitCode;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "rune", version, about = "Centralized script runner for JS/TS monorepos")]
struct Cli {
  #[command(subcommand)]
  command: Command,
}

#[derive(Subcommand)]
enum Command {
  /// Run a script by name
  Run { name: String },
  /// List all available scripts
  List,
  /// Show how a script resolves and where it comes from
  Inspect { name: String },
}

#[expect(clippy::print_stderr, reason = "temporary stubs until output goes through rune-out")]
fn main() -> ExitCode {
  let cli = Cli::parse();

  match cli.command {
    Command::Run { name } => eprintln!("run {name}: not implemented yet"),
    Command::List => eprintln!("rune rune list: not implemented yet"),
    Command::Inspect { name } => eprintln!("rune inspect {name}: not implemented yet"),
  }

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
