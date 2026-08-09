//! Arguments that have to survive being read twice.
//!
//! On Windows nearly every tool in `node_modules/.bin` is a batch file. `cmd.exe` reads
//! the command line, the batch file re-reads its own arguments through `%*`, and an
//! argument escaped for one reader arrives corrupted at the other. Of the three outcomes
//! that produces — a character silently changed, an argument silently dropped, and the
//! tail of an argument executed as a command — the last is what makes this more than a
//! formatting defect, and none of the three look broken on screen.
//!
//! The oracle here is always the child. Asserting on what rune emitted would pass while
//! the tool received something else, which is how this shipped in the first place.

#![cfg(windows)]

mod harness;

use std::process::{Command, Stdio};

use harness::{Test, package_manager, with_rune_on_path};

/// npm's own setting, which pins both sides to the shell this defect lives under.
const SHELL_VARIABLE: &str = "npm_config_script_shell";

const TOOL: &str = "faketool.exe";
/// The batch file, named the way a package manager names one.
const SHIM: &str = "faketool-shim";

/// The four characters `cmd.exe` acts on that a filename is allowed to hold.
///
/// One row rather than four, because they share one repair and a fix that rescues three of
/// them is the worst outcome available: it looks finished.
const CORRUPTED: [&str; 4] = ["zz-a&cd.ts", "zz-a^b.ts", "zz-a|b.ts", "zz-a<b>c.ts"];

fn config(scripts: &str) -> String {
  format!("export default {{ scripts: {scripts} }};\n")
}

/// A repository whose `probe` script runs `command`, with both a batch shim and the real
/// executable it wraps available on the child's own `PATH`.
fn repository(command: &str) -> Test {
  Test::new()
    .config(&config(&format!(r#"{{ probe: {{ command: "{command}" }} }}"#)))
    .tool(&format!("node_modules/.bin/{TOOL}"))
    .shim(&format!("node_modules/.bin/{SHIM}.cmd"), TOOL)
    // The harness pins a POSIX shell everywhere else for determinism. This defect lives
    // under `cmd.exe`, so these rows pin that one instead of opting out into whatever the
    // machine happens to have set.
    .shell(false)
    .env(SHELL_VARIABLE, "cmd.exe")
    .stdout_regex(r"(?s)^\{.*\}\n$")
    .status(0)
}

/// Test R3.1 — from the command line.
#[test]
fn every_character_reaches_a_batch_file_child_intact() {
  let test = repository(SHIM).args(["run", "probe"]).args(CORRUPTED);

  assert_eq!(argv_in(&test.run().stdout), expected());
}

/// Test R3.2 — from the config, where a config author meets it with no user input at all.
#[test]
fn every_character_reaches_a_batch_file_child_through_append_args() {
  let test = appending().args(["run", "probe"]);

  assert_eq!(argv_in(&test.run().stdout), expected());
}

/// Test R3.3 — the outcome the whole change exists for. Everything after the `&` was run
/// as a command of its own; the marker is what that command would leave behind.
#[test]
fn the_tail_of_an_argument_is_never_executed() {
  let test = repository(SHIM).args(["run", "probe", "a&mkdir executed-tail"]);
  let output = test.run();

  assert_eq!(argv_in(&output.stdout), ["report-env", "a&mkdir executed-tail"]);
  assert!(!test.dir().join("executed-tail").exists(), "the tail of the argument ran");
}

/// Test R3.4 — a repair that stops the tail executing and still loses the next argument
/// passes R3.3 and helps nobody. The reported case lost the second filename entirely.
#[test]
fn an_argument_after_a_corrupting_one_still_arrives() {
  let test = repository(SHIM).args(["run", "probe", "zz-a&cd.ts", "second.ts"]);

  assert_eq!(argv_in(&test.run().stdout), ["report-env", "zz-a&cd.ts", "second.ts"]);
}

/// Test R3.5 — the case that already worked. A real executable is one reader, so a second
/// escaping round applied to it would break exactly what this change protects.
#[test]
fn a_real_executable_child_is_untouched() {
  let test = repository(&format!("{TOOL} report-env")).args(["run", "probe"]).args(CORRUPTED);

  assert_eq!(argv_in(&test.run().stdout), expected());
}

/// Test R3.8 — npm performs this second round too, so the claim is measured against it
/// rather than asserted. Both sides call the same batch shim with the same arguments.
#[test]
fn a_batch_file_child_is_handed_the_same_arguments_by_rune_and_by_npm() {
  let Some(npm) = package_manager("npm") else {
    report_skip();
    return;
  };

  let manifest = serde_json::json!({
    "name": "batch-parity",
    "version": "1.0.0",
    "private": true,
    "scripts": { "probe": SHIM },
  });
  let test = repository(SHIM).file("package.json", &manifest.to_string());

  let mut oracle = Command::new(&npm);
  oracle
    .current_dir(test.dir())
    .args(["run", "--silent", "probe", "--"])
    .args(CORRUPTED)
    .env(SHELL_VARIABLE, "cmd.exe")
    .env("FORCE_COLOR", "0")
    .env_remove("NO_COLOR")
    .env_remove("CI");

  let mut rune = test.command(test.dir());
  rune.args(["run", "probe"]).args(CORRUPTED);
  with_rune_on_path(&mut rune);

  assert_eq!(argv_in(&observe(&mut rune)), argv_in(&observe(&mut oracle)));
}

/// The line `inspect` explains has to be the line `run` hands over, or the explanation
/// describes an invocation that never happens.
#[test]
fn inspect_shows_the_line_the_shell_will_receive() {
  let output = appending().args(["inspect", "probe"]).stdout_regex(r"(?s).").status(0).run();
  let explained = String::from_utf8_lossy(&output.stdout);

  assert!(explained.contains("^^^&"), "inspect must show what the run escapes:\n{explained}");
}

/// The same repository, with the four filenames declared by the config instead of typed.
fn appending() -> Test {
  let appended: Vec<String> =
    CORRUPTED.iter().map(|argument| serde_json::to_string(argument).expect("json")).collect();
  let scripts = format!(
    r#"{{
      base: {{ command: "{SHIM}" }},
      probe: {{ extends: "base", appendArgs: [{}] }},
    }}"#,
    appended.join(", ")
  );

  Test::new()
    .config(&config(&scripts))
    .tool(&format!("node_modules/.bin/{TOOL}"))
    .shim(&format!("node_modules/.bin/{SHIM}.cmd"), TOOL)
    // The harness pins a POSIX shell everywhere else for determinism. This defect lives
    // under `cmd.exe`, so these rows pin that one instead of opting out into whatever the
    // machine happens to have set.
    .shell(false)
    .env(SHELL_VARIABLE, "cmd.exe")
    .stdout_regex(r"(?s)^\{.*\}\n$")
    .status(0)
}

/// What the child must report back: its own name for the subcommand, then the four
/// filenames exactly as they were written.
fn expected() -> Vec<String> {
  std::iter::once("report-env".to_owned())
    .chain(CORRUPTED.iter().map(|argument| (*argument).to_owned()))
    .collect()
}

fn observe(command: &mut Command) -> Vec<u8> {
  let output = command
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .output()
    .expect("run one side of the comparison");

  assert!(
    output.status.success(),
    "exited {:?}\nstderr:\n{}",
    output.status.code(),
    String::from_utf8_lossy(&output.stderr)
  );

  output.stdout
}

fn argv_in(stdout: &[u8]) -> Vec<String> {
  let report: serde_json::Value = serde_json::from_slice(stdout).unwrap_or_else(|_| {
    panic!("expected the child's report, got {:?}", String::from_utf8_lossy(stdout))
  });

  report["argv"]
    .as_array()
    .expect("the report carries an argv")
    .iter()
    .map(|argument| argument.as_str().expect("an argument is a string").to_owned())
    .collect()
}

#[expect(clippy::print_stderr, reason = "a comparison that did not happen has to say so")]
fn report_skip() {
  eprintln!("SKIPPED: no usable `npm` on PATH for the batch-file parity comparison");
}
