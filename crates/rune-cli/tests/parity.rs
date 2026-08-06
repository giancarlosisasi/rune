//! Test 3.19 — the npm comparison.
//!
//! This change claims npm parity, and the only honest way to test a claim about another
//! tool is to run that tool. Every string in the table goes through rune and through
//! `npm run` with the same shell pinned on both sides; the bytes on stdout and the exit
//! code have to match.
//!
//! Quoting is where a runner diverges from npm invisibly. A user does not report "rune
//! escapes nested quotes differently" — they report that one script in one repository
//! stopped working, months later.

mod harness;

use std::fmt::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};

use harness::{Test, pinned_shell};

/// Set on both sides, so `$PARITY_VAR` has something to resolve to and `%PARITY_VAR%`
/// has something to visibly not resolve to.
const VARIABLE: (&str, &str) = ("PARITY_VAR", "from-the-parent");

/// The command strings, and the shapes each one is here for.
///
/// Fixed rather than generated: parity is a claim about a known set of shapes, and a
/// generated table turns a failure into a puzzle.
const TABLE: &[(&str, &str)] = &[
  ("spaces", r#"echo "a b c""#),
  ("path_with_spaces", r#"echo "/opt/some tool/bin/thing""#),
  ("nested_quotes", r#"echo "she said \"hi\"""#),
  ("apostrophe_inside_quotes", r#"echo "it's fine""#),
  ("dollar_variable", "echo $PARITY_VAR"),
  ("percent_variable", "echo %PARITY_VAR%"),
  ("chain", "echo one && echo two"),
  ("short_circuit", "false && echo unreachable"),
  ("glob_expands", "echo parity-*.txt"),
  ("glob_quoted", "echo 'a*b'"),
  ("exit_code", "exit 7"),
];

/// What one side of the comparison produced.
type Observed = (Vec<u8>, Option<i32>);

#[test]
fn every_command_string_behaves_the_same_through_rune_and_through_npm() {
  let Some(npm) = oracle() else {
    return;
  };

  let test = fixture();
  let mut report = String::new();

  for (name, command) in TABLE {
    let expected = observe(&mut npm_command(&npm, test.dir(), name));

    let mut rune = test.command(test.dir());
    rune.args(["run", name]);
    let actual = observe(&mut rune);

    // Every entry is compared before anything is reported, so one divergence cannot hide
    // the rest of the table behind it.
    if actual != expected {
      describe(name, command, &expected, &actual, &mut report);
    }
  }

  assert!(report.is_empty(), "\nrune and npm disagree:\n{report}");
}

/// A directory both tools can run: one config for rune, one `package.json` for npm, the
/// same command strings in each, and two files for the glob to find.
fn fixture() -> Test {
  let scripts: Vec<String> = TABLE
    .iter()
    .map(|(name, command)| format!("{name}: {{ command: {} }}", quoted(command)))
    .collect();

  let package = serde_json::json!({
    "name": "parity",
    "version": "1.0.0",
    "private": true,
    "scripts": TABLE
      .iter()
      .map(|(name, command)| ((*name).to_owned(), serde_json::Value::from(*command)))
      .collect::<serde_json::Map<String, serde_json::Value>>(),
  });

  Test::new()
    .config(&format!("export default {{ scripts: {{ {} }} }};\n", scripts.join(", ")))
    .file("package.json", &package.to_string())
    .file("parity-a.txt", "")
    .file("parity-b.txt", "")
    .env(VARIABLE.0, VARIABLE.1)
}

/// The npm side, given exactly what the rune side gets.
fn npm_command(npm: &str, dir: &Path, script: &str) -> Command {
  let mut command = Command::new(npm);
  command
    .current_dir(dir)
    // Without this npm prints its own two-line banner, on stdout, ahead of the script's
    // output — npm's bytes, not the script's, and not something rune should imitate.
    .args(["run", "--silent", script])
    .env("npm_config_script_shell", pinned_shell())
    .env("FORCE_COLOR", "0")
    .env_remove("NO_COLOR")
    .env_remove("CI")
    .env(VARIABLE.0, VARIABLE.1);

  command
}

fn observe(command: &mut Command) -> Observed {
  let output = command
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .output()
    .expect("run one side of the comparison");

  (output.stdout, output.status.code())
}

/// npm is the oracle: without it there is nothing to compare against.
///
/// A missing npm skips the comparison on a developer's machine and fails outright on CI,
/// where it means a broken job rather than a machine without node. Either way it is said
/// out loud. A parity suite that passes quietly after comparing nothing is worse than no
/// parity suite, because it reports confidence it never earned.
#[expect(clippy::print_stderr, reason = "a comparison that did not happen has to say so")]
fn oracle() -> Option<String> {
  // `Command` on Windows searches PATH but only ever appends `.exe`, and npm ships as a
  // batch file.
  let npm = if cfg!(windows) { "npm.cmd" } else { "npm" };

  let usable = Command::new(npm)
    .arg("--version")
    .stdin(Stdio::null())
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .status()
    .is_ok_and(|status| status.success());

  if usable {
    return Some(npm.to_owned());
  }

  assert!(
    std::env::var_os("CI").is_none(),
    "no usable `npm` on PATH, and CI is where the parity claim is actually checked"
  );
  eprintln!("SKIPPED: the npm parity comparison found no usable `npm` on PATH");

  None
}

fn describe(
  name: &str,
  command: &str,
  expected: &Observed,
  actual: &Observed,
  report: &mut String,
) {
  let render = |(stdout, code): &Observed| {
    format!("exit {:?}, stdout {:?}", code, String::from_utf8_lossy(stdout))
  };

  writeln!(
    report,
    "  {name}: {command}\n    npm:  {}\n    rune: {}",
    render(expected),
    render(actual)
  )
  .expect("write to a String");
}

/// A JSON string literal, which is also a valid TypeScript one — so the config and the
/// `package.json` are escaped by the same code and cannot drift apart.
fn quoted(command: &str) -> String {
  serde_json::to_string(command).expect("a string always serializes")
}
