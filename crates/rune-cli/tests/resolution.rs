//! What happens to a command between the config and the shell: the per-operating-system
//! pick, and the arguments a user appends after `--`.
//!
//! The selection rules themselves are unit tested against every platform on every
//! machine. What only the real binary can show is that those rules are wired to the
//! machine this is running on, and that a shell receives what the user typed.

mod harness;

use harness::{Test, testkit};

const TOOL: &str = "faketool.exe";

fn config(scripts: &str) -> String {
  format!("export default {{ scripts: {scripts} }};\n")
}

/// The name this operating system is known by, worked out here rather than read from
/// rune, so that agreeing with rune means something.
const RUNNING_ON: &str = if cfg!(target_os = "windows") {
  "win32"
} else if cfg!(target_os = "macos") {
  "darwin"
} else {
  "linux"
};

/// Test 4a.4 — the pure selection function is wired to the real platform.
///
/// Every entry differs, so picking `default` — or picking somebody else's entry — is a
/// visible failure rather than a coincidence.
#[test]
fn the_running_system_runs_its_own_entry() {
  let script = r#"{ where: { command: {
    default: "echo default",
    win32: "echo win32",
    darwin: "echo darwin",
    linux: "echo linux",
  } } }"#;

  Test::new()
    .config(&config(script))
    .args(["run", "where"])
    .stdout(&format!("{RUNNING_ON}\n"))
    .status(0)
    .run();
}

/// A per-OS object that names no entry for this system falls back, end to end.
#[test]
fn a_system_without_an_entry_runs_default() {
  let script = r#"{ where: { command: { default: "echo fallback" } } }"#;

  Test::new().config(&config(script)).args(["run", "where"]).stdout("fallback\n").status(0).run();
}

/// Test 4a.1 — everything after `--`, appended and quoted per element.
///
/// The child reports its own argv, which is the only assertion that can tell "one
/// argument containing a space" from "two arguments" — the difference a quoting mistake
/// makes, and the one that echoing the command line back would hide.
#[test]
fn arguments_after_the_separator_reach_the_child_one_by_one() {
  let cases: &[(&str, &[&str], &[&str])] = &[
    ("a single flag", &["--watch"], &["report-env", "--watch"]),
    ("a value with spaces", &["--filter", "a b"], &["report-env", "--filter", "a b"]),
    ("nothing after the separator", &[], &["report-env"]),
    ("a separator of its own", &["--", "--watch"], &["report-env", "--", "--watch"]),
    ("shell metacharacters", &["a&b|c", "$HOME"], &["report-env", "a&b|c", "$HOME"]),
  ];

  for (description, passed, expected) in cases {
    assert_eq!(&argv_of(passed), expected, "{description}");
  }
}

/// Runs the fixture tool with `passed` after `--` and reports the argv it received.
fn argv_of(passed: &[&str]) -> Vec<String> {
  let mut args = vec!["run".to_owned(), "probe".to_owned(), "--".to_owned()];
  args.extend(passed.iter().map(|argument| (*argument).to_owned()));

  let test = Test::new()
    .config(&config(&format!(r#"{{ probe: {{ command: "{TOOL} report-env" }} }}"#)))
    .tool(&format!("node_modules/.bin/{TOOL}"))
    .args(args)
    .stdout_regex(r"(?s)^\{.*\}\n$")
    .status(0);

  let report: serde_json::Value =
    serde_json::from_slice(&test.run().stdout).expect("rune-testkit reports JSON");

  report["argv"]
    .as_array()
    .expect("the report carries an argv")
    .iter()
    .map(|argument| argument.as_str().expect("an argument is a string").to_owned())
    .collect()
}

/// The fixture binary has to be built for the argument tests to mean anything.
#[test]
fn the_fixture_binary_exists() {
  assert!(testkit().is_file());
}
