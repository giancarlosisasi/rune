//! `rune inspect <name>` — what would happen, without making it happen.
//!
//! Inheritance is the first feature that makes a script name ambiguous: the answer now
//! depends on where the user is standing and how far the chain goes. This command is what
//! keeps that explainable, so it is a product surface rather than a debugging aid. A
//! confusing explanation is as severe a defect as a wrong execution, because from the
//! outside the two look the same.
//!
//! Nothing here spawns a process. Not the command, not the shell — the shell is only
//! consulted for its name, because that is what decides how arguments are quoted.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use rune_config::env::PLATFORM;
use rune_config::inherit::{Resolved, Scope};
use rune_config::load::Loaded;
use rune_config::paths::relative_to;
use rune_exec::quote::command_line;
use rune_exec::shell::{SHELL_VARIABLE, Shell};

use crate::script::{load_here, unknown};

/// Prints the resolution of `name`: what runs, where, with what, and how that was reached.
pub fn run(name: &str, scope: Scope) -> Result<(), String> {
  let loaded = load_here()?;
  let resolved = loaded
    .resolve(name, scope)
    .map_err(|error| error.to_string())?
    .ok_or_else(|| unknown(name, &loaded, scope))?;

  rune_out::line(&render(name, &resolved, &loaded));

  Ok(())
}

/// The width of the label column, so a value that spans lines stays under its own value.
const LABEL: usize = 11;

fn render(name: &str, resolved: &Resolved<'_>, loaded: &Loaded) -> String {
  let root = &loaded.discovered.root;
  let mut report = format!("{name}\n\n");

  let _ = writeln!(report, "{:<LABEL$}  {}", "command", assembled_command(resolved));
  let _ = writeln!(
    report,
    "{:<LABEL$}  {}",
    "directory",
    relative_to(root, &directory(resolved, loaded))
  );

  for (position, (key, value)) in resolved.env.iter().enumerate() {
    let label = if position == 0 { "environment" } else { "" };
    let _ = writeln!(report, "{label:<LABEL$}  {key}={value}");
  }

  report.push_str("\nresolved through\n");
  let sources: Vec<String> =
    resolved.chain.iter().map(|link| relative_to(root, link.source)).collect();
  let source_width = sources.iter().map(String::len).max().unwrap_or(0);
  let name_width = resolved.chain.iter().map(|link| link.name.len()).max().unwrap_or(0);

  for (link, source) in resolved.chain.iter().zip(&sources) {
    let contribution = if link.append_args.is_empty() {
      format!("runs `{}`", resolved.command.select(PLATFORM))
    } else {
      format!("appends `{}`", link.append_args.join(" "))
    };

    let _ =
      writeln!(report, "  {source:<source_width$}  {:<name_width$}  {contribution}", link.name);
  }

  report.trim_end().to_owned()
}

/// The command line the shell would be handed, quoted the way that shell reads it.
///
/// The shell is identified by name only. Locating it on disk is what `run` does, and
/// doing it here would make an explanation fail on a machine where the tool is missing.
fn assembled_command(resolved: &Resolved<'_>) -> String {
  let configured = std::env::var_os(SHELL_VARIABLE);
  let shell = Shell::select(configured.as_deref());

  command_line(resolved.command.select(PLATFORM), &resolved.append_args, shell.kind)
}

/// Where the script would run: the same rule `run` applies, so the two cannot disagree.
fn directory(resolved: &Resolved<'_>, loaded: &Loaded) -> PathBuf {
  let package_dir = &loaded.discovered.package_dir;

  match resolved.cwd.map(Path::new) {
    Some(cwd) if cwd.is_absolute() => cwd.to_path_buf(),
    Some(cwd) => package_dir.join(cwd),
    None => package_dir.clone(),
  }
}
