//! One fixture binary for the whole test suite.
//!
//! Tests that need a real child process spawn this instead of relying on `echo`, `sleep`
//! or a shell one-liner: those differ between platforms and between shells, and the
//! differences show up as flaky assertions rather than as honest failures.
//!
//! Two conventions matter to callers:
//!
//! - Copies of this binary placed on `PATH` as fake tools are always named `*.exe`, on
//!   every operating system. Windows needs the extension to consider a file executable;
//!   Unix treats it as an ordinary part of the name.
//! - Anything a test must synchronize on is announced with the `READY` token on stdout.
//!   Nothing here waits on a clock, with the single exception documented on [`SETTLE`].

use std::io::{self, BufRead as _, IsTerminal as _, Write as _};
use std::path::PathBuf;
use std::process::{Command, ExitCode, Stdio};
use std::time::Duration;

/// Printed by every subcommand that then blocks, so a test can act instead of wait.
const READY: &str = "READY";

/// How long [`after`] keeps running once the process it waits for has gone.
///
/// A test about the order of two exits needs them far enough apart that nothing can
/// report them the other way round. rune learns that a child has gone from its async
/// runtime rather than from the operating system, and the two answers do not arrive in
/// the same instant: exits a microsecond apart can reach rune in either order. Nothing
/// out here can watch rune's own bookkeeping and wait for it, so the margin is put
/// between the exits instead. It is generous because it is paid twice in the whole
/// suite, and a margin that is sometimes too small is worth nothing at all.
const SETTLE: Duration = Duration::from_millis(250);

fn main() -> ExitCode {
  let arguments: Vec<String> = std::env::args().skip(1).collect();
  let rest: Vec<&str> = arguments.iter().skip(1).map(String::as_str).collect();

  match arguments.first().map(String::as_str) {
    // Being invoked as a shell, not as a subcommand: `<shell> -c "<command>"` is how
    // rune calls whatever `npm_config_script_shell` names. Reporting the command back
    // is what lets a test prove which shell actually ran.
    Some("-c") => report_shell_invocation(&rest),
    Some("report-env") => report_env(&arguments),
    Some("ready-then-wait") => ready_then_wait(rest.contains(&"--exit-on-int")),
    Some("exit-code") => exit_code(rest.first().copied()),
    Some("trap-term") => trap_term(rest.first().copied()),
    Some("spawn-grandchild") => spawn_grandchild(rest.first().copied(), true),
    Some("spawn-child") => spawn_grandchild(rest.first().copied(), false),
    Some("isatty") => report_isatty(),
    Some("emit") => emit(&rest),
    Some("chatty") => chatty(&rest, false),
    Some("chatty-hold") => chatty(&rest, true),
    Some("mark") => mark(&rest),
    Some("await") => wait_for(&rest),
    Some("after") => after(&rest),
    Some("colorize") => colorize(&rest),
    Some("fail-once") => fail_once(rest.first().copied()),
    Some("hang-once") => hang_once(rest.first().copied()),
    Some("touch") => touch(rest.first().copied()),
    other => usage(other),
  }
}

fn report_shell_invocation(command: &[&str]) -> ExitCode {
  print_line(&format!("FAKE SHELL: {}", command.join(" ")))
}

/// Environment, working directory and argv as JSON — the cross-platform way to assert
/// what a child actually received.
fn report_env(arguments: &[String]) -> ExitCode {
  let environment: serde_json::Map<String, serde_json::Value> =
    std::env::vars().map(|(name, value)| (name, serde_json::Value::String(value))).collect();

  let Ok(cwd) = std::env::current_dir() else {
    return fail("cannot read the working directory");
  };

  let report = serde_json::json!({
    "argv": arguments,
    "cwd": cwd.to_string_lossy(),
    "env": environment,
  });

  print_line(&report.to_string())
}

/// Announces readiness, then holds the process open.
///
/// A line on stdin ends it, which is how the terminal-reading tests prove the child was
/// never stopped for reading a terminal it does not own. With stdin closed there is
/// nothing to read, so it blocks until a signal arrives.
fn ready_then_wait(exit_on_int: bool) -> ExitCode {
  if exit_on_int {
    install_interrupt_exit();
  }

  if print_line(READY) == ExitCode::FAILURE {
    return ExitCode::FAILURE;
  }

  let mut line = String::new();
  if io::stdin().lock().read_line(&mut line).unwrap_or(0) > 0 {
    return print_line(&format!("GOT {}", line.trim_end()));
  }

  block_forever()
}

fn exit_code(code: Option<&str>) -> ExitCode {
  match code.and_then(|code| code.parse::<u8>().ok()) {
    Some(code) => ExitCode::from(code),
    None => fail("exit-code needs a number from 0 to 255"),
  }
}

/// Catches `SIGTERM`, says so, and keeps running — the fixture for escalation paths that
/// must prove a second, stronger signal was needed.
///
/// The optional marker is written once the handler is installed, so a sibling can wait
/// for that fact rather than for a line on a stream something else is already reading.
fn trap_term(marker: Option<&str>) -> ExitCode {
  install_term_report();

  if let Some(marker) = marker
    && let Err(error) = std::fs::write(PathBuf::from(marker), "armed\n")
  {
    return fail(&format!("cannot write {marker}: {error}"));
  }

  if print_line(READY) == ExitCode::FAILURE {
    return ExitCode::FAILURE;
  }

  block_forever()
}

/// Spawns a child of its own and records both process ids, so a test can ask what
/// survived a teardown.
///
/// `detached` decides which of the two stories the fixture tells. Detached, it steps out
/// of the process group it was born into, which is the one way a POSIX grandchild escapes
/// a teardown — the documented limitation. Attached, it is the ordinary case, and nothing
/// short of a bug lets it survive.
fn spawn_grandchild(pid_file: Option<&str>, detached: bool) -> ExitCode {
  let Some(pid_file) = pid_file else {
    return fail("spawning a grandchild needs a path to write the process ids to");
  };

  let Ok(own_binary) = std::env::current_exe() else {
    return fail("cannot find my own binary");
  };

  let mut command = Command::new(own_binary);
  command.arg("ready-then-wait").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
  if detached {
    detach(&mut command);
  }

  let child = match command.spawn() {
    Ok(child) => child,
    Err(error) => return fail(&format!("cannot spawn a grandchild: {error}")),
  };

  let report = serde_json::json!({ "child": std::process::id(), "grandchild": child.id() });
  if let Err(error) = std::fs::write(PathBuf::from(pid_file), report.to_string()) {
    return fail(&format!("cannot write {pid_file}: {error}"));
  }

  if print_line(READY) == ExitCode::FAILURE {
    return ExitCode::FAILURE;
  }

  block_forever()
}

fn report_isatty() -> ExitCode {
  let report = serde_json::json!({
    "stdin": io::stdin().is_terminal(),
    "stdout": io::stdout().is_terminal(),
    "stderr": io::stderr().is_terminal(),
  });

  print_line(&report.to_string())
}

/// Writes each argument to stdout exactly as given, flushing between them, so a test
/// controls where the chunk boundaries fall.
fn emit(chunks: &[&str]) -> ExitCode {
  let mut stdout = io::stdout().lock();
  for chunk in chunks {
    if write!(stdout, "{chunk}").and_then(|()| stdout.flush()).is_err() {
      return ExitCode::FAILURE;
    }
  }
  ExitCode::SUCCESS
}

/// Writes `<label> <n>` for n from 1 to count, as fast as the pipe will take it.
///
/// The label goes on every line so a test can tell which process wrote it without relying
/// on rune's own prefix — which is the thing under test and cannot also be the evidence.
/// With `hold`, a marker file is created after the last line and the process then blocks,
/// so a sibling can act while this one still has output that must not be lost.
fn chatty(arguments: &[&str], hold: bool) -> ExitCode {
  let (Some(label), Some(count)) = (arguments.first(), arguments.get(1)) else {
    return fail("chatty needs a label and a number of lines");
  };
  let Ok(count) = count.parse::<u32>() else {
    return fail("chatty needs a number of lines");
  };

  let mut stdout = io::stdout().lock();
  for line in 1..=count {
    if writeln!(stdout, "{label} {line}").is_err() {
      return ExitCode::FAILURE;
    }
  }
  if stdout.flush().is_err() {
    return ExitCode::FAILURE;
  }

  if !hold {
    return ExitCode::SUCCESS;
  }

  let Some(marker) = arguments.get(2) else {
    return fail("chatty-hold needs a marker path");
  };
  if let Err(error) = std::fs::write(PathBuf::from(marker), "done\n") {
    return fail(&format!("cannot write {marker}: {error}"));
  }

  block_forever()
}

/// Creates a marker naming this process, then exits with a chosen code.
///
/// The process id is the point. Writing the marker happens *before* exiting, so a sibling
/// that only waited for the file could still exit first — which is exactly the order a
/// test about exit order must not leave to chance. Recording the id lets [`after`] wait
/// for the exit itself.
fn mark(arguments: &[&str]) -> ExitCode {
  let (Some(path), Some(code)) = (arguments.first(), arguments.get(1)) else {
    return fail("mark needs a path and an exit code");
  };

  let report = serde_json::json!({ "pid": std::process::id() });
  if let Err(error) = std::fs::write(PathBuf::from(path), report.to_string()) {
    return fail(&format!("cannot write {path}: {error}"));
  }

  exit_code(Some(code))
}

/// Blocks until a marker exists, then exits with a chosen code.
///
/// For the markers whose writer goes on running: a fixture that announces itself and then
/// holds the process open.
fn wait_for(arguments: &[&str]) -> ExitCode {
  let (Some(path), Some(code)) = (arguments.first(), arguments.get(1)) else {
    return fail("await needs a path and an exit code");
  };

  let path = PathBuf::from(path);
  while !path.exists() {
    std::thread::yield_now();
  }

  exit_code(Some(code))
}

/// Blocks until the process that wrote a marker has gone, waits out [`SETTLE`], then exits
/// with a chosen code.
///
/// This is what makes "this member exits before that one" a fact rather than a race.
fn after(arguments: &[&str]) -> ExitCode {
  let (Some(path), Some(code)) = (arguments.first(), arguments.get(1)) else {
    return fail("after needs a marker path and an exit code");
  };

  let path = PathBuf::from(path);
  loop {
    if let Ok(recorded) = std::fs::read(&path)
      && let Ok(report) = serde_json::from_slice::<serde_json::Value>(&recorded)
      && let Some(pid) = report["pid"].as_u64().and_then(|pid| u32::try_from(pid).ok())
      && !rune_exec::teardown::is_running(pid)
    {
      std::thread::sleep(SETTLE);
      return exit_code(Some(code));
    }

    std::thread::yield_now();
  }
}

/// Emits color only when told its output supports it, exactly as a real tool does.
///
/// A tool that always colors would prove nothing: what has to hold is that a piped child
/// still learns rune's own output supports color, and acts on it.
fn colorize(arguments: &[&str]) -> ExitCode {
  let Some(text) = arguments.first() else {
    return fail("colorize needs some text");
  };

  let wanted = std::env::var("FORCE_COLOR").is_ok_and(|level| level != "0");
  if wanted { print_line(&format!("\u{1b}[31m{text}\u{1b}[0m")) } else { print_line(text) }
}

/// Fails the first time and succeeds afterwards, with the attempt recorded on disk so
/// the state survives the process. The fixture for retry behavior.
fn fail_once(state_file: Option<&str>) -> ExitCode {
  let Some(state_file) = state_file else {
    return fail("fail-once needs a path to keep its state in");
  };

  let path = PathBuf::from(state_file);
  if path.exists() {
    return print_line("attempt succeeded");
  }

  if let Err(error) = std::fs::write(&path, "attempted\n") {
    return fail(&format!("cannot write {state_file}: {error}"));
  }

  let _ = print_line("attempt failed");
  ExitCode::FAILURE
}

/// Blocks forever the first time and succeeds afterwards, with the attempt recorded on
/// disk so the state survives the process. The fixture for a timeout that is retried.
fn hang_once(state_file: Option<&str>) -> ExitCode {
  let Some(state_file) = state_file else {
    return fail("hang-once needs a path to keep its state in");
  };

  let path = PathBuf::from(state_file);
  if path.exists() {
    return print_line(READY);
  }

  if let Err(error) = std::fs::write(&path, "attempted\n") {
    return fail(&format!("cannot write {state_file}: {error}"));
  }

  if print_line(READY) == ExitCode::FAILURE {
    return ExitCode::FAILURE;
  }

  block_forever()
}

/// Leaves a file behind and exits.
///
/// The evidence that a process ran, readable after it is gone. A command that must never
/// execute is tested by pointing it at this and asserting the file was never created —
/// an assertion that fails loudly, unlike "we did not observe a spawn".
fn touch(marker: Option<&str>) -> ExitCode {
  let Some(marker) = marker else {
    return fail("touch needs a path to create");
  };

  match std::fs::write(PathBuf::from(marker), "ran\n") {
    Ok(()) => ExitCode::SUCCESS,
    Err(error) => fail(&format!("cannot write {marker}: {error}")),
  }
}

fn usage(unknown: Option<&str>) -> ExitCode {
  let named = unknown.unwrap_or("<nothing>");
  fail(&format!(
    "unknown subcommand `{named}`\n\nknown: report-env, ready-then-wait, exit-code, \
     trap-term, spawn-grandchild, spawn-child, isatty, emit, chatty, chatty-hold, mark, \
     await, colorize, fail-once, hang-once, touch"
  ))
}

fn print_line(text: &str) -> ExitCode {
  let mut stdout = io::stdout().lock();
  match writeln!(stdout, "{text}").and_then(|()| stdout.flush()) {
    Ok(()) => ExitCode::SUCCESS,
    Err(_) => ExitCode::FAILURE,
  }
}

fn fail(message: &str) -> ExitCode {
  let _ = writeln!(io::stderr().lock(), "rune-testkit: {message}");
  ExitCode::FAILURE
}

fn block_forever() -> ! {
  loop {
    std::thread::park();
  }
}

#[cfg(unix)]
fn detach(command: &mut Command) {
  use std::os::unix::process::CommandExt as _;

  // Its own process group, so a teardown that signals rune's group cannot reach it.
  // This is the process the POSIX limitation in the design is about.
  command.process_group(0);
}

#[cfg(windows)]
fn detach(_command: &mut Command) {
  // Nothing to do: job object membership is inherited, which is the whole point of the
  // Windows teardown story — a grandchild cannot escape by being spawned normally.
}

#[cfg(unix)]
fn install_term_report() {
  use nix::libc;

  extern "C" fn on_term(_signal: libc::c_int) {
    const MESSAGE: &[u8] = b"trapped SIGTERM\n";
    // One raw write and nothing else: everything else a handler could do here is
    // forbidden while a signal is being delivered.
    let written = unsafe { libc::write(1, MESSAGE.as_ptr().cast(), MESSAGE.len()) };
    let _ = written;
  }

  install(nix::sys::signal::Signal::SIGTERM, on_term);
}

#[cfg(unix)]
fn install_interrupt_exit() {
  use nix::libc;

  extern "C" fn on_interrupt(_signal: libc::c_int) {
    // Exiting 0 on an interrupt is the case that separates rune from runners which
    // latch the signal and report `128 + n` whatever the child decided.
    unsafe { libc::_exit(0) }
  }

  install(nix::sys::signal::Signal::SIGINT, on_interrupt);
}

#[cfg(unix)]
fn install(signal: nix::sys::signal::Signal, handler: extern "C" fn(nix::libc::c_int)) {
  use nix::sys::signal::{SaFlags, SigAction, SigHandler, SigSet, sigaction};

  let action = SigAction::new(SigHandler::Handler(handler), SaFlags::SA_RESTART, SigSet::empty());
  unsafe { sigaction(signal, &action) }.expect("install a signal handler");
}

#[cfg(windows)]
fn install_term_report() {
  // Windows has no SIGTERM. The fixture still needs to block, which the caller does.
}

#[cfg(windows)]
fn install_interrupt_exit() {
  // Console control events are delivered to a handler rune installs, not to this
  // fixture; the Windows interrupt row asserts rune's behavior instead.
}
