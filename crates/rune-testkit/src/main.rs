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
//!   No test in this project waits by sleeping.

use std::io::{self, BufRead as _, IsTerminal as _, Write as _};
use std::path::PathBuf;
use std::process::{Command, ExitCode, Stdio};

/// Printed by every subcommand that then blocks, so a test can act instead of wait.
const READY: &str = "READY";

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
    Some("trap-term") => trap_term(),
    Some("spawn-grandchild") => spawn_grandchild(rest.first().copied()),
    Some("isatty") => report_isatty(),
    Some("emit") => emit(&rest),
    Some("fail-once") => fail_once(rest.first().copied()),
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
fn trap_term() -> ExitCode {
  install_term_report();

  if print_line(READY) == ExitCode::FAILURE {
    return ExitCode::FAILURE;
  }

  block_forever()
}

/// Spawns a child of its own and records both process ids, so a test can ask what
/// survived a teardown.
fn spawn_grandchild(pid_file: Option<&str>) -> ExitCode {
  let Some(pid_file) = pid_file else {
    return fail("spawn-grandchild needs a path to write the process ids to");
  };

  let Ok(own_binary) = std::env::current_exe() else {
    return fail("cannot find my own binary");
  };

  let mut command = Command::new(own_binary);
  command.arg("ready-then-wait").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
  detach(&mut command);

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

fn usage(unknown: Option<&str>) -> ExitCode {
  let named = unknown.unwrap_or("<nothing>");
  fail(&format!(
    "unknown subcommand `{named}`\n\nknown: report-env, ready-then-wait, exit-code, \
     trap-term, spawn-grandchild, isatty, emit, fail-once"
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
