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
use std::path::PathBuf;

use rune_config::env::PLATFORM;
use rune_config::envfile::Files;
use rune_config::inherit::{Link, Resolved, Runs, Scope};
use rune_config::load::Loaded;
use rune_config::paths::relative_to;
use rune_config::schema::{Command, KillSignal, Lifecycle, RetryDelay, SuccessPolicy};
use rune_exec::environment::{self, ChildEnvironment, Descriptor, FileLayer, Layering};
use rune_exec::quote::{self, command_line};
use rune_exec::shell::{SHELL_VARIABLE, Shell};

use crate::script::{self, env_files, load_here, unknown};

/// Prints the resolution of `name`: what runs, where, with what, and how that was reached.
pub fn run(name: &str, scope: Scope) -> Result<(), String> {
  let loaded = load_here()?;
  let resolved = loaded
    .resolve(name, scope)
    .map_err(|error| error.to_string())?
    .ok_or_else(|| unknown(name, &loaded, scope))?;

  // Read before anything is rendered: a report that describes an environment built from a
  // file rune could not open would be an explanation of a run that cannot happen.
  let files = env_files(&resolved, name, &loaded, &mut Files::default())?;

  rune_out::line(&render(name, &resolved, &loaded, &files));

  Ok(())
}

/// The width of the label column, so a value that spans lines stays under its own value.
const LABEL: usize = 11;

fn render(name: &str, resolved: &Resolved<'_>, loaded: &Loaded, files: &[FileLayer]) -> String {
  let root = &loaded.discovered.root;
  let mut report = format!("{name}\n\n");

  // Before the command line, because the command line's escaping depends on which child
  // the script's own `PATH` resolves to.
  let layering = layered(name, resolved, loaded, files);

  match resolved.runs {
    Runs::Command(command) => {
      let line = assembled_command(resolved, command, &layering.environment);
      let _ = writeln!(report, "{:<LABEL$}  {}", "command", line);
    }
    Runs::Serial { members, continue_on_error } => {
      let _ = writeln!(report, "{:<LABEL$}  {}", "runs", members.join(" → "));
      if continue_on_error {
        let _ = writeln!(report, "{:<LABEL$}  keeps going, then reports the first", "on failure");
      }
    }
    Runs::Parallel { members, continue_on_error, policy } => {
      let _ = writeln!(report, "{:<LABEL$}  {}", "all at once", members.join(", "));
      let _ = writeln!(report, "{:<LABEL$}  {}", "succeeds if", succeeds_if(policy));
      if continue_on_error {
        let _ = writeln!(report, "{:<LABEL$}  lets the others finish, then reports", "on failure");
      }
    }
  }

  if !resolved.depends_on.is_empty() {
    let _ = writeln!(report, "{:<LABEL$}  {}", "runs first", resolved.depends_on.join(" → "));
  }

  // A retry that nothing announces is how a deterministic failure hides for a month. The
  // configured options are printed as what they do, not as the words that set them.
  for (position, line) in lifecycle(&resolved.lifecycle).iter().enumerate() {
    let label = if position == 0 { "lifecycle" } else { "" };
    let _ = writeln!(report, "{label:<LABEL$}  {line}");
  }

  let _ = writeln!(
    report,
    "{:<LABEL$}  {}",
    "directory",
    relative_to(root, &directory(resolved, loaded))
  );

  for (position, (key, value)) in layering.applied.iter().enumerate() {
    let label = if position == 0 { "environment" } else { "" };
    let _ = writeln!(report, "{label:<LABEL$}  {key}={value}");
  }

  // Separately, and always: a delta showing only what got through cannot tell a user that
  // the file they just edited had no effect.
  for (position, ignored) in layering.ignored.iter().enumerate() {
    let label = if position == 0 { "ignored" } else { "" };
    let _ = writeln!(report, "{label:<LABEL$}  {ignored}");
  }

  report.push_str("\nresolved through\n");
  let sources: Vec<String> =
    resolved.chain.iter().map(|link| relative_to(root, link.source)).collect();
  let source_width = sources.iter().map(String::len).max().unwrap_or(0);
  let name_width = resolved.chain.iter().map(|link| link.name.len()).max().unwrap_or(0);

  for (link, source) in resolved.chain.iter().zip(&sources) {
    let contribution = contribution(link, resolved);

    let _ =
      writeln!(report, "  {source:<source_width$}  {:<name_width$}  {contribution}", link.name);
  }

  report.trim_end().to_owned()
}

/// What one step of the chain put into the answer.
///
/// A link that appends arguments says so; the one at the base is the one that decides what
/// actually runs, which is a command for most scripts and a member list for a group.
fn contribution(link: &Link<'_>, resolved: &Resolved<'_>) -> String {
  if !link.append_args.is_empty() {
    return format!("appends `{}`", link.append_args.join(" "));
  }

  match resolved.runs {
    Runs::Command(command) => format!("runs `{}`", command.select(PLATFORM)),
    Runs::Serial { members, .. } => format!("runs {}", members.join(", ")),
    Runs::Parallel { members, .. } => format!("runs {} at once", members.join(", ")),
  }
}

/// The lifecycle options a script declared, one line each, and nothing for the ones it
/// left alone.
///
/// Defaults are deliberately absent: a report that listed `SIGTERM after 5000 ms` for
/// every script would bury the one line that was a decision under four that were not.
fn lifecycle(declared: &Lifecycle) -> Vec<String> {
  let mut lines = Vec::new();

  if let Some(timeout) = declared.timeout {
    lines.push(format!("ends the whole tree after {} ms", timeout.as_millis()));
  }

  if let Some(retries) = declared.retries {
    let wait = match declared.retry_delay {
      None => String::new(),
      Some(RetryDelay::Fixed(delay)) => format!(", waiting {} ms between them", delay.as_millis()),
      Some(RetryDelay::Exponential) => ", waiting 2^attempt seconds between them".to_owned(),
    };
    lines.push(format!("retries {retries} more {} on failure{wait}", plural(retries, "time")));
  }

  if let Some(signal) = declared.kill_signal {
    lines.push(format!("ends with {}", named(signal)));
  }

  if let Some(timeout) = declared.kill_timeout {
    lines.push(format!("waits {} ms before SIGKILL", timeout.as_millis()));
  }

  lines
}

fn plural(count: u32, word: &str) -> String {
  if count == 1 { word.to_owned() } else { format!("{word}s") }
}

/// The signal spelled the way the config writes it.
fn named(signal: KillSignal) -> &'static str {
  match signal {
    KillSignal::Hup => "SIGHUP",
    KillSignal::Int => "SIGINT",
    KillSignal::Quit => "SIGQUIT",
    KillSignal::Term => "SIGTERM",
    KillSignal::Kill => "SIGKILL",
  }
}

/// What the success policy means, said rather than named. `first` is the value a user
/// writes; "the member that finishes first" is what they need to know it does.
fn succeeds_if(policy: SuccessPolicy) -> &'static str {
  match policy {
    SuccessPolicy::All => "every member succeeds",
    SuccessPolicy::First => "the member that finishes first succeeds",
    SuccessPolicy::Last => "the member that finishes last succeeds",
  }
}

/// The environment `run` would build, worked out without running anything.
///
/// The same function `run` uses, against the same process environment, so what the report
/// calls ignored is exactly what the run would drop.
fn layered(name: &str, resolved: &Resolved<'_>, loaded: &Loaded, files: &[FileLayer]) -> Layering {
  environment::build(
    std::env::vars_os(),
    &Descriptor {
      script_name: name,
      root: &loaded.discovered.root,
      package_dir: &loaded.discovered.package_dir,
      env: &resolved.env,
      env_files: files,
    },
  )
}

/// The command line the shell would be handed, quoted the way that shell reads it.
///
/// The shell is identified by name only. Locating it on disk is what `run` does, and
/// doing it here would make an explanation fail on a machine where the tool is missing.
/// The child is looked for, because how many times its arguments get read decides how they
/// are escaped — but a child that cannot be found only means the explanation says what a
/// single reader would receive, which is what a run would then do too.
fn assembled_command(
  resolved: &Resolved<'_>,
  command: &Command,
  environment: &ChildEnvironment,
) -> String {
  let configured = std::env::var_os(SHELL_VARIABLE);
  let shell = Shell::select(configured.as_deref());
  let selected = command.select(PLATFORM);
  let reads =
    quote::reads(selected, shell.kind, environment.get("PATH"), environment.get("PATHEXT"));

  command_line(selected, &resolved.append_args, shell.kind, reads)
}

/// Where the script would run: literally the function `run` calls, so the explanation and
/// the run cannot describe two different directories.
fn directory(resolved: &Resolved<'_>, loaded: &Loaded) -> PathBuf {
  script::directory(resolved.cwd, &loaded.discovered.package_dir)
}
