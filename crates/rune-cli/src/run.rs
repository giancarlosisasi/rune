//! `rune run <name>`.
//!
//! A name may stand for one command or for a whole ordered run. Both go through the same
//! loop here: resolve a plan, then execute its steps one at a time, with the child owning
//! the terminal throughout. One child at a time is what keeps the output of a chained
//! script indistinguishable from the output of running its command directly.

use std::path::Path;

use rune_config::compose::Plan;
use rune_config::env::PLATFORM;
use rune_config::inherit::{Runs, Scope};
use rune_config::load::Loaded;
use rune_exec::{Completion, ExecRequest};

use crate::script::{env_files, load_here, unknown};

/// Resolves `name` and runs everything it stands for, in order.
///
/// `arguments` are what the user typed after `--`. They go last, after everything the
/// configuration contributed along the extension chain, because the config is the default
/// and the command line is the override.
pub fn run(name: &str, arguments: &[String], scope: Scope) -> Result<Completion, String> {
  let loaded = load_here()?;
  let resolved =
    loaded.resolve(name, scope).map_err(stringify)?.ok_or_else(|| unknown(name, &loaded, scope))?;

  if !arguments.is_empty() && matches!(resolved.runs, Runs::Serial { .. }) {
    return Err(arguments_for_a_group(name));
  }

  let plan =
    loaded.plan(name, scope).map_err(stringify)?.ok_or_else(|| unknown(name, &loaded, scope))?;

  execute(&plan, &Run { loaded: &loaded, scope, entry: name, arguments })
}

/// What every step of one invocation shares.
struct Run<'a> {
  loaded: &'a Loaded,
  scope: Scope,
  /// The name the user typed, which is the only step the arguments after `--` reach.
  /// A prerequisite was not asked for by name, so it does not receive them.
  entry: &'a str,
  arguments: &'a [String],
}

fn execute(plan: &Plan, run: &Run<'_>) -> Result<Completion, String> {
  match plan {
    Plan::Command { script } => one(script, run),
    Plan::Serial { steps, continue_on_error } => sequence(steps, *continue_on_error, run),
  }
}

/// Runs `steps` one at a time and reports how the whole sequence ended.
fn sequence(steps: &[Plan], continue_on_error: bool, run: &Run<'_>) -> Result<Completion, String> {
  let mut failure = None;
  let mut caught = None;

  for step in steps {
    // Nothing new starts once a signal has arrived. The flag travels with the previous
    // step's result rather than in a global, so a caller cannot silently skip reading it.
    if caught.is_some() {
      break;
    }

    let completion = execute(step, run)?;
    caught = caught.or(completion.caught_signal);

    if completion.code != 0 {
      // The first failure's code, never the last. In a serial run first-in-time is also
      // first-in-list, so it is the failure the user reads at the top of the log.
      failure.get_or_insert(completion.code);

      if !continue_on_error {
        break;
      }
    }
  }

  Ok(Completion { code: failure.unwrap_or(0), caught_signal: caught })
}

fn one(script: &str, run: &Run<'_>) -> Result<Completion, String> {
  let resolved = run
    .loaded
    .resolve(script, run.scope)
    .map_err(stringify)?
    .ok_or_else(|| unknown(script, run.loaded, run.scope))?;

  let command = resolved.command().expect("a plan names a script's own command, never a group");

  let mut all_arguments = resolved.append_args.clone();
  if script == run.entry {
    all_arguments.extend_from_slice(run.arguments);
  }
  let files = env_files(&resolved);

  let request = ExecRequest {
    script_name: script,
    // `PLATFORM` is the same constant a config reads as `rune.platform`, so a config
    // branching by hand and a per-OS object cannot disagree about which system this is.
    command: command.select(PLATFORM),
    arguments: &all_arguments,
    root: &run.loaded.discovered.root,
    package_dir: &run.loaded.discovered.package_dir,
    cwd: resolved.cwd.map(Path::new),
    env: &resolved.env,
    env_files: &files,
  };

  rune_exec::run(&request).map_err(stringify)
}

/// Dropping the arguments silently is the failure worth avoiding: the group runs, looks
/// right, and quietly loses the flag the user asked for.
fn arguments_for_a_group(name: &str) -> String {
  format!(
    "`{name}` runs other scripts, so it has no command for these arguments to join\n\n\
     arguments after `--` go to one command. Name the member that needs them:\n  \
     rune run <member> -- ..."
  )
}

fn stringify(error: impl std::fmt::Display) -> String {
  error.to_string()
}
