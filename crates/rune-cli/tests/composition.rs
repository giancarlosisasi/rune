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

fn stderr(output: &std::process::Output) -> String {
  String::from_utf8_lossy(&output.stderr).replace("\r\n", "\n")
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
    .stderr("build stopped: prerequisite `clean` exited 2\n")
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
    .stderr(
      "
      → ci: lint
      → ci: typecheck
      ci failed at step 2 of 3: `typecheck` exited 2
      ",
    )
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
    .stderr(
      "
      → both: a
      → both: b
      ",
    )
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
    // Two failures, two lines: every step that failed is named where it failed, and the
    // group still answers with the first of them.
    .stderr(
      "
      → all: a
      all failed at step 1 of 3: `a` exited 3
      → all: b
      all failed at step 2 of 3: `b` exited 1
      → all: c
      ",
    )
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
    // The nesting the config declares is the nesting the log shows: the outer group names
    // `inner` as one of its steps, and `inner` then names its own.
    .stderr(
      "
      → outer: a
      → outer: inner
      → inner: b
      → inner: c
      ",
    )
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
    // Each group reports its own step, so a failure two levels down is attributable to
    // both the group that held it and the group that carried on afterwards.
    .stderr(
      "
      → outer: inner
      → inner: a
      inner failed at step 1 of 2: `a` exited 4
      outer failed at step 1 of 2: `inner` exited 4
      → outer: c
      ",
    )
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
    // Two lines, not three: `codegen` is a prerequisite, and a prerequisite that works is
    // invisible at the call site by design.
    .stderr(
      "
      → ci: lint
      → ci: test
      ",
    )
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
    // Rune's own line about the step, and nothing of it on the stream under test.
    .stderr("→ chain: quiet\n")
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

/// Test R15.1 — the chain a pull request runs, and the three lines that make its log
/// readable a week later.
///
/// Nothing in 127 lines of a green `ci` named a step before this, so a reader had to know
/// the config by heart to tell which output belonged to which step.
#[test]
fn every_step_of_a_chain_is_named_as_it_starts() {
  let output = Test::new()
    .config(&config(
      r#"{
        lint: { command: "echo lint" },
        typecheck: { command: "echo typecheck" },
        test: { command: "echo test" },
        ci: { serial: ["lint", "typecheck", "test"] },
      }"#,
    ))
    .args(["run", "ci"])
    .stdout("lint\ntypecheck\ntest\n")
    .stderr_regex(r"(?s)^→ ci: lint\n")
    .status(0)
    .run();

  insta::assert_snapshot!(stderr(&output));
}

/// Test R15.2 — the case the change exists for.
///
/// A step that passes without printing appeared nowhere at all, so its silence and its
/// non-existence looked identical to whoever read the log.
#[test]
fn a_step_that_prints_nothing_still_leaves_a_trace() {
  let output = Test::new()
    .config(&config(
      r#"{
        quiet: { command: "exit 0" },
        loud: { command: "echo loud" },
        both: { serial: ["quiet", "loud"] },
      }"#,
    ))
    .args(["run", "both"])
    .stdout("loud\n")
    .stderr_regex(r"(?s).*")
    .status(0)
    .run();

  assert!(
    stderr(&output).contains("→ both: quiet"),
    "a step that writes nothing itself must still leave the line that says it ran"
  );
}

/// Test R15.3 — the failing step, its position, and its code.
///
/// The account of this failure used to be the two words the child wrote. In a realistic
/// run the only locator anywhere is the child tool's own, which names a `package.json`
/// script in one package and not the step rune stopped at.
#[test]
fn the_failing_step_is_named_with_its_position() {
  let test = Test::new()
    .config(&config(&format!(
      r#"{{
        lint: {{ command: "echo lint" }},
        build: {{ command: "echo build refused && exit 42" }},
        test: {{ command: "{TOOL} touch tests-ran.txt" }},
        ci: {{ serial: ["lint", "build", "test"] }},
      }}"#
    )))
    .tool(&format!("node_modules/.bin/{TOOL}"))
    .args(["run", "ci"])
    .stdout("lint\nbuild refused\n")
    .stderr_regex(r"(?s)exited 42\n$")
    .status(42);

  let output = test.run();

  assert!(!test.dir().join("tests-ran.txt").exists(), "`test` must never start");
  insta::assert_snapshot!(stderr(&output));
}

/// Test R15.4 — an exit code from a script the user never named.
///
/// Both streams were empty here: `rune run build` ended 42, and 42 belongs to a script
/// that is not `build`. Anyone reading it attributes the code to `build`'s own command.
#[test]
fn a_failing_prerequisite_names_the_script_it_was_declared_for() {
  let test = Test::new()
    .config(&config(&format!(
      r#"{{
        "clean:all": {{ command: "echo clean blew up && exit 42" }},
        build: {{ command: "{TOOL} touch built.txt", dependsOn: ["clean:all"] }},
      }}"#
    )))
    .tool(&format!("node_modules/.bin/{TOOL}"))
    .args(["run", "build"])
    .stdout("clean blew up\n")
    .stderr_regex(r"(?s)exited 42\n$")
    .status(42);

  let output = test.run();

  assert!(!test.dir().join("built.txt").exists(), "`build` must never start");
  insta::assert_snapshot!(stderr(&output));
}

/// Test R15.5 — `dependsOn` exists to be invisible when it works.
///
/// The harness compares stderr on every run, so the empty expectation here is the
/// assertion: a prerequisite that succeeds adds nothing to the log of the script that
/// declared it.
#[test]
fn a_prerequisite_that_works_says_nothing() {
  Test::new()
    .config(&config(
      r#"{
        clean: { command: "echo clean" },
        build: { command: "echo build", dependsOn: ["clean"] },
      }"#,
    ))
    .args(["run", "build"])
    .stdout("clean\nbuild\n")
    .stderr("")
    .status(0)
    .run();
}

/// Test R15.6 — the rule that keeps the lines from becoming noise.
///
/// A lone script has nothing to disambiguate, and neither does one whose only extra is a
/// prerequisite nobody needs to hear about.
#[test]
fn a_single_script_writes_nothing_of_runes_own() {
  let test = Test::new()
    .config(&config(
      r#"{
        clean: { command: "echo clean" },
        alone: { command: "echo alone" },
        chained: { command: "echo chained", dependsOn: ["clean"] },
      }"#,
    ))
    .args(["run", "alone"])
    .stdout("alone\n")
    .stderr("")
    .status(0);

  test.run();

  let chained = test.then_run(&["run", "chained"]);
  assert!(chained.stderr.is_empty(), "a prerequisite that works leaves the log alone");
}

/// Test R15.7 — stdout belongs to the children, in a chain as much as alone.
///
/// The oracle is the members run one at a time. A literal would pass just as well against
/// a chain that had quietly added a line of rune's own to the stream a pipe reads.
#[test]
fn nothing_rune_writes_about_steps_reaches_stdout() {
  let test = Test::new()
    .config(&config(
      r#"{
        a: { command: "echo a" },
        b: { command: "echo b" },
        chain: { serial: ["a", "b"] },
      }"#,
    ))
    .args(["run", "chain"])
    .stdout("a\nb\n")
    .stderr_regex(r"(?s)^→ chain: a\n→ chain: b\n$")
    .status(0);

  let chained = test.run();
  let a = test.then_run(&["run", "a"]);
  let b = test.then_run(&["run", "b"]);

  let alone: Vec<u8> = a.stdout.iter().chain(b.stdout.iter()).copied().collect();
  assert_eq!(chained.stdout, alone, "a chain's stdout must be exactly its members' bytes");
}

/// Test R15.8 — a chain nested inside a group that is prefixing its members.
///
/// The members' bytes and rune's own lines travel on different streams, which is what
/// makes this true rather than lucky. Nothing here asserts an order between the two
/// members: that is a coincidence, not a property.
#[test]
fn a_nested_chains_lines_never_split_a_members_line() {
  let output = Test::new()
    .config(&config(
      r#"{
        one: { command: "echo one" },
        two: { command: "echo two" },
        inner: { serial: ["one", "two"] },
        other: { command: "echo other" },
        dev: { parallel: ["inner", "other"] },
      }"#,
    ))
    .args(["run", "dev"])
    .stdout_regex(r"(?s).*")
    .stderr_regex(r"(?s).*")
    .status(0)
    .run();

  let stdout = String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n");
  let mut lines: Vec<&str> = stdout.lines().collect();
  lines.sort_unstable();

  assert_eq!(lines, vec!["[inner] one", "[inner] two", "[other] other"]);
  assert!(
    stderr(&output).contains("→ inner: one") && stderr(&output).contains("→ inner: two"),
    "a nested chain still names its steps"
  );
}

/// Test R16.4 — two dependents, two runs, in both kinds of group.
///
/// Counted rather than observed: each run of the prerequisite leaves a file named after
/// its own process, so the answer is a number instead of an inference from what happened
/// to be on screen. Two files rather than two appends to one, because the members of a
/// parallel group reach the prerequisite at the same moment.
///
/// Rune orders scripts and does not deduplicate. Deciding a prerequisite had already run
/// would need the result cache and the ordering model this product lives without.
#[test]
fn a_shared_prerequisite_runs_once_per_dependent() {
  for kind in ["serial", "parallel"] {
    let test = Test::new()
      .config(&config(&format!(
        r#"{{
          setup: {{ command: "echo ran > ran-$$.txt" }},
          a: {{ command: "echo a", dependsOn: ["setup"] }},
          b: {{ command: "echo b", dependsOn: ["setup"] }},
          both: {{ {kind}: ["a", "b"] }},
        }}"#
      )))
      .args(["run", "both"])
      .stdout_regex(r"(?s).*")
      .stderr_regex(r"(?s).*")
      .status(0);

    test.run();

    let runs = std::fs::read_dir(test.dir())
      .expect("read the fixture directory")
      .filter_map(Result::ok)
      .filter(|entry| entry.file_name().to_string_lossy().starts_with("ran-"))
      .count();

    assert_eq!(runs, 2, "`setup` must run once for each dependent, in a {kind} group");
  }
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
