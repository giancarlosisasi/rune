//! Where a script runs, and what happens when that directory cannot be entered.
//!
//! A relative `cwd` is relative to the config that declared it. That is the whole rule,
//! and it is the same one `envFile` follows, so a config author learns it once. Anchoring
//! on the caller instead makes a shared definition mean a different place from every
//! package, which is the one thing a shared config exists to prevent.
//!
//! The default — no `cwd` at all — is unchanged and lives with the other environment rows
//! in `execution.rs`, because it is npm parity rather than part of this rule.

mod harness;

use harness::{Test, canonical, redact};

const TOOL: &str = "faketool.exe";

fn config(scripts: &str) -> String {
  format!("export default {{ scripts: {scripts} }};\n")
}

/// A repository whose root declares a script that runs somewhere else in the repository,
/// plus a package deep enough to have a subdirectory of its own.
fn repository(cwd: &str) -> Test {
  let script = format!(r#"{{ api: {{ command: "{TOOL} report-env", cwd: "{cwd}" }} }}"#);

  Test::new()
    .config(&config(&script))
    .file("package.json", "{ \"name\": \"fixture-root\", \"private\": true }\n")
    .file("apps/api/package.json", "{ \"name\": \"api\" }\n")
    .file("packages/ui/package.json", "{ \"name\": \"ui\" }\n")
    .file("packages/ui/src/keep.txt", "")
    .tool(&format!("node_modules/.bin/{TOOL}"))
}

/// Test R2.1 — the defect, in one assertion.
///
/// The child reports where it actually landed, which is the only oracle that can tell the
/// root's `apps/api` from a `packages/ui/apps/api` that never existed.
#[test]
fn a_relative_cwd_resolves_against_the_config_that_declared_it() {
  let test = repository("apps/api").args(["run", "api"]).stdout_regex(r"(?s)^\{.*\}\n$").status(0);

  let expected = canonical(&test.dir().join("apps/api"));
  let output = test.run_in("packages/ui");

  assert_eq!(cwd_of(&output.stdout), expected);
}

/// Test R2.2 — an explanation that changes with the explainer's location is worse than no
/// explanation, because the user has no way to know which of the answers is real.
#[test]
fn inspect_reports_one_directory_from_everywhere() {
  let test = repository("apps/api")
    .args(["inspect", "api"])
    .stdout_regex(r"(?m)^directory\s+apps/api$")
    .status(0);

  for from in [".", "packages/ui", "packages/ui/src"] {
    test.run_in(from);
  }
}

/// Test R2.6 — two different mistakes, told apart. The operating system reports both as
/// "not found", which is false for a path holding a perfectly real file.
#[test]
fn a_directory_that_is_not_there_and_a_path_that_is_a_file_read_differently() {
  let missing =
    repository("apps/nope").args(["run", "api"]).stdout("").stderr_regex(r"(?s).").status(1);
  let missing_output = missing.run_in("packages/ui");

  let file =
    repository("package.json").args(["run", "api"]).stdout("").stderr_regex(r"(?s).").status(1);
  let file_output = file.run_in("packages/ui");

  insta::with_settings!({ description => "a cwd naming a directory that is not there" }, {
    insta::assert_snapshot!(redact(missing.dir(), &stderr_of(&missing_output)));
  });
  insta::with_settings!({ description => "a cwd naming a file" }, {
    insta::assert_snapshot!(redact(file.dir(), &stderr_of(&file_output)));
  });
}

/// Test R2.7 — the check happens before the spawn, proven by a command that would announce
/// itself the moment it started.
#[test]
fn nothing_runs_when_the_directory_cannot_be_entered() {
  let script = format!(r#"{{ api: {{ command: "{TOOL} emit started", cwd: "apps/nope" }} }}"#);

  Test::new()
    .config(&config(&script))
    .file("package.json", "{ \"name\": \"fixture-root\", \"private\": true }\n")
    .tool(&format!("node_modules/.bin/{TOOL}"))
    .args(["run", "api"])
    .stdout("")
    .stderr_regex(r"(?s)cwd that does not exist")
    .status(1)
    .run();
}

fn cwd_of(stdout: &[u8]) -> std::path::PathBuf {
  let report: serde_json::Value =
    serde_json::from_slice(stdout).expect("rune-testkit reports JSON");
  let cwd = report["cwd"].as_str().expect("the report carries a working directory");

  canonical(std::path::Path::new(cwd))
}

fn stderr_of(output: &std::process::Output) -> String {
  String::from_utf8_lossy(&output.stderr).replace("\r\n", "\n")
}
