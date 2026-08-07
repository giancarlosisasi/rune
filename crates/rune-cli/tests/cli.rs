//! The CLI's own surface: version, help, and the exit/stream contract every later
//! subcommand inherits.

mod harness;

use harness::Test;

/// Test 1.1 — and the harness's first exercise. A failure here is as likely to be the
/// harness as the binary.
#[test]
fn version_is_printed_on_stdout() {
  Test::new().args(["--version"]).stdout_regex(r"^rune \d+\.\d+\.\d+\n$").status(0).run();
}

/// Test 1.2 — no golden snapshot: the CLI surface still grows in five later changes, so
/// a full help file would churn five more times and assert nothing in between.
#[test]
fn help_names_every_subcommand() {
  let output = Test::new().args(["--help"]).stdout_regex(r"(?m)^Usage: rune").status(0).run();

  let stdout = String::from_utf8(output.stdout).expect("help is utf-8");
  for subcommand in ["run", "list", "inspect"] {
    assert!(stdout.contains(subcommand), "`--help` does not name `{subcommand}`:\n{stdout}");
  }
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
