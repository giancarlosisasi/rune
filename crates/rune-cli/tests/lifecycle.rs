//! Timeouts, retries, and how a tree is asked to end.
//!
//! Nothing here is asserted by elapsed time. A retry is proved by what the fixtures wrote,
//! a timeout by the exit code and the process ids left behind, and the order of a signal
//! and the kill that follows it by the fixture that reports which signal it received.

mod harness;

use std::time::Duration;

use harness::{Test, process_ids_at, wait_until};

/// How long an operating system is given to finish tearing a process tree down. Generous
/// on purpose: it bounds a failure, it does not sequence anything.
const TEARDOWN_LIMIT: Duration = Duration::from_secs(10);

/// The fake tool the fixtures call. Always `.exe`, on every operating system.
const TOOL: &str = "faketool.exe";

/// What rune exits with when it ended a script for taking too long.
const TIMED_OUT: i32 = 124;

fn fixture(scripts: &str) -> Test {
  Test::new()
    .config(&format!("export default {{ scripts: {scripts} }};\n"))
    .tool(&format!("node_modules/.bin/{TOOL}"))
}

/// The line rune writes when it is the one that decided a script was over.
///
/// One line per attempt rune ended, which is what counts the attempts in the tests below.
/// A child's own output cannot do that job: whether it reaches its first `println` inside a
/// budget measured in milliseconds depends on how busy the machine is.
fn overran(script: &str, millis: u32) -> String {
  format!("`{script}` exceeded its {millis} ms timeout — its process tree was terminated\n")
}

/// Whatever the fixture managed to say before it was ended, and nothing else.
const ONLY_READINESS: &str = r"^(READY\n)*$";

/// Test 5d.1 — a flaky script is retried to success, and nothing downstream ever learns
/// there was a failure.
///
/// The shape is the point. `work` runs the flaky script and then a marker, and `watch`
/// waits for that marker. If the retry loop sat outside the unit a group observes, the
/// serial run would stop at the first attempt's failure, the marker would never be
/// written, and the group would tear `watch` down instead of finishing. The oracle is
/// therefore the whole group's outcome, not the flaky script's own code.
#[test]
fn a_flaky_script_is_retried_and_the_group_sees_one_success() {
  let test = fixture(&format!(
    r#"{{
      flaky: {{ command: "{TOOL} fail-once state.txt", retries: 1 }},
      after: {{ command: "{TOOL} touch proof.marker" }},
      work: {{ serial: ["flaky", "after"] }},
      watch: {{ command: "{TOOL} await proof.marker 0" }},
      both: {{ parallel: ["work", "watch"] }}
    }}"#
  ))
  .args(["run", "both"])
  .stdout_regex(r"(?s).*")
  // Exactly two lines for two steps: a retried script is announced once, because the
  // retry loop sits inside the unit a group observes.
  .stderr(
    "
    → work: flaky
    → work: after
    ",
  )
  .status(0);

  let output = test.run();
  let text = String::from_utf8_lossy(&output.stdout);

  assert!(text.contains("attempt failed"), "the first attempt never ran: {text}");
  assert!(text.contains("attempt succeeded"), "the second attempt never ran: {text}");
  assert!(test.dir().join("proof.marker").exists(), "the step after the retried script never ran");
}

/// Test 5d.2 — every attempt fails, and the last one's code is what rune reports.
///
/// The command writes a word per attempt, so the count is read off the output rather than
/// assumed: three attempts is `retries: 2` plus the first run.
#[test]
fn an_exhausted_retry_reports_the_final_attempts_code() {
  fixture(&format!(r#"{{ doomed: {{ command: "{TOOL} emit tried; exit 3", retries: 2 }} }}"#))
    .args(["run", "doomed"])
    .stdout("triedtriedtried")
    .status(3)
    .run();
}

/// The count is exact in both directions: a script that asks for no retries is run once.
#[test]
fn a_script_without_retries_is_run_once() {
  fixture(&format!(r#"{{ doomed: {{ command: "{TOOL} emit tried; exit 3" }} }}"#))
    .args(["run", "doomed"])
    .stdout("tried")
    .status(3)
    .run();
}

/// Retries are spent on failure and on nothing else. A script that succeeds runs once
/// however many attempts it was granted.
#[test]
fn a_successful_script_is_never_retried() {
  fixture(&format!(r#"{{ fine: {{ command: "{TOOL} emit tried", retries: 2 }} }}"#))
    .args(["run", "fine"])
    .stdout("tried")
    .status(0)
    .run();
}

/// Test 5d.4 — a timeout ends the whole tree, not just the process rune is holding.
///
/// The grandchild is the assertion with teeth. Rune holds a shell, the shell holds the
/// tool, and the tool holds a child of its own; a timeout that reached only the first of
/// those would look identical from the exit code alone.
#[test]
fn a_timeout_ends_the_whole_tree_and_reports_the_timeout_code() {
  let test = fixture(&format!(
    r#"{{ hang: {{ command: "{TOOL} spawn-child pids.json", timeout: 2500 }} }}"#
  ))
  .args(["run", "hang"])
  .stdout_regex(ONLY_READINESS)
  .stderr(&overran("hang", 2500))
  .status(TIMED_OUT);

  test.run();

  let (direct, grandchild) = process_ids_at(&test.dir().join("pids.json"));
  for (name, pid) in [("the child", direct), ("its grandchild", grandchild)] {
    assert!(
      wait_until(TEARDOWN_LIMIT, || !rune_exec::teardown::is_running(pid)),
      "{name} outlived the timeout"
    );
  }
}

/// Test 5d.5 — no false positives. A script that finishes inside its budget is untouched,
/// and its own exit code is what rune reports.
#[test]
fn a_script_that_finishes_inside_its_budget_keeps_its_own_code() {
  fixture(&format!(r#"{{ quick: {{ command: "{TOOL} exit-code 7", timeout: 60000 }} }}"#))
    .args(["run", "quick"])
    .status(7)
    .run();
}

/// Test 5d.6, the half that composes — the timeout applies per attempt.
///
/// The fixture blocks on its first run and exits on every later one, so the second attempt
/// can only succeed if it was given a budget of its own. One timeout line and an exit of
/// zero is the pair that says so: the first attempt was ended, the second was not.
///
/// The budget is generous because the first attempt has to get far enough to record that
/// it ran. That is a real property of the fixture, not a wait the assertion depends on.
#[test]
fn a_timed_out_attempt_is_retried_with_a_fresh_budget() {
  fixture(&format!(
    r#"{{ flaky: {{ command: "{TOOL} hang-once state.txt", timeout: 2500, retries: 1 }} }}"#
  ))
  .args(["run", "flaky"])
  .stdout_regex(ONLY_READINESS)
  .stderr(&overran("flaky", 2500))
  .status(0)
  .run();
}

/// Test 5d.6, the half that ends — the timeout code surfaces from the final attempt only.
///
/// Two timeout lines and one exit code: both attempts were ended, and only the last one
/// decided what rune answered with. The first is reported too, because a killed process
/// with no explanation is worse than a noisy log.
#[test]
fn the_timeout_code_comes_from_the_last_attempt() {
  let both_attempts = overran("stuck", 300).repeat(2);

  fixture(&format!(
    r#"{{ stuck: {{ command: "{TOOL} ready-then-wait", timeout: 300, retries: 1 }} }}"#
  ))
  .args(["run", "stuck"])
  .stdout_regex(ONLY_READINESS)
  .stderr(&both_attempts)
  .status(TIMED_OUT)
  .run();
}

/// A retry nothing announces is how a deterministic failure hides for a month, so the
/// options a script declared are part of what `inspect` explains — said as what they do,
/// and only for the ones the config actually chose.
#[test]
fn inspect_explains_the_lifecycle_a_script_declared() {
  let output = Test::new()
    .config(
      "export default { scripts: { \
       e2e: { command: 'playwright test', timeout: 600000, retries: 2, retryDelay: 'exponential' }, \
       api: { command: 'node server.js', killSignal: 'SIGINT', killTimeout: 2000 }, \
       plain: { command: 'tsc -b' } } };\n",
    )
    .args(["inspect", "e2e"])
    .stdout_regex(r"(?s)resolved through")
    .status(0)
    .run();

  insta::with_settings!({ description => "a script declaring a timeout and retries" }, {
    insta::assert_snapshot!(String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n"));
  });
}

/// A script that declared nothing gets no lifecycle block at all. Printing the defaults
/// would bury the one line that was a decision under four that were not.
#[test]
fn inspect_says_nothing_about_a_script_that_declared_nothing() {
  let output = Test::new()
    .config("export default { scripts: { plain: { command: 'tsc -b' } } };\n")
    .args(["inspect", "plain"])
    .stdout_regex(r"(?s)resolved through")
    .status(0)
    .run();

  let text = String::from_utf8_lossy(&output.stdout);
  assert!(!text.contains("lifecycle"), "a script that chose nothing was given a lifecycle: {text}");
}

/// Test 5d.8 — the configured signal is delivered, and the unconditional kill follows it.
///
/// The fixture traps the signal, reports it, and then refuses to exit, so the run can only
/// finish if something stronger arrived afterwards. Both halves are asserted through what
/// the fixture wrote and whether the run ended at all — never through how long it took. The
/// budget is generous because the handler has to be installed before the signal arrives,
/// which is a property of the fixture rather than a wait the assertion depends on.
///
/// Windows has no signal for a process to trap: a job object is terminated or it is not,
/// so there is no first half to observe there.
#[test]
#[cfg(unix)]
fn the_configured_signal_arrives_before_the_kill_that_follows_it() {
  fixture(&format!(
    r#"{{
      stubborn: {{
        command: "exec {TOOL} trap-term",
        timeout: 1500,
        killSignal: "SIGTERM",
        killTimeout: 200
      }}
    }}"#
  ))
  .args(["run", "stubborn"])
  .stdout_regex(r"^READY\ntrapped SIGTERM\n$")
  .stderr(&overran("stubborn", 1500))
  .status(TIMED_OUT)
  .run();
}
