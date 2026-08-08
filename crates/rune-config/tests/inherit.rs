//! Resolution failures, and the words used to report them.
//!
//! The unit tests in `inherit.rs` hold the structure of each error. These go through
//! `load` and snapshot what a user actually reads, because for this change the message
//! *is* the deliverable: inheritance is the first feature that makes resolution
//! non-obvious, and a confusing explanation is as severe a defect as a wrong result.

mod paths;

use std::fs;

use paths::redact;
use rune_config::env::Environment;
use rune_config::load::load;
use tempfile::TempDir;

fn repo(files: &[(&str, &str)]) -> TempDir {
  let dir = tempfile::tempdir().expect("create tempdir");
  fs::create_dir_all(dir.path().join(".git")).expect("create the boundary directory");
  fs::write(dir.path().join(".git/HEAD"), "ref: refs/heads/main\n").expect("write the boundary");

  for (relative, contents) in files {
    let path = dir.path().join(relative);
    fs::create_dir_all(path.parent().expect("a fixture path has a parent"))
      .expect("create parents");
    fs::write(&path, contents).expect("write fixture file");
  }

  dir
}

fn config(scripts: &str) -> String {
  format!("export default {{ scripts: {scripts} }};\n")
}

/// The error `load` reports for a repository, starting the walk at `from`.
fn rejection(files: &[(&str, &str)], from: &str) -> String {
  let dir = repo(files);
  let error = load(&dir.path().join(from), &Environment::default()).unwrap_err().to_string();

  redact(dir.path(), &error)
}

/// Test 4b.4 — the path, rendered.
///
/// "cycle detected" would be true and useless: the user would have to rebuild their own
/// config graph by hand to find which two scripts were involved. Rejected at load time,
/// so a cycle in a script nobody ran today is still reported today.
#[test]
fn a_cycle_is_rejected_at_load_time_with_the_path_it_followed() {
  let error = rejection(
    &[("rune.config.ts", &config(r#"{ a: { extends: "b" }, b: { extends: "a" } }"#))],
    ".",
  );

  assert!(error.contains("a → b → a"), "the message must render the path: {error}");

  insta::with_settings!({ description => "two scripts extending each other" }, {
    insta::assert_snapshot!(error);
  });
}

/// Test 4b.3 — the missing target is named, and so is the name that was probably meant.
#[test]
fn extending_a_script_that_does_not_exist_names_it_and_suggests_the_closest() {
  let error = rejection(
    &[(
      "rune.config.ts",
      &config(
        r#"{ test: { command: "vitest" }, ci: { extends: "tests", appendArgs: ["--run"] } }"#,
      ),
    )],
    ".",
  );

  assert!(error.contains("`ci`"), "the message must name the script: {error}");
  assert!(error.contains("`tests`"), "the message must name the missing target: {error}");
  assert!(error.contains("`test`"), "the message must offer the close name: {error}");

  insta::with_settings!({ description => "extending a name no script defines" }, {
    insta::assert_snapshot!(error);
  });
}

/// Test 4b.8 — the guard the whole feature rests on.
///
/// Replacement is refused rather than warned about. The failure it prevents is silent:
/// the root command changes to fix a bug, every package picks it up except this one, and
/// the symptom surfaces weeks later in one package's CI with no obvious cause.
#[test]
fn a_package_replacing_a_root_script_is_rejected() {
  let error = rejection(
    &[
      ("rune.config.ts", &config(r#"{ test: { command: "jest" } }"#)),
      ("packages/legacy/package.json", "{ \"name\": \"legacy\" }\n"),
      (
        "packages/legacy/rune.config.ts",
        &config(r#"{ test: { command: "jest --maxWorkers=1" } }"#),
      ),
    ],
    "packages/legacy",
  );

  assert!(error.contains("`test`"), "the message must name the script: {error}");
  assert!(error.contains("extend"), "the message must point at the alternative: {error}");
  assert!(
    error.contains("packages/legacy/rune.config.ts"),
    "the message must name the config that did it: {error}"
  );

  insta::with_settings!({ description => "a package config replacing a root command" }, {
    insta::assert_snapshot!(error);
  });
}

/// A package defining a name the root does not is an ordinary script, not an override,
/// so the replacement guard must leave it alone.
#[test]
fn a_package_script_with_a_name_of_its_own_is_not_an_override() {
  let dir = repo(&[
    ("rune.config.ts", &config(r#"{ test: { command: "jest" } }"#)),
    ("packages/legacy/package.json", "{ \"name\": \"legacy\" }\n"),
    ("packages/legacy/rune.config.ts", &config(r#"{ codegen: { command: "echo generating" } }"#)),
  ]);

  let loaded = load(&dir.path().join("packages/legacy"), &Environment::default())
    .expect("a non-colliding package script is legal");

  assert_eq!(
    loaded.names(rune_config::inherit::Scope::Nearest).into_iter().collect::<Vec<_>>(),
    vec!["codegen", "test"]
  );
}
