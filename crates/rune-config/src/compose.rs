//! Turning one script name into the ordered list of runs it stands for.
//!
//! Two config shapes reduce to the same thing. `dependsOn` attaches an ordered list to a
//! script that also has a command of its own; `serial` is a script that is *only* a list.
//! Both come out of here as one [`Plan`], so a group whose member brings its own
//! prerequisites needs no second mechanism to express it.
//!
//! Building a plan is also what validates one. Every name a config mentions is checked
//! here, when the config loads, rather than when the group runs: a typo in the third
//! member of a chain must not surface after the first two have already run and had
//! effects.

use std::collections::BTreeSet;

use thiserror::Error;

use crate::inherit::{self, InheritError, Layer, Resolved, Runs, Scope};
use crate::schema::SuccessPolicy;
use crate::suggest::{closest, did_you_mean};

#[derive(Debug, Error)]
pub enum ComposeError {
  #[error(transparent)]
  Inherit(#[from] InheritError),

  #[error(
    "script `{script}` names `{member}` as a member, which no script defines{}",
    did_you_mean(.suggestion.as_deref())
  )]
  UnknownMember { script: String, member: String, suggestion: Option<String> },

  #[error(
    "script `{script}` depends on `{prerequisite}`, which no script defines{}",
    did_you_mean(.suggestion.as_deref())
  )]
  UnknownPrerequisite { script: String, prerequisite: String, suggestion: Option<String> },

  #[error(
    "scripts run in a circle:\n\n  {path}\n\n\
     a script cannot be part of what has to finish before it starts."
  )]
  Cycle { path: String },
}

/// A run, flattened from however many groups and prerequisites produced it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Plan {
  /// Runs this script's own command. Its prerequisites, when it has any, are steps of
  /// their own beside it rather than something hidden inside it.
  Command { script: String },
  /// Runs each step to completion, in order.
  Serial {
    steps: Vec<Plan>,
    /// Runs every step even after one fails. The run still ends with the first failure's
    /// exit code.
    continue_on_error: bool,
  },
  /// Starts every member at once and waits for all of them.
  Parallel {
    members: Vec<Member>,
    /// Lets a failing member's siblings run to their own completion. The group still
    /// ends non-zero.
    continue_on_error: bool,
    policy: SuccessPolicy,
  },
}

/// One member of a parallel group.
///
/// The name travels with the plan because it is what the member's output is labelled
/// with. A nested group has no name of its own, so without this the label would have to
/// be invented at the moment it was needed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
  pub name: String,
  pub plan: Plan,
}

/// What running `name` would do, or `None` when no config in scope defines it.
///
/// `None` is not an error here for the same reason it is not one in `resolve`: the caller
/// knows which names it can offer instead and says so in its own words.
pub fn plan(layers: &[Layer], name: &str, scope: Scope) -> Result<Option<Plan>, ComposeError> {
  let Some(resolved) = inherit::resolve(layers, name, scope)? else {
    return Ok(None);
  };

  build(layers, scope, name, &resolved, &mut Vec::new()).map(Some)
}

/// Checks every reference every script in scope makes.
///
/// Planning is the check: a name that cannot be planned cannot be run, and doing it for
/// all of them means a broken reference is reported to whoever wrote it rather than to
/// whoever happens to run that one script first.
pub fn validate(layers: &[Layer], scope: Scope) -> Result<(), ComposeError> {
  for name in inherit::names(layers, scope).collect::<BTreeSet<_>>() {
    plan(layers, name, scope)?;
  }

  Ok(())
}

/// Which kind of reference a name was reached through. Only the wording differs.
#[derive(Debug, Clone, Copy)]
enum Edge {
  Member,
  Prerequisite,
}

impl Edge {
  fn unknown(self, layers: &[Layer], scope: Scope, script: &str, reference: &str) -> ComposeError {
    let suggestion = closest(reference, inherit::names(layers, scope)).map(str::to_owned);

    match self {
      Self::Member => ComposeError::UnknownMember {
        script: script.to_owned(),
        member: reference.to_owned(),
        suggestion,
      },
      Self::Prerequisite => ComposeError::UnknownPrerequisite {
        script: script.to_owned(),
        prerequisite: reference.to_owned(),
        suggestion,
      },
    }
  }
}

fn build(
  layers: &[Layer],
  scope: Scope,
  name: &str,
  resolved: &Resolved<'_>,
  path: &mut Vec<String>,
) -> Result<Plan, ComposeError> {
  path.push(name.to_owned());
  let plan = assemble(layers, scope, name, resolved, path);
  path.pop();

  plan
}

fn assemble(
  layers: &[Layer],
  scope: Scope,
  name: &str,
  resolved: &Resolved<'_>,
  path: &mut Vec<String>,
) -> Result<Plan, ComposeError> {
  let mut steps = Vec::new();
  for prerequisite in resolved.depends_on {
    steps.push(step(layers, scope, name, Edge::Prerequisite, prerequisite, path)?);
  }

  match resolved.runs {
    Runs::Command(_) => {
      let own = Plan::Command { script: name.to_owned() };
      if steps.is_empty() {
        return Ok(own);
      }

      // Prerequisites first, then the command they were declared for. Stopping at the
      // first failure is not negotiable here: the whole point of a prerequisite is that
      // what follows it is not worth running if it failed.
      steps.push(own);
      Ok(Plan::Serial { steps, continue_on_error: false })
    }
    Runs::Serial { members, continue_on_error } => {
      // A group carries no prerequisites of its own — the schema refuses them, because a
      // group runs the scripts it names and nothing before them.
      for member in members {
        steps.push(step(layers, scope, name, Edge::Member, member, path)?);
      }

      Ok(Plan::Serial { steps, continue_on_error })
    }
    Runs::Parallel { members, continue_on_error, policy } => {
      let members = members
        .iter()
        .map(|member| {
          let plan = step(layers, scope, name, Edge::Member, member, path)?;
          Ok(Member { name: member.clone(), plan })
        })
        .collect::<Result<Vec<_>, ComposeError>>()?;

      Ok(Plan::Parallel { members, continue_on_error, policy })
    }
  }
}

/// One name another script reached: checked for a circle, checked for existence, planned.
fn step(
  layers: &[Layer],
  scope: Scope,
  script: &str,
  edge: Edge,
  reference: &str,
  path: &mut Vec<String>,
) -> Result<Plan, ComposeError> {
  if path.iter().any(|seen| seen == reference) {
    return Err(ComposeError::Cycle { path: render(path, reference) });
  }

  let Some(resolved) = inherit::resolve(layers, reference, scope)? else {
    return Err(edge.unknown(layers, scope, script, reference));
  };

  build(layers, scope, reference, &resolved, path)
}

/// The walk so far, then the name it came back to: `a → b → a`.
///
/// The path is the debugging information. "cycle detected" leaves the user rebuilding
/// their own config graph by hand to find out which scripts were involved.
fn render(path: &[String], repeated: &str) -> String {
  let mut rendered = String::new();
  for name in path {
    rendered.push_str(name);
    rendered.push_str(" → ");
  }
  rendered.push_str(repeated);

  rendered
}

#[cfg(test)]
mod tests {
  use std::path::{Path, PathBuf};

  use super::{ComposeError, Plan, plan, validate};
  use crate::inherit::{Layer, Scope};
  use crate::schema::parse;

  fn layers(scripts: &str) -> Vec<Layer> {
    let value: serde_json::Value =
      serde_json::from_str(&format!("{{ \"scripts\": {scripts} }}")).expect("valid fixture JSON");

    vec![
      Layer::new(
        Path::new(""),
        PathBuf::from("rune.config.ts"),
        parse(&value).expect("fixture parses"),
      )
      .expect("no fixture here declares an envFile"),
    ]
  }

  fn planned(scripts: &str, name: &str) -> Plan {
    plan(&layers(scripts), name, Scope::Nearest).expect("plans").expect("defined")
  }

  fn command(name: &str) -> Plan {
    Plan::Command { script: name.to_owned() }
  }

  /// A script with nothing to run first is one step, not a group of one. The distinction
  /// is what keeps the common case free of the composition machinery.
  #[test]
  fn a_plain_script_plans_to_a_single_step() {
    assert_eq!(planned(r#"{ "build": { "command": "tsc" } }"#, "build"), command("build"));
  }

  /// Test 5a.1's unit half — the prerequisite comes first and the script's own command
  /// comes last.
  #[test]
  fn prerequisites_run_before_the_command_that_declared_them() {
    let plan = planned(
      r#"{
        "clean": { "command": "rm -rf dist" },
        "build": { "command": "tsc -b", "dependsOn": ["clean"] }
      }"#,
      "build",
    );

    assert_eq!(
      plan,
      Plan::Serial { steps: vec![command("clean"), command("build")], continue_on_error: false }
    );
  }

  /// Test 5a.7's unit half — a group inside a group is one plan, in the order the nesting
  /// implies, and the inner group keeps its own failure policy.
  #[test]
  fn a_nested_group_keeps_its_own_failure_policy() {
    let plan = planned(
      r#"{
        "a": { "command": "echo a" },
        "b": { "command": "echo b" },
        "c": { "command": "echo c" },
        "inner": { "serial": ["b", "c"], "continueOnError": true },
        "outer": { "serial": ["a", "inner"] }
      }"#,
      "outer",
    );

    assert_eq!(
      plan,
      Plan::Serial {
        steps: vec![
          command("a"),
          Plan::Serial { steps: vec![command("b"), command("c")], continue_on_error: true },
        ],
        continue_on_error: false,
      }
    );
  }

  /// Test 5a.8's unit half — the row that proves `dependsOn` and `serial` are one
  /// mechanism rather than two implementations that happen to agree.
  #[test]
  fn a_member_brings_its_own_prerequisites() {
    let plan = planned(
      r#"{
        "lint": { "command": "eslint ." },
        "codegen": { "command": "graphql-codegen" },
        "test": { "command": "vitest", "dependsOn": ["codegen"] },
        "ci": { "serial": ["lint", "test"] }
      }"#,
      "ci",
    );

    assert_eq!(
      plan,
      Plan::Serial {
        steps: vec![
          command("lint"),
          Plan::Serial {
            steps: vec![command("codegen"), command("test")],
            continue_on_error: false,
          },
        ],
        continue_on_error: false,
      }
    );
  }

  /// Two scripts naming the same prerequisite each get it: rune orders scripts, it does
  /// not deduplicate work. Collapsing the second run would be caching, which is the one
  /// thing this tool deliberately leaves to the build systems above it.
  #[test]
  fn a_prerequisite_two_members_share_runs_for_each_of_them() {
    let plan = planned(
      r#"{
        "setup": { "command": "echo setup" },
        "a": { "command": "echo a", "dependsOn": ["setup"] },
        "b": { "command": "echo b", "dependsOn": ["setup"] },
        "both": { "serial": ["a", "b"] }
      }"#,
      "both",
    );

    let Plan::Serial { steps, .. } = &plan else { panic!("`both` did not plan as a group") };
    assert_eq!(steps.len(), 2);
  }

  /// Test 5a.5's unit half — the path, not merely the fact.
  #[test]
  fn a_circle_through_members_reports_the_path() {
    let error = plan(
      &layers(r#"{ "a": { "serial": ["b"] }, "b": { "serial": ["a"] } }"#),
      "a",
      Scope::Nearest,
    )
    .unwrap_err();

    let ComposeError::Cycle { path } = error else { panic!("expected a circle, got {error}") };
    assert_eq!(path, "a → b → a");
  }

  /// A circle can run through both mechanisms, so the walk has to cover them in one pass
  /// rather than checking members and prerequisites separately.
  #[test]
  fn a_circle_through_a_prerequisite_and_a_member_is_found() {
    let error = plan(
      &layers(
        r#"{
          "ci": { "serial": ["build"] },
          "build": { "command": "tsc", "dependsOn": ["ci"] }
        }"#,
      ),
      "ci",
      Scope::Nearest,
    )
    .unwrap_err();

    let ComposeError::Cycle { path } = error else { panic!("expected a circle, got {error}") };
    assert_eq!(path, "ci → build → ci");
  }

  /// Test 5a.6's unit half — the group, the member, and the likeliest correction.
  #[test]
  fn a_member_nothing_defines_offers_the_closest_name() {
    let error = plan(
      &layers(r#"{ "test": { "command": "vitest" }, "ci": { "serial": ["tset"] } }"#),
      "ci",
      Scope::Nearest,
    )
    .unwrap_err();

    let ComposeError::UnknownMember { script, member, suggestion } = error else {
      panic!("expected a missing member, got {error}")
    };
    assert_eq!(script, "ci");
    assert_eq!(member, "tset");
    assert_eq!(suggestion.as_deref(), Some("test"));
  }

  #[test]
  fn a_prerequisite_nothing_defines_names_the_script_that_wanted_it() {
    let error = plan(
      &layers(r#"{ "clean": { "command": "rm -rf dist" }, "build": { "command": "tsc", "dependsOn": ["clen"] } }"#),
      "build",
      Scope::Nearest,
    )
    .unwrap_err();

    let ComposeError::UnknownPrerequisite { script, prerequisite, suggestion } = error else {
      panic!("expected a missing prerequisite, got {error}")
    };
    assert_eq!(script, "build");
    assert_eq!(prerequisite, "clen");
    assert_eq!(suggestion.as_deref(), Some("clean"));
  }

  /// The whole point of validating at load: the broken script is not the one being run.
  #[test]
  fn validation_finds_a_break_in_a_script_nobody_asked_for() {
    let layers =
      layers(r#"{ "build": { "command": "tsc" }, "ci": { "serial": ["nothing-defines-this"] } }"#);

    assert!(plan(&layers, "build", Scope::Nearest).is_ok());
    assert!(validate(&layers, Scope::Nearest).is_err());
  }
}
