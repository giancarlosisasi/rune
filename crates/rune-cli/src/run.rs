//! `rune run <name>`.
//!
//! A name may stand for one command, an ordered run, or a set of scripts running at once.
//! All three are resolved here into one task: every command the run could reach is looked
//! up before anything starts, so a typo in the third member of a chain is reported before
//! the first two have run and had effects.
//!
//! Resolving and running are kept apart on purpose. This module knows what a name means;
//! `rune-exec` knows how to make it happen. Neither has to know the other's job.

use std::collections::BTreeMap;
use std::path::Path;

use rune_config::compose::Plan;
use rune_config::env::PLATFORM;
use rune_config::inherit::{Runs, Scope};
use rune_config::load::Loaded;
use rune_config::schema::SuccessPolicy;
use rune_exec::environment::FileLayer;
use rune_exec::{Completion, ExecRequest, Member, Task};

use crate::script::{env_files, load_here, unknown};

/// Resolves `name` and runs everything it stands for.
///
/// `arguments` are what the user typed after `--`. They go last, after everything the
/// configuration contributed along the extension chain, because the config is the default
/// and the command line is the override.
pub fn run(name: &str, arguments: &[String], scope: Scope) -> Result<Completion, String> {
  let loaded = load_here()?;
  let resolved =
    loaded.resolve(name, scope).map_err(stringify)?.ok_or_else(|| unknown(name, &loaded, scope))?;

  if !arguments.is_empty() && !matches!(resolved.runs, Runs::Command(_)) {
    return Err(arguments_for_a_group(name));
  }

  let plan =
    loaded.plan(name, scope).map_err(stringify)?.ok_or_else(|| unknown(name, &loaded, scope))?;

  let run = Run { loaded: &loaded, scope, entry: name, arguments };
  let mut commands = Vec::new();
  prepare(&plan, &run, &mut commands)?;

  let mut next = 0;
  let task = compose(&plan, &commands, &loaded, &mut next);

  rune_exec::run_task(&task).map_err(stringify)
}

/// What every command of one invocation shares.
struct Run<'a> {
  loaded: &'a Loaded,
  scope: Scope,
  /// The name the user typed, which is the only command the arguments after `--` reach.
  /// A prerequisite or a group member was not asked for by name, so it does not receive
  /// them.
  entry: &'a str,
  arguments: &'a [String],
}

/// One command of a plan, resolved and owned so that a request can borrow it.
///
/// Everything a plan can reach is prepared before anything runs. A member that starts at
/// the same moment as its siblings has no useful place to report "no such script" from.
struct Prepared<'a> {
  script: String,
  command: &'a str,
  arguments: Vec<String>,
  cwd: Option<&'a str>,
  env: BTreeMap<String, String>,
  files: Vec<FileLayer<'a>>,
  interactive: bool,
}

impl Prepared<'_> {
  fn request<'r>(&'r self, loaded: &'r Loaded) -> ExecRequest<'r> {
    ExecRequest {
      script_name: &self.script,
      command: self.command,
      arguments: &self.arguments,
      root: &loaded.discovered.root,
      package_dir: &loaded.discovered.package_dir,
      cwd: self.cwd.map(Path::new),
      env: &self.env,
      env_files: &self.files,
      interactive: self.interactive,
    }
  }
}

/// Resolves every command the plan can reach, in the order the plan holds them.
fn prepare<'a>(plan: &Plan, run: &Run<'a>, into: &mut Vec<Prepared<'a>>) -> Result<(), String> {
  match plan {
    Plan::Command { script } => into.push(one(script, run)?),
    Plan::Serial { steps, .. } => {
      for step in steps {
        prepare(step, run, into)?;
      }
    }
    Plan::Parallel { members, .. } => {
      for member in members {
        prepare(&member.plan, run, into)?;
      }
    }
  }

  Ok(())
}

/// Turns the plan into the task rune-exec runs, drawing commands in the order `prepare`
/// resolved them.
fn compose<'a>(
  plan: &Plan,
  commands: &'a [Prepared<'_>],
  loaded: &'a Loaded,
  next: &mut usize,
) -> Task<'a> {
  match plan {
    Plan::Command { .. } => {
      let prepared = &commands[*next];
      *next += 1;
      Task::Command(prepared.request(loaded))
    }
    Plan::Serial { steps, continue_on_error } => Task::Serial {
      steps: steps.iter().map(|step| compose(step, commands, loaded, next)).collect(),
      continue_on_error: *continue_on_error,
    },
    Plan::Parallel { members, continue_on_error, policy } => Task::Parallel {
      members: members
        .iter()
        .map(|member| Member {
          name: member.name.clone(),
          task: compose(&member.plan, commands, loaded, next),
        })
        .collect(),
      continue_on_error: *continue_on_error,
      policy: policy_of(*policy),
    },
  }
}

/// The same three choices, spelled for the crate that acts on them. rune-config describes
/// what a config said; rune-exec decides what to do about it, and neither depends on the
/// other's vocabulary.
fn policy_of(policy: SuccessPolicy) -> rune_exec::SuccessPolicy {
  match policy {
    SuccessPolicy::All => rune_exec::SuccessPolicy::All,
    SuccessPolicy::First => rune_exec::SuccessPolicy::First,
    SuccessPolicy::Last => rune_exec::SuccessPolicy::Last,
  }
}

fn one<'a>(script: &str, run: &Run<'a>) -> Result<Prepared<'a>, String> {
  let resolved = run
    .loaded
    .resolve(script, run.scope)
    .map_err(stringify)?
    .ok_or_else(|| unknown(script, run.loaded, run.scope))?;

  let command = resolved
    .command()
    .expect("a plan names a script's own command, never a group")
    // `PLATFORM` is the same constant a config reads as `rune.platform`, so a config
    // branching by hand and a per-OS object cannot disagree about which system this is.
    .select(PLATFORM);
  let cwd = resolved.cwd;
  let interactive = resolved.interactive;
  let files = env_files(&resolved);

  let mut arguments = resolved.append_args;
  if script == run.entry {
    arguments.extend_from_slice(run.arguments);
  }

  Ok(Prepared {
    script: script.to_owned(),
    command,
    arguments,
    cwd,
    env: resolved.env,
    files,
    interactive,
  })
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
