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

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandScript {
  pub command: String,
  #[serde(default)]
  pub description: Option<String>,
  #[serde(default)]
  pub cwd: Option<String>,
  #[serde(default)]
  pub env: BTreeMap<String, String>,
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
      serde_json::from_value(entry.clone())
        .map(Script::Command)
        .map_err(|source| SchemaError::Invalid { script: name.to_owned(), source })
    }
    other => unreachable!("`{other}` is listed as a discriminant but has no arm"),
  }
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

  use super::{Script, parse};

  #[test]
  fn a_command_script_parses() {
    let config = parse(&json!({
      "scripts": { "dev": { "command": "vite", "description": "start the dev server" } }
    }))
    .expect("parses");

    let Script::Command(dev) = &config.scripts["dev"];
    assert_eq!(dev.command, "vite");
    assert_eq!(dev.description.as_deref(), Some("start the dev server"));
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
