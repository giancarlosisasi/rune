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
use std::path::PathBuf;

use rune_config::compose::{self, Plan};
use rune_config::env::PLATFORM;
use rune_config::envfile::Files;
use rune_config::inherit::{Runs, Scope};
use rune_config::load::Loaded;
use rune_config::paths::relative_to;
use rune_config::schema::{self, SuccessPolicy};
use rune_exec::environment::FileLayer;
use rune_exec::{Completion, Directory, ExecRequest, Member, Step, Task};

use crate::script::{directory, env_files, load_here, unknown};

/// Resolves `name` and runs everything it stands for.
///
/// `arguments` are what the user typed after the script name. They go last, after
/// everything the configuration contributed along the extension chain, because the config
/// is the default and the command line is the override.
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
  let mut files = Files::default();
  let mut commands = Vec::new();
  prepare(&plan, &run, &mut files, &mut commands)?;

  let mut next = 0;
  let task = compose(&plan, &commands, &loaded, &mut next);

  rune_exec::run_task(&task).map_err(stringify)
}

/// What every command of one invocation shares.
struct Run<'a> {
  loaded: &'a Loaded,
  scope: Scope,
  /// The name the user typed, which is the only command the pass-through arguments
  /// reach. A prerequisite or a group member was not asked for by name, so it does not
  /// receive them.
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
  /// Already anchored on the config that declared it, and labelled with that config for
  /// the message when it cannot be entered.
  directory: PathBuf,
  declared_in: Option<String>,
  env: BTreeMap<String, String>,
  files: Vec<FileLayer>,
  interactive: bool,
  lifecycle: rune_exec::Lifecycle,
}

impl Prepared<'_> {
  fn request<'r>(&'r self, loaded: &'r Loaded) -> ExecRequest<'r> {
    ExecRequest {
      script_name: &self.script,
      command: self.command,
      arguments: &self.arguments,
      root: &loaded.discovered.root,
      package_dir: &loaded.discovered.package_dir,
      directory: Directory { path: &self.directory, declared_in: self.declared_in.as_deref() },
      env: &self.env,
      env_files: &self.files,
      interactive: self.interactive,
      lifecycle: self.lifecycle,
    }
  }
}

/// Resolves every command the plan can reach, in the order the plan holds them, and
/// reads every dotenv file they name.
///
/// This is what makes a missing file on the third member of a chain refuse before the
/// first member has run and had effects: the whole plan is prepared before any of it
/// starts. `files` is shared across the plan, so a file four members name is opened once.
fn prepare<'a>(
  plan: &Plan,
  run: &Run<'a>,
  files: &mut Files,
  into: &mut Vec<Prepared<'a>>,
) -> Result<(), String> {
  match plan {
    Plan::Command { script } => into.push(one(script, run, files)?),
    Plan::Serial { steps, .. } => {
      for step in steps {
        prepare(&step.plan, run, files, into)?;
      }
    }
    Plan::Parallel { members, .. } => {
      for member in members {
        prepare(&member.plan, run, files, into)?;
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
    Plan::Serial { script, steps, continue_on_error } => Task::Serial {
      script: script.clone(),
      steps: steps
        .iter()
        .map(|step| Step {
          name: step.name.clone(),
          role: role_of(step.role),
          task: compose(&step.plan, commands, loaded, next),
        })
        .collect(),
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

/// The same three roles, spelled for the crate that acts on them.
fn role_of(role: compose::Role) -> rune_exec::Role {
  match role {
    compose::Role::Member => rune_exec::Role::Member,
    compose::Role::Prerequisite => rune_exec::Role::Prerequisite,
    compose::Role::Own => rune_exec::Role::Own,
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

/// The lifecycle options a config declared, filled in with what rune does when it said
/// nothing. The defaults live in rune-exec, so the value a script omits and the value it
/// never could have written are the same value.
fn lifecycle_of(declared: schema::Lifecycle) -> rune_exec::Lifecycle {
  let default = rune_exec::Lifecycle::default();

  rune_exec::Lifecycle {
    timeout: declared.timeout,
    retries: declared.retries.unwrap_or(default.retries),
    retry_delay: declared.retry_delay.map_or(default.retry_delay, delay_of),
    kill: rune_exec::Kill {
      signal: declared.kill_signal.map_or(default.kill.signal, signal_of),
      timeout: declared.kill_timeout.unwrap_or(default.kill.timeout),
    },
  }
}

fn delay_of(delay: schema::RetryDelay) -> rune_exec::RetryDelay {
  match delay {
    schema::RetryDelay::Fixed(wait) => rune_exec::RetryDelay::Fixed(wait),
    schema::RetryDelay::Exponential => rune_exec::RetryDelay::Exponential,
  }
}

fn signal_of(signal: schema::KillSignal) -> rune_exec::KillSignal {
  match signal {
    schema::KillSignal::Hup => rune_exec::KillSignal::Hup,
    schema::KillSignal::Int => rune_exec::KillSignal::Int,
    schema::KillSignal::Quit => rune_exec::KillSignal::Quit,
    schema::KillSignal::Term => rune_exec::KillSignal::Term,
    schema::KillSignal::Kill => rune_exec::KillSignal::Kill,
  }
}

fn one<'a>(script: &str, run: &Run<'a>, files: &mut Files) -> Result<Prepared<'a>, String> {
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
  let discovered = &run.loaded.discovered;
  let interactive = resolved.interactive;
  let files = env_files(&resolved, script, run.loaded, files)?;

  let mut arguments = resolved.append_args;
  if script == run.entry {
    arguments.extend_from_slice(run.arguments);
  }

  Ok(Prepared {
    script: script.to_owned(),
    command,
    arguments,
    directory: directory(resolved.cwd, &discovered.package_dir),
    declared_in: resolved.cwd.map(|cwd| relative_to(&discovered.root, cwd.source)),
    env: resolved.env,
    files,
    interactive,
    lifecycle: lifecycle_of(resolved.lifecycle),
  })
}

/// Dropping the arguments silently is the failure worth avoiding: the group runs, looks
/// right, and quietly loses the flag the user asked for.
fn arguments_for_a_group(name: &str) -> String {
  format!(
    "`{name}` runs other scripts, so it has no command for these arguments to join\n\n\
     arguments go to one command. Name the member that needs them:\n  \
     rune run <member> ..."
  )
}

fn stringify(error: impl std::fmt::Display) -> String {
  error.to_string()
}
