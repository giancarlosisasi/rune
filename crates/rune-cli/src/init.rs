//! `rune init` — the first config, written for the user.
//!
//! What this command produces is a documentation surface before it is anything else: it
//! is the first rune config most people read, so the comments carry as much weight as
//! the fields. That is also why the file is generated as text rather than serialized —
//! no serializer keeps comments, and the comments are the point.

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::Path;

use rune_config::discover::{CONFIG_FILE, nearest_package_json};

use crate::script::working_directory;

/// The opening lines of a config written from nothing.
const STARTER_INTRO: &str = r"// Scripts for this repository. Every package runs them with `rune run <name>`, so a
// shared command changes here once instead of in every package.json.
";

/// The opening lines of a config seeded from an existing package.
const SEEDED_INTRO: &str = r"// Scripts for this repository, taken from package.json one for one. Every package runs
// them with `rune run <name>`.
//
// Where several of these repeat the same command, keep one and let the others `extends`
// it. That duplication is what rune exists to remove.
";

/// What a script may hold in the version that generated the file.
///
/// This is the part that dates. A change that adds a field to the schema decides here
/// whether a new user should be shown it.
const GUIDE: &str = r#"//
// A script runs a `command` of its own, `extends` another one and adds to it, or runs
// several others in `serial`:
//
//   build:    { command: "tsc -b" }
//   build:ci: { extends: "build", appendArgs: ["--force"] }
//   ci:       { serial: ["lint", "build:ci", "test"] }
//
// A group stops at the first member that fails and exits with that member's code; add
// `continueOnError: true` to run the rest anyway. `dependsOn` puts other scripts before
// a command of its own:
//
//   build: { command: "tsc -b", dependsOn: ["clean"] }
//
// A `command` may instead name one per operating system, with a `default` for the rest:
//
//   clean: { command: { default: "rm -rf dist", win32: "rmdir /s /q dist" } }
//
// Any script may add `description`, `cwd`, `env` and `envFile`.
//
// This file is TypeScript: variables, template strings and relative imports all work.
// Node globals are not available. To branch on the machine or the environment, import
// what rune supplies:
//
//   import { rune } from "@gio-labs/rune";
//
//   test: { command: rune.isCI ? "vitest --run" : "vitest" }
//
// That gives `rune.env`, `rune.platform` and `rune.isCI`.
"#;

/// One script in the file being generated.
#[derive(Debug)]
struct Entry<'a> {
  name: &'a str,
  command: &'a str,
  description: Option<&'a str>,
}

/// The single script a starter config defines: enough to prove the install works, and
/// small enough that its shape is the example.
const STARTER_SCRIPT: Entry<'static> = Entry {
  name: "hello",
  command: "echo rune is set up",
  description: Some("Check that rune can run a script"),
};

/// Writes a starter config into the directory rune was started in.
pub fn run(from_package_json: bool) -> Result<(), String> {
  let directory = working_directory()?;
  let path = directory.join(CONFIG_FILE);

  // The manifest is read before anything is created, so a run that cannot find one
  // leaves the directory exactly as it was.
  let contents = if from_package_json {
    let source =
      nearest_package_json(&directory).ok_or_else(|| missing_package_json(&directory))?;
    let manifest = read_manifest(&source)?;

    render(SEEDED_INTRO, &seeded(&manifest, &source)?)
  } else {
    render(STARTER_INTRO, &[STARTER_SCRIPT])
  };

  write_new(&path, &contents)?;
  // The product of this command is a file, so the report of it is a diagnostic like
  // everything else rune says about itself.
  rune_out::diagnostic(&format!("created {}", path.display()));

  Ok(())
}

fn read_manifest(path: &Path) -> Result<serde_json::Value, String> {
  let text = std::fs::read_to_string(path)
    .map_err(|error| format!("cannot read {}: {error}", path.display()))?;

  serde_json::from_str(&text)
    .map_err(|error| format!("{} is not valid JSON: {error}", path.display()))
}

/// Every `scripts` entry, unchanged.
///
/// Deliberately mechanical: nothing here tries to spot commands that repeat and factor
/// them into an `extends` chain. Guessing which duplicates were intentional would produce
/// a config the user has to audit line by line, which is more work than writing one.
fn seeded<'a>(manifest: &'a serde_json::Value, source: &Path) -> Result<Vec<Entry<'a>>, String> {
  let Some(scripts) = manifest.get("scripts").and_then(serde_json::Value::as_object) else {
    return Ok(Vec::new());
  };

  scripts
    .iter()
    .map(|(name, command)| {
      let command = command.as_str().ok_or_else(|| {
        format!(
          "`{name}` in {} is not a command\n\n\
           an npm script is a string, and this one is not, so there is nothing to copy.",
          source.display()
        )
      })?;

      Ok(Entry { name, command, description: None })
    })
    .collect()
}

fn render(intro: &str, entries: &[Entry<'_>]) -> String {
  let mut body = String::new();
  for entry in entries {
    let _ = writeln!(body, "    {}: {{", key(entry.name));
    let _ = writeln!(body, "      command: {},", literal(entry.command));
    if let Some(description) = entry.description {
      let _ = writeln!(body, "      description: {},", literal(description));
    }
    body.push_str("    },\n");
  }

  let scripts = if body.is_empty() {
    "  scripts: {},\n".to_owned()
  } else {
    format!("  scripts: {{\n{body}  }},\n")
  };

  format!("{intro}{GUIDE}\nexport default {{\n{scripts}}};\n")
}

/// A script name as an object key: bare when it reads as an identifier, quoted otherwise.
///
/// `test:ci` is an ordinary npm name and is not an identifier, so the quoting is the
/// common case rather than the exotic one.
fn key(name: &str) -> String {
  let mut characters = name.chars();
  let is_identifier = characters.next().is_some_and(starts_identifier)
    && characters.all(|character| starts_identifier(character) || character.is_ascii_digit());

  if is_identifier { name.to_owned() } else { literal(name) }
}

fn starts_identifier(character: char) -> bool {
  character.is_ascii_alphabetic() || character == '_' || character == '$'
}

/// A TypeScript string literal.
///
/// JSON's escaping is a subset of JavaScript's, so a serialized JSON string is already a
/// valid TypeScript one — quotes, backslashes and control characters included.
fn literal(text: &str) -> String {
  serde_json::Value::String(text.to_owned()).to_string()
}

/// Creates the file, or refuses because something is already there.
///
/// `create_new` is what makes the refusal airtight. Asking whether the file exists and
/// writing afterwards leaves a window in between, and the whole point of the refusal is
/// that hand-written work survives.
fn write_new(path: &Path, contents: &str) -> Result<(), String> {
  let mut file = match std::fs::File::create_new(path) {
    Ok(file) => file,
    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
      return Err(already_exists(path));
    }
    Err(error) => return Err(unwritable(path, &error)),
  };

  file.write_all(contents.as_bytes()).map_err(|error| unwritable(path, &error))
}

fn already_exists(path: &Path) -> String {
  format!(
    "{CONFIG_FILE} already exists\n\n  {}\n\n\
     nothing was written. a config is hand-written work, so this command never replaces \
     one:\nmove or remove that file if you meant to start over.",
    path.display()
  )
}

fn unwritable(path: &Path, error: &std::io::Error) -> String {
  format!("cannot write {}: {error}", path.display())
}

fn missing_package_json(started_from: &Path) -> String {
  format!(
    "no package.json found\n\n\
     searched upward from {} and stopped at the repository boundary.\n\n\
     run `rune init` on its own to write a starter config instead.",
    started_from.display()
  )
}

#[cfg(test)]
mod tests {
  use serde_json::json;

  use super::{key, literal, render, seeded};

  #[test]
  fn a_name_that_is_not_an_identifier_is_quoted() {
    assert_eq!(key("build"), "build");
    assert_eq!(key("build2"), "build2");
    assert_eq!(key("_private"), "_private");
    assert_eq!(key("test:ci"), "\"test:ci\"");
    assert_eq!(key("2fast"), "\"2fast\"");
    assert_eq!(key("with space"), "\"with space\"");
  }

  /// A command holding the quote character that ends its own literal is the case this
  /// generator has to survive: get it wrong and the file does not parse.
  #[test]
  fn a_command_carrying_quotes_and_backslashes_stays_a_valid_literal() {
    assert_eq!(literal(r#"eslint "src/**/*.ts""#), r#""eslint \"src/**/*.ts\"""#);
    assert_eq!(literal(r"copy a\b"), r#""copy a\\b""#);
  }

  #[test]
  fn a_package_without_scripts_seeds_nothing_rather_than_failing() {
    let manifest = json!({ "name": "fixture" });

    let entries = seeded(&manifest, std::path::Path::new("package.json"))
      .expect("a package with no scripts is not an error");

    assert!(entries.is_empty());
  }

  #[test]
  fn a_script_that_is_not_a_string_names_itself() {
    let manifest = json!({ "scripts": { "build": ["tsc", "-b"] } });

    let error = seeded(&manifest, std::path::Path::new("package.json")).unwrap_err();

    assert!(error.contains("`build`"), "{error}");
  }

  /// An empty `scripts` object still has to be an object the loader accepts.
  #[test]
  fn a_config_with_no_scripts_still_declares_the_object() {
    let generated = render("// none\n", &[]);

    assert!(generated.contains("scripts: {},"), "{generated}");
  }
}
