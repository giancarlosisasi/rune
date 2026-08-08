//! Turning a script name into the command that would run.
//!
//! Two things make a name ambiguous: a script may build on another script, and a package
//! may narrow a definition the repository shares. Both are answered here, by the same
//! walk, so that `run`, `list` and `inspect` can never disagree about what a name means.
//!
//! The walk is layered. Configs are ordered nearest first, a name resolves in the first
//! layer that defines it, and a script overriding a name resolves *its own* name in the
//! layer underneath — which is what makes `test: { extends: "test" }` inside a package
//! mean "the root's test, plus this".

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::envfile::{self, EnvFile, EnvFileError};
use crate::schema::{Command, Config, Kind, Lifecycle, Script, SuccessPolicy};
use crate::suggest::{closest, did_you_mean};

/// One config taking part in resolution.
#[derive(Debug, Clone)]
pub struct Layer {
  /// The config file, for naming a contribution in `inspect` output.
  pub source: PathBuf,
  pub config: Config,
  /// The dotenv files this config's scripts declare, keyed by the path as written and
  /// read once. Resolving them here is what makes a path mean what it means relative to
  /// this file rather than relative to wherever the user happens to be standing.
  pub env_files: BTreeMap<String, EnvFile>,
}

impl Layer {
  /// Parses nothing and reads everything: the config is already typed, and this is where
  /// the files it names are found and read.
  pub fn new(root: &Path, source: PathBuf, config: Config) -> Result<Self, EnvFileError> {
    let env_files = envfile::read_all(root, &source, &config)?;

    Ok(Self { source, config, env_files })
  }
}

/// How far resolution is allowed to look.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
  /// The nearest definition wins: a package's own config first, then the root's.
  Nearest,
  /// The root config alone, as if the run had started at the repository root.
  Root,
}

#[derive(Debug, Error)]
pub enum InheritError {
  #[error(
    "script `{script}` extends `{target}`, which no script defines{}",
    did_you_mean(.suggestion.as_deref())
  )]
  UnknownTarget { script: String, target: String, suggestion: Option<String> },

  #[error(
    "`extends` goes in a circle:\n\n  {path}\n\n\
     a chain of `extends` has to end at a script with a `command`."
  )]
  Cycle { path: String },

  #[error(
    "script `{script}` extends `{target}`, which is a group\n\n\
     `extends` builds on the command a script runs, and a group has none of its own — it \
     names\nthe scripts it runs. To reuse a group, name it as a member of another group."
  )]
  ExtendsGroup { script: String, target: String },
}

/// A script name, answered: what runs, where, with what, and how that was arrived at.
#[derive(Debug)]
pub struct Resolved<'a> {
  pub runs: Runs<'a>,
  /// Accumulated from the base outward, so the outermost script's arguments come last.
  pub append_args: Vec<String>,
  /// The scripts that run before this one, in order. Empty when it has none.
  pub depends_on: &'a [String],
  pub description: Option<&'a str>,
  pub cwd: Option<&'a str>,
  /// The script keeps rune's own terminal even as one member of a group.
  pub interactive: bool,
  /// The lifecycle options the chain declared, each one taken from the last script that
  /// named it.
  pub lifecycle: Lifecycle,
  pub env: BTreeMap<String, String>,
  /// The dotenv files the chain declares, nearest to the script first.
  ///
  /// Nearest first is the order they are consulted in, and it is the same rule every
  /// other field follows: what the extending script says wins over what it built on.
  pub env_files: Vec<&'a EnvFile>,
  /// The base first, then every script that built on it.
  pub chain: Vec<Link<'a>>,
}

impl<'a> Resolved<'a> {
  /// The command this name runs, or nothing when it is a group of other scripts.
  pub fn command(&self) -> Option<&'a Command> {
    match self.runs {
      Runs::Command(command) => Some(command),
      Runs::Serial { .. } | Runs::Parallel { .. } => None,
    }
  }
}

/// What running a name actually does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Runs<'a> {
  /// A command of its own.
  Command(&'a Command),
  /// Other scripts, one at a time, in the order written.
  Serial { members: &'a [String], continue_on_error: bool },
  /// Other scripts, all at once.
  Parallel { members: &'a [String], continue_on_error: bool, policy: SuccessPolicy },
}

/// One step of a resolution: a definition, and what it contributed.
#[derive(Debug)]
pub struct Link<'a> {
  pub name: &'a str,
  pub source: &'a Path,
  pub append_args: &'a [String],
}

/// Resolves `name` against `layers`, or reports why it cannot be.
///
/// `Ok(None)` means no layer defines the name at all. That is not an error here: the
/// caller knows which names it can offer as alternatives, and says so in its own words.
pub fn resolve<'a>(
  layers: &'a [Layer],
  name: &str,
  scope: Scope,
) -> Result<Option<Resolved<'a>>, InheritError> {
  let floor = floor_of(layers, scope);

  let Some(entry) = find(layers, name, floor) else {
    return Ok(None);
  };

  // Outermost first while walking inward, then reversed: the merge and the chain both
  // read base-outward, which is the order the arguments have to end up in.
  let mut walked = vec![entry];
  loop {
    let (layer, key, script) = *walked.last().expect("the walk starts with one entry");
    let Kind::Extends { target, .. } = &script.kind else {
      break;
    };

    // A script overriding a name resolves that name in the layer underneath. Anything
    // else resolves from the nearest layer, so a package narrowing a shared script also
    // narrows every script built on it.
    let from = if target == key { layer + 1 } else { floor };
    let Some(next) = find(layers, target, from) else {
      return Err(missing_target(layers, floor, &walked, key, target));
    };

    if matches!(next.2.kind, Kind::Serial { .. } | Kind::Parallel { .. }) {
      return Err(InheritError::ExtendsGroup { script: key.to_owned(), target: target.to_owned() });
    }

    if walked.iter().any(|(seen_layer, seen_key, _)| (*seen_layer, *seen_key) == (next.0, next.1)) {
      return Err(InheritError::Cycle { path: render_path(&walked, next.1) });
    }
    walked.push(next);
  }

  walked.reverse();
  Ok(Some(merge(layers, &walked)))
}

/// Every name any layer in scope defines, nearest first. A name both layers define
/// appears once per layer; callers either sort them or only look for the closest.
pub fn names(layers: &[Layer], scope: Scope) -> impl Iterator<Item = &str> {
  visible(layers, floor_of(layers, scope))
}

fn visible(layers: &[Layer], floor: usize) -> impl Iterator<Item = &str> {
  layers[floor..].iter().flat_map(|layer| layer.config.scripts.keys().map(String::as_str))
}

/// The first layer a scope is allowed to look in.
fn floor_of(layers: &[Layer], scope: Scope) -> usize {
  match scope {
    Scope::Nearest => 0,
    Scope::Root => layers.len().saturating_sub(1),
  }
}

/// One step of the walk: which layer it was found in, its name there, and its definition.
type Step<'a> = (usize, &'a str, &'a Script);

fn find<'a>(layers: &'a [Layer], name: &str, from: usize) -> Option<Step<'a>> {
  layers.iter().enumerate().skip(from).find_map(|(index, layer)| {
    layer.config.scripts.get_key_value(name).map(|(key, script)| (index, key.as_str(), script))
  })
}

/// A target nothing defines — unless the script was extending its own name, in which case
/// there was no definition underneath it and the chain closed on itself.
fn missing_target(
  layers: &[Layer],
  floor: usize,
  walked: &[Step<'_>],
  script: &str,
  target: &str,
) -> InheritError {
  if script == target {
    return InheritError::Cycle { path: render_path(walked, target) };
  }

  InheritError::UnknownTarget {
    script: script.to_owned(),
    target: target.to_owned(),
    suggestion: closest(target, visible(layers, floor)).map(str::to_owned),
  }
}

/// The walk so far, then the name it came back to: `a → b → a`.
///
/// The path is the debugging information. "cycle detected" leaves the user rebuilding
/// their own config graph by hand to find out which two scripts were involved.
fn render_path(walked: &[Step<'_>], repeated: &str) -> String {
  let mut path = String::new();
  for (_, name, _) in walked {
    path.push_str(name);
    path.push_str(" → ");
  }
  path.push_str(repeated);
  path
}

/// Folds the chain base-outward. Later levels win per key, so an extending script
/// overrides only what it declares and inherits everything it does not.
fn merge<'a>(layers: &'a [Layer], walked: &[Step<'a>]) -> Resolved<'a> {
  let mut runs = None;
  let mut append_args = Vec::new();
  let mut depends_on: &[String] = &[];
  let mut description = None;
  let mut cwd = None;
  let mut interactive = false;
  let mut lifecycle = Lifecycle::default();
  let mut env: BTreeMap<String, String> = BTreeMap::new();
  let mut env_files = Vec::new();
  let mut chain = Vec::with_capacity(walked.len());

  for (layer, name, script) in walked {
    let contributed: &[String] = match &script.kind {
      Kind::Command(own) => {
        runs = Some(Runs::Command(own));
        &[]
      }
      Kind::Serial { members, continue_on_error } => {
        runs = Some(Runs::Serial { members, continue_on_error: *continue_on_error });
        &[]
      }
      Kind::Parallel { members, continue_on_error, policy } => {
        runs =
          Some(Runs::Parallel { members, continue_on_error: *continue_on_error, policy: *policy });
        &[]
      }
      Kind::Extends { append_args, .. } => append_args,
    };

    append_args.extend_from_slice(contributed);
    // A declared list wins whole, so an extending script can also say that nothing runs
    // first. Merging the two would leave a prerequisite impossible to drop.
    if let Some(declared) = &script.depends_on {
      depends_on = declared;
    }
    if script.description.is_some() {
      description = script.description.as_deref();
    }
    if script.cwd.is_some() {
      cwd = script.cwd.as_deref();
    }
    if let Some(declared) = script.interactive {
      interactive = declared;
    }
    lifecycle.absorb(script.lifecycle);
    // Per key, never whole-map: a package adding one variable must not discard the
    // base's others.
    env.extend(script.env.iter().map(|(key, value)| (key.clone(), value.clone())));

    if let Some(declared) = &script.env_file {
      env_files.push(
        layers[*layer]
          .env_files
          .get(declared)
          .expect("a layer reads every envFile its scripts declare when it is built"),
      );
    }

    chain.push(Link { name, source: &layers[*layer].source, append_args: contributed });
  }

  // The walk reads base-outward; the files are consulted the other way round.
  env_files.reverse();

  Resolved {
    runs: runs.expect("the walk only stops at a script that runs something"),
    append_args,
    depends_on,
    description,
    cwd,
    interactive,
    lifecycle,
    env,
    env_files,
    chain,
  }
}

#[cfg(test)]
mod tests {
  use std::path::{Path, PathBuf};

  use super::{InheritError, Layer, Resolved, Runs, Scope, resolve};
  use crate::schema::parse;

  const PLATFORM: &str = "linux";

  /// What a resolution runs, for the tests that only care about the command.
  fn command_of<'a>(resolved: &Resolved<'a>) -> &'a str {
    match resolved.runs {
      Runs::Command(command) => command.select(PLATFORM),
      Runs::Serial { .. } | Runs::Parallel { .. } => {
        panic!("resolved to a group rather than a command")
      }
    }
  }

  fn layer(source: &str, scripts: &str) -> Layer {
    let value: serde_json::Value =
      serde_json::from_str(&format!("{{ \"scripts\": {scripts} }}")).expect("valid fixture JSON");

    Layer::new(Path::new(""), PathBuf::from(source), parse(&value).expect("fixture parses"))
      .expect("no fixture here declares an envFile")
  }

  fn root(scripts: &str) -> Vec<Layer> {
    vec![layer("rune.config.ts", scripts)]
  }

  /// Test 4b.1 — three levels, each adding arguments, ordered from the base outward.
  ///
  /// Any other order runs `vitest --reporter=dot --run`, which a tool may accept, ignore
  /// or reject depending on how it parses its own flags — a difference that shows up as
  /// a mystery months later rather than as a failure now.
  #[test]
  fn append_args_accumulate_from_the_base_outward() {
    let layers = root(
      r#"{
        "test": { "command": "vitest" },
        "test:ci": { "extends": "test", "appendArgs": ["--run"] },
        "test:ci:dot": { "extends": "test:ci", "appendArgs": ["--reporter=dot", "--bail=1"] }
      }"#,
    );

    let resolved =
      resolve(&layers, "test:ci:dot", Scope::Nearest).expect("resolves").expect("defined");

    assert_eq!(command_of(&resolved), "vitest");
    assert_eq!(resolved.append_args, ["--run", "--reporter=dot", "--bail=1"]);
  }

  /// The chain reads base first, which is the order `inspect` renders and the order the
  /// arguments were accumulated in.
  #[test]
  fn the_chain_names_every_step_from_the_base_outward() {
    let layers = root(
      r#"{
        "test": { "command": "vitest" },
        "test:ci": { "extends": "test", "appendArgs": ["--run"] }
      }"#,
    );

    let resolved = resolve(&layers, "test:ci", Scope::Nearest).expect("resolves").expect("defined");

    let names: Vec<&str> = resolved.chain.iter().map(|link| link.name).collect();
    assert_eq!(names, ["test", "test:ci"]);
  }

  /// Test 4b.2 — the extending script wins per key, and `env` merges rather than
  /// replaces. D2 stated the rule; nothing tested it until this row.
  #[test]
  fn merging_is_per_key_and_the_extending_script_wins() {
    let layers = root(
      r#"{
        "base": {
          "command": "vitest",
          "description": "run the unit tests",
          "cwd": "packages/core",
          "env": { "NODE_ENV": "test", "TZ": "UTC" }
        },
        "ci": { "extends": "base", "env": { "NODE_ENV": "ci" } }
      }"#,
    );

    let resolved = resolve(&layers, "ci", Scope::Nearest).expect("resolves").expect("defined");

    assert_eq!(resolved.env["NODE_ENV"], "ci", "the extending script must win");
    assert_eq!(resolved.env["TZ"], "UTC", "the base's other variables must survive");
    assert_eq!(resolved.cwd, Some("packages/core"), "an undeclared field is inherited");
    assert_eq!(resolved.description, Some("run the unit tests"));
  }

  #[test]
  fn an_extending_script_overrides_the_scalar_fields_it_declares() {
    let layers = root(
      r#"{
        "base": { "command": "vitest", "description": "unit tests", "cwd": "a" },
        "other": { "extends": "base", "description": "the same, elsewhere", "cwd": "b" }
      }"#,
    );

    let resolved = resolve(&layers, "other", Scope::Nearest).expect("resolves").expect("defined");

    assert_eq!(resolved.cwd, Some("b"));
    assert_eq!(resolved.description, Some("the same, elsewhere"));
  }

  /// Test 4b.4 — the path, not merely the fact.
  #[test]
  fn a_cycle_reports_the_path_it_followed() {
    let layers = root(r#"{ "a": { "extends": "b" }, "b": { "extends": "a" } }"#);

    let error = resolve(&layers, "a", Scope::Nearest).unwrap_err();

    let InheritError::Cycle { path } = error else {
      panic!("expected a cycle, got {error}");
    };
    assert_eq!(path, "a → b → a");
  }

  /// A script naming itself has no definition underneath to fall through to, so it is a
  /// circle of one rather than a missing target.
  #[test]
  fn a_script_extending_itself_is_a_cycle_of_one() {
    let layers = root(r#"{ "a": { "extends": "a" } }"#);

    let error = resolve(&layers, "a", Scope::Nearest).unwrap_err();

    let InheritError::Cycle { path } = error else {
      panic!("expected a cycle, got {error}");
    };
    assert_eq!(path, "a → a");
  }

  /// Test 4b.3 — the missing target is named, and so is the likeliest correction.
  #[test]
  fn extending_a_name_nothing_defines_offers_the_closest_one() {
    let layers = root(r#"{ "test": { "command": "vitest" }, "ci": { "extends": "tests" } }"#);

    let error = resolve(&layers, "ci", Scope::Nearest).unwrap_err();

    let InheritError::UnknownTarget { script, target, suggestion } = error else {
      panic!("expected a missing target, got {error}");
    };
    assert_eq!(script, "ci");
    assert_eq!(target, "tests");
    assert_eq!(suggestion.as_deref(), Some("test"));
  }

  #[test]
  fn a_name_no_layer_defines_resolves_to_nothing() {
    let layers = root(r#"{ "test": { "command": "vitest" } }"#);

    assert!(resolve(&layers, "absent", Scope::Nearest).expect("not an error").is_none());
  }

  /// A package overriding a name extends the definition it shadows, which is the whole
  /// mechanism the change exists for.
  #[test]
  fn an_override_extends_the_definition_it_shadows() {
    let layers = vec![
      layer(
        "packages/legacy/rune.config.ts",
        r#"{ "test": { "extends": "test", "appendArgs": ["--maxWorkers=1"] } }"#,
      ),
      layer("rune.config.ts", r#"{ "test": { "command": "jest" } }"#),
    ];

    let resolved = resolve(&layers, "test", Scope::Nearest).expect("resolves").expect("defined");

    assert_eq!(command_of(&resolved), "jest");
    assert_eq!(resolved.append_args, ["--maxWorkers=1"]);
    assert_eq!(resolved.chain.len(), 2, "the root and the package both contributed");
  }

  /// Test 4b.10's unit half — the root scope cannot see the package layer at all.
  #[test]
  fn the_root_scope_ignores_the_package_layer() {
    let layers = vec![
      layer(
        "packages/legacy/rune.config.ts",
        r#"{
          "test": { "extends": "test", "appendArgs": ["--maxWorkers=1"] },
          "legacy-only": { "command": "echo legacy" }
        }"#,
      ),
      layer("rune.config.ts", r#"{ "test": { "command": "jest" } }"#),
    ];

    let resolved = resolve(&layers, "test", Scope::Root).expect("resolves").expect("defined");

    assert!(resolved.append_args.is_empty(), "the package's arguments must not be added");
    assert_eq!(resolved.chain.len(), 1);
    assert!(
      resolve(&layers, "legacy-only", Scope::Root).expect("not an error").is_none(),
      "a package-only script is invisible at the root"
    );
  }

  /// A group resolves to its member list, not to a command, and the walk stops there the
  /// same way it stops at a `command`.
  #[test]
  fn a_group_resolves_to_its_members() {
    let layers = root(
      r#"{
        "lint": { "command": "eslint ." },
        "test": { "command": "vitest" },
        "ci": { "serial": ["lint", "test"], "continueOnError": true }
      }"#,
    );

    let resolved = resolve(&layers, "ci", Scope::Nearest).expect("resolves").expect("defined");

    let Runs::Serial { members, continue_on_error } = resolved.runs else {
      panic!("`ci` did not resolve to a group");
    };
    assert_eq!(members, ["lint".to_owned(), "test".to_owned()]);
    assert!(continue_on_error);
    assert!(resolved.command().is_none());
  }

  /// A script says nothing about prerequisites, so it keeps the ones it inherits — the
  /// same rule `cwd` and `description` follow.
  #[test]
  fn an_undeclared_depends_on_is_inherited() {
    let layers = root(
      r#"{
        "clean": { "command": "rm -rf dist" },
        "build": { "command": "tsc -b", "dependsOn": ["clean"] },
        "build:ci": { "extends": "build", "appendArgs": ["--force"] }
      }"#,
    );

    let resolved =
      resolve(&layers, "build:ci", Scope::Nearest).expect("resolves").expect("defined");

    assert_eq!(resolved.depends_on, ["clean".to_owned()]);
  }

  /// The list wins whole, which is what lets a package drop a prerequisite it does not
  /// want. Merging the two lists instead would leave one impossible to remove.
  #[test]
  fn a_declared_depends_on_replaces_the_inherited_one() {
    let layers = root(
      r#"{
        "clean": { "command": "rm -rf dist" },
        "build": { "command": "tsc -b", "dependsOn": ["clean"] },
        "build:fast": { "extends": "build", "dependsOn": [] }
      }"#,
    );

    let resolved =
      resolve(&layers, "build:fast", Scope::Nearest).expect("resolves").expect("defined");

    assert!(resolved.depends_on.is_empty());
  }

  /// A group has no command for the inherited arguments to join, so extending one is
  /// refused rather than silently dropping them.
  #[test]
  fn extending_a_group_is_rejected() {
    let layers = root(
      r#"{ "ci": { "serial": ["lint"] }, "lint": { "command": "eslint ." }, "x": { "extends": "ci" } }"#,
    );

    let error = resolve(&layers, "x", Scope::Nearest).unwrap_err();

    let InheritError::ExtendsGroup { script, target } = error else {
      panic!("expected a group rejection, got {error}");
    };
    assert_eq!(script, "x");
    assert_eq!(target, "ci");
  }

  /// A package narrowing a shared script narrows everything built on it too. Without
  /// this, `test:ci` would quietly keep running the unnarrowed command.
  #[test]
  fn a_script_built_on_an_overridden_name_picks_up_the_override() {
    let layers = vec![
      layer(
        "packages/legacy/rune.config.ts",
        r#"{ "test": { "extends": "test", "appendArgs": ["--maxWorkers=1"] } }"#,
      ),
      layer(
        "rune.config.ts",
        r#"{
          "test": { "command": "jest" },
          "test:ci": { "extends": "test", "appendArgs": ["--ci"] }
        }"#,
      ),
    ];

    let resolved = resolve(&layers, "test:ci", Scope::Nearest).expect("resolves").expect("defined");

    assert_eq!(resolved.append_args, ["--maxWorkers=1", "--ci"]);
  }
}
