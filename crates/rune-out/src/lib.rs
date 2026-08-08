//! Where rune's own output goes.
//!
//! Two rules this crate exists to enforce. Stdout belongs to the child process, so
//! everything rune says about itself goes to stderr; only a command whose *product* is
//! text — `rune list` — writes to stdout. And output goes through here rather than
//! through `println!`, which is why `print_stdout` and `print_stderr` are denied
//! workspace-wide.
//!
//! A broken pipe is not an error worth reporting: `rune list | head` closes the pipe on
//! purpose, and a panic there would be rune's bug, not the user's.
//!
//! When several scripts run at once their output has to stay attributable, and that is
//! what [`multiplex`] is: a pure function from a sequence of chunks to the exact bytes a
//! terminal should receive. [`channel`] is the queue feeding it, and [`color`] decides how
//! much color rune may use and what it tells a child about color.

pub mod channel;
pub mod color;
pub mod multiplex;

use std::io::{self, Write};

/// Writes one line of a command's product to stdout.
pub fn line(text: &str) {
  let mut stdout = io::stdout().lock();
  let _ = writeln!(stdout, "{text}");
}

/// Writes a diagnostic to stderr. Everything rune says about itself lands here.
pub fn diagnostic(text: &str) {
  let mut stderr = io::stderr().lock();
  let _ = writeln!(stderr, "{text}");
}
