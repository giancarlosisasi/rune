//! Which file an `envFile` names, and what happens when it cannot be used.
//!
//! Everything here is about the file itself rather than about the environment it builds:
//! where a relative path points, and the two ways a declared file can be unusable. The
//! layering those files feed is exercised in `rune-cli/tests/precedence.rs`.
//!
//! All three checks run at load time. A config naming a file that is not there is broken
//! for every command, not only for the one script that happens to be run first.

use std::fs;

use rune_config::env::Environment;
use rune_config::inherit::Scope;
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

  load(&dir.path().join(from), &Environment::default()).unwrap_err().to_string()
}

/// Test 4c.6 — settled as a hard error rather than warn-and-continue.
///
/// Continuing would put the child in front of a missing variable, and the failure would
/// arrive as whatever that script does with an undefined `API_URL`: a wrong default, a
/// connection refused, or a test that passed because it tested nothing.
#[test]
fn a_declared_env_file_that_does_not_exist_is_an_error() {
  let error = rejection(
    &[("rune.config.ts", &config(r#"{ test: { command: "vitest", envFile: ".env" } }"#))],
    ".",
  );

  assert!(error.contains("`test`"), "the message must name the script: {error}");
  assert!(error.contains(".env"), "the message must name the resolved path: {error}");
  assert!(error.contains("rune.config.ts"), "the message must name the config: {error}");

  insta::with_settings!({ description => "a script naming a file that is not there" }, {
    insta::assert_snapshot!(error);
  });
}

/// The message earns the config it names when there are two of them: a package declaring
/// `.env` and a root declaring `.env` mean different files, and only the declaring config
/// says which one was wanted.
#[test]
fn a_missing_file_names_the_config_that_declared_it() {
  let error = rejection(
    &[
      ("rune.config.ts", &config(r#"{ test: { command: "vitest" } }"#)),
      ("package.json", "{ \"name\": \"fixture-root\" }\n"),
      ("packages/legacy/package.json", "{ \"name\": \"legacy\" }\n"),
      (
        "packages/legacy/rune.config.ts",
        &config(r#"{ test: { extends: "test", envFile: "./.env" } }"#),
      ),
    ],
    "packages/legacy",
  );

  assert!(
    error.contains("packages/legacy/.env"),
    "the path must resolve against the package, not the root: {error}"
  );
  assert!(
    error.contains("packages/legacy/rune.config.ts"),
    "the message must name the config that declared it: {error}"
  );
}

/// Test 4c.8 — `dotenvy` reports the text of the line it rejected and an offset inside
/// that text, never the line's position in the file, so rune counts the lines itself.
#[test]
fn a_malformed_assignment_names_the_file_and_the_line() {
  let error = rejection(
    &[
      ("rune.config.ts", &config(r#"{ test: { command: "vitest", envFile: ".env" } }"#)),
      (".env", "# the endpoint\nAPI_URL=https://example.test\nnot an assignment\n"),
    ],
    ".",
  );

  assert!(error.contains(".env"), "the message must name the file: {error}");
  assert!(error.contains("line 3"), "the message must name the line: {error}");

  insta::with_settings!({ description => "a line that is not an assignment" }, {
    insta::assert_snapshot!(error);
  });
}

/// Test 4c.5 — the same relative path in two configs is two files, because a path in a
/// config means what it means relative to that config. Resolving against the working
/// directory would make the answer depend on where the user was standing.
#[test]
fn each_config_resolves_its_own_relative_path() {
  let dir = repo(&[
    ("rune.config.ts", &config(r#"{ test: { command: "vitest", envFile: "./.env" } }"#)),
    ("package.json", "{ \"name\": \"fixture-root\" }\n"),
    (".env", "FROM_ROOT=yes\n"),
    ("packages/legacy/package.json", "{ \"name\": \"legacy\" }\n"),
    (
      "packages/legacy/rune.config.ts",
      &config(r#"{ test: { extends: "test", envFile: "./.env" } }"#),
    ),
    ("packages/legacy/.env", "FROM_PACKAGE=yes\n"),
  ]);

  let loaded = load(&dir.path().join("packages/legacy"), &Environment::default())
    .expect("both files exist and parse");
  let resolved = loaded.resolve("test", Scope::Nearest).expect("resolves").expect("defined");

  let sources: Vec<&str> = resolved.env_files.iter().map(|file| file.source.as_str()).collect();
  assert_eq!(sources, ["packages/legacy/.env", ".env"], "nearest first, and both took part");

  assert_eq!(resolved.env_files[0].assignments, [("FROM_PACKAGE".to_owned(), "yes".to_owned())]);
  assert_eq!(resolved.env_files[1].assignments, [("FROM_ROOT".to_owned(), "yes".to_owned())]);
}
