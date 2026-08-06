//! Golden-test harness: runs the real `rune` binary inside a throwaway directory.
//!
//! Status, stdout and stderr are every one of them compared before the test panics. A
//! harness that stops at the first mismatch hides the other two regressions behind it.
#![expect(
  dead_code,
  reason = "escape hatches required by design D5; their first users land in changes 2 and 3"
)]

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use regex::Regex;
use tempfile::TempDir;
use unindent::unindent;

/// A shell that exists on Windows, macOS and Linux alike, so a script's observable
/// behavior does not depend on which machine ran the test.
const PINNED_SHELL: &str = "bash";

/// How one output stream is compared against what the test expects.
enum Expect {
  Literal(String),
  Pattern(Box<Regex>),
}

impl Expect {
  fn matches(&self, actual: &str) -> bool {
    match self {
      Self::Literal(expected) => expected == actual,
      Self::Pattern(pattern) => pattern.is_match(actual),
    }
  }

  fn describe(&self) -> (&str, &str) {
    match self {
      Self::Literal(expected) => ("expected", expected),
      Self::Pattern(pattern) => ("expected to match", pattern.as_str()),
    }
  }
}

/// One invocation of `rune`: the directory it runs in, the arguments, and what the
/// three observable outputs must be.
pub struct Test {
  dir: TempDir,
  args: Vec<String>,
  env: Vec<(String, Option<String>)>,
  stdin: Option<String>,
  status: i32,
  stdout: Expect,
  stderr: Expect,
  pin_shell: bool,
}

impl Default for Test {
  fn default() -> Self {
    Self::new()
  }
}

impl Test {
  pub fn new() -> Self {
    Self {
      dir: tempfile::tempdir().expect("create tempdir"),
      args: Vec::new(),
      env: Vec::new(),
      stdin: None,
      status: 0,
      stdout: Expect::Literal(String::new()),
      stderr: Expect::Literal(String::new()),
      pin_shell: true,
    }
  }

  /// Writes `contents` to `rune.config.ts` in the test directory.
  pub fn config(self, contents: &str) -> Self {
    self.file("rune.config.ts", contents)
  }

  /// Writes an arbitrary fixture file, creating parent directories as needed.
  pub fn file(self, relative: &str, contents: &str) -> Self {
    let path = self.dir.path().join(relative);
    let parent = path.parent().expect("a fixture path always has a parent");
    std::fs::create_dir_all(parent).expect("create fixture parent directories");
    std::fs::write(&path, unindent(contents)).expect("write fixture file");
    self
  }

  pub fn args<I, S>(mut self, args: I) -> Self
  where
    I: IntoIterator<Item = S>,
    S: Into<String>,
  {
    self.args.extend(args.into_iter().map(Into::into));
    self
  }

  /// Sets a variable for the child. Overrides the harness's own pinning.
  pub fn env(mut self, key: &str, value: &str) -> Self {
    self.env.push((key.to_owned(), Some(value.to_owned())));
    self
  }

  /// Removes a variable from the child's environment.
  pub fn env_remove(mut self, key: &str) -> Self {
    self.env.push((key.to_owned(), None));
    self
  }

  pub fn stdin(mut self, contents: &str) -> Self {
    self.stdin = Some(contents.to_owned());
    self
  }

  /// Opts out of the pinned `npm_config_script_shell`, for tests *about* shell selection.
  pub fn shell(mut self, pin: bool) -> Self {
    self.pin_shell = pin;
    self
  }

  pub fn status(mut self, status: i32) -> Self {
    self.status = status;
    self
  }

  pub fn stdout(mut self, expected: &str) -> Self {
    self.stdout = Expect::Literal(unindent(expected));
    self
  }

  pub fn stderr(mut self, expected: &str) -> Self {
    self.stderr = Expect::Literal(unindent(expected));
    self
  }

  pub fn stdout_regex(mut self, pattern: &str) -> Self {
    self.stdout = Expect::Pattern(Box::new(Regex::new(pattern).expect("valid stdout regex")));
    self
  }

  pub fn stderr_regex(mut self, pattern: &str) -> Self {
    self.stderr = Expect::Pattern(Box::new(Regex::new(pattern).expect("valid stderr regex")));
    self
  }

  /// The test directory, for assertions about files the run left behind.
  pub fn dir(&self) -> &Path {
    self.dir.path()
  }

  /// Runs the binary and compares all three outputs. Returns the raw output so a
  /// caller can assert more than the three standard expectations.
  pub fn run(self) -> Output {
    let output = self.spawn(self.dir.path());

    let stdout = String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n");
    let stderr = String::from_utf8_lossy(&output.stderr).replace("\r\n", "\n");
    let status = output.status.code();

    let mut report = String::new();
    // `|`, never `||`: every check must run so one mismatch cannot mask the others.
    let ok = check_status(self.status, status, &mut report)
      | check_stream("stdout", &self.stdout, &stdout, &mut report)
      | check_stream("stderr", &self.stderr, &stderr, &mut report);

    assert!(ok, "\nrune {}\n{report}", self.args.join(" "));

    output
  }

  /// Runs the binary from `cwd` instead of the test root — for the nested-directory cases.
  pub fn run_in(self, relative: &str) -> Output {
    let cwd = self.dir.path().join(relative);
    std::fs::create_dir_all(&cwd).expect("create working directory");
    let output = self.spawn(&cwd);

    let stdout = String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n");
    let stderr = String::from_utf8_lossy(&output.stderr).replace("\r\n", "\n");
    let status = output.status.code();

    let mut report = String::new();
    let ok = check_status(self.status, status, &mut report)
      | check_stream("stdout", &self.stdout, &stdout, &mut report)
      | check_stream("stderr", &self.stderr, &stderr, &mut report);

    assert!(ok, "\nrune {} (in {relative})\n{report}", self.args.join(" "));

    output
  }

  fn spawn(&self, cwd: &Path) -> Output {
    let mut command = Command::new(binary());
    command
      .current_dir(cwd)
      .args(&self.args)
      .stdin(if self.stdin.is_some() { Stdio::piped() } else { Stdio::null() })
      .stdout(Stdio::piped())
      .stderr(Stdio::piped())
      // Color must never depend on whether the runner attached a terminal, and `CI`
      // is an input the config can read — leaving it inherited makes results differ
      // between a laptop and a CI runner.
      .env("FORCE_COLOR", "0")
      .env_remove("NO_COLOR")
      .env_remove("CI");

    if self.pin_shell {
      command.env("npm_config_script_shell", PINNED_SHELL);
    }

    for (key, value) in &self.env {
      match value {
        Some(value) => command.env(key, value),
        None => command.env_remove(key),
      };
    }

    let Some(stdin) = self.stdin.as_ref() else {
      return command.output().expect("run the rune binary");
    };

    let mut child = command.spawn().expect("spawn the rune binary");
    child
      .stdin
      .as_mut()
      .expect("stdin was piped")
      .write_all(stdin.as_bytes())
      .expect("write to child stdin");
    child.wait_with_output().expect("collect rune output")
  }
}

/// Path to the binary cargo just built. Never a `rune` found on `PATH`.
fn binary() -> PathBuf {
  PathBuf::from(env!("CARGO_BIN_EXE_rune"))
}

fn check_status(expected: i32, actual: Option<i32>, report: &mut String) -> bool {
  if actual == Some(expected) {
    return true;
  }

  let actual = actual.map_or_else(|| "killed by a signal".to_owned(), |code| code.to_string());
  writeln!(report, "  status: expected {expected}, got {actual}").expect("write to a String");
  false
}

fn check_stream(name: &str, expected: &Expect, actual: &str, report: &mut String) -> bool {
  if expected.matches(actual) {
    return true;
  }

  let (label, wanted) = expected.describe();
  writeln!(report, "  {name}: {label}\n{}\n  got\n{}", indent(wanted), indent(actual))
    .expect("write to a String");
  false
}

fn indent(text: &str) -> String {
  if text.is_empty() {
    return "    <empty>".to_owned();
  }

  let mut indented = String::new();
  for line in text.lines() {
    writeln!(indented, "    {line}").expect("write to a String");
  }
  indented
}
