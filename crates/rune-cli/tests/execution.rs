//! `rune run` through the real binary: shells, environment, working directory and exit
//! codes.
//!
//! Signals, process trees and terminals live in `proc.rs`; the npm comparison lives in
//! `parity.rs`.

mod harness;

use harness::{FIXTURE_SUFFIX, Test, assert_same_path, canonical, testkit};

/// The fake tool every fixture that needs a real child calls. Always `.exe`, on every
/// operating system.
const TOOL: &str = "faketool.exe";

fn config(scripts: &str) -> String {
  format!("export default {{ scripts: {scripts} }};\n")
}

/// Test 3.1 — the whole change in one line: a command reaches a shell and its output
/// reaches the terminal untouched.
#[test]
fn a_simple_command_runs() {
  Test::new()
    .config(&config(r#"{ greet: { command: "echo hello" } }"#))
    .args(["run", "greet"])
    .stdout("hello\n")
    .status(0)
    .run();
}

/// Test 3.2 — operators are the shell's job, which is why rune spawns through one.
#[test]
fn shell_operators_chain_and_short_circuit() {
  Test::new()
    .config(&config(r#"{ chain: { command: "echo first && echo second" } }"#))
    .args(["run", "chain"])
    .stdout("first\nsecond\n")
    .status(0)
    .run();

  Test::new()
    .config(&config(r#"{ chain: { command: "exit 3 && echo unreachable" } }"#))
    .args(["run", "chain"])
    .stdout("")
    .status(3)
    .run();
}

/// Test 3.5 — the feature that makes `"command": "jest --watch"` work at all. The tool
/// exists only at the root and the script runs from a nested package.
#[test]
fn a_bare_tool_resolves_from_the_roots_bin_directory() {
  Test::new()
    .config(&config(&format!(r#"{{ probe: {{ command: "{TOOL} emit resolved" }} }}"#)))
    .file("packages/foo/package.json", "{}\n")
    .tool(&format!("node_modules/.bin/{TOOL}"))
    .args(["run", "probe"])
    .stdout("resolved")
    .status(0)
    .run_in("packages/foo");
}

/// The order half of the same feature: a package's own copy wins over the root's, or a
/// package that pinned an older tool silently runs the newer one.
#[test]
fn a_packages_own_tool_wins_over_the_roots() {
  Test::new()
    .config(&config(&format!(r#"{{ probe: {{ command: "{TOOL} emit nested" }} }}"#)))
    .file("packages/foo/package.json", "{}\n")
    .tool(&format!("node_modules/.bin/{TOOL}"))
    .tool(&format!("packages/foo/node_modules/.bin/{TOOL}"))
    .args(["run", "probe"])
    .stdout("nested")
    .status(0)
    .run_in("packages/foo");
}

/// Test 3.6 — what the child actually received, read back out of the child itself.
#[test]
fn the_child_environment_layers_correctly() {
  let script =
    format!(r#"{{ probe: {{ command: "{TOOL} report-env", env: {{ SHARED: "from-script" }} }} }}"#);

  let test = Test::new()
    .config(&config(&script))
    .file("packages/foo/package.json", "{}\n")
    .tool(&format!("node_modules/.bin/{TOOL}"))
    .env("SHARED", "from-parent")
    .args(["run", "probe"])
    .stdout_regex(r"(?s)^\{.*\}\n$")
    .status(0);

  let root = test.dir().to_path_buf();
  let output = test.run_in("packages/foo");
  let report = report_of(&output.stdout);

  assert_eq!(report["env"]["SHARED"], "from-script", "the script's env must win");
  assert_eq!(report["env"]["RUNE_SCRIPT_NAME"], "probe");
  assert_same_path(report["env"]["RUNE_ROOT"].as_str(), &root, "RUNE_ROOT");
  assert_same_path(
    report["env"]["RUNE_PACKAGE_DIR"].as_str(),
    &root.join("packages").join("foo"),
    "RUNE_PACKAGE_DIR",
  );
}

/// Test 3.7, first of three. Kept apart from the environment row so none of them can
/// pass by accident.
#[test]
fn the_default_working_directory_is_the_invoking_package() {
  let test = Test::new()
    .config(&config(&format!(r#"{{ probe: {{ command: "{TOOL} report-env" }} }}"#)))
    .file("packages/foo/package.json", "{}\n")
    .tool(&format!("node_modules/.bin/{TOOL}"))
    .args(["run", "probe"])
    .stdout_regex(r"(?s)^\{.*\}\n$")
    .status(0);

  let expected = canonical(&test.dir().join("packages/foo"));
  let output = test.run_in("packages/foo");

  assert_eq!(cwd_of(&output.stdout), expected);
}

#[test]
fn a_relative_cwd_resolves_against_the_invoking_package() {
  let script = format!(r#"{{ probe: {{ command: "{TOOL} report-env", cwd: "src" }} }}"#);

  let test = Test::new()
    .config(&config(&script))
    .file("packages/foo/package.json", "{}\n")
    .file("packages/foo/src/keep.txt", "")
    .tool(&format!("node_modules/.bin/{TOOL}"))
    .args(["run", "probe"])
    .stdout_regex(r"(?s)^\{.*\}\n$")
    .status(0);

  let expected = canonical(&test.dir().join("packages/foo/src"));
  let output = test.run_in("packages/foo");

  assert_eq!(cwd_of(&output.stdout), expected);
}

#[test]
fn an_absolute_cwd_is_used_as_given() {
  let elsewhere = tempfile::tempdir().expect("create a second temporary directory");
  let script = format!(
    r#"{{ probe: {{ command: "{TOOL} report-env", cwd: {} }} }}"#,
    serde_json::to_string(&elsewhere.path().to_string_lossy().to_string()).expect("json")
  );

  let output = Test::new()
    .config(&config(&script))
    .file("packages/foo/package.json", "{}\n")
    .tool(&format!("node_modules/.bin/{TOOL}"))
    .args(["run", "probe"])
    .stdout_regex(r"(?s)^\{.*\}\n$")
    .status(0)
    .run_in("packages/foo");

  assert_eq!(cwd_of(&output.stdout), canonical(elsewhere.path()));
}

/// Test 3.9 — the harness pins this variable for determinism everywhere else, which is
/// the opposite of asserting it, so this row opts out and asserts it.
#[test]
fn the_configured_shell_is_honored() {
  let test = Test::new()
    .config(&config(r#"{ greet: { command: "echo hello" } }"#))
    .tool(&format!("fake-shell{FIXTURE_SUFFIX}"))
    .shell(false)
    .args(["run", "greet"])
    .stdout("FAKE SHELL: echo hello\n")
    .status(0);

  let shell = test.dir().join(format!("fake-shell{FIXTURE_SUFFIX}"));
  test.env("npm_config_script_shell", &shell.to_string_lossy()).run();
}

/// Test 3.10 — `io::ErrorKind`-specialized, and never a panic.
#[test]
fn an_unusable_shell_names_itself() {
  let output = Test::new()
    .config(&config(r#"{ greet: { command: "echo hello" } }"#))
    .shell(false)
    .env("npm_config_script_shell", "no-such-shell-anywhere")
    .args(["run", "greet"])
    .stdout("")
    .stderr_regex(r"(?s).")
    .status(1)
    .run();

  insta::assert_snapshot!(String::from_utf8_lossy(&output.stderr).replace("\r\n", "\n"));
}

/// Test 3.11 — the whole point of propagating: tools speak through these numbers, and a
/// runner that collapses them to 0 or 1 throws away what the caller needed.
#[test]
fn exit_codes_pass_through_unchanged() {
  for code in [0, 1, 2, 7, 127] {
    Test::new()
      .config(&config(&format!(r#"{{ finish: {{ command: "exit {code}" }} }}"#)))
      .args(["run", "finish"])
      .stdout("")
      .status(code)
      .run();
  }
}

/// Test 3.3 — the message is the deliverable, so it is snapshotted.
#[test]
fn an_unknown_script_names_the_miss_and_suggests_the_close_match() {
  let output = Test::new()
    .config(&config(r#"{ build: { command: "tsc -b" }, test: { command: "vitest" } }"#))
    .args(["run", "biuld"])
    .stdout("")
    .stderr_regex(r"(?s).")
    .status(1)
    .run();

  insta::assert_snapshot!(String::from_utf8_lossy(&output.stderr).replace("\r\n", "\n"));
}

/// Test 3.20 — rune contributes nothing of its own to stdout, in success and in failure.
/// Stdout belongs to the child; anything rune added there would be indistinguishable
/// from the script's own output.
#[test]
fn rune_writes_nothing_to_stdout_around_the_child() {
  let output = Test::new()
    .config(&config(&format!(r#"{{ quiet: {{ command: "{TOOL} exit-code 0" }} }}"#)))
    .tool(&format!("node_modules/.bin/{TOOL}"))
    .args(["run", "quiet"])
    .stdout("")
    .stderr("")
    .status(0)
    .run();

  assert!(output.stdout.is_empty());

  Test::new()
    .config(&config(&format!(r#"{{ loud: {{ command: "{TOOL} exit-code 5" }} }}"#)))
    .tool(&format!("node_modules/.bin/{TOOL}"))
    .args(["run", "loud"])
    .stdout("")
    .stderr("")
    .status(5)
    .run();
}

/// Test 3.21 — the value proposition, asserted rather than described. One flag, changed
/// once at the root, reaches two packages, and no `package.json` was touched to do it.
#[test]
fn one_definition_at_the_root_serves_every_package() {
  let script = format!(r#"{{ check: {{ command: "{TOOL} emit checking --strict" }} }}"#);
  let test = Test::new()
    .config(&config(&script))
    .file("packages/foo/package.json", "{ \"name\": \"foo\" }\n")
    .file("packages/bar/package.json", "{ \"name\": \"bar\" }\n")
    .tool(&format!("node_modules/.bin/{TOOL}"))
    .args(["run", "check"])
    .stdout("checking--strict")
    .status(0);

  let manifests = ["packages/foo/package.json", "packages/bar/package.json"]
    .map(|relative| test.dir().join(relative));
  let before = manifests.clone().map(|path| std::fs::read(path).expect("read the manifest"));

  test.run_in("packages/foo");
  test.run_in("packages/bar");

  let after = manifests.map(|path| std::fs::read(path).expect("read the manifest"));
  assert_eq!(after, before, "a package.json changed");
}

fn report_of(stdout: &[u8]) -> serde_json::Value {
  serde_json::from_slice(stdout).expect("rune-testkit reports JSON")
}

fn cwd_of(stdout: &[u8]) -> std::path::PathBuf {
  let report = report_of(stdout);
  let cwd = report["cwd"].as_str().expect("the report carries a working directory");
  canonical(std::path::Path::new(cwd))
}

/// The fixture binary has to be built for any of this to mean anything.
#[test]
fn the_fixture_binary_exists() {
  assert!(testkit().is_file());
}
