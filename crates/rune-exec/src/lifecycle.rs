//! How a script is run, as opposed to what it runs.
//!
//! Five options answer the same kind of question — how long may this take, how many times
//! may it be tried, how hard is it ended — and none of them is a property of the command
//! string. They belong to one script and one process tree, which is why a group never
//! carries them.
//!
//! The retry schedule lives here as a pure function so that the arithmetic can be asserted
//! without a clock, and the waiting itself goes through [`hold`] so the two can never drift
//! apart.

use std::time::Duration;

use tokio::time::Instant;

/// What rune exits with when it ended a script for taking too long.
///
/// The same code GNU `timeout` reports, so a job that already branches on it keeps working.
/// A child can exit 124 of its own accord; the line rune writes to stderr is what tells
/// the two apart.
pub const TIMEOUT_CODE: u32 = 124;

/// How long a tree is given to act on the signal before it is ended for it.
///
/// Long enough that a server flushing its state on the way out is not cut off, short enough
/// that a script ignoring the request cannot hold the whole run open.
const DEFAULT_KILL_TIMEOUT: Duration = Duration::from_secs(5);

/// Everything one script's execution needs beyond the command itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lifecycle {
  /// How long one attempt may run before its whole tree is terminated.
  pub timeout: Option<Duration>,
  /// How many further attempts a failing script gets. Retries happen only on failure.
  pub retries: u32,
  pub retry_delay: RetryDelay,
  pub kill: Kill,
}

impl Default for Lifecycle {
  fn default() -> Self {
    Self {
      timeout: None,
      retries: 0,
      retry_delay: RetryDelay::Fixed(Duration::ZERO),
      kill: Kill::default(),
    }
  }
}

/// How long rune waits before trying a failing script again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryDelay {
  /// The same wait before every attempt.
  Fixed(Duration),
  /// `2^attempt` seconds, so the wait grows with each attempt for something that is slow
  /// to become ready.
  Exponential,
}

impl RetryDelay {
  /// The wait before `attempt`, where the first retry is attempt 1.
  pub fn before(self, attempt: u32) -> Duration {
    match self {
      Self::Fixed(delay) => delay,
      // Saturating rather than wrapping: an absurd retry count must not turn a long wait
      // into a short one.
      Self::Exponential => Duration::from_secs(2_u64.checked_pow(attempt).unwrap_or(u64::MAX)),
    }
  }
}

/// How a script's process tree is ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Kill {
  /// What the tree is asked with first, which a child may catch and act on.
  pub signal: KillSignal,
  /// How long that request is given before the tree is ended whatever it wanted.
  pub timeout: Duration,
}

impl Default for Kill {
  fn default() -> Self {
    Self { signal: KillSignal::Term, timeout: DEFAULT_KILL_TIMEOUT }
  }
}

impl Kill {
  /// A tree with no gentle form: the first blow is already the last one.
  pub const UNCONDITIONAL: Self = Self { signal: KillSignal::Kill, timeout: Duration::ZERO };

  /// When a tree asked to end at `asked_at` must be ended for it, or nothing at all when
  /// the signal was already the unconditional kill and there is nothing to escalate to.
  pub fn escalation(self, asked_at: Instant) -> Option<Instant> {
    (self.signal != KillSignal::Kill).then(|| asked_at + self.timeout)
  }
}

/// The signal rune sends first when it ends a script's tree.
///
/// A closed set rather than whatever the platform happens to name, so a config written on
/// one operating system loads on every other.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum KillSignal {
  Hup,
  Int,
  Quit,
  #[default]
  Term,
  /// Nothing can catch this one.
  Kill,
}

/// Waits out the delay before `attempt`.
pub(crate) async fn hold(delay: RetryDelay, attempt: u32) {
  tokio::time::sleep(delay.before(attempt)).await;
}

#[cfg(test)]
mod tests {
  use super::{Duration, Instant, Kill, KillSignal, RetryDelay, hold};

  /// Test 5d.3, the fixed half — a number is the same wait before every attempt.
  ///
  /// The clock is paused, so the schedule is asserted at full speed: the assertion is on
  /// what was waited for, never on how long the test took.
  #[tokio::test(start_paused = true)]
  async fn a_numeric_delay_is_waited_before_every_attempt() {
    let delay = RetryDelay::Fixed(Duration::from_millis(250));

    for attempt in 1..=3 {
      let started = Instant::now();
      hold(delay, attempt).await;

      assert_eq!(started.elapsed(), Duration::from_millis(250), "before attempt {attempt}");
    }
  }

  /// Test 5d.3, the exponential half — `2^attempt` seconds, the same formula and the same
  /// unit as the implementation this follows.
  #[tokio::test(start_paused = true)]
  async fn an_exponential_delay_doubles_with_every_attempt() {
    for (attempt, seconds) in [(1, 2), (2, 4), (3, 8)] {
      let started = Instant::now();
      hold(RetryDelay::Exponential, attempt).await;

      assert_eq!(started.elapsed(), Duration::from_secs(seconds), "before attempt {attempt}");
    }
  }

  /// A script that says nothing about the delay is retried at once.
  #[tokio::test(start_paused = true)]
  async fn no_delay_means_no_waiting() {
    let started = Instant::now();
    hold(RetryDelay::Fixed(Duration::ZERO), 1).await;

    assert_eq!(started.elapsed(), Duration::ZERO);
  }

  /// The escalation timer exists to give a catchable signal time to be caught. A signal
  /// nothing can catch has nothing to escalate to, so no timer is started.
  #[test]
  fn an_unconditional_signal_starts_no_escalation_timer() {
    let asked_at = Instant::now();

    assert_eq!(Kill::UNCONDITIONAL.escalation(asked_at), None);
    assert_eq!(
      Kill { signal: KillSignal::Term, timeout: Duration::from_millis(200) }.escalation(asked_at),
      Some(asked_at + Duration::from_millis(200))
    );
  }
}
