//! Turning a name nobody defined into the name the user meant.
//!
//! A script name can be missed in three places — running one, extending one, inspecting
//! one — and all three deserve the same answer. One function serves them so the three
//! cannot drift into three different opinions about what counts as close.

/// The distance at which a name stops being a plausible typo and starts being a
/// different word. Two edits catches the transpositions and the dropped letters; three
/// starts proposing `build` for `clean`.
const LIMIT: usize = 3;

/// The defined name closest to `name`, when one is close enough to be worth offering.
///
/// Ties go to the first candidate in the order given. Callers pass names from a sorted
/// map, so the same typo always draws the same suggestion.
pub fn closest<'a, I>(name: &str, candidates: I) -> Option<&'a str>
where
  I: IntoIterator<Item = &'a str>,
{
  candidates
    .into_iter()
    .map(|defined| (strsim::levenshtein(name, defined), defined))
    .filter(|(distance, _)| *distance < LIMIT)
    .min_by_key(|(distance, _)| *distance)
    .map(|(_, defined)| defined)
}

#[cfg(test)]
mod tests {
  use super::closest;

  #[test]
  fn a_one_letter_typo_is_suggested() {
    assert_eq!(closest("biuld", ["build", "test"]), Some("build"));
  }

  #[test]
  fn an_unrelated_name_gets_no_suggestion() {
    assert_eq!(closest("deploy", ["build", "test"]), None);
  }

  #[test]
  fn nothing_defined_means_nothing_to_suggest() {
    assert_eq!(closest("build", []), None);
  }

  /// Same distance from two names, so the answer has to come from the order rather than
  /// from whichever one the iterator happened to reach first.
  #[test]
  fn a_tie_takes_the_first_candidate() {
    assert_eq!(closest("tes", ["test", "text"]), Some("test"));
  }
}
