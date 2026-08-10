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
/// This is the part that dates. A change that adds a field to the schema, or a kind to
/// the script union, decides here whether a new user is shown it — and the snapshot over
/// the generated text is what makes that decision impossible to skip.
const GUIDE: &str = r#"//
// A script runs a `command` of its own, `extends` another one and adds to it, or runs
// several others in `serial` or in `parallel`:
//
//   build:    { command: "tsc -b" }
//   build:ci: { extends: "build", appendArgs: ["--force"] }
//   ci:       { serial: ["lint", "build:ci", "test"] }
//   dev:      { parallel: ["dev:api", "dev:web"] }
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
  body: Body<'a>,
  description: Option<&'a str>,
}

/// What a generated script does.
#[derive(Debug, Clone, Copy)]
enum Body<'a> {
  Command(&'a str),
  /// A seeded `rune run <target>`: the entry stands for another name instead of starting
  /// rune again to reach it.
  Alias(&'a str),
}

/// The single script a starter config defines: enough to prove the install works, and
/// small enough that its shape is the example.
const STARTER_SCRIPT: Entry<'static> = Entry {
  name: "hello",
  body: Body::Command("echo rune is set up"),
  description: Some("Check that rune can run a script"),
};

/// What seeding a manifest produced: the scripts to write, and the two things the header
/// has to say about what was left out.
///
/// A warning printed once, during a command run once, does not survive to the moment the
/// script is run. The file is what survives, so the file carries both lists.
#[derive(Debug, Default)]
struct Seed<'a> {
  entries: Vec<Entry<'a>>,
  /// Names whose npm script only ran rune under that same name.
  already_managed: Vec<&'a str>,
  /// Alias targets nothing in the manifest defines, each named once.
  unseen_targets: Vec<&'a str>,
}

/// Writes a starter config beside the nearest `package.json`, or where the user is
/// standing when there is none.
///
/// One anchor serves the whole command. Writing where the terminal happens to be open
/// while seeding from a manifest above it gave the command two ideas of where the project
/// was, and a config below the root is invisible to the walk that looks for one — so the
/// write succeeded and nothing could use it.
pub fn run(from_package_json: bool) -> Result<(), String> {
  let started_in = working_directory()?;
  let manifest_path = nearest_package_json(&started_in);
  let directory =
    manifest_path.as_deref().and_then(Path::parent).map_or(started_in.clone(), Path::to_path_buf);
  let path = directory.join(CONFIG_FILE);

  // The manifest is read before anything is created, so a run that cannot find one
  // leaves the directory exactly as it was.
  let contents = if from_package_json {
    let source = manifest_path.ok_or_else(|| missing_package_json(&started_in))?;
    let manifest = read_manifest(&source)?;

    render(SEEDED_INTRO, &seeded(&manifest, &source)?)
  } else {
    render(STARTER_INTRO, &Seed { entries: vec![STARTER_SCRIPT], ..Seed::default() })
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

/// Every `scripts` entry, with the ones that only call rune read for what they name.
///
/// Deliberately mechanical otherwise: nothing here tries to spot commands that repeat and
/// factor them into an `extends` chain. Guessing which duplicates were intentional would
/// produce a config the user has to audit line by line, which is more work than writing
/// one.
fn seeded<'a>(manifest: &'a serde_json::Value, source: &Path) -> Result<Seed<'a>, String> {
  let Some(scripts) = manifest.get("scripts").and_then(serde_json::Value::as_object) else {
    return Ok(Seed::default());
  };

  let mut seed = Seed::default();
  for (name, command) in scripts {
    let command = command.as_str().ok_or_else(|| {
      format!(
        "`{name}` in {} is not a command\n\n\
         an npm script is a string, and this one is not, so there is nothing to copy.",
        source.display()
      )
    })?;

    let body = match redirection(command) {
      // Reaching its own name leaves nothing to copy: the script would start rune to run
      // the script that starts rune. Recording the name is the whole of what is left.
      Some(target) if target == name => {
        seed.already_managed.push(name);
        continue;
      }
      Some(target) => {
        if !scripts.contains_key(target) {
          seed.unseen_targets.push(target);
        }
        Body::Alias(target)
      }
      None => Body::Command(command),
    };

    seed.entries.push(Entry { name, body, description: None });
  }

  seed.unseen_targets.sort_unstable();
  seed.unseen_targets.dedup();

  Ok(seed)
}

/// The script name a command redirects to, when the command is nothing but rune running
/// one name.
///
/// Narrow on purpose. A command that chains rune with something else, or passes arguments
/// of its own, still does what it says, so it is copied as written. The trailing separator
/// is part of the shape because repositories wrote it to get arguments through a package
/// manager.
fn redirection(command: &str) -> Option<&str> {
  let words: Vec<&str> = command.split_whitespace().collect();
  let ["rune", "run", target, rest @ ..] = words.as_slice() else {
    return None;
  };

  (matches!(rest, [] | ["--"]) && !target.starts_with('-')).then_some(*target)
}

fn render(intro: &str, seed: &Seed<'_>) -> String {
  let mut body = String::new();
  for entry in &seed.entries {
    let _ = writeln!(body, "    {}: {{", key(entry.name));
    match entry.body {
      Body::Command(command) => {
        let _ = writeln!(body, "      command: {},", literal(command));
      }
      Body::Alias(target) => {
        let _ = writeln!(body, "      extends: {},", literal(target));
      }
    }
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

  format!("{intro}{}{GUIDE}\nexport default {{\n{scripts}}};\n", notes(seed))
}

/// The part of the header that reports what seeding did not copy, and what it could not
/// account for.
fn notes(seed: &Seed<'_>) -> String {
  let mut notes = String::new();

  if !seed.already_managed.is_empty() {
    let _ = write!(
      notes,
      "//\n\
       // These package.json scripts did nothing but call rune under their own name, so \
       rune\n// already manages them and they are not repeated here:\n//\n{}\n",
      listed(&seed.already_managed)
    );
  }

  if !seed.unseen_targets.is_empty() {
    let _ = write!(
      notes,
      "//\n\
       // The scripts above stand for these names, and nothing here defines them. Add \
       them, or\n// check the config they already live in:\n//\n{}\n",
      listed(&seed.unseen_targets)
    );
  }

  notes
}

/// `names` as indented comment lines, wrapped so the generated file stays readable.
fn listed(names: &[&str]) -> String {
  const WIDTH: usize = 88;
  const INDENT: &str = "//   ";

  let mut lines = vec![INDENT.to_owned()];
  for (at, name) in names.iter().enumerate() {
    let piece = format!("{name}{}", if at + 1 == names.len() { "" } else { "," });
    let line = lines.last_mut().expect("the list starts with one line");

    if line.len() == INDENT.len() {
      line.push_str(&piece);
    } else if line.len() + 1 + piece.len() > WIDTH {
      lines.push(format!("{INDENT}{piece}"));
    } else {
      line.push(' ');
      line.push_str(&piece);
    }
  }

  lines.join("\n")
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

  use super::{Body, Seed, key, literal, redirection, render, seeded};

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

    let seed = seeded(&manifest, std::path::Path::new("package.json"))
      .expect("a package with no scripts is not an error");

    assert!(seed.entries.is_empty());
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
    let generated = render("// none\n", &Seed::default());

    assert!(generated.contains("scripts: {},"), "{generated}");
  }

  fn body_of<'a>(seed: &Seed<'a>, name: &str) -> Body<'a> {
    seed
      .entries
      .iter()
      .find(|entry| entry.name == name)
      .unwrap_or_else(|| panic!("`{name}` is not in the generated config"))
      .body
  }

  /// Test R4.7 — a command that only runs another script is a redirection. Copying it
  /// writes a script that starts rune to reach a name rune could have reached itself.
  #[test]
  fn a_command_that_only_runs_another_script_becomes_an_alias_of_it() {
    let manifest = json!({ "scripts": {
      "clean": "rune run clean:all",
      "format": "rune run format:fix --",
    }});

    let seed = seeded(&manifest, std::path::Path::new("package.json")).expect("both are strings");

    assert!(matches!(body_of(&seed, "clean"), Body::Alias("clean:all")));
    assert!(
      matches!(body_of(&seed, "format"), Body::Alias("format:fix")),
      "the trailing separator some repositories wrote is part of the shape"
    );
  }

  /// The other half of R4.7: an entry that names itself has nothing left to copy, so the
  /// file records the name instead of writing a script that reaches itself.
  #[test]
  fn a_command_that_runs_its_own_name_is_recorded_rather_than_written() {
    let manifest = json!({ "scripts": { "build": "rune run build", "test": "vitest" } });

    let seed = seeded(&manifest, std::path::Path::new("package.json")).expect("both are strings");

    assert_eq!(seed.already_managed, ["build"]);
    assert!(seed.entries.iter().all(|entry| entry.name != "build"));
    assert!(matches!(body_of(&seed, "test"), Body::Command("vitest")));
  }

  /// An alias the manifest cannot account for is still written, and the header says which
  /// ones. The target usually lives in a rune config the seeder never reads.
  #[test]
  fn an_alias_target_the_manifest_never_defines_is_named_once() {
    let manifest = json!({ "scripts": {
      "clean": "rune run clean:all",
      "wipe": "rune run clean:all",
      "format": "rune run format:fix",
      "format:fix": "prettier --write .",
    }});

    let seed = seeded(&manifest, std::path::Path::new("package.json")).expect("all are strings");

    assert_eq!(seed.unseen_targets, ["clean:all"]);
  }

  /// Test R4.8 — recognition is deliberately narrow. Anything that is not exactly rune
  /// running one name still does what it says, so rewriting it would change what it does.
  #[test]
  fn anything_that_is_not_exactly_rune_running_one_name_is_copied_as_written() {
    const COPIED: [&str; 7] = [
      "rune run a && rune run b",
      "rune run build --watch",
      "rune run build -- --watch",
      "npx rune run build",
      "rune list",
      "rune run",
      "echo rune run build",
    ];

    for command in COPIED {
      assert_eq!(redirection(command), None, "`{command}` is a command, not a redirection");
    }

    let manifest = json!({ "scripts": { "build": COPIED[0] } });

    let seed = seeded(&manifest, std::path::Path::new("package.json")).expect("a string");

    assert!(matches!(body_of(&seed, "build"), Body::Command(copied) if copied == COPIED[0]));
  }
}
