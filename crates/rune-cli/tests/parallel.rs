//! Scripts running at the same time: what their output looks like, and how the group
//! answers for them.
//!
//! Two rules run through every test here. Nothing asserts that one member's line came
//! before another's — between concurrent processes that is not a property, it is a
//! coincidence — so the assertions are about attributability and about sorted sets. And
//! nothing sleeps: where an order genuinely matters it is forced with a marker file the
//! members hand between themselves.

mod harness;

use std::time::Duration;

use harness::{Test, wait_until};

/// The fake tool the fixtures call. Always `.exe`, on every operating system.
const TOOL: &str = "faketool.exe";

/// How long an operating system is given to finish tearing a process tree down. Generous
/// on purpose: it bounds a failure, it does not sequence anything.
const TEARDOWN_LIMIT: Duration = Duration::from_secs(20);

fn config(scripts: &str) -> String {
  format!("export default {{ scripts: {scripts} }};\n")
}

/// A fixture with the fake tool installed where a package's own binaries live.
fn fixture(scripts: &str) -> Test {
  Test::new().config(&config(scripts)).tool(&format!("node_modules/.bin/{TOOL}"))
}

/// Every line of a run's output, with the trailing blank one dropped.
fn lines(output: &[u8]) -> Vec<String> {
  String::from_utf8_lossy(output).replace("\r\n", "\n").lines().map(str::to_owned).collect()
}

/// Test 5c.1 — two members writing continuously, and every line still belongs to exactly
/// one of them.
///
/// The payload carries its own label, so the prefix rune wrote can be checked against the
/// process that actually produced the bytes. Without that the test would be asserting the
/// prefix against itself.
#[test]
fn every_line_of_a_parallel_run_carries_one_prefix_and_one_members_bytes() {
  const EACH: u32 = 200;

  let test = fixture(&format!(
    r#"{{
      one: {{ command: "{TOOL} chatty ONE {EACH}" }},
      two: {{ command: "{TOOL} chatty TWO {EACH}" }},
      both: {{ parallel: ["one", "two"] }}
    }}"#
  ))
  .args(["run", "both"])
  .stdout_regex(r"(?s).*")
  .status(0);

  let output = test.run();
  let lines = lines(&output.stdout);

  for line in &lines {
    let attributed = line.starts_with("[one] ONE ") || line.starts_with("[two] TWO ");
    assert!(attributed, "a line carried the wrong prefix or two members' bytes: {line:?}");
  }

  let mut expected: Vec<String> = (1..=EACH)
    .map(|number| format!("[one] ONE {number}"))
    .chain((1..=EACH).map(|number| format!("[two] TWO {number}")))
    .collect();
  expected.sort();

  let mut seen = lines.clone();
  seen.sort();

  assert_eq!(seen, expected, "every line each member wrote must arrive exactly once");
}

/// Test 5c.2 — a group of one is a single script wearing a group's name.
///
/// Stated in the architecture and never tested. Prefixing it is the easy regression to
/// introduce while making the general case work, and it would silently degrade the
/// terminal behavior of a config that happens to wrap one script.
#[test]
fn a_parallel_group_of_one_member_writes_no_prefix() {
  fixture(&format!(
    r#"{{
      only: {{ command: "{TOOL} chatty SOLO 2" }},
      wrapped: {{ parallel: ["only"] }}
    }}"#
  ))
  .args(["run", "wrapped"])
  .stdout("SOLO 1\nSOLO 2\n")
  .run();
}

/// Test 5c.3 — a failing member takes its sibling's whole tree with it.
///
/// The grandchild is the assertion with teeth. Every command runs through a shell, so the
/// process rune holds is the shell and the tool doing the work is its child: killing only
/// what rune holds leaves the real work running.
#[test]
fn a_failing_member_ends_its_siblings_whole_tree_and_the_group_exits_its_code() {
  let test = fixture(&format!(
    r#"{{
      hold: {{ command: "{TOOL} spawn-child pids.json" }},
      boom: {{ command: "{TOOL} await pids.json 2" }},
      both: {{ parallel: ["hold", "boom"] }}
    }}"#
  ))
  .args(["run", "both"])
  .stdout_regex(r"(?s).*")
  .status(2);

  test.run();

  let (child, grandchild) = harness::process_ids_at(&test.dir().join("pids.json"));
  for (name, pid) in [("the sibling", child), ("its grandchild", grandchild)] {
    assert!(
      wait_until(TEARDOWN_LIMIT, || !rune_exec::teardown::is_running(pid)),
      "{name} outlived the member that failed"
    );
  }
}

/// Test 5c.4 — `continueOnError` lets the sibling finish, and the group still fails.
///
/// Both halves matter. A group that stopped the sibling would make the option useless,
/// and a group that reported success would hide a failure the user has to act on.
#[test]
fn continue_on_error_lets_the_sibling_finish_and_the_group_still_fails() {
  let test = fixture(&format!(
    r#"{{
      quick: {{ command: "{TOOL} mark stop.marker 3" }},
      slow: {{ command: "{TOOL} await stop.marker 0 && {TOOL} chatty FINISHED 1" }},
      both: {{ parallel: ["quick", "slow"], continueOnError: true }}
    }}"#
  ))
  .args(["run", "both"])
  .stdout_regex(r"(?s).*")
  .status(3);

  let output = test.run();
  let lines = lines(&output.stdout);

  assert!(
    lines.iter().any(|line| line == "[slow] FINISHED 1"),
    "the sibling was cut short: {lines:?}"
  );
}

/// Test 5c.5, the branch that needs a clock — a member that traps the termination signal
/// is killed unconditionally once the kill timeout has passed.
///
/// Windows has no signal for a process to trap: a job object is terminated or it is not,
/// so there is no second branch there to assert.
#[test]
#[cfg(unix)]
fn a_member_that_traps_the_signal_is_killed_once_the_timeout_passes() {
  // The marker is written the moment the handler is installed, so the sibling fails
  // against a member that is genuinely armed rather than one that merely started.
  let test = fixture(&format!(
    r#"{{
      stubborn: {{ command: "exec {TOOL} trap-term armed.marker" }},
      boom: {{ command: "{TOOL} await armed.marker 9" }},
      both: {{ parallel: ["stubborn", "boom"] }}
    }}"#
  ))
  .args(["run", "both"])
  .stdout_regex(r"(?s).*")
  .status(9);

  let output = test.run();
  let text = String::from_utf8_lossy(&output.stdout);

  assert!(
    text.contains("trapped SIGTERM"),
    "the member was never asked politely before being killed: {text}"
  );
}

/// Test 5c.5, the branch with no clock in it — ending a tree that has already gone is
/// never an error.
///
/// The reference implementation's version of this path has a real bug and no test, which
/// is why it is asserted here on its own rather than left implied by a bigger scenario.
#[test]
fn a_member_that_already_exited_leaves_the_group_shutting_down_quietly() {
  let test = fixture(&format!(
    r#"{{
      brief: {{ command: "{TOOL} mark gone.marker 0" }},
      boom: {{ command: "{TOOL} await gone.marker 4" }},
      linger: {{ command: "{TOOL} await never.marker 0" }},
      all: {{ parallel: ["brief", "boom", "linger"] }}
    }}"#
  ))
  .args(["run", "all"])
  .stdout_regex(r"(?s).*")
  .stderr("")
  .status(4);

  test.run();
}

/// Test 5c.6 — the three success policies, with the exit order forced rather than hoped
/// for.
///
/// `early` is listed second and exits first, which is the whole point: a member's place
/// in a list says nothing about when it finishes. `continueOnError` keeps a teardown out
/// of the way, so what is under test is the policy and nothing else.
#[test]
fn a_success_policy_reads_the_exits_in_the_order_they_happened() {
  for (policy, expected) in [("first", 0), ("last", 5), ("all", 5)] {
    let test = fixture(&format!(
      r#"{{
        early: {{ command: "{TOOL} mark first.marker 0" }},
        late: {{ command: "{TOOL} after first.marker 5" }},
        both: {{
          parallel: ["late", "early"],
          continueOnError: true,
          successPolicy: "{policy}"
        }}
      }}"#
    ))
    .args(["run", "both"])
    .stdout_regex(r"(?s).*")
    .status(expected);

    test.run();
  }
}

/// Test 5c.7 — two members fail, and the group reports the one that failed first in time.
///
/// The failing member listed second is the one that exits first. Reporting the other
/// would point a reader at the failure that was caused rather than the one that caused it.
#[test]
fn two_failing_members_report_the_code_of_the_one_that_failed_first() {
  fixture(&format!(
    r#"{{
      later: {{ command: "{TOOL} after earlier.marker 1" }},
      earlier: {{ command: "{TOOL} mark earlier.marker 3" }},
      both: {{ parallel: ["later", "earlier"], continueOnError: true }}
    }}"#
  ))
  .args(["run", "both"])
  .stdout_regex(r"(?s).*")
  .status(3)
  .run();
}

/// Test 5c.11 — a member terminated because a sibling failed keeps everything it said.
///
/// Truncating the tail is a standard multiplexer bug: the reader tasks are dropped along
/// with the processes, and the last thing the member wrote — usually the line explaining
/// what went wrong — never reaches the terminal.
#[test]
fn a_terminated_members_output_arrives_in_full() {
  const LINES: u32 = 500;

  let test = fixture(&format!(
    r#"{{
      loud: {{ command: "{TOOL} chatty-hold LOUD {LINES} said.marker" }},
      boom: {{ command: "{TOOL} await said.marker 7" }},
      both: {{ parallel: ["loud", "boom"] }}
    }}"#
  ))
  .args(["run", "both"])
  .stdout_regex(r"(?s).*")
  .status(7);

  let output = test.run();
  let seen: Vec<String> =
    lines(&output.stdout).into_iter().filter(|line| line.starts_with("[loud] ")).collect();

  let expected: Vec<String> = (1..=LINES).map(|number| format!("[loud] LOUD {number}")).collect();

  assert_eq!(seen, expected, "the terminated member's output was cut short");
}

/// Test 5c.13 — a tool that colors only when told its output supports color still does so
/// under a prefix.
///
/// A piped child sees no terminal and turns its own color off, so without rune passing
/// the level on, running two scripts together would silently drain the color out of both.
#[test]
fn a_piped_member_still_colors_its_output() {
  let test = fixture(&format!(
    r#"{{
      paint: {{ command: "{TOOL} colorize hello" }},
      plain: {{ command: "{TOOL} chatty X 1" }},
      both: {{ parallel: ["paint", "plain"] }}
    }}"#
  ))
  .env("FORCE_COLOR", "1")
  .args(["run", "both"])
  .stdout_regex(r"(?s).*")
  .status(0);

  let output = test.run();
  let text = String::from_utf8_lossy(&output.stdout);

  assert!(text.contains("\u{1b}[31mhello\u{1b}[0m"), "the member's color was lost: {text:?}");
  assert!(text.contains("[paint]"), "the colored line lost its prefix: {text:?}");
}

/// The `dev` fixture: a server and a watcher, which is the case this whole change exists
/// for.
///
/// Both are long-running, both have a worker process of their own, and neither ends on its
/// own — so every question the feature has to answer is live at once. The two tests below
/// are the two ways such a run actually ends.
fn dev_fixture(watcher: &str) -> Test {
  fixture(&format!(
    r#"{{
      "dev:server": {{ command: "{TOOL} spawn-child server.json" }},
      "dev:watch": {{ command: "{watcher}" }},
      dev: {{ parallel: ["dev:server", "dev:watch"] }}
    }}"#
  ))
  .args(["run", "dev"])
}

/// Test 5c.14, the failure path — the watcher fails and the server's whole tree goes with
/// it.
///
/// A dev run whose watcher has died is not a dev run, and a server left holding the port
/// is what makes the next `rune run dev` fail for a reason that has nothing to do with the
/// code. The worker process is the probe: killing only what rune holds would leave it.
#[test]
fn the_dev_fixture_ends_the_server_when_the_watcher_fails() {
  let test = dev_fixture(&format!("{TOOL} await server.json 1")).stdout_regex(r"(?s).*").status(1);

  test.run();

  let (server, worker) = harness::process_ids_at(&test.dir().join("server.json"));
  for (name, pid) in [("the server", server), ("its worker", worker)] {
    assert!(
      wait_until(TEARDOWN_LIMIT, || !rune_exec::teardown::is_running(pid)),
      "{name} outlived the failing watcher"
    );
  }
}

/// Test 5c.14, the interrupt path — Ctrl+C over a live dev run leaves nothing behind.
///
/// This is the ending a dev run actually gets, every day. An orphan here is the worst
/// failure the feature has: it survives the terminal that started it, and the user finds
/// out at the next run.
#[test]
fn the_dev_fixture_leaves_nothing_behind_when_interrupted() {
  let test = dev_fixture(&format!("{TOOL} spawn-child watcher.json"));
  let mut command = test.command(test.dir());
  let mut child = harness::interruptible(&mut command).spawn().expect("spawn rune");

  let recorded = [test.dir().join("server.json"), test.dir().join("watcher.json")];
  assert!(
    wait_until(TEARDOWN_LIMIT, || recorded.iter().all(|path| path.exists())),
    "the dev run never came up"
  );
  let trees: Vec<(u32, u32)> = recorded.iter().map(|path| harness::process_ids_at(path)).collect();

  harness::interrupt(&child);
  let status = child.wait().expect("collect rune");

  assert!(status.code() != Some(0), "an interrupted dev run must not report success");

  for (member, (direct, worker)) in trees.iter().enumerate() {
    for (name, pid) in [("the member", *direct), ("its worker", *worker)] {
      assert!(
        wait_until(TEARDOWN_LIMIT, || !rune_exec::teardown::is_running(pid)),
        "{name} of dev member {member} outlived the interrupt"
      );
    }
  }
}

/// Both of a member's streams are prefixed, and rune's own diagnostics stay on stderr
/// where they cannot be mistaken for a script's product.
#[test]
fn a_members_standard_error_is_prefixed_too() {
  let test = fixture(&format!(
    r#"{{
      loud: {{ command: "{TOOL} chatty OUT 1 1>&2" }},
      quiet: {{ command: "{TOOL} chatty OUT2 1" }},
      both: {{ parallel: ["loud", "quiet"] }}
    }}"#
  ))
  .args(["run", "both"])
  .stdout_regex(r"(?s).*")
  .status(0);

  let output = test.run();
  let mut lines = lines(&output.stdout);
  lines.sort();

  assert_eq!(lines, ["[loud] OUT 1", "[quiet] OUT2 1"]);
}
