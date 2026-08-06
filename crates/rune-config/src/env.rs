//! What a config is allowed to know about the machine it is being evaluated on.
//!
//! Reads are recorded. The cache key covers only the variables a config actually asked
//! for, so an unrelated variable changing between two runs does not throw away a valid
//! cached result — and a variable the config *did* read always invalidates it.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

/// The operating system name a config sees, matching Node's `process.platform`.
pub const PLATFORM: &str = if cfg!(target_os = "windows") {
  "win32"
} else if cfg!(target_os = "macos") {
  "darwin"
} else {
  "linux"
};

/// The variable that decides `rune.isCI`.
const CI_VARIABLE: &str = "CI";

#[derive(Debug, Clone, Default)]
pub struct Environment {
  vars: BTreeMap<String, String>,
}

impl Environment {
  /// The real process environment.
  pub fn from_process() -> Self {
    Self { vars: std::env::vars().collect() }
  }

  /// A fixed environment. Tests use this instead of mutating the process, which is
  /// unsound while other tests are running in the same process.
  pub fn from_pairs<I, K, V>(pairs: I) -> Self
  where
    I: IntoIterator<Item = (K, V)>,
    K: Into<String>,
    V: Into<String>,
  {
    Self { vars: pairs.into_iter().map(|(key, value)| (key.into(), value.into())).collect() }
  }

  pub fn get(&self, key: &str) -> Option<&str> {
    self.vars.get(key).map(String::as_str)
  }

  /// Follows the convention every CI provider sets: `CI` present and not a falsy word.
  pub fn is_ci(&self) -> bool {
    is_ci_value(self.get(CI_VARIABLE))
  }
}

fn is_ci_value(value: Option<&str>) -> bool {
  matches!(value, Some(value) if !matches!(value, "" | "0" | "false"))
}

/// An [`Environment`] that remembers which variables were read through it.
#[derive(Debug, Clone)]
pub struct ObservedEnvironment {
  environment: Environment,
  observed: Rc<RefCell<BTreeSet<String>>>,
}

impl ObservedEnvironment {
  pub fn new(environment: Environment) -> Self {
    Self { environment, observed: Rc::new(RefCell::new(BTreeSet::new())) }
  }

  /// Records the read, then answers it.
  pub fn read(&self, key: &str) -> Option<String> {
    self.observed.borrow_mut().insert(key.to_owned());
    self.environment.get(key).map(str::to_owned)
  }

  /// Recorded like any other read.
  ///
  /// `isCI` is derived from exactly one variable, so a config that branches on it has
  /// to miss the cache when that variable changes. Answering from the environment
  /// directly would leave the read invisible and serve a CI result on a laptop.
  pub fn is_ci(&self) -> bool {
    is_ci_value(self.read(CI_VARIABLE).as_deref())
  }

  /// Every variable the config asked for, with the value it saw. This is what the
  /// cache key covers.
  pub fn observations(&self) -> BTreeMap<String, Option<String>> {
    self
      .observed
      .borrow()
      .iter()
      .map(|key| (key.clone(), self.environment.get(key).map(str::to_owned)))
      .collect()
  }
}

#[cfg(test)]
mod tests {
  use super::{Environment, ObservedEnvironment};

  #[test]
  fn ci_is_false_for_falsy_words() {
    for value in ["", "0", "false"] {
      assert!(!Environment::from_pairs([("CI", value)]).is_ci(), "`CI={value}` read as CI");
    }
  }

  #[test]
  fn ci_is_true_when_set_to_anything_else() {
    assert!(Environment::from_pairs([("CI", "1")]).is_ci());
    assert!(Environment::from_pairs([("CI", "true")]).is_ci());
  }

  #[test]
  fn ci_is_false_when_unset() {
    assert!(!Environment::default().is_ci());
  }

  #[test]
  fn only_the_variables_that_were_read_are_observed() {
    let env = ObservedEnvironment::new(Environment::from_pairs([("A", "1"), ("B", "2")]));

    assert_eq!(env.read("A"), Some("1".to_owned()));
    assert_eq!(env.read("MISSING"), None);

    let observations = env.observations();
    assert_eq!(observations.len(), 2, "{observations:?}");
    assert_eq!(observations["A"], Some("1".to_owned()));
    assert_eq!(observations["MISSING"], None);
  }
}
