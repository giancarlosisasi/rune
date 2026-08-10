//! The CLI's own surface: version, help, and the exit/stream contract every later
//! subcommand inherits.

mod harness;

use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use harness::Test;

/// Test 1.1 — and the harness's first exercise. A failure here is as likely to be the
/// harness as the binary.
#[test]
fn version_is_printed_on_stdout() {
  Test::new().args(["--version"]).stdout_regex(r"^rune \d+\.\d+\.\d+\n").status(0).run();
}

/// Test 6a.13 — the printed version is the first line of the file the packaging step
/// rewrites on every bump, so the binary and the package it ships in cannot disagree.
#[test]
fn version_is_the_first_line_of_the_file_the_packaging_step_writes() {
  let file = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../version.txt");
  let recorded = fs::read_to_string(&file).expect("version.txt is readable");
  // `lines` drops a carriage return of its own, which is the same trim the binary does.
  let expected = recorded.lines().next().expect("version.txt has a first line");

  let output =
    Test::new().args(["--version"]).stdout_regex(r"^rune \d+\.\d+\.\d+\n").status(0).run();
  let stdout = String::from_utf8(output.stdout).expect("the version is utf-8");

  assert_eq!(stdout.lines().next(), Some(format!("rune {expected}").as_str()));
}

/// Test R5.6 — which copy of rune is running is a question rune has to be able to answer
/// about itself. Establishing it used to need sampling the process from outside the tool,
/// which is no help to a developer whose machine quietly disagrees with everyone else's.
///
/// Compared as a path, not as a string: a temporary or short-name spelling of the same
/// file is still the same file. A golden file would pin the test runner's layout instead.
#[test]
fn the_version_names_the_binary_that_answered() {
  let output = Test::new().args(["--version"]).stdout_regex(r"(?s)^rune .*\n.+").status(0).run();
  let stdout = String::from_utf8(output.stdout).expect("the version is utf-8");
  let mut lines = stdout.lines();

  assert_eq!(lines.next(), Some(format!("rune {}", env!("CARGO_PKG_VERSION")).as_str()));
  harness::assert_same_path(
    lines.next(),
    &harness::binary(),
    "the version query named a binary other than the one that ran",
  );
}

/// Test 1.2 — help is reachable and names what the binary accepts. The exact text is
/// test 6b.7's business; this one only has to keep failing if a subcommand disappears.
#[test]
fn help_names_every_subcommand() {
  let output = Test::new().args(["--help"]).stdout_regex(r"(?m)^Usage: rune").status(0).run();

  let stdout = String::from_utf8(output.stdout).expect("help is utf-8");
  for subcommand in ["run", "list", "inspect"] {
    assert!(stdout.contains(subcommand), "`--help` does not name `{subcommand}`:\n{stdout}");
  }
}

/// Test 6b.7 — the whole command-line surface, captured.
///
/// Deliberately not written in change 1: the surface grew in six later changes, and a
/// golden file written then would have churned six times while asserting nothing in
/// between. It is final for 1.0, so from here a diff in this snapshot is a change to a
/// published interface — something to review, never something to accept.
#[test]
fn the_help_surface_is_the_published_one() {
  const SURFACE: [&[&str]; 7] = [
    &["--help"],
    &["run", "--help"],
    &["list", "--help"],
    &["inspect", "--help"],
    &["init", "--help"],
    &["cache", "--help"],
    &["cache", "clear", "--help"],
  ];

  let mut surface = String::new();
  for args in SURFACE {
    let output = Test::new().args(args.iter().copied()).stdout_regex(r"(?s).*").run();
    let stdout = String::from_utf8(output.stdout).expect("help is utf-8").replace("\r\n", "\n");

    write!(surface, "$ rune {}\n{stdout}\n", args.join(" ")).expect("a String never fails");
  }

  insta::assert_snapshot!(surface);
}

/// Test 1.3 — the contract every subcommand inherits: rune's own failures exit non-zero
/// and go to stderr, because stdout belongs to the child process.
#[test]
fn a_failing_subcommand_reports_on_stderr_with_empty_stdout() {
  Test::new()
    .args(["inspect", "build"])
    .stdout("")
    .stderr_regex(r"(?s)no rune\.config\.ts found")
    .status(1)
    .run();
}

/// clap accepts the grammar but rejects an unknown subcommand itself. Same contract.
#[test]
fn unknown_subcommand_fails_on_stderr_with_empty_stdout() {
  Test::new()
    .args(["nope"])
    .stdout("")
    .stderr_regex(r"(?s)unrecognized subcommand")
    .status(2)
    .run();
}
