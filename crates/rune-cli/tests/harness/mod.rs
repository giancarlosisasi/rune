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

/// The entry that stops rune's upward config walk.
///
/// Every fixture gets one. Without it the walk leaves the temporary directory and finds
/// whatever config happens to sit above it on the machine running the test, and the
/// result stops being the same for everyone.
const BOUNDARY: (&str, &str) = (".git/HEAD", "ref: refs/heads/main\n");

/// Fake tools and fixture binaries always carry this suffix, on every operating system.
/// Windows needs it to consider a file executable; Unix treats it as part of the name.
pub const FIXTURE_SUFFIX: &str = ".exe";

/// The readiness token `rune-testkit` prints before it blocks.
pub const READY: &str = "READY";

/// A shell that exists on Windows, macOS and Linux alike, so a script's observable
/// behavior does not depend on which machine ran the test.
///
/// On Windows the plain name is not enough: the `bash.exe` shipped in `System32` is the
/// Windows Subsystem for Linux launcher, which fails outright when no distribution is
/// installed. Git for Windows is the bash that Windows developers and the CI runners both
/// actually have.
pub fn pinned_shell() -> PathBuf {
  #[cfg(windows)]
  {
    let git_bash = PathBuf::from(r"C:\Program Files\Git\bin\bash.exe");
    if git_bash.is_file() {
      return git_bash;
    }
  }

  PathBuf::from("bash")
}

/// Path to the binary cargo just built. Never a `rune` found on `PATH`.
pub fn binary() -> PathBuf {
  PathBuf::from(env!("CARGO_BIN_EXE_rune"))
}

/// The fixture binary the tests spawn as a child.
///
/// Cargo only exports `CARGO_BIN_EXE_*` for binaries of the crate under test, so it is
/// located next to `rune` instead. It is a workspace member, which is what puts it there.
pub fn testkit() -> PathBuf {
  let path = binary().with_file_name(format!("rune-testkit{}", std::env::consts::EXE_SUFFIX));
  assert!(path.is_file(), "{} is missing — build the whole workspace", path.display());
  path
}

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
    let test = Self {
      dir: tempfile::tempdir().expect("create tempdir"),
      args: Vec::new(),
      env: Vec::new(),
      stdin: None,
      status: 0,
      stdout: Expect::Literal(String::new()),
      stderr: Expect::Literal(String::new()),
      pin_shell: true,
    };

    test.file(BOUNDARY.0, BOUNDARY.1)
  }

  /// Writes `contents` to `rune.config.ts` in the test directory.
  pub fn config(self, contents: &str) -> Self {
    self.file("rune.config.ts", contents)
  }

  /// Puts a copy of the fixture binary at `relative`, as a fake tool named after whatever
  /// the fixture's command calls.
  pub fn tool(self, relative: &str) -> Self {
    let path = self.dir.path().join(relative);
    let parent = path.parent().expect("a fixture path always has a parent");
    std::fs::create_dir_all(parent).expect("create fixture parent directories");
    std::fs::copy(testkit(), &path).expect("copy the fixture binary");
    make_executable(&path);
    self
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
  pub fn run(&self) -> Output {
    let output = self.spawn(self.dir.path());
    self.compare(&output, "");

    output
  }

  /// Runs the binary from `cwd` instead of the test root — for the nested-directory cases.
  ///
  /// Borrowing rather than consuming keeps the fixture alive, so a test can run the same
  /// tree twice and then look at what the runs left behind.
  pub fn run_in(&self, relative: &str) -> Output {
    let cwd = self.dir.path().join(relative);
    std::fs::create_dir_all(&cwd).expect("create working directory");
    let output = self.spawn(&cwd);
    self.compare(&output, &format!(" (in {relative})"));

    output
  }

  fn compare(&self, output: &Output, where_from: &str) {
    let stdout = String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n");
    let stderr = String::from_utf8_lossy(&output.stderr).replace("\r\n", "\n");

    let mut report = String::new();
    // `&`, never `&&`: all three must hold, and all three must be evaluated, so one
    // mismatch cannot hide the others behind it.
    let ok = check_status(self.status, output.status.code(), &mut report)
      & check_stream("stdout", &self.stdout, &stdout, &mut report)
      & check_stream("stderr", &self.stderr, &stderr, &mut report);

    assert!(ok, "\nrune {}{where_from}\n{report}", self.args.join(" "));
  }

  /// Runs the binary again in the same fixture, with other arguments, and hands back
  /// the raw output.
  ///
  /// The three expectations describe the run under test, so this one is judged by the
  /// caller. It exists for the case where one command's product is another command's
  /// input: `init` writes a config, and only the loader can say whether it wrote one.
  pub fn then_run(&self, args: &[&str]) -> Output {
    let owned: Vec<String> = args.iter().map(|argument| (*argument).to_owned()).collect();

    self
      .command_with(self.dir.path(), &owned)
      .stdin(Stdio::null())
      .stdout(Stdio::piped())
      .stderr(Stdio::piped())
      .output()
      .expect("run the rune binary")
  }

  /// The invocation this test describes, without the piping `run` adds.
  ///
  /// For callers that need the child itself rather than its collected output: a PTY, a
  /// signal, a console control event.
  pub fn command(&self, cwd: &Path) -> Command {
    self.command_with(cwd, &self.args)
  }

  fn command_with(&self, cwd: &Path, args: &[String]) -> Command {
    let mut command = Command::new(binary());
    command
      .current_dir(cwd)
      .args(args)
      // Color must never depend on whether the runner attached a terminal, and `CI` is
      // an input the config can read — leaving it inherited makes results differ between
      // a laptop and a CI runner.
      .env("FORCE_COLOR", "0")
      .env_remove("NO_COLOR")
      .env_remove("CI");

    if self.pin_shell {
      command.env("npm_config_script_shell", pinned_shell());
    }

    for (key, value) in &self.env {
      match value {
        Some(value) => command.env(key, value),
        None => command.env_remove(key),
      };
    }

    command
  }

  fn spawn(&self, cwd: &Path) -> Output {
    let mut command = self.command(cwd);
    command
      .stdin(if self.stdin.is_some() { Stdio::piped() } else { Stdio::null() })
      .stdout(Stdio::piped())
      .stderr(Stdio::piped());

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

/// The fixture monorepo: a root config, a package that narrows one of its scripts, and a
/// package that does not.
///
/// Built in a temporary directory with no package manager anywhere near it — the layout
/// is three files and two manifests, and installing a node_modules tree to get them would
/// make every test that uses it depend on a network. `packages/legacy` is the package
/// that needs one extra flag, which is the story this whole feature exists for.
///
/// `root_scripts` is spliced into the root config so a test can decide what the shared
/// definitions actually run; `legacy_scripts` does the same for the overriding package.
pub fn monorepo(root_scripts: &str, legacy_scripts: &str) -> Test {
  Test::new()
    .config(&format!("export default {{ scripts: {root_scripts} }};\n"))
    .file("package.json", "{ \"name\": \"fixture-root\", \"private\": true }\n")
    .file("packages/legacy/package.json", "{ \"name\": \"legacy\" }\n")
    .file(
      "packages/legacy/rune.config.ts",
      &format!("export default {{ scripts: {legacy_scripts} }};\n"),
    )
    .file("packages/modern/package.json", "{ \"name\": \"modern\" }\n")
}

/// Blocks until `settled` reports true, or gives up after `limit`.
///
/// Only for an operating system event that offers nothing to wait on — a process tree
/// finishing its own teardown, for instance. Never for synchronizing with a fixture:
/// those announce themselves with [`READY`].
pub fn wait_until(limit: std::time::Duration, settled: impl Fn() -> bool) -> bool {
  let deadline = std::time::Instant::now() + limit;
  while std::time::Instant::now() < deadline {
    if settled() {
      return true;
    }
    std::thread::yield_now();
  }

  settled()
}

/// Reads one line from a child's piped stdout — the readiness handshake every test that
/// signals a running script begins with.
pub fn read_ready(stdout: &mut std::process::ChildStdout) -> String {
  use std::io::BufRead as _;

  let mut line = String::new();
  std::io::BufReader::new(stdout).read_line(&mut line).expect("read the readiness token");
  assert!(line.starts_with(READY), "expected a readiness token, got {line:?}");
  line
}

#[cfg(unix)]
fn make_executable(path: &Path) {
  use std::os::unix::fs::PermissionsExt as _;

  std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
    .expect("mark the fixture executable");
}

#[cfg(windows)]
fn make_executable(_path: &Path) {
  // Windows decides from the file extension, which is why every fixture ends in `.exe`.
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
