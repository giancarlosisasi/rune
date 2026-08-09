//! What happens to a command between the config and the shell: the per-operating-system
//! pick, and the arguments a user appends to the script name.
//!
//! The selection rules themselves are unit tested against every platform on every
//! machine. What only the real binary can show is that those rules are wired to the
//! machine this is running on, and that a shell receives what the user typed.

mod harness;

use harness::{Test, monorepo, testkit};

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

/// Test R1.2 — the defect this grammar change exists for, reduced to one line.
///
/// A package manager appends what follows its own `--` to the script's command string,
/// so the separator is spent on the append and never reaches rune. Everything after the
/// script name is the command's.
#[test]
fn a_flag_with_no_separator_reaches_the_child() {
  assert_eq!(argv_of_run(&["--watch"]), ["report-env", "--watch"]);
}

/// Test R1.6 — the boundary is the script name, not rune's list of options. A token that
/// merely starts with a hyphen is an argument like any other, and rune says nothing.
#[test]
fn a_hyphenated_argument_rune_does_not_recognize_is_collected() {
  assert_eq!(argv_of_run(&["--nonesuch", "-x"]), ["report-env", "--nonesuch", "-x"]);
}

/// Test R1.3 — every spelling that worked before the separator became optional.
#[test]
fn the_separator_still_means_what_it_always_meant() {
  let cases: &[(&str, &[&str], &[&str])] = &[
    ("a single flag", &["--", "--watch"], &["report-env", "--watch"]),
    ("a value with spaces", &["--", "--filter", "a b"], &["report-env", "--filter", "a b"]),
    ("an empty separator", &["--"], &["report-env"]),
    ("a literal separator", &["--", "--", "--watch"], &["report-env", "--", "--watch"]),
  ];

  for (description, passed, expected) in cases {
    assert_eq!(&argv_of_run(passed), expected, "{description}");
  }
}

/// Test R1.4 — the whole of the new rule: rune's options come before the script name.
///
/// The narrowed definition is what makes both halves visible at once. `--root` read as
/// rune's drops `--nested`; `--watch` eaten by the parser never reaches the child.
#[test]
fn an_option_before_the_script_name_is_runes() {
  let argv = narrowed_probe(&["run", "--root", "probe", "--watch"]).0;

  assert_eq!(argv, ["report-env", "--watch"]);
}

/// Test R1.5 — the one case the positional rule can surprise someone, so it is never
/// silent. Resolution stays narrowed and the child receives the token.
#[test]
fn an_option_after_the_script_name_is_the_commands_and_is_reported() {
  let (argv, stderr) = narrowed_probe(&["run", "probe", "--root"]);

  assert_eq!(argv, ["report-env", "--nested", "--root"]);
  insta::assert_snapshot!(stderr);
}

/// Runs `args` against a root script the package narrows, from inside that package.
/// Returns the child's argv and what rune wrote to stderr.
fn narrowed_probe(args: &[&str]) -> (Vec<String>, String) {
  let test = monorepo(
    &format!(r#"{{ probe: {{ command: "{TOOL} report-env" }} }}"#),
    r#"{ probe: { extends: "probe", appendArgs: ["--nested"] } }"#,
  )
  .tool(&format!("node_modules/.bin/{TOOL}"))
  .args(args.iter().copied())
  .stdout_regex(r"(?s)^\{.*\}\n$")
  .stderr_regex(r"(?s)^.*$")
  .status(0);

  let output = test.run_in("packages/legacy");

  (argv_in(&output.stdout), String::from_utf8_lossy(&output.stderr).replace("\r\n", "\n"))
}

/// Runs the fixture tool with `passed` after `--` and reports the argv it received.
fn argv_of(passed: &[&str]) -> Vec<String> {
  let mut args = vec!["--".to_owned()];
  args.extend(passed.iter().map(|argument| (*argument).to_owned()));

  argv_of_run(&args.iter().map(String::as_str).collect::<Vec<_>>())
}

/// Runs `rune run probe` with `passed` spelled exactly as a user would type it, and
/// reports the argv the child received.
fn argv_of_run(passed: &[&str]) -> Vec<String> {
  let mut args = vec!["run".to_owned(), "probe".to_owned()];
  args.extend(passed.iter().map(|argument| (*argument).to_owned()));

  let test = Test::new()
    .config(&config(&format!(r#"{{ probe: {{ command: "{TOOL} report-env" }} }}"#)))
    .tool(&format!("node_modules/.bin/{TOOL}"))
    .args(args)
    .stdout_regex(r"(?s)^\{.*\}\n$")
    .status(0);

  argv_in(&test.run().stdout)
}

/// The argv the fixture tool reported back on stdout.
fn argv_in(stdout: &[u8]) -> Vec<String> {
  let report: serde_json::Value =
    serde_json::from_slice(stdout).expect("rune-testkit reports JSON");

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
