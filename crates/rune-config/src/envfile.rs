//! Reading the dotenv files a config declares.
//!
//! Parsing is `dotenvy`'s job: quoting, escapes, comments, `export` prefixes and values
//! that span lines are solved problems with edge cases nobody wants to rediscover. What
//! stays here is everything about *which* file — a path means what it means relative to
//! the config that wrote it, and a file that is named but absent is a broken config
//! rather than a degraded one — and everything about the bytes: the encoding, and where
//! in the file each assignment was written.
//!
//! The iterator API is the one rune can use. `dotenvy::from_path` loads a file straight
//! into rune's own process environment, which is the opposite of what a runner wants:
//! every assignment has to be inspected before anything decides whether it applies.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::inherit::Declared;
use crate::paths::relative_to;
use crate::resolve::lexically_normalize;

/// The three bytes a Windows editor writes in front of a file it saves as UTF-8.
const MARK: [u8; 3] = [0xEF, 0xBB, 0xBF];

/// The same three bytes as a character, which is what they are anywhere but the start.
const MARK_CHARACTER: char = '\u{feff}';

/// One assignment a file made, and where it was written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assignment {
  pub name: String,
  pub value: String,
  /// The physical line the assignment opened on, counting from one.
  pub line: usize,
}

/// One dotenv file, read once for the config that declared it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvFile {
  /// The file, spelled the way rune spells every path it prints. Warnings name it, so it
  /// is a label rather than a path: an absolute one would bury the part that identifies
  /// the file and would differ between machines.
  pub source: String,
  /// Every assignment, in the order the file wrote them.
  pub assignments: Vec<Assignment>,
}

#[derive(Debug, Error)]
pub enum EnvFileError {
  #[error(
    "script `{script}` declares an envFile that does not exist\n\n  {path}\n\n\
     an `envFile` is resolved against the config that declares it, which is {config}.\n\n\
     create the file, or remove `envFile` from the script."
  )]
  Missing { script: String, path: String, config: String },

  #[error("cannot read {path}, declared by script `{script}` in {config}\n\n{message}")]
  Unreadable { script: String, path: String, config: String, message: String },

  #[error(
    "{path} is UTF-16, and rune reads env files as UTF-8\n\n\
     save it as UTF-8 in your editor, or convert it in place:\n\n  \
     on Windows: powershell -Command \"(Get-Content {path}) | \
     Set-Content -Encoding utf8 {path}\"\n  \
     elsewhere:  iconv -f UTF-16 -t UTF-8 {path} > converted && mv converted {path}"
  )]
  Utf16 { path: String },

  #[error(
    "{path} is not valid UTF-8\n\n\
     rune reads env files as UTF-8, and decoding stopped at byte {offset}."
  )]
  NotUtf8 { path: String, offset: usize },

  #[error(
    "{path} line {line} is not an assignment\n\n  {text}\n\n\
     every line of an env file is `NAME=value`, a `#` comment, or empty."
  )]
  Parse { path: String, line: usize, text: String },
}

/// The files one invocation has read, keyed by the path each resolved to.
///
/// A file named by four members of a group is opened once. Keying on the resolved path
/// rather than on what was written is what makes `.env` and `./.env` in the same config
/// one file, which is what they are.
#[derive(Debug, Default)]
pub struct Files {
  read: BTreeMap<PathBuf, EnvFile>,
}

impl Files {
  /// Reads what `script` declared, or answers from what this invocation already read.
  pub fn read(
    &mut self,
    root: &Path,
    script: &str,
    declared: &Declared<'_>,
  ) -> Result<&EnvFile, EnvFileError> {
    let path = lexically_normalize(&declared.anchor().join(declared.value));

    if !self.read.contains_key(&path) {
      let file = read(root, declared.source, script, declared.value)?;
      self.read.insert(path.clone(), file);
    }

    Ok(&self.read[&path])
  }

  /// How many distinct files this invocation opened.
  pub fn len(&self) -> usize {
    self.read.len()
  }

  pub fn is_empty(&self) -> bool {
    self.read.is_empty()
  }
}

/// Reads the file `script` declared as `declared`, resolved against `config_path`.
pub fn read(
  root: &Path,
  config_path: &Path,
  script: &str,
  declared: &str,
) -> Result<EnvFile, EnvFileError> {
  let directory = config_path.parent().unwrap_or_else(|| Path::new("."));
  let source = relative_to(root, &lexically_normalize(&directory.join(declared)));
  let config = relative_to(root, config_path);

  let raw = match std::fs::read(directory.join(declared)) {
    Ok(raw) => raw,
    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
      return Err(EnvFileError::Missing { script: script.to_owned(), path: source, config });
    }
    Err(error) => {
      return Err(EnvFileError::Unreadable {
        script: script.to_owned(),
        path: source,
        config,
        message: error.to_string(),
      });
    }
  };

  let contents = decode(&raw, &source)?;
  let mut positions = Positions::new(&contents);
  let mut assignments = Vec::new();

  for item in dotenvy::Iter::new(contents.as_bytes()) {
    match item {
      Ok((name, value)) => {
        let line = positions.of_assignment(&name);
        assignments.push(Assignment { name, value, line });
      }
      Err(dotenvy::Error::LineParse(reported, _)) => {
        let text = first_line(&reported);
        let line = positions.of_line_starting_with(&text);
        return Err(EnvFileError::Parse { path: source, line, text: printable(&text) });
      }
      // The reader is a string already in memory, so no I/O is left to fail here.
      Err(other) => {
        return Err(EnvFileError::Unreadable {
          script: script.to_owned(),
          path: source,
          config,
          message: other.to_string(),
        });
      }
    }
  }

  Ok(EnvFile { source, assignments })
}

/// Turns the file's bytes into text, or says what is in it instead.
///
/// A mark at the very start carries no information in a file already required to be
/// UTF-8, and every mainstream reader of this format removes it; leaving it in makes the
/// first key unreadable and the message about it unbelievable, because the byte does not
/// show on screen. The same bytes anywhere else are data.
fn decode(raw: &[u8], path: &str) -> Result<String, EnvFileError> {
  let body = raw.strip_prefix(&MARK).unwrap_or(raw);

  String::from_utf8(body.to_vec()).map_err(|error| {
    if raw.starts_with(&[0xFF, 0xFE]) || raw.starts_with(&[0xFE, 0xFF]) {
      return EnvFileError::Utf16 { path: path.to_owned() };
    }

    EnvFileError::NotUtf8 { path: path.to_owned(), offset: error.utf8_error().valid_up_to() }
  })
}

/// Where each assignment was written, found as the reader walks the file.
///
/// `dotenvy` reports a name and a value and never a position. The name cannot simply be
/// looked up either: the same name may be assigned twice, and every report that needs a
/// position is about the assignment that *lost*, while a search from the start of the
/// file finds the one that won. Walking forward answers both, because the iterator yields
/// in file order.
struct Positions<'a> {
  lines: std::iter::Enumerate<std::str::Lines<'a>>,
  last: usize,
}

impl<'a> Positions<'a> {
  fn new(contents: &'a str) -> Self {
    Self { lines: contents.lines().enumerate(), last: 1 }
  }

  /// The line `name` was assigned on, searching forward from the last one found.
  fn of_assignment(&mut self, name: &str) -> usize {
    self.find(|line| opens_assignment(line, name))
  }

  /// The line the given text was quoted from. A value spanning several lines is reported
  /// whole, so its first line is where it began, and a line cut short at a `#` comment
  /// still starts the physical line it came from.
  fn of_line_starting_with(&mut self, text: &str) -> usize {
    self.find(|line| line.trim_end().starts_with(text))
  }

  fn find(&mut self, matches: impl Fn(&str) -> bool) -> usize {
    for (index, line) in self.lines.by_ref() {
      if matches(line) {
        self.last = index + 1;
        return self.last;
      }
    }

    // Unreachable for a file `dotenvy` read: every assignment it yields came from a line
    // in this text. Reporting the last position found beats reporting none.
    self.last
  }
}

/// Whether this line is where `name` is assigned, allowing the `export` prefix and the
/// spacing `dotenvy` accepts around the name.
fn opens_assignment(line: &str, name: &str) -> bool {
  let trimmed = line.trim_start();
  let rest = trimmed.strip_prefix("export ").unwrap_or(trimmed).trim_start();

  rest.strip_prefix(name).is_some_and(|after| after.trim_start().starts_with('='))
}

/// Quotes a line back with nothing in it a terminal would swallow.
///
/// The mark this reader deliberately does not strip anywhere but the start is exactly
/// what turns up in a line that will not parse, and a raw one written to a terminal
/// removes the evidence at the moment it is needed.
fn printable(text: &str) -> String {
  text
    .chars()
    .map(|character| {
      if character.is_control() || character == MARK_CHARACTER {
        format!("\\u{{{:x}}}", character as u32)
      } else {
        character.to_string()
      }
    })
    .collect()
}

fn first_line(reported: &str) -> String {
  reported.lines().next().unwrap_or_default().trim_end().to_owned()
}

#[cfg(test)]
mod tests {
  use super::{Positions, printable};

  #[test]
  fn a_rejected_line_is_numbered_from_one() {
    let contents = "A=1\nB=2\nnot an assignment\n";

    assert_eq!(Positions::new(contents).of_line_starting_with("not an assignment"), 3);
    assert_eq!(Positions::new(contents).of_line_starting_with("A=1"), 1);
  }

  /// A value that spans lines is reported whole, and the line to name is the one the
  /// assignment started on rather than wherever the file ran out.
  #[test]
  fn a_multi_line_value_is_numbered_where_it_opened() {
    let contents = "A=1\nB=\"unterminated\nstill going\n";

    assert_eq!(Positions::new(contents).of_line_starting_with("B=\"unterminated"), 2);
  }

  /// `dotenvy` cuts a line short at a `#`, so the reported text is a prefix of the line
  /// that is actually in the file.
  #[test]
  fn a_line_cut_short_at_a_comment_still_finds_its_place() {
    let contents = "A=1\nnot an assignment # explained\n";

    assert_eq!(Positions::new(contents).of_line_starting_with("not an assignment"), 2);
  }

  /// Test R10.6 — a value spanning three lines advances the count by three, because the
  /// walk consumes the lines it passes rather than counting what it was handed.
  #[test]
  fn an_assignment_after_a_multi_line_value_is_numbered_past_it() {
    let contents = "A=\"one\ntwo\nthree\"\nB=2\n";
    let mut positions = Positions::new(contents);

    assert_eq!(positions.of_assignment("A"), 1);
    assert_eq!(positions.of_assignment("B"), 4);
  }

  /// The repeated name is the case the whole numbering exists for: a search from the
  /// start of the file finds the winner, and every warning is about the loser.
  #[test]
  fn a_repeated_name_is_found_at_its_own_occurrence() {
    let contents = "DUP=first\nOTHER=1\nDUP=second\n";
    let mut positions = Positions::new(contents);

    assert_eq!(positions.of_assignment("DUP"), 1);
    assert_eq!(positions.of_assignment("OTHER"), 2);
    assert_eq!(positions.of_assignment("DUP"), 3);
  }

  #[test]
  fn the_export_prefix_and_spacing_do_not_hide_an_assignment() {
    let contents = "  export A = 1\nB=2\n";
    let mut positions = Positions::new(contents);

    assert_eq!(positions.of_assignment("A"), 1);
    assert_eq!(positions.of_assignment("B"), 2);
  }

  #[test]
  fn a_quoted_line_shows_what_a_terminal_would_swallow() {
    assert_eq!(printable("\u{feff}B=2"), "\\u{feff}B=2");
    assert_eq!(printable("plain=value"), "plain=value");
  }
}
