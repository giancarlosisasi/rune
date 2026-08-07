//! Reading the dotenv files a config declares.
//!
//! Parsing is `dotenvy`'s job: quoting, escapes, comments, `export` prefixes and values
//! that span lines are solved problems with edge cases nobody wants to rediscover. What
//! stays here is everything about *which* file — a path means what it means relative to
//! the config that wrote it, and a file that is named but absent is a broken config
//! rather than a degraded one.
//!
//! The iterator API is the one rune can use. `dotenvy::from_path` loads a file straight
//! into rune's own process environment, which is the opposite of what a runner wants:
//! every assignment has to be inspected before anything decides whether it applies.

use std::collections::BTreeMap;
use std::path::Path;

use thiserror::Error;

use crate::paths::relative_to;
use crate::resolve::lexically_normalize;
use crate::schema::Config;

/// One dotenv file, read once for the config that declared it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvFile {
  /// The file, spelled the way rune spells every path it prints. Warnings name it, so it
  /// is a label rather than a path: an absolute one would bury the part that identifies
  /// the file and would differ between machines.
  pub source: String,
  /// Every assignment, in the order the file wrote them.
  pub assignments: Vec<(String, String)>,
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
    "{path} line {line} is not an assignment\n\n  {text}\n\n\
     every line of an env file is `NAME=value`, a `#` comment, or empty."
  )]
  Parse { path: String, line: usize, text: String },
}

/// Reads every file the scripts in `config` declare, keyed by the path as written.
///
/// Two scripts naming the same path share one entry, so a file shared by ten scripts is
/// still read once.
pub fn read_all(
  root: &Path,
  config_path: &Path,
  config: &Config,
) -> Result<BTreeMap<String, EnvFile>, EnvFileError> {
  let mut files = BTreeMap::new();

  for (script, entry) in &config.scripts {
    let Some(declared) = entry.env_file.as_deref() else {
      continue;
    };
    if files.contains_key(declared) {
      continue;
    }

    files.insert(declared.to_owned(), read(root, config_path, script, declared)?);
  }

  Ok(files)
}

fn read(
  root: &Path,
  config_path: &Path,
  script: &str,
  declared: &str,
) -> Result<EnvFile, EnvFileError> {
  let directory = config_path.parent().unwrap_or_else(|| Path::new("."));
  let source = relative_to(root, &lexically_normalize(&directory.join(declared)));
  let config = relative_to(root, config_path);

  let contents = match std::fs::read_to_string(directory.join(declared)) {
    Ok(contents) => contents,
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

  let mut assignments = Vec::new();
  for item in dotenvy::Iter::new(contents.as_bytes()) {
    match item {
      Ok(assignment) => assignments.push(assignment),
      Err(dotenvy::Error::LineParse(reported, _)) => {
        let text = first_line(&reported);
        return Err(EnvFileError::Parse { path: source, line: line_of(&contents, &text), text });
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

/// Where in the file the rejected assignment starts, counting from one.
///
/// `dotenvy` reports the text it could not parse and an offset inside that text, never
/// the position of the text in the file. The text is what finds the position: a value
/// spanning several lines is reported whole, so its first line is where it began, and
/// a line cut short at a `#` comment still starts the physical line it came from.
fn line_of(contents: &str, reported: &str) -> usize {
  contents
    .lines()
    .position(|line| line.trim_end().starts_with(reported))
    .map_or(1, |index| index + 1)
}

fn first_line(reported: &str) -> String {
  reported.lines().next().unwrap_or_default().trim_end().to_owned()
}

#[cfg(test)]
mod tests {
  use super::{first_line, line_of};

  #[test]
  fn a_rejected_line_is_numbered_from_one() {
    let contents = "A=1\nB=2\nnot an assignment\n";

    assert_eq!(line_of(contents, "not an assignment"), 3);
    assert_eq!(line_of(contents, "A=1"), 1);
  }

  /// A value that spans lines is reported whole, and the line to name is the one the
  /// assignment started on rather than wherever the file ran out.
  #[test]
  fn a_multi_line_value_is_numbered_where_it_opened() {
    let contents = "A=1\nB=\"unterminated\nstill going\n";

    assert_eq!(line_of(contents, &first_line("B=\"unterminated\nstill going\n")), 2);
  }

  /// `dotenvy` cuts a line short at a `#`, so the reported text is a prefix of the line
  /// that is actually in the file.
  #[test]
  fn a_line_cut_short_at_a_comment_still_finds_its_place() {
    let contents = "A=1\nnot an assignment # explained\n";

    assert_eq!(line_of(contents, "not an assignment"), 2);
  }
}
