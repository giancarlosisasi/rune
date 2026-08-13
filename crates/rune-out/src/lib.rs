//! Where rune's own output goes.
//!
//! Two rules this crate exists to enforce. The first is about which stream, and it is a
//! property of what is being written rather than a list of the commands that write it: a
//! command whose *product* is text writes that text to stdout, everything rune says about
//! itself goes to stderr, and a command that spawns a child leaves stdout to the child.
//! `rune list` and `rune inspect` are examples of the rule, not the rule — a list of names
//! goes stale the moment a query command is added, which is exactly how this comment came
//! to name one of the two. The second rule is that output goes through here rather than
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
