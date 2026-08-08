//! Signals, interrupts and process trees.
//!
//! Every test here waits on a readiness token before it acts. Nothing in this file is
//! timed, because a test that sleeps to make a race come out right is a test that will
//! come out wrong on a slower machine.

mod harness;

use std::io::BufRead as _;
use std::process::Stdio;
use std::time::Duration;

use harness::{Test, interrupt, interruptible, process_ids_at, read_ready, wait_until};

/// How long an operating system is given to finish tearing a process tree down. Generous
/// on purpose: it bounds a failure, it does not sequence anything.
const TEARDOWN_LIMIT: Duration = Duration::from_secs(10);

/// The fake tool the fixtures call. Always `.exe`, on every operating system.
const TOOL: &str = "faketool.exe";

fn config(scripts: &str) -> String {
  format!("export default {{ scripts: {scripts} }};\n")
}

/// A fixture whose script blocks after announcing itself.
fn blocking(command: &str) -> Test {
  Test::new()
    .config(&config(&format!(r#"{{ hold: {{ command: "{command}" }} }}"#)))
    .tool(&format!("node_modules/.bin/{TOOL}"))
    .args(["run", "hold"])
}

/// Test 3.12 — a signal death is not an exit code, and flattening it to one loses the
/// only evidence that the run was killed rather than failed.
#[test]
#[cfg(unix)]
fn a_child_killed_by_sigterm_makes_rune_exit_143() {
  Test::new()
    .config(&config(r#"{ die: { command: "kill -TERM $$" } }"#))
    .args(["run", "die"])
    .stdout("")
    .status(143)
    .run();
}

/// Test 3.13, first half — the interrupt reaches the child and rune reports what the
/// child did with it.
#[test]
#[cfg(unix)]
fn an_interrupt_reaches_the_child_and_rune_reports_its_death() {
  let status = interrupt_and_wait(&blocking(&format!("exec {TOOL} ready-then-wait")));

  assert_eq!(status.code(), Some(130), "an interrupted child that dies reports 128 + SIGINT");
}

/// Test 3.13, second half — and the divergence that matters. A child that handles the
/// interrupt and exits 0 makes rune exit 0.
///
/// This is also the assertion that rune never exits first: rune cannot report the
/// child's own 0 unless it stayed to collect it.
#[test]
#[cfg(unix)]
fn a_child_that_handles_the_interrupt_and_exits_cleanly_makes_rune_exit_cleanly() {
  let status = interrupt_and_wait(&blocking(&format!("exec {TOOL} ready-then-wait --exit-on-int")));

  assert_eq!(status.code(), Some(0), "rune must report the child's status, not the signal");
}

/// Test 3.16 — the direct child dies, and the surviving grandchild is asserted as the
/// documented behavior rather than left as a surprise. When parallel runs change this,
/// the test says so out loud instead of quietly continuing to pass.
#[test]
#[cfg(unix)]
fn terminating_rune_kills_the_direct_child_and_leaves_a_detached_grandchild() {
  use nix::sys::signal::{Signal, kill};
  use nix::unistd::Pid;

  let test = blocking(&format!("exec {TOOL} spawn-grandchild pids.json"));
  let mut command = test.command(test.dir());
  let mut child = command.stdin(Stdio::null()).stdout(Stdio::piped()).spawn().expect("spawn rune");

  read_ready(child.stdout.as_mut().expect("stdout was piped"));
  let (direct, grandchild) = process_ids(test.dir());

  let rune = i32::try_from(child.id()).expect("a process id fits");
  kill(Pid::from_raw(rune), Signal::SIGTERM).expect("terminate rune");
  child.wait().expect("collect rune");

  assert!(
    wait_until(TEARDOWN_LIMIT, || !rune_exec::teardown::is_running(direct)),
    "the direct child survived rune"
  );
  assert!(
    rune_exec::teardown::is_running(grandchild),
    "a detached grandchild surviving is the documented limitation; if this now fails, \
     the teardown story changed and the design note has to change with it"
  );

  let _ = nix::sys::signal::kill(
    Pid::from_raw(i32::try_from(grandchild).expect("a process id fits")),
    Signal::SIGKILL,
  );
}

/// Test 3.15 — the Windows half of the same story, and the better answer. Job membership
/// is inherited, so a grandchild cannot escape by being spawned normally.
#[test]
#[cfg(windows)]
fn killing_rune_takes_the_whole_windows_process_tree_with_it() {
  let test = blocking(&format!("{TOOL} spawn-grandchild pids.json"));
  let mut command = test.command(test.dir());
  let mut child = command.stdin(Stdio::null()).stdout(Stdio::piped()).spawn().expect("spawn rune");

  read_ready(child.stdout.as_mut().expect("stdout was piped"));
  let (direct, grandchild) = process_ids(test.dir());

  child.kill().expect("terminate rune");
  child.wait().expect("collect rune");

  for (name, pid) in [("the direct child", direct), ("the grandchild", grandchild)] {
    assert!(
      wait_until(TEARDOWN_LIMIT, || !rune_exec::teardown::is_running(pid)),
      "{name} survived the job object closing"
    );
  }
}

/// Test 3.14 — a real console control event.
///
/// The assertion with teeth is not the exit code: delivery to a process group races with
/// the group's own teardown, which is why concurrently's equivalent test accepts a set of
/// codes too. What must hold every time is that rune outlived its child, exited, and left
/// nothing behind.
#[test]
#[cfg(windows)]
fn a_console_control_event_leaves_no_orphan() {
  use std::os::windows::process::CommandExt as _;

  use windows_sys::Win32::System::Console::{CTRL_BREAK_EVENT, GenerateConsoleCtrlEvent};

  /// Delivering an event to one group means creating that group. The flag belongs to the
  /// test, never to rune's own children — on those it would stop console events reaching
  /// them, which is the behavior under test.
  const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;

  /// What Windows reports for a process the console control handler took down.
  const CONTROL_C_EXIT: i32 = 0xC000_013A_u32.cast_signed();

  let test = blocking(&format!("{TOOL} spawn-grandchild pids.json"));
  let mut command = test.command(test.dir());
  let mut child = command
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    .creation_flags(CREATE_NEW_PROCESS_GROUP)
    .spawn()
    .expect("spawn rune");

  read_ready(child.stdout.as_mut().expect("stdout was piped"));
  let (direct, grandchild) = process_ids(test.dir());

  let delivered = unsafe { GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, child.id()) };
  assert!(
    delivered != 0,
    "cannot deliver a console control event: {}",
    std::io::Error::last_os_error()
  );

  let status = child.wait().expect("collect rune");

  assert!(
    matches!(status.code(), Some(CONTROL_C_EXIT | 0 | 1)),
    "unexpected exit status {:?}",
    status.code()
  );
  for (name, pid) in [("the direct child", direct), ("the grandchild", grandchild)] {
    assert!(
      wait_until(TEARDOWN_LIMIT, || !rune_exec::teardown::is_running(pid)),
      "{name} outlived rune"
    );
  }
}

/// Test 5c.8 — interrupting a group ends every member's tree and leaves nothing behind.
///
/// A piped member sits in a process group of its own, which is what lets its tree be torn
/// down as a unit — and also what stops the terminal's own interrupt from reaching it. So
/// this is not the single-script path with more children: rune has to do the reaching.
///
/// The orphan probe is the assertion with teeth. Delivery races with the teardown, so the
/// exit code is only ever asked to be honest about having been interrupted.
#[test]
fn interrupting_a_group_leaves_no_member_and_no_grandchild_behind() {
  let test = group_of_holders();
  let mut command = test.command(test.dir());
  let mut child = interruptible(&mut command).spawn().expect("spawn rune");

  let recorded = [test.dir().join("one.json"), test.dir().join("two.json")];
  assert!(
    wait_until(TEARDOWN_LIMIT, || recorded.iter().all(|path| path.exists())),
    "the members never reported their process ids"
  );
  let trees: Vec<(u32, u32)> = recorded.iter().map(|path| process_ids_at(path)).collect();

  interrupt(&child);
  let status = child.wait().expect("collect rune");

  assert!(status.code() != Some(0), "an interrupted group must not report success");

  for (member, (direct, grandchild)) in trees.iter().enumerate() {
    for (name, pid) in [("the member", *direct), ("its grandchild", *grandchild)] {
      assert!(
        wait_until(TEARDOWN_LIMIT, || !rune_exec::teardown::is_running(pid)),
        "{name} of group member {member} outlived rune"
      );
    }
  }
}

/// Test 5c.9 — after an interrupt, the next step of an ordered run never starts.
///
/// Re-scoped from the original row: rune has no spawn queue, because a parallel group
/// starts every member at once. The invariant is real, and it lives where a "next" exists.
/// The marker is the evidence — a file that was never created fails loudly, unlike "we
/// did not observe a spawn".
#[test]
fn no_later_step_starts_after_an_interrupt() {
  let test = Test::new()
    .config(&config(&format!(
      r#"{{
        hold: {{ command: "{TOOL} ready-then-wait" }},
        never: {{ command: "{TOOL} touch never.marker" }},
        chain: {{ serial: ["hold", "never"] }}
      }}"#
    )))
    .tool(&format!("node_modules/.bin/{TOOL}"))
    .args(["run", "chain"]);

  let mut command = test.command(test.dir());
  let mut child = interruptible(&mut command).stdout(Stdio::piped()).spawn().expect("spawn rune");

  read_ready(child.stdout.as_mut().expect("stdout was piped"));
  interrupt(&child);
  child.wait().expect("collect rune");

  assert!(
    !test.dir().join("never.marker").exists(),
    "the step after the interrupted one ran anyway"
  );
}

/// Test 5c.12 — the reader of rune's own output going away ends the run cleanly.
///
/// `SIGPIPE` is ignored by default in Rust, so this arrives as an ordinary broken-pipe
/// write error rather than as process death. Treating it as a fault to report would put a
/// stack trace on the screen of anyone who typed `| head`.
#[test]
fn a_closed_output_ends_the_run_without_a_panic() {
  let test = fixture_group(&format!(
    r#"{{
      one: {{ command: "{TOOL} chatty ONE 100000" }},
      two: {{ command: "{TOOL} chatty TWO 100000" }},
      both: {{ parallel: ["one", "two"] }}
    }}"#
  ))
  .args(["run", "both"]);

  let mut command = test.command(test.dir());
  let mut child = command
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .spawn()
    .expect("spawn rune");

  // One line read, then the reader goes away — which is all `| head -1` is.
  let mut first = String::new();
  std::io::BufReader::new(child.stdout.as_mut().expect("stdout was piped"))
    .read_line(&mut first)
    .expect("read one line");
  drop(child.stdout.take());

  let output = child.wait_with_output().expect("collect rune");
  let complaints = String::from_utf8_lossy(&output.stderr);

  assert!(!complaints.contains("panicked"), "rune panicked on a closed output: {complaints}");
}

/// Two members, each holding a grandchild, so an interrupt has four trees to reach.
fn group_of_holders() -> Test {
  fixture_group(&format!(
    r#"{{
      one: {{ command: "{TOOL} spawn-child one.json" }},
      two: {{ command: "{TOOL} spawn-child two.json" }},
      both: {{ parallel: ["one", "two"] }}
    }}"#
  ))
  .args(["run", "both"])
}

fn fixture_group(scripts: &str) -> Test {
  Test::new().config(&config(scripts)).tool(&format!("node_modules/.bin/{TOOL}"))
}

/// Sends `SIGINT` to rune's whole process group, the way a terminal does, and returns what
/// rune exited with.
#[cfg(unix)]
fn interrupt_and_wait(test: &Test) -> std::process::ExitStatus {
  use std::os::unix::process::CommandExt as _;

  use nix::sys::signal::{Signal, kill};
  use nix::unistd::Pid;

  let mut command = test.command(test.dir());
  let mut child = command
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    // Its own group, so the interrupt lands on rune and its child and not on the test
    // running them. A terminal does the same thing to whatever it has in the foreground.
    .process_group(0)
    .spawn()
    .expect("spawn rune");

  read_ready(child.stdout.as_mut().expect("stdout was piped"));

  let group = i32::try_from(child.id()).expect("a process id fits");
  kill(Pid::from_raw(-group), Signal::SIGINT).expect("interrupt the group");

  child.wait().expect("collect rune")
}

/// The process ids `spawn-grandchild` recorded.
fn process_ids(directory: &std::path::Path) -> (u32, u32) {
  let recorded = std::fs::read(directory.join("pids.json")).expect("the fixture wrote its pids");
  let report: serde_json::Value = serde_json::from_slice(&recorded).expect("valid JSON");

  let read = |name: &str| {
    u32::try_from(report[name].as_u64().expect("a process id")).expect("a process id fits")
  };

  (read("child"), read("grandchild"))
}
