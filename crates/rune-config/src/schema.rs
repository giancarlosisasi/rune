//! The shape a resolved config must have.
//!
//! Dispatch is manual and explicit. `#[serde(untagged)]` was the obvious choice and is
//! the wrong one: it does not compose with `deny_unknown_fields`, and its failure reads
//! "data did not match any variant", which tells a user nothing about the mistake they
//! made. Here the discriminant keys are inspected by name, so the error can say which
//! script is wrong and which word in it is the problem.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;
use thiserror::Error;

/// The keys that decide which kind of script an entry is. Each later change adds one.
const DISCRIMINANTS: &[&str] = &["command"];

/// Fields any script may carry, whatever its discriminant.
const COMMON_FIELDS: &[&str] = &["description", "cwd", "env"];

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

  #[error("script `{script}` sets both `{first}` and `{second}` — a script may only be one kind")]
  ManyDiscriminants { script: String, first: String, second: String },

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

  #[error("script `{script}`: {source}")]
  Invalid { script: String, source: serde_json::Error },
}

fn list(names: &[&str]) -> String {
  names.iter().map(|name| format!("`{name}`")).collect::<Vec<_>>().join(", ")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
  /// Sorted by name, because `rune list` prints them in this order and a map with a
  /// random iteration order would make that output differ between runs.
  pub scripts: BTreeMap<String, Script>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Script {
  Command(CommandScript),
}

impl Script {
  pub fn description(&self) -> Option<&str> {
    match self {
      Self::Command(script) => script.description.as_deref(),
    }
  }

  /// Where the script runs. A relative value is resolved against the invoking package.
  pub fn cwd(&self) -> Option<&Path> {
    match self {
      Self::Command(script) => script.cwd.as_deref().map(Path::new),
    }
  }

  /// Variables the script sets for its own child, which win over inherited values.
  pub fn env(&self) -> &BTreeMap<String, String> {
    match self {
      Self::Command(script) => &script.env,
    }
  }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandScript {
  pub command: Command,
  pub description: Option<String>,
  pub cwd: Option<String>,
  pub env: BTreeMap<String, String>,
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

  let Some(discriminant) = found else {
    // A misspelled discriminant is indistinguishable from a missing one, and it is by
    // far the likelier mistake, so name the offending word when there is one.
    reject_unknown_fields(name, object, &allowed_with(DISCRIMINANTS))?;
    return Err(SchemaError::NoDiscriminant { script: name.to_owned() });
  };

  match *discriminant {
    "command" => {
      // Checked before deserializing so the message can name the script. serde's own
      // unknown-field error knows the field but not which entry it came from.
      reject_unknown_fields(name, object, &allowed_with(&["command"]))?;
      let command = parse_command(name, &object["command"])?;
      let common: Common = serde_json::from_value(entry.clone())
        .map_err(|source| SchemaError::Invalid { script: name.to_owned(), source })?;

      Ok(Script::Command(CommandScript {
        command,
        description: common.description,
        cwd: common.cwd,
        env: common.env,
      }))
    }
    other => unreachable!("`{other}` is listed as a discriminant but has no arm"),
  }
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

  use super::{PerOsCommand, Script, parse};

  #[test]
  fn a_command_script_parses() {
    let config = parse(&json!({
      "scripts": { "dev": { "command": "vite", "description": "start the dev server" } }
    }))
    .expect("parses");

    let Script::Command(dev) = &config.scripts["dev"];
    assert_eq!(dev.command.select("linux"), "vite");
    assert_eq!(dev.description.as_deref(), Some("start the dev server"));
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

    let Script::Command(clean) = &config.scripts["clean"];
    assert_eq!(clean.command.select("win32"), "rmdir /s /q dist");
    assert_eq!(clean.command.select("linux"), "rm -rf dist");
  }

  /// An object with only `default` is legal, and is the string form written the long way.
  #[test]
  fn a_per_os_command_with_only_default_behaves_like_a_string() {
    let config = parse(&json!({ "scripts": { "build": { "command": { "default": "tsc" } } } }))
      .expect("parses");

    let Script::Command(build) = &config.scripts["build"];
    assert_eq!(build.command.select("win32"), "tsc");
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
