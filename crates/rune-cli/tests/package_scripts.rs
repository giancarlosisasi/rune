//! Test R1.1 — the path a user actually takes to reach `rune run`.
//!
//! Nobody types `rune run test`. They write `"test": "rune run test"` in a `package.json`
//! and type `npm test -- --watch`, and every package manager implements that `--` by
//! appending what follows to the script's command string. The separator is spent on the
//! append and never reaches rune.
//!
//! So this is the one row no test that calls the binary directly can satisfy: the defect
//! lives in the trip, not in either end of it. The real manager runs the real script and
//! the child reports the argv it was handed.

mod harness;

use std::process::{Command, Stdio};

use harness::{Test, package_manager, pinned_shell, with_rune_on_path};

const TOOL: &str = "faketool.exe";

/// The managers this row is claimed against. Both append rather than forward, and both
/// are what a JavaScript monorepo is driven by.
const MANAGERS: [&str; 2] = ["npm", "pnpm"];

/// What a user appends. Long enough to be recognisable in an argv and shaped like the
/// flag a test runner actually takes.
const APPENDED: &str = "--reporter=verbose";

#[test]
fn a_package_manager_script_delivers_its_appended_argument() {
  let fixture = fixture();
  let mut ran = Vec::new();

  for manager in MANAGERS {
    let Some(program) = package_manager(manager) else {
      report_skip(manager);
      continue;
    };

    let mut command = Command::new(&program);
    command
      .current_dir(fixture.dir())
      // Without `--silent` the manager writes its own banner to stdout, ahead of the
      // child's report and in nobody's voice but its own.
      .args(["run", "--silent", "probe", "--", APPENDED])
      .env("npm_config_script_shell", pinned_shell())
      .env("FORCE_COLOR", "0")
      .env_remove("NO_COLOR")
      .env_remove("CI");
    with_rune_on_path(&mut command);

    let output = command
      .stdin(Stdio::null())
      .stdout(Stdio::piped())
      .stderr(Stdio::piped())
      .output()
      .unwrap_or_else(|error| panic!("run `{program} run probe`: {error}"));

    assert!(
      output.status.success(),
      "`{program} run probe -- {APPENDED}` exited {:?}\nstderr:\n{}",
      output.status.code(),
      String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(argv_in(&output.stdout), ["report-env", APPENDED], "through {program}");

    ran.push(manager);
  }

  assert!(
    !ran.is_empty() || std::env::var_os("CI").is_none(),
    "no package manager on PATH, and CI is where this claim is actually checked"
  );
}

/// A repository laid out the way the product documents: one line per script in the
/// manifest, and the definition itself in the config at the root.
fn fixture() -> Test {
  let manifest = serde_json::json!({
    "name": "package-scripts",
    "version": "1.0.0",
    "private": true,
    "scripts": { "probe": "rune run probe" },
  });

  Test::new()
    .config(&format!(
      "export default {{ scripts: {{ probe: {{ command: \"{TOOL} report-env\" }} }} }};\n"
    ))
    .file("package.json", &manifest.to_string())
    .tool(&format!("node_modules/.bin/{TOOL}"))
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

/// A comparison that did not happen has to say so. A row that passes quietly after
/// testing nothing reports confidence it never earned.
#[expect(clippy::print_stderr, reason = "a skipped manager has to announce itself")]
fn report_skip(manager: &str) {
  eprintln!("SKIPPED: no usable `{manager}` on PATH");
}
