//! Where the version rune reports comes from.
//!
//! One file, `version.txt`, embedded at compile time. The packaging step rewrites it on
//! every bump, so the binary can never report a version the package it ships in does not
//! carry.

/// The version `rune --version` prints.
pub const VERSION: &str = first_line(include_str!("../../../version.txt"));

/// The text before the first line ending, of either kind.
///
/// A checkout on Windows turns the file's single newline into a carriage return and a
/// newline. Keeping the carriage return would put it inside the version string, where it
/// would reach every place the version is printed or compared.
const fn first_line(text: &str) -> &str {
  let bytes = text.as_bytes();

  let mut end = 0;
  while end < bytes.len() && bytes[end] != b'\r' && bytes[end] != b'\n' {
    end += 1;
  }

  // The scan stops only on ASCII bytes, and those never appear inside a multi-byte
  // character, so the cut cannot land in the middle of one.
  match str::from_utf8(bytes.split_at(end).0) {
    Ok(line) => line,
    Err(_) => panic!("version.txt is not valid UTF-8"),
  }
}

#[cfg(test)]
mod tests {
  use super::first_line;

  /// Test 6a.13 — the carriage return is in the fixture on purpose, so the trim is
  /// proven rather than assumed. `.gitattributes` keeps it there through a checkout.
  #[test]
  fn the_carriage_return_a_windows_checkout_leaves_is_not_part_of_the_version() {
    let fixture = include_str!("../tests/fixtures/version-crlf.txt");

    assert_eq!(first_line(fixture), "1.2.3");
  }

  #[test]
  fn a_file_of_one_line_and_no_ending_is_the_whole_of_it() {
    assert_eq!(first_line("0.1.0"), "0.1.0");
  }

  /// The config cache keys on the crate version, and `rune --version` prints the
  /// embedded one. A bump that moved only one of them would leave an upgraded rune
  /// serving the previous version's cached answers.
  #[test]
  fn the_version_the_crates_are_built_with_is_the_one_that_is_printed() {
    assert_eq!(super::VERSION, env!("CARGO_PKG_VERSION"));
  }

  #[test]
  fn the_embedded_version_is_three_numbers() {
    let parts: Vec<&str> = super::VERSION.split('.').collect();

    assert_eq!(parts.len(), 3, "expected MAJOR.MINOR.PATCH, got {}", super::VERSION);
    for part in parts {
      assert!(part.parse::<u32>().is_ok(), "segment `{part}` is not a number");
    }
  }
}
