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
use std::path::{Path, PathBuf};
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

/// The script the argument comparison appends to, and its command. Named `test` because
/// `rune run test -- --watch` is the invocation this change exists to make work.
const APPEND_TO: (&str, &str) = ("test", "echo");

/// Argument lists worth running through both tools.
///
/// Every entry is a shape where a quoting mistake changes the result rather than the
/// spelling: a value that would split in two, a character a shell would act on, a quote
/// that would close early, and the separator itself.
const ARGUMENTS: &[&[&str]] = &[
  &["--watch"],
  &["--flag", "a b"],
  &[],
  &["--"],
  &["a&b"],
  &[r#"say "hi""#],
  &["it's"],
  &["$PARITY_VAR"],
  &["%PARITY_VAR%"],
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

/// Test 4a.2 — the same claim, about the arguments a user appends.
///
/// A pass-through argument is quoted by rune and by npm independently, so this is the
/// row that would catch rune quoting for the wrong shell. It runs once per shell kind
/// both tools handle the same way, which is what makes the `cmd.exe` rules a tested
/// claim on Windows rather than a table somebody wrote from memory.
#[test]
fn appended_arguments_behave_the_same_through_rune_and_through_npm() {
  let Some(npm) = oracle() else {
    return;
  };

  let test = fixture();
  let mut report = String::new();

  for (kind, shell) in comparable_shells() {
    for passed in ARGUMENTS {
      let mut oracle = npm_command(&npm, test.dir(), APPEND_TO.0);
      oracle.env(pinned_shell_variable(), &shell).arg("--").args(*passed);
      let expected = observe(&mut oracle);

      let mut rune = test.command(test.dir());
      rune.env(pinned_shell_variable(), &shell).args(["run", APPEND_TO.0]).arg("--").args(*passed);
      let actual = observe(&mut rune);

      if actual != expected {
        describe(kind, &format!("{APPEND_TO:?} -- {passed:?}"), &expected, &actual, &mut report);
      }
    }
  }

  assert!(report.is_empty(), "\nrune and npm disagree:\n{report}");
}

/// The shells rune and npm claim to hand arguments to the same way.
///
/// PowerShell is deliberately absent. npm quotes POSIX-style for every shell that is not
/// `cmd.exe`, PowerShell included, which is the bug this change exists not to repeat —
/// comparing there would assert that rune reproduces it. PowerShell quoting is held to
/// its own rules by the unit table in `rune-exec`.
fn comparable_shells() -> Vec<(&'static str, PathBuf)> {
  let mut shells = vec![("posix", pinned_shell())];

  if cfg!(windows) {
    shells.push(("cmd", PathBuf::from("cmd.exe")));
  }

  shells
}

/// npm's own setting, which rune reads too — the one knob that pins both sides to the
/// same shell.
fn pinned_shell_variable() -> &'static str {
  "npm_config_script_shell"
}

/// A directory both tools can run: one config for rune, one `package.json` for npm, the
/// same command strings in each, and two files for the glob to find.
fn fixture() -> Test {
  let scripts: Vec<String> = TABLE
    .iter()
    .chain(std::iter::once(&APPEND_TO))
    .map(|(name, command)| format!("{name}: {{ command: {} }}", quoted(command)))
    .collect();

  let package = serde_json::json!({
    "name": "parity",
    "version": "1.0.0",
    "private": true,
    "scripts": TABLE
      .iter()
      .chain(std::iter::once(&APPEND_TO))
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
