//! What evaluating a config may spend.
//!
//! Every rune command evaluates the config before it does its own work, so an evaluation
//! that never returns takes every command with it and an evaluation that allocates without
//! end takes the machine. Both ceilings are the engine's own: it stops itself and names the
//! file it was reading, where anything watching from outside could only end the process and
//! leave the terminal empty.
//!
//! Each ceiling can be raised from the environment rather than from the config, because the
//! config is what has not finished being read. The names live under the prefix a config can
//! never set, so a repository cannot raise its own ceiling for its children either.

use std::time::Duration;

use crate::env::Environment;

/// How long a config may take to evaluate, in milliseconds.
pub const TIME_VARIABLE: &str = "RUNE_CONFIG_TIME_LIMIT_MS";

/// How much memory a config may use while it is evaluated, in megabytes.
pub const MEMORY_VARIABLE: &str = "RUNE_CONFIG_MEMORY_LIMIT_MB";

/// Two orders of magnitude above a measured cold evaluation, which is tens of
/// milliseconds. Only a config that was never going to finish reaches it.
const DEFAULT_TIME: Duration = Duration::from_secs(5);

/// Far more than any config assembles and far less than a laptop has, so the ceiling is met
/// by a runaway and by nothing that was going to stop on its own.
const DEFAULT_MEMORY_MB: u64 = 256;

const BYTES_PER_MB: u64 = 1024 * 1024;

/// The two ceilings one evaluation runs under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
  pub time: Duration,
  pub memory_mb: u64,
}

impl Limits {
  /// What the environment asks for, or what rune asks for where it says nothing usable.
  pub fn from_environment(environment: &Environment) -> Self {
    Self {
      time: raised(environment, TIME_VARIABLE).map_or(DEFAULT_TIME, Duration::from_millis),
      memory_mb: raised(environment, MEMORY_VARIABLE).unwrap_or(DEFAULT_MEMORY_MB),
    }
  }

  /// The memory ceiling in the unit the engine takes.
  ///
  /// A value too large for this machine's pointer saturates, which the engine reads as no
  /// ceiling at all — the same thing the user asked for by naming a number that big.
  pub fn memory_bytes(&self) -> usize {
    usize::try_from(self.memory_mb.saturating_mul(BYTES_PER_MB)).unwrap_or(usize::MAX)
  }
}

/// A number a user meant, and nothing for anything else.
///
/// A value that cannot be read leaves rune's own limit in force rather than refusing every
/// command: one stray export must never become a repository nobody can build. The limit
/// actually in force is named in the refusal, so a value that did not take is visible in
/// the one place it matters.
fn raised(environment: &Environment, name: &str) -> Option<u64> {
  environment.get(name)?.trim().parse::<u64>().ok().filter(|value| *value > 0)
}

#[cfg(test)]
mod tests {
  use std::time::Duration;

  use super::{DEFAULT_MEMORY_MB, DEFAULT_TIME, Limits, MEMORY_VARIABLE, TIME_VARIABLE};
  use crate::env::Environment;

  fn limits(pairs: &[(&str, &str)]) -> Limits {
    Limits::from_environment(&Environment::from_pairs(pairs.iter().copied()))
  }

  #[test]
  fn an_environment_that_says_nothing_gets_runes_own_limits() {
    let limits = limits(&[]);

    assert_eq!(limits.time, DEFAULT_TIME);
    assert_eq!(limits.memory_mb, DEFAULT_MEMORY_MB);
  }

  #[test]
  fn a_number_the_environment_names_is_the_limit() {
    let limits = limits(&[(TIME_VARIABLE, "250"), (MEMORY_VARIABLE, "32")]);

    assert_eq!(limits.time, Duration::from_millis(250));
    assert_eq!(limits.memory_mb, 32);
  }

  #[test]
  fn a_value_that_cannot_be_read_leaves_the_limit_where_it_was() {
    for value in ["", " ", "soon", "-1", "2.5", "0"] {
      let limits = limits(&[(TIME_VARIABLE, value), (MEMORY_VARIABLE, value)]);

      assert_eq!(limits.time, DEFAULT_TIME, "{value:?}");
      assert_eq!(limits.memory_mb, DEFAULT_MEMORY_MB, "{value:?}");
    }
  }

  #[test]
  fn a_ceiling_larger_than_this_machine_can_address_becomes_no_ceiling() {
    assert_eq!(limits(&[(MEMORY_VARIABLE, &u64::MAX.to_string())]).memory_bytes(), usize::MAX);
  }
}
