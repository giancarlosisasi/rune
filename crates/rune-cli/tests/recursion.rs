//! A script whose command runs rune, through the real binary.
//!
//! The call is inside a shell command string, which rune deliberately does not parse, so
//! nothing in a config declares it and no cycle check can see it. What stops it is the
//! count rune carries into every child, and the only way to show the count works is to let
//! rune actually climb a chain of itself.
//!
//! Every fixture here starts at no depth and lets rune do the counting. Setting the
//! variable by hand would prove the refusal and say nothing about how the number got
//! there — except in the one row that is about the refusal arriving before the config is
//! read, where the count is beside the point.

mod harness;

use harness::Test;

/// The fake tool the legitimate nesting calls, so a real child proves the run went
/// through.
const TOOL: &str = "faketool.exe";

fn config(scripts: &str) -> String {
  format!("export default {{ scripts: {scripts} }};\n")
}

fn stderr_of(output: &std::process::Output) -> String {
  String::from_utf8_lossy(&output.stderr).replace("\r\n", "\n")
}

/// A repository whose one script runs rune on itself. The copy-paste a real user writes.
fn calls_itself() -> Test {
  Test::new()
    .config(&config(r#"{ loop: { command: "rune run loop" } }"#))
    .rune_as_a_tool()
    .args(["run", "loop"])
    .stdout("")
    .stderr_regex(r"^rune has started rune \d+ times")
    .status(1)
}

/// Test R19.1 — the runaway ends by itself, and says which script to look at.
///
/// Nothing stopped this before: it recursed until the operating system refused a fork,
/// two processes a level, writing nothing on either stream the whole way down.
#[test]
fn a_script_that_runs_itself_is_stopped_and_named() {
  let output = calls_itself().run();

  insta::with_settings!({ description => "a script whose command runs itself" }, {
    insta::assert_snapshot!(stderr_of(&output));
  });
}

/// Test R19.2 — the shape that decides the mechanism.
///
/// Neither name repeats before the next level, so a guard keyed on the script's own name
/// would let this straight through while it stops the row above. The oracle is that row's
/// own refusal, with the one thing that may differ substituted: the script it points at.
/// Everything else being identical is what says the two shapes meet one rule.
#[test]
fn two_scripts_that_run_each_other_are_stopped_the_same_way() {
  let output = Test::new()
    .config(&config(r#"{ a: { command: "rune run b" }, b: { command: "rune run a" } }"#))
    .rune_as_a_tool()
    .args(["run", "a"])
    .stdout("")
    .stderr_regex(r"^rune has started rune \d+ times")
    .status(1)
    .run();

  let counterpart = stderr_of(&calls_itself().run()).replace("inspect loop", "inspect a");
  assert_eq!(stderr_of(&output), counterpart, "one rule, one refusal, both shapes");
}

/// Test R19.4 — one level of nesting is the legitimate use of this whole path, and a
/// ceiling that caught it would be worse than no ceiling.
#[test]
fn a_script_that_runs_rune_on_another_script_still_works() {
  Test::new()
    .config(&config(&format!(
      r#"{{
        outer: {{ command: "rune run inner" }},
        inner: {{ command: "{TOOL} emit nested" }},
      }}"#
    )))
    .rune_as_a_tool()
    .tool(&format!("node_modules/.bin/{TOOL}"))
    .args(["run", "outer"])
    .stdout("nested")
    .status(0)
    .run();
}

/// Test R19.8 — the refusal comes before the config is read.
///
/// A config that cannot be parsed at all is the oracle: whichever of the two happens
/// first is the message on screen. Reading first would make every level of a runaway cost
/// an evaluation as well as a process start, and a runaway in a repository with a slow
/// config would take as long to stop as the config takes to load.
#[test]
fn the_refusal_arrives_before_the_config_is_read() {
  Test::new()
    .config("export default { scripts: {")
    .env("RUNE_DEPTH", "10")
    .args(["run", "anything"])
    .stdout("")
    .stderr_regex(r"^rune has started rune \d+ times")
    .status(1)
    .run();
}
