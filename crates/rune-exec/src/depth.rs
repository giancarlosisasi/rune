//! How many times rune has started rune to reach this process.
//!
//! A script's command may run rune, and the rune it runs may run it again. Nothing in a
//! config says so: the call sits inside a shell command string, which rune deliberately
//! does not parse, so the only way to see the recursion is to count it.
//!
//! The count travels in the environment because the child of a script is a shell, and an
//! argument, a file descriptor or a handle does not survive one. It carries a level and
//! never a name — two scripts running each other repeat no name before the next level, so
//! a name-keyed guard lets that shape through and a counted one stops both.

use std::ffi::OsStr;

/// How deep rune has started rune to reach here.
pub const VARIABLE: &str = "RUNE_DEPTH";

/// How deep rune goes before it refuses.
///
/// Ten levels is already past anything anyone designs — a real config uses one,
/// occasionally two — and low enough that a runaway ends in well under a second. It is
/// fixed rather than configurable: a generator that emits eleven levels would raise it.
pub const LIMIT: u32 = 10;

/// The depth a value stands for, and no depth at all for anything rune would not have
/// written.
///
/// A config cannot set this variable, but a user's shell can. Refusing to run on the
/// strength of a stray value would turn one export into a repository nobody can build,
/// while reading it as no depth costs one level of a runaway that rune then catches on the
/// next.
pub fn of(value: Option<&OsStr>) -> u32 {
  value
    .and_then(OsStr::to_str)
    .and_then(|value| value.parse().ok())
    .filter(|depth| *depth <= LIMIT)
    .unwrap_or(0)
}

#[cfg(test)]
mod tests {
  use std::ffi::OsStr;

  use super::{LIMIT, of};

  /// Test R19.7 — the reader alone. A number rune wrote answers itself; everything else
  /// answers no depth, because everything else was written by something that is not rune.
  #[test]
  fn a_value_rune_wrote_is_read_back_as_the_depth_it_holds() {
    assert_eq!(of(Some(OsStr::new("0"))), 0);
    assert_eq!(of(Some(OsStr::new("3"))), 3);
    assert_eq!(of(Some(OsStr::new(&LIMIT.to_string()))), LIMIT);
  }

  #[test]
  fn anything_rune_would_not_have_written_is_no_depth_at_all() {
    for value in ["", " ", "deep", "-1", "3.5", "2 ", "99999999999999999999"] {
      assert_eq!(of(Some(OsStr::new(value))), 0, "{value:?}");
    }

    assert_eq!(of(Some(OsStr::new(&(LIMIT + 1).to_string()))), 0, "deeper than rune ever goes");
    assert_eq!(of(None), 0, "the first rune of a chain");
  }
}
