//! The shape a resolved config must have.
//!
//! Dispatch is manual and explicit. `#[serde(untagged)]` was the obvious choice and is
//! the wrong one: it does not compose with `deny_unknown_fields`, and its failure reads
//! "data did not match any variant", which tells a user nothing about the mistake they
//! made. Here the discriminant keys are inspected by name, so the error can say which
//! script is wrong and which word in it is the problem.

use std::collections::BTreeMap;

use serde::Deserialize;
use thiserror::Error;

/// The keys that decide which kind of script an entry is. Each later change adds one.
const DISCRIMINANTS: &[&str] = &["command", "extends", "serial"];

/// Fields any script may carry, whatever its discriminant.
const COMMON_FIELDS: &[&str] = &["description", "cwd", "env", "envFile"];

/// The arguments an extending script adds to what it inherits.
const APPEND_ARGS: &str = "appendArgs";

/// The scripts that run before a script's own command.
const DEPENDS_ON: &str = "dependsOn";

/// Makes a group run every member instead of stopping at the first failure.
const CONTINUE_ON_ERROR: &str = "continueOnError";

/// The keys a per-OS `command` object may hold. The names are Node's `process.platform`
/// values, so a config reading `rune.platform` and a config using this object spell the
/// same operating system the same way.
const PER_OS_KEYS: &[&str] = &["default", "win32", "darwin", "linux"];

/// The key every per-OS object must have.
const FALLBACK_KEY: &str = "default";

#[derive(Debug, Error)]
pub enum SchemaError {
  #[error("the config must be an object with a `scripts` object; found {found}")]
  NotAConfig { found: String },

  #[error("script `{script}` must be an object; found {found}")]
  NotAScript { script: String, found: String },

  #[error("script `{script}` has no command\n\nevery script needs one of: {}", list(DISCRIMINANTS))]
  NoDiscriminant { script: String },

  #[error(
    "script `{script}` sets both `{first}` and `{second}`\n\n\
     a script is exactly one kind, so keep the one you meant and remove the other:\n  \
     `{first}` {}\n  `{second}` {}",
    meaning(.first),
    meaning(.second)
  )]
  ManyDiscriminants { script: String, first: String, second: String },

  #[error(
    "script `{script}` sets `{APPEND_ARGS}` but does not extend anything\n\n\
     `{APPEND_ARGS}` adds arguments to a command a script inherits, so it only means \
     something\nnext to `extends`. A script with a `command` of its own writes its \
     arguments into it."
  )]
  AppendArgsWithoutExtends { script: String },

  #[error("script `{script}` has an unknown field `{field}`\n\nallowed here: {allowed}")]
  UnknownField { script: String, field: String, allowed: String },

  #[error(
    "script `{script}` has a `command` that is {found}\n\n\
     a command is either a string, or an object naming one command per operating \
     system: {}",
    list(PER_OS_KEYS)
  )]
  CommandShape { script: String, found: String },

  #[error(
    "script `{script}` has a per-operating-system `command` with no `{FALLBACK_KEY}`\n\n\
     `{FALLBACK_KEY}` is what runs on every system without an entry of its own.\n\
     Without it this script would exist on some machines and not on others."
  )]
  NoFallback { script: String },

  #[error("script `{script}` extends {found}\n\n`extends` is the name of another script")]
  ExtendsShape { script: String, found: String },

  #[error("script `{script}` has a `{key}` that is {found}\n\n{explanation}")]
  NotAList { script: String, key: &'static str, found: String, explanation: &'static str },

  #[error("script `{script}` has `{key}` element {position} that is {found}\n\n{explanation}")]
  NotAListElement {
    script: String,
    key: &'static str,
    position: usize,
    found: String,
    explanation: &'static str,
  },

  #[error(
    "script `{script}` sets `{DEPENDS_ON}` beside `serial`\n\n\
     a group already says what runs and in which order — its member list. A second \
     ordering\non the same entry would have no unambiguous meaning. Put the prerequisite \
     first in\nthe member list instead."
  )]
  DependsOnGroup { script: String },

  #[error(
    "script `{script}` has a `{CONTINUE_ON_ERROR}` that is {found}\n\n\
     `{CONTINUE_ON_ERROR}` is true or false. Set, every member runs even after one fails, \
     and\nthe group still ends with the first failure's exit code."
  )]
  ContinueOnErrorShape { script: String, found: String },

  #[error("script `{script}`: {source}")]
  Invalid { script: String, source: serde_json::Error },
}

/// What a list-valued key holds, printed when the value is not a list of strings.
///
/// The pairs sit together because the two messages a user can meet for one key have to
/// agree with each other about what that key is for.
const APPEND_ARGS_LIST: (&str, &str) = (
  "`appendArgs` is a list of arguments, one element per argument:\n\n  \
   appendArgs: [\"--maxWorkers=1\"]",
  "every element is one argument, written as a string",
);

const SERIAL_LIST: (&str, &str) = (
  "`serial` is a list of script names, run one at a time in the order written:\n\n  \
   serial: [\"lint\", \"typecheck\", \"test\"]",
  "every element is the name of another script, written as a string",
);

const DEPENDS_ON_LIST: (&str, &str) = (
  "`dependsOn` is a list of script names, run to completion before this script's own \
   command:\n\n  dependsOn: [\"clean\"]",
  "every element is the name of another script, written as a string",
);

fn list(names: &[&str]) -> String {
  names.iter().map(|name| format!("`{name}`")).collect::<Vec<_>>().join(", ")
}

/// What each discriminant makes a script do, in one line.
///
/// The conflict error prints this for both keys it found, so a user picks the one they
/// meant without opening the documentation. Every variant a later change adds gets a
/// line here, and the conflict message keeps working without being rewritten.
fn meaning(discriminant: &str) -> &'static str {
  match discriminant {
    "command" => "runs a command of its own",
    "extends" => "builds on another script and adds to it",
    "serial" => "runs other scripts, one after another",
    other => unreachable!("`{other}` is listed as a discriminant but has no meaning"),
  }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
  /// Sorted by name, because `rune list` prints them in this order and a map with a
  /// random iteration order would make that output differ between runs.
  pub scripts: BTreeMap<String, Script>,
}

/// One entry of the `scripts` object: the fields every script may carry, plus the one
/// thing that makes it the kind of script it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Script {
  pub description: Option<String>,
  /// Where the script runs. A relative value is resolved against the invoking package.
  pub cwd: Option<String>,
  /// Variables the script sets for its own child, which win over inherited values.
  pub env: BTreeMap<String, String>,
  /// A file of variables that fill the gaps the process environment leaves. Resolved
  /// against the config that declares it, never against the working directory.
  pub env_file: Option<String>,
  /// Scripts that run to completion before this one's own command.
  ///
  /// `None` means the entry said nothing, so an extending script inherits what it builds
  /// on; an empty list means it said "nothing runs first" and that wins. A group never
  /// carries one — its member list is already the order.
  pub depends_on: Option<Vec<String>>,
  pub kind: Kind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
  /// Runs a command of its own.
  Command(Command),
  /// Runs another script's command, with more arguments after it.
  Extends { target: String, append_args: Vec<String> },
  /// Runs other scripts, one at a time, in the order written.
  Serial { members: Vec<String>, continue_on_error: bool },
}

/// What a script runs: one command everywhere, or one per operating system.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
  Everywhere(String),
  PerOs(PerOsCommand),
}

impl Command {
  /// The command string for `platform`, named the way `process.platform` names it.
  pub fn select(&self, platform: &str) -> &str {
    match self {
      Self::Everywhere(command) => command,
      Self::PerOs(per_os) => per_os.select(platform),
    }
  }
}

/// `rm -rf dist` and `rmdir /s /q dist` are one intent with two spellings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PerOsCommand {
  /// What a system with no entry of its own runs. Required, so that a config cannot
  /// define a script which silently does not exist on somebody else's machine.
  pub default: String,
  pub win32: Option<String>,
  pub darwin: Option<String>,
  pub linux: Option<String>,
}

impl PerOsCommand {
  /// The entry matching `platform`, or `default`.
  ///
  /// The platform is an argument rather than something read from `cfg!` here: that is
  /// what lets one machine test the choice every other machine would make.
  pub fn select(&self, platform: &str) -> &str {
    let matched = match platform {
      "win32" => self.win32.as_deref(),
      "darwin" => self.darwin.as_deref(),
      "linux" => self.linux.as_deref(),
      _ => None,
    };

    matched.unwrap_or(&self.default)
  }
}

/// The fields a script carries whatever kind it is. Deserialized apart from the
/// discriminant so that the discriminant's own errors can name the script.
#[derive(Debug, Deserialize)]
struct Common {
  #[serde(default)]
  description: Option<String>,
  #[serde(default)]
  cwd: Option<String>,
  #[serde(default)]
  env: BTreeMap<String, String>,
  #[serde(default, rename = "envFile")]
  env_file: Option<String>,
}

/// Turns an evaluated config into the typed shape, or says exactly what is wrong with it.
pub fn parse(value: &serde_json::Value) -> Result<Config, SchemaError> {
  let scripts = value
    .get("scripts")
    .and_then(serde_json::Value::as_object)
    .ok_or_else(|| SchemaError::NotAConfig { found: describe(value) })?;

  let mut parsed = BTreeMap::new();
  for (name, entry) in scripts {
    parsed.insert(name.clone(), parse_script(name, entry)?);
  }

  Ok(Config { scripts: parsed })
}

fn parse_script(name: &str, entry: &serde_json::Value) -> Result<Script, SchemaError> {
  let object = entry
    .as_object()
    .ok_or_else(|| SchemaError::NotAScript { script: name.to_owned(), found: describe(entry) })?;

  let mut present = DISCRIMINANTS.iter().filter(|key| object.contains_key(**key));
  let found = present.next();
  if let (Some(first), Some(second)) = (found, present.next()) {
    return Err(SchemaError::ManyDiscriminants {
      script: name.to_owned(),
      first: (*first).to_owned(),
      second: (*second).to_owned(),
    });
  }

  // Before the discriminant dispatch, so that the message is about the mistake the user
  // made rather than about a field that happens to be unknown to the arm they landed in.
  if object.contains_key(APPEND_ARGS) && !object.contains_key("extends") {
    return Err(SchemaError::AppendArgsWithoutExtends { script: name.to_owned() });
  }

  if object.contains_key(DEPENDS_ON) && object.contains_key("serial") {
    return Err(SchemaError::DependsOnGroup { script: name.to_owned() });
  }

  let Some(discriminant) = found else {
    // A misspelled discriminant is indistinguishable from a missing one, and it is by
    // far the likelier mistake, so name the offending word when there is one.
    // `dependsOn` is legal beside any variant but is not one, so an entry carrying only
    // that is missing a discriminant rather than holding an unknown field.
    let mut legal = DISCRIMINANTS.to_vec();
    legal.push(DEPENDS_ON);
    reject_unknown_fields(name, object, &allowed_with(&legal))?;
    return Err(SchemaError::NoDiscriminant { script: name.to_owned() });
  };

  let kind = match *discriminant {
    "command" => {
      // Checked before deserializing so the message can name the script. serde's own
      // unknown-field error knows the field but not which entry it came from.
      reject_unknown_fields(name, object, &allowed_with(&["command", DEPENDS_ON]))?;
      Kind::Command(parse_command(name, &object["command"])?)
    }
    "extends" => {
      reject_unknown_fields(name, object, &allowed_with(&["extends", APPEND_ARGS, DEPENDS_ON]))?;
      parse_extends(name, object)?
    }
    "serial" => {
      reject_unknown_fields(name, object, &allowed_with(&["serial", CONTINUE_ON_ERROR]))?;
      parse_serial(name, object)?
    }
    other => unreachable!("`{other}` is listed as a discriminant but has no arm"),
  };

  let depends_on = match object.get(DEPENDS_ON) {
    None => None,
    Some(value) => Some(string_list(name, DEPENDS_ON, value, DEPENDS_ON_LIST)?),
  };

  let common: Common = serde_json::from_value(entry.clone())
    .map_err(|source| SchemaError::Invalid { script: name.to_owned(), source })?;

  Ok(Script {
    description: common.description,
    cwd: common.cwd,
    env: common.env,
    env_file: common.env_file,
    depends_on,
    kind,
  })
}

fn parse_serial(
  script: &str,
  object: &serde_json::Map<String, serde_json::Value>,
) -> Result<Kind, SchemaError> {
  let members = string_list(script, "serial", &object["serial"], SERIAL_LIST)?;

  let continue_on_error = match object.get(CONTINUE_ON_ERROR) {
    None => false,
    Some(value) => value.as_bool().ok_or_else(|| SchemaError::ContinueOnErrorShape {
      script: script.to_owned(),
      found: describe(value),
    })?,
  };

  Ok(Kind::Serial { members, continue_on_error })
}

/// A list of strings, or the reason `value` is not one.
///
/// `explanation` carries what the key is for, once for the whole value and once for a
/// single bad element, so three keys share the checking without sharing their wording.
fn string_list(
  script: &str,
  key: &'static str,
  value: &serde_json::Value,
  explanation: (&'static str, &'static str),
) -> Result<Vec<String>, SchemaError> {
  let (whole, element) = explanation;

  let items = value.as_array().ok_or_else(|| SchemaError::NotAList {
    script: script.to_owned(),
    key,
    found: describe(value),
    explanation: whole,
  })?;

  items
    .iter()
    .enumerate()
    .map(|(position, item)| {
      item.as_str().map(str::to_owned).ok_or_else(|| SchemaError::NotAListElement {
        script: script.to_owned(),
        key,
        position,
        found: describe(item),
        explanation: element,
      })
    })
    .collect()
}

fn parse_extends(
  script: &str,
  object: &serde_json::Map<String, serde_json::Value>,
) -> Result<Kind, SchemaError> {
  let value = &object["extends"];
  let target = value
    .as_str()
    .ok_or_else(|| SchemaError::ExtendsShape { script: script.to_owned(), found: describe(value) })?
    .to_owned();

  let Some(arguments) = object.get(APPEND_ARGS) else {
    return Ok(Kind::Extends { target, append_args: Vec::new() });
  };

  let append_args = string_list(script, APPEND_ARGS, arguments, APPEND_ARGS_LIST)?;

  Ok(Kind::Extends { target, append_args })
}

fn parse_command(script: &str, value: &serde_json::Value) -> Result<Command, SchemaError> {
  if let Some(command) = value.as_str() {
    return Ok(Command::Everywhere(command.to_owned()));
  }

  let Some(object) = value.as_object() else {
    return Err(SchemaError::CommandShape { script: script.to_owned(), found: describe(value) });
  };

  for key in object.keys() {
    if !PER_OS_KEYS.contains(&key.as_str()) {
      return Err(SchemaError::UnknownField {
        script: script.to_owned(),
        field: key.clone(),
        allowed: list(PER_OS_KEYS),
      });
    }
  }

  let entry = |key: &str| -> Result<Option<String>, SchemaError> {
    match object.get(key) {
      None => Ok(None),
      Some(value) => value.as_str().map(|command| Some(command.to_owned())).ok_or_else(|| {
        SchemaError::CommandShape { script: script.to_owned(), found: describe(value) }
      }),
    }
  };

  let default =
    entry(FALLBACK_KEY)?.ok_or_else(|| SchemaError::NoFallback { script: script.to_owned() })?;

  Ok(Command::PerOs(PerOsCommand {
    default,
    win32: entry("win32")?,
    darwin: entry("darwin")?,
    linux: entry("linux")?,
  }))
}

/// The fields legal for one variant: its own discriminant plus everything shared.
fn allowed_with(specific: &[&'static str]) -> Vec<&'static str> {
  specific.iter().copied().chain(COMMON_FIELDS.iter().copied()).collect()
}

fn reject_unknown_fields(
  script: &str,
  object: &serde_json::Map<String, serde_json::Value>,
  allowed: &[&'static str],
) -> Result<(), SchemaError> {
  for field in object.keys() {
    if !allowed.contains(&field.as_str()) {
      return Err(SchemaError::UnknownField {
        script: script.to_owned(),
        field: field.clone(),
        allowed: list(allowed),
      });
    }
  }
  Ok(())
}

fn describe(value: &serde_json::Value) -> String {
  match value {
    serde_json::Value::Null => "null".to_owned(),
    serde_json::Value::Bool(_) => "a boolean".to_owned(),
    serde_json::Value::Number(_) => "a number".to_owned(),
    serde_json::Value::String(_) => "a string".to_owned(),
    serde_json::Value::Array(_) => "an array".to_owned(),
    serde_json::Value::Object(_) => "an object".to_owned(),
  }
}

#[cfg(test)]
mod tests {
  use serde_json::json;

  use super::{Kind, PerOsCommand, parse};

  /// The command of a script that has one, for the tests that only care about that.
  fn command_of(config: &super::Config, name: &str, platform: &str) -> String {
    match &config.scripts[name].kind {
      Kind::Command(command) => command.select(platform).to_owned(),
      _ => panic!("`{name}` does not run a command of its own"),
    }
  }

  #[test]
  fn a_command_script_parses() {
    let config = parse(&json!({
      "scripts": { "dev": { "command": "vite", "description": "start the dev server" } }
    }))
    .expect("parses");

    assert_eq!(command_of(&config, "dev", "linux"), "vite");
    assert_eq!(config.scripts["dev"].description.as_deref(), Some("start the dev server"));
  }

  /// The `extends` half of the discriminant dispatch, with both of its fields.
  #[test]
  fn an_extending_script_parses() {
    let config = parse(&json!({
      "scripts": {
        "test": { "command": "vitest" },
        "test:ci": { "extends": "test", "appendArgs": ["--run", "--reporter=dot"] },
      }
    }))
    .expect("parses");

    let Kind::Extends { target, append_args } = &config.scripts["test:ci"].kind else {
      panic!("`test:ci` did not parse as an extending script");
    };
    assert_eq!(target, "test");
    assert_eq!(append_args, &["--run".to_owned(), "--reporter=dot".to_owned()]);
  }

  /// `appendArgs` is optional: extending a script without adding anything is how a
  /// package renames a shared command or changes only its `env`.
  #[test]
  fn an_extending_script_without_append_args_parses() {
    let config = parse(&json!({
      "scripts": { "a": { "command": "vitest" }, "b": { "extends": "a" } }
    }))
    .expect("parses");

    let Kind::Extends { append_args, .. } = &config.scripts["b"].kind else {
      panic!("`b` did not parse as an extending script");
    };
    assert!(append_args.is_empty());
  }

  #[test]
  fn an_extends_that_is_not_a_name_is_rejected() {
    let error =
      parse(&json!({ "scripts": { "b": { "extends": ["a"] } } })).unwrap_err().to_string();

    assert!(error.contains("`b`"), "{error}");
  }

  #[test]
  fn append_args_must_be_a_list_of_strings() {
    let error = parse(&json!({ "scripts": { "b": { "extends": "a", "appendArgs": "--run" } } }))
      .unwrap_err()
      .to_string();
    assert!(error.contains("appendArgs"), "{error}");

    let error = parse(&json!({ "scripts": { "b": { "extends": "a", "appendArgs": [1] } } }))
      .unwrap_err()
      .to_string();
    assert!(error.contains("appendArgs"), "{error}");
  }

  /// Test 4a.3 — all four branches, on every machine.
  ///
  /// The platform is an argument precisely so this table runs everywhere. Reading `cfg!`
  /// inside `select` would leave a Linux runner exercising one branch out of four, and
  /// the fallback branch exercised on no machine at all.
  #[test]
  fn per_os_selection_covers_every_branch() {
    let per_os = PerOsCommand {
      default: "make build".to_owned(),
      win32: Some("build.cmd".to_owned()),
      darwin: Some("make build-mac".to_owned()),
      linux: Some("make build-linux".to_owned()),
    };

    for (platform, expected) in [
      ("win32", "build.cmd"),
      ("darwin", "make build-mac"),
      ("linux", "make build-linux"),
      ("freebsd", "make build"),
    ] {
      assert_eq!(per_os.select(platform), expected, "on `{platform}`");
    }
  }

  /// A system the object names no entry for is the same case as a system rune has never
  /// heard of: `default` is what both get.
  #[test]
  fn an_absent_entry_falls_back_the_same_way_an_unknown_system_does() {
    let per_os = PerOsCommand {
      default: "rm -rf dist".to_owned(),
      win32: Some("rmdir /s /q dist".to_owned()),
      darwin: None,
      linux: None,
    };

    assert_eq!(per_os.select("win32"), "rmdir /s /q dist");
    assert_eq!(per_os.select("darwin"), "rm -rf dist");
  }

  #[test]
  fn a_per_os_command_parses() {
    let config = parse(&json!({
      "scripts": { "clean": { "command": { "default": "rm -rf dist", "win32": "rmdir /s /q dist" } } }
    }))
    .expect("parses");

    assert_eq!(command_of(&config, "clean", "win32"), "rmdir /s /q dist");
    assert_eq!(command_of(&config, "clean", "linux"), "rm -rf dist");
  }

  /// An object with only `default` is legal, and is the string form written the long way.
  #[test]
  fn a_per_os_command_with_only_default_behaves_like_a_string() {
    let config = parse(&json!({ "scripts": { "build": { "command": { "default": "tsc" } } } }))
      .expect("parses");

    assert_eq!(command_of(&config, "build", "win32"), "tsc");
  }

  #[test]
  fn a_per_os_command_rejects_a_key_that_is_not_an_operating_system() {
    let error = parse(&json!({
      "scripts": { "clean": { "command": { "default": "rm -rf dist", "windows": "rmdir" } } }
    }))
    .unwrap_err()
    .to_string();

    assert!(error.contains("`clean`"), "{error}");
    assert!(error.contains("`windows`"), "{error}");
  }

  #[test]
  fn a_command_that_is_neither_a_string_nor_an_object_is_rejected() {
    let error =
      parse(&json!({ "scripts": { "build": { "command": ["tsc"] } } })).unwrap_err().to_string();

    assert!(error.contains("`build`"), "{error}");
  }

  /// Test 2.13 — the typo case. Naming the field without naming the script leaves the
  /// user searching a config that may have thirty entries.
  #[test]
  fn an_unknown_field_names_the_script_and_the_field() {
    let error =
      parse(&json!({ "scripts": { "test": { "comand": "vitest" } } })).unwrap_err().to_string();

    assert!(error.contains("`test`"), "{error}");
    assert!(error.contains("`comand`"), "{error}");
  }

  /// Test 2.14 — listing the legal discriminants matters more as later changes add them.
  #[test]
  fn a_script_with_no_discriminant_lists_the_legal_ones() {
    let error = parse(&json!({ "scripts": { "empty": {} } })).unwrap_err().to_string();

    assert!(error.contains("`empty`"), "{error}");
    assert!(error.contains("`command`"), "{error}");
    assert!(error.contains("`extends`"), "{error}");
  }

  /// The `serial` half of the dispatch, with both of its fields.
  #[test]
  fn a_serial_group_parses() {
    let config = parse(&json!({
      "scripts": {
        "ci": { "serial": ["lint", "test"], "continueOnError": true },
      }
    }))
    .expect("parses");

    let Kind::Serial { members, continue_on_error } = &config.scripts["ci"].kind else {
      panic!("`ci` did not parse as a group");
    };
    assert_eq!(members, &["lint".to_owned(), "test".to_owned()]);
    assert!(*continue_on_error);
  }

  /// Stopping at the first failure is the default, so a group that says nothing about it
  /// must not quietly run everything.
  #[test]
  fn a_group_stops_at_the_first_failure_unless_it_says_otherwise() {
    let config = parse(&json!({ "scripts": { "ci": { "serial": ["lint"] } } })).expect("parses");

    let Kind::Serial { continue_on_error, .. } = &config.scripts["ci"].kind else {
      panic!("`ci` did not parse as a group");
    };
    assert!(!*continue_on_error);
  }

  #[test]
  fn a_serial_that_is_not_a_list_of_names_is_rejected() {
    let error =
      parse(&json!({ "scripts": { "ci": { "serial": "lint" } } })).unwrap_err().to_string();
    assert!(error.contains("`ci`"), "{error}");
    assert!(error.contains("`serial`"), "{error}");

    let error = parse(&json!({ "scripts": { "ci": { "serial": [1] } } })).unwrap_err().to_string();
    assert!(error.contains("element 0"), "{error}");
  }

  #[test]
  fn a_continue_on_error_that_is_not_a_boolean_is_rejected() {
    let error =
      parse(&json!({ "scripts": { "ci": { "serial": ["a"], "continueOnError": "yes" } } }))
        .unwrap_err()
        .to_string();

    assert!(error.contains("`ci`"), "{error}");
    assert!(error.contains("`continueOnError`"), "{error}");
  }

  /// Test 5a's schema half — a group also declaring a command has no meaning rune could
  /// pick, and the conflict message has to name both words.
  #[test]
  fn a_group_that_also_declares_a_command_is_rejected() {
    let error = parse(&json!({ "scripts": { "ci": { "serial": ["a"], "command": "vitest" } } }))
      .unwrap_err()
      .to_string();

    assert!(error.contains("`ci`"), "{error}");
    assert!(error.contains("`command`"), "{error}");
    assert!(error.contains("`serial`"), "{error}");
  }

  #[test]
  fn a_group_that_also_extends_is_rejected() {
    let error = parse(&json!({ "scripts": { "ci": { "serial": ["a"], "extends": "b" } } }))
      .unwrap_err()
      .to_string();

    assert!(error.contains("`extends`"), "{error}");
    assert!(error.contains("`serial`"), "{error}");
  }

  /// `appendArgs` on a group is caught by the rule that catches it anywhere else: there
  /// is no inherited command for the arguments to join.
  #[test]
  fn a_group_that_appends_arguments_is_rejected() {
    let error = parse(&json!({ "scripts": { "ci": { "serial": ["a"], "appendArgs": ["--x"] } } }))
      .unwrap_err()
      .to_string();

    assert!(error.contains("`ci`"), "{error}");
    assert!(error.contains("appendArgs"), "{error}");
  }

  #[test]
  fn a_command_script_carries_its_prerequisites() {
    let config = parse(&json!({
      "scripts": { "build": { "command": "tsc -b", "dependsOn": ["clean"] } }
    }))
    .expect("parses");

    assert_eq!(
      config.scripts["build"].depends_on.as_deref(),
      Some(["clean".to_owned()].as_slice())
    );
  }

  /// A group already says what runs and in which order. A second ordering on the same
  /// entry would have no unambiguous meaning, so it is refused rather than picked between.
  #[test]
  fn depends_on_beside_serial_is_rejected() {
    let error = parse(&json!({ "scripts": { "ci": { "serial": ["a"], "dependsOn": ["b"] } } }))
      .unwrap_err()
      .to_string();

    assert!(error.contains("`ci`"), "{error}");
    assert!(error.contains("`dependsOn`"), "{error}");
    assert!(error.contains("`serial`"), "{error}");
  }

  /// `dependsOn` says when a script runs, never what it runs, so it cannot stand on its
  /// own. The message has to be the missing-discriminant one, not "unknown field".
  #[test]
  fn depends_on_alone_is_not_a_variant() {
    let error = parse(&json!({ "scripts": { "build": { "dependsOn": ["clean"] } } }))
      .unwrap_err()
      .to_string();

    assert!(error.contains("`build`"), "{error}");
    assert!(error.contains("no command"), "{error}");
    assert!(error.contains("`serial`"), "{error}");
  }

  #[test]
  fn a_depends_on_that_is_not_a_list_of_names_is_rejected() {
    let error = parse(&json!({ "scripts": { "b": { "command": "x", "dependsOn": "a" } } }))
      .unwrap_err()
      .to_string();

    assert!(error.contains("`dependsOn`"), "{error}");
  }

  #[test]
  fn a_config_without_scripts_is_rejected() {
    assert!(parse(&json!({ "other": {} })).is_err());
  }

  #[test]
  fn a_script_that_is_not_an_object_is_rejected() {
    let error = parse(&json!({ "scripts": { "dev": "vite" } })).unwrap_err().to_string();
    assert!(error.contains("`dev`"), "{error}");
  }

  #[test]
  fn scripts_come_back_sorted() {
    let config = parse(&json!({ "scripts": { "z": { "command": "z" }, "a": { "command": "a" } } }))
      .expect("parses");

    assert_eq!(config.scripts.keys().collect::<Vec<_>>(), vec!["a", "z"]);
  }
}
