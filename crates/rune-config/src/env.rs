//! What a config is allowed to know about the machine it is being evaluated on.
//!
//! Reads are recorded. The cache key covers only the variables a config actually asked
//! for, so an unrelated variable changing between two runs does not throw away a valid
//! cached result — and a variable the config *did* read always invalidates it.
//!
//! A name is looked up the way the running operating system stores one. Windows keeps the
//! search path as `Path`, and every other reader on the platform answers `PATH` from it;
//! a config that could not would work on two systems and build the wrong command on the
//! third.

use std::cell::{Cell, RefCell};
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
  /// Lookup key to the name as the machine spells it and its value. The spelling is kept
  /// because it is what an enumeration returns: a config that spreads the environment has
  /// to see the names its scripts will receive.
  vars: BTreeMap<String, (String, String)>,
}

impl Environment {
  /// The real process environment.
  pub fn from_process() -> Self {
    Self::from_pairs(std::env::vars())
  }

  /// A fixed environment. Tests use this instead of mutating the process, which is
  /// unsound while other tests are running in the same process.
  pub fn from_pairs<I, K, V>(pairs: I) -> Self
  where
    I: IntoIterator<Item = (K, V)>,
    K: Into<String>,
    V: Into<String>,
  {
    let vars = pairs
      .into_iter()
      .map(|(key, value)| {
        let name = key.into();
        (lookup_key(&name), (name, value.into()))
      })
      .collect();

    Self { vars }
  }

  pub fn get(&self, key: &str) -> Option<&str> {
    self.vars.get(&lookup_key(key)).map(|(_, value)| value.as_str())
  }

  /// Every variable, spelled the way the machine spells it.
  pub fn names(&self) -> impl Iterator<Item = &str> {
    self.vars.values().map(|(name, _)| name.as_str())
  }

  /// Follows the convention every CI provider sets: `CI` present and not a falsy word.
  pub fn is_ci(&self) -> bool {
    is_ci_value(self.get(CI_VARIABLE))
  }
}

/// Looks a variable up the way the running operating system would.
///
/// The rule Rune already applies when it builds a child's environment, applied here too,
/// so the value a config reads and the value its script receives cannot disagree about
/// which variable a name means. Away from Windows two spellings are two variables, and
/// folding them would make one of them unreachable.
fn lookup_key(name: &str) -> String {
  if cfg!(windows) { name.to_uppercase() } else { name.to_owned() }
}

fn is_ci_value(value: Option<&str>) -> bool {
  matches!(value, Some(value) if !matches!(value, "" | "0" | "false"))
}

/// What a config read while it was evaluated, and what the cache key therefore covers.
#[derive(Debug, Clone, Default)]
pub struct Observations {
  /// Each variable the config asked for by name, with the value it saw.
  pub values: BTreeMap<String, Option<String>>,
  /// Set when the config asked for the whole environment. The names it saw are all in
  /// `values`, but a variable that did not exist then is a change those names cannot
  /// describe, so the lookup has to know it was built from everything.
  pub enumerated: bool,
}

/// An [`Environment`] that remembers which variables were read through it.
#[derive(Debug, Clone)]
pub struct ObservedEnvironment {
  environment: Environment,
  observed: Rc<RefCell<BTreeSet<String>>>,
  enumerated: Rc<Cell<bool>>,
}

impl ObservedEnvironment {
  pub fn new(environment: Environment) -> Self {
    Self {
      environment,
      observed: Rc::new(RefCell::new(BTreeSet::new())),
      enumerated: Rc::new(Cell::new(false)),
    }
  }

  /// Records the read, then answers it.
  pub fn read(&self, key: &str) -> Option<String> {
    self.observed.borrow_mut().insert(key.to_owned());
    self.environment.get(key).map(str::to_owned)
  }

  /// Every name, recorded as a read of each one.
  ///
  /// A config that enumerates is built from the whole environment, so the whole
  /// environment is what its cached result stands or falls by.
  pub fn names(&self) -> Vec<String> {
    self.enumerated.set(true);

    let mut observed = self.observed.borrow_mut();
    self
      .environment
      .names()
      .map(|name| {
        observed.insert(name.to_owned());
        name.to_owned()
      })
      .collect()
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
  pub fn observations(&self) -> Observations {
    let values = self
      .observed
      .borrow()
      .iter()
      .map(|key| (key.clone(), self.environment.get(key).map(str::to_owned)))
      .collect();

    Observations { values, enumerated: self.enumerated.get() }
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

  /// Test R9.7 — the lookup applies the rule the operating system applies, and the two
  /// halves are one row: a fold that ran everywhere would make `path` and `PATH` the same
  /// variable on Linux, where they are two.
  #[test]
  fn the_lookup_folds_case_only_on_windows() {
    let environment = Environment::from_pairs([("Path", "one")]);

    assert_eq!(environment.get("Path"), Some("one"));

    if cfg!(windows) {
      assert_eq!(environment.get("PATH"), Some("one"));
      assert_eq!(environment.get("path"), Some("one"));
    } else {
      assert_eq!(environment.get("PATH"), None);
      assert_eq!(environment.get("path"), None);
    }
  }

  #[test]
  fn two_spellings_are_two_variables_only_away_from_windows() {
    let environment = Environment::from_pairs([("Path", "one"), ("PATH", "two")]);

    let expected = if cfg!(windows) { 1 } else { 2 };
    assert_eq!(environment.names().count(), expected);
  }

  #[test]
  fn only_the_variables_that_were_read_are_observed() {
    let env = ObservedEnvironment::new(Environment::from_pairs([("A", "1"), ("B", "2")]));

    assert_eq!(env.read("A"), Some("1".to_owned()));
    assert_eq!(env.read("MISSING"), None);

    let observations = env.observations();
    assert_eq!(observations.values.len(), 2, "{observations:?}");
    assert_eq!(observations.values["A"], Some("1".to_owned()));
    assert_eq!(observations.values["MISSING"], None);
    assert!(!observations.enumerated, "nothing here asked for the whole environment");
  }

  #[test]
  fn an_enumeration_is_a_read_of_everything_it_returned() {
    let env = ObservedEnvironment::new(Environment::from_pairs([("A", "1"), ("B", "2")]));

    assert_eq!(env.names(), ["A", "B"]);

    let observations = env.observations();
    assert!(observations.enumerated);
    assert_eq!(observations.values["A"], Some("1".to_owned()));
    assert_eq!(observations.values["B"], Some("2".to_owned()));
  }
}
