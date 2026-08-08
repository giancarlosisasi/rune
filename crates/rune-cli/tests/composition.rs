//! Scripts that run other scripts: ordering, failure, and the promise that a chained
//! child's output is still its own.
//!
//! Every ordering assertion here is made with a token in the output. None is made with
//! elapsed time: a chain that happens to be fast enough would pass a timing assertion on
//! the day the order broke.

mod harness;

use harness::Test;

/// The fake tool the fixtures use to leave evidence a run happened. Always `.exe`, on
/// every operating system.
const TOOL: &str = "faketool.exe";

fn config(scripts: &str) -> String {
  format!("export default {{ scripts: {scripts} }};\n")
}

/// Test 5a.1 — the prerequisite finishes before the command that declared it starts.
#[test]
fn a_prerequisite_runs_before_the_command_that_declared_it() {
  Test::new()
    .config(&config(
      r#"{
        clean: { command: "echo clean" },
        build: { command: "echo build", dependsOn: ["clean"] },
      }"#,
    ))
    .args(["run", "build"])
    .stdout("clean\nbuild\n")
    .status(0)
    .run();
}

/// Test 5a.2 — a prerequisite that fails takes the whole chain with it.
///
/// "The command never started" is asserted with a file the command would have created,
/// not with the absence of a token. An absent token also matches a command that ran and
/// printed nothing.
#[test]
fn a_failing_prerequisite_stops_the_chain_and_keeps_its_exit_code() {
  let test = Test::new()
    .config(&config(&format!(
      r#"{{
        clean: {{ command: "exit 2" }},
        build: {{ command: "{TOOL} touch built.txt", dependsOn: ["clean"] }},
      }}"#
    )))
    .tool(&format!("node_modules/.bin/{TOOL}"))
    .args(["run", "build"])
    .status(2);

  test.run();

  assert!(!test.dir().join("built.txt").exists(), "`build` must never start");
}

/// Test 5a.3 — the change's "done when", as a fixture: lint, then typecheck, then test.
///
/// A serial group is what replaces `npm-run-all` and a chain of `&&` inside a command
/// string. The exit code being the failing step's own, rather than a flat 1, is what lets
/// a CI job tell a lint failure from a type error.
#[test]
fn the_ci_chain_stops_at_the_first_failure_with_that_steps_exit_code() {
  let test = Test::new()
    .config(&config(&format!(
      r#"{{
        lint: {{ command: "echo lint" }},
        typecheck: {{ command: "echo typecheck && exit 2" }},
        test: {{ command: "{TOOL} touch tests-ran.txt" }},
        ci: {{ serial: ["lint", "typecheck", "test"] }},
      }}"#
    )))
    .tool(&format!("node_modules/.bin/{TOOL}"))
    .args(["run", "ci"])
    .stdout("lint\ntypecheck\n")
    .status(2);

  test.run();

  assert!(!test.dir().join("tests-ran.txt").exists(), "`test` must never start");
}

#[test]
fn a_group_whose_members_all_succeed_exits_zero() {
  Test::new()
    .config(&config(
      r#"{
        a: { command: "echo a" },
        b: { command: "echo b" },
        both: { serial: ["a", "b"] },
      }"#,
    ))
    .args(["run", "both"])
    .stdout("a\nb\n")
    .status(0)
    .run();
}

/// Test 5a.4 — every member runs, and the code is the **first** failure's.
///
/// First rather than last, because in a serial run first-in-time is also first-in-list:
/// it is the failure the user reads at the top of the log.
#[test]
fn continue_on_error_runs_every_member_and_reports_the_first_failure() {
  Test::new()
    .config(&config(
      r#"{
        a: { command: "echo a && exit 3" },
        b: { command: "echo b && exit 1" },
        c: { command: "echo c" },
        all: { serial: ["a", "b", "c"], continueOnError: true },
      }"#,
    ))
    .args(["run", "all"])
    .stdout("a\nb\nc\n")
    .status(3)
    .run();
}

/// Test 5a.7 — nesting runs flat, in the order the nesting implies.
///
/// Nesting is the only way to express structure inside a group. There is deliberately no
/// dependency graph between the members of one group: that is a task graph, and rune is a
/// script registry and runner.
#[test]
fn a_group_inside_a_group_runs_flat() {
  Test::new()
    .config(&config(
      r#"{
        a: { command: "echo a" },
        b: { command: "echo b" },
        c: { command: "echo c" },
        inner: { serial: ["b", "c"] },
        outer: { serial: ["a", "inner"] },
      }"#,
    ))
    .args(["run", "outer"])
    .stdout("a\nb\nc\n")
    .status(0)
    .run();
}

/// A nested group keeps its own failure policy: the inner one stops, the outer one carries
/// on to the member after it.
#[test]
fn a_nested_group_stops_while_the_outer_one_continues() {
  Test::new()
    .config(&config(
      r#"{
        a: { command: "echo a && exit 4" },
        b: { command: "echo b" },
        c: { command: "echo c" },
        inner: { serial: ["a", "b"] },
        outer: { serial: ["inner", "c"], continueOnError: true },
      }"#,
    ))
    .args(["run", "outer"])
    .stdout("a\nc\n")
    .status(4)
    .run();
}

/// Test 5a.8 — the row that proves `dependsOn` and `serial` are one mechanism.
///
/// If they were two implementations, a member's own prerequisites would be the thing the
/// second one forgot.
#[test]
fn a_member_brings_its_own_prerequisites() {
  Test::new()
    .config(&config(
      r#"{
        lint: { command: "echo lint" },
        codegen: { command: "echo codegen" },
        test: { command: "echo test", dependsOn: ["codegen"] },
        ci: { serial: ["lint", "test"] },
      }"#,
    ))
    .args(["run", "ci"])
    .stdout("lint\ncodegen\ntest\n")
    .status(0)
    .run();
}

/// Test 5a.9 — the guard against routing serial output through a multiplexer.
///
/// The oracle is the same script run on its own: identical bytes, chained or not. No
/// prefix, no injected newline, nothing rewritten. The payload deliberately ends without
/// a newline and carries a character outside ASCII, because those are the two things a
/// writer that reframes output gets wrong first.
#[test]
fn a_script_inside_a_chain_produces_exactly_its_own_bytes() {
  const PAYLOAD: &str = "half a line → 50%";

  let test = Test::new()
    .config(&config(&format!(
      r#"{{
        quiet: {{ command: "{TOOL} emit '{PAYLOAD}'" }},
        chain: {{ serial: ["quiet"] }},
      }}"#
    )))
    .tool(&format!("node_modules/.bin/{TOOL}"))
    .args(["run", "chain"])
    .stdout(PAYLOAD)
    .status(0);

  let chained = test.run();
  let alone = test.then_run(&["run", "quiet"]);

  assert_eq!(chained.stdout, alone.stdout, "a chained script's bytes must be its own");
}

/// Arguments after `--` go to one command, and a group has none. Appending them to every
/// member would run a flag against tools that never asked for it; dropping them silently
/// would leave a group that looks right and quietly loses what the user typed.
#[test]
fn arguments_after_the_separator_are_refused_for_a_group() {
  Test::new()
    .config(&config(
      r#"{
        test: { command: "echo test" },
        ci: { serial: ["test"] },
      }"#,
    ))
    .args(["run", "ci", "--", "--watch"])
    .stderr_regex(r"`ci` runs other scripts")
    .status(1)
    .run();
}

/// The arguments reach the script the user named, and no prerequisite of it.
#[test]
fn arguments_after_the_separator_reach_only_the_script_that_was_named() {
  Test::new()
    .config(&config(
      r#"{
        clean: { command: "echo clean" },
        build: { command: "echo build", dependsOn: ["clean"] },
      }"#,
    ))
    .args(["run", "build", "--", "--force"])
    .stdout("clean\nbuild --force\n")
    .status(0)
    .run();
}

/// `inspect` explains a group without running it. The command exists so that a name whose
/// meaning is not obvious can be explained, and a group is the least obvious name yet.
#[test]
fn inspect_explains_a_group_without_running_it() {
  let test = Test::new()
    .config(&config(&format!(
      r#"{{
        lint: {{ command: "echo lint" }},
        test: {{ command: "{TOOL} touch tests-ran.txt" }},
        ci: {{ serial: ["lint", "test"], continueOnError: true }},
      }}"#
    )))
    .tool(&format!("node_modules/.bin/{TOOL}"))
    .args(["inspect", "ci"])
    .stdout_regex(r"(?s)^ci\n\nruns +lint → test\n")
    .status(0);

  test.run();

  assert!(!test.dir().join("tests-ran.txt").exists(), "inspect must never spawn anything");
}
