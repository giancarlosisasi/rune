//! Turning an engine stack trace back into positions in the TypeScript the user wrote.
//!
//! Stripping is transform-then-reprint, not an in-place edit, so the engine's line
//! numbers belong to generated JavaScript. A config is code its author is expected to
//! debug, and "roughly line 14" in a file full of template strings is not good enough.
//!
//! The map is built here, on the failure path only. A config that does not throw never
//! pays for it.

use std::ops::Range;
use std::path::Path;

use crate::paths::relative_to;
use crate::strip::strip_types_with_map;

/// Where one `at name (file:line:column)` frame points.
pub struct Frame<'a> {
  pub path: &'a str,
  pub row: u32,
  pub column: u32,
  /// Where `file:line:column` sits inside the frame, for rewriting it in place.
  position: Range<usize>,
}

/// Reads a stack frame, or nothing for a line that is not one.
///
/// The one place that knows the shape of a frame. Every reader of a trace comes through
/// here, so a second reader cannot come to a different answer about the same line.
pub fn frame(line: &str) -> Option<Frame<'_>> {
  let open = line.find('(')?;
  let close = line.rfind(')')?;
  let location = line.get(open + 1..close)?;

  // Split from the right: a Windows path carries its own colon in `D:\`.
  let (rest, column) = location.rsplit_once(':')?;
  let (path, row) = rest.rsplit_once(':')?;

  Some(Frame {
    path,
    row: row.parse().ok()?,
    column: column.parse().ok()?,
    position: open + 1..close,
  })
}

/// The position every module starts at, which the engine reports for an error it raised
/// itself rather than one the config threw.
///
/// Measured across five error shapes: a property read on undefined, a use before
/// declaration and a missing global all report exactly this, whatever line the mistake is
/// on, while a `throw` reports its own position — column 10 even when it is the first
/// statement of the file. So no statement a user can write occupies this position, and a
/// frame carrying it is the engine saying nothing rather than saying line one.
const MODULE_START: (u32, u32) = (1, 1);

/// Rewrites every `at name (file:line:col)` frame to the original `.ts` position, and
/// drops the frames that point at nothing the user can act on.
///
/// A frame that cannot be remapped keeps its position. A half-translated trace is still
/// more useful than none, and guessing would be worse than both.
pub fn remap(trace: &str, root: &Path) -> String {
  trace.lines().filter_map(|line| rewrite(line, root)).collect::<Vec<_>>().join("\n")
}

/// One line of a trace, rewritten for the reader. `None` drops it.
fn rewrite(line: &str, root: &Path) -> Option<String> {
  let Some(frame) = frame(line) else {
    return Some(line.to_owned());
  };

  // Rune's own bootstrap, and the position the engine gives when it will not say where.
  if frame.path == crate::globals::BOOTSTRAP_NAME || (frame.row, frame.column) == MODULE_START {
    return None;
  }

  let path = Path::new(frame.path);
  let (row, column) =
    original_position(path, frame.row, frame.column).unwrap_or((frame.row, frame.column));

  let shown = relative_to(root, path);
  let position = frame.position;
  Some(format!("{}{shown}:{row}:{column}{}", &line[..position.start], &line[position.end..]))
}

/// Looks a generated position up in a freshly built map for `path`.
///
/// Codegen is deterministic, so re-stripping the file now reproduces exactly the text
/// the engine ran, and the map is valid for it.
fn original_position(path: &Path, row: u32, column: u32) -> Option<(u32, u32)> {
  let source = std::fs::read_to_string(path).ok()?;
  let (_, map) = strip_types_with_map(&source, path).ok()?;
  let lookup = map.generate_lookup_table();

  // The engine counts from one and the map counts from zero.
  let token = map
    .lookup_token(&lookup, row.checked_sub(1)?, column.saturating_sub(1))
    .or_else(|| map.lookup_token(&lookup, row.checked_sub(1)?, 0))?;

  Some((token.get_src_line() + 1, token.get_src_col() + 1))
}

#[cfg(test)]
mod tests {
  use std::path::{Path, PathBuf};

  use super::remap;

  fn root() -> PathBuf {
    PathBuf::from(if cfg!(windows) { r"C:\repo" } else { "/repo" })
  }

  fn frame_for(name: &str, position: &str) -> String {
    format!("    at <anonymous> ({}:{position})", root().join(name).display())
  }

  /// A frame naming a file that is not there keeps its position: there is nothing to
  /// remap it against, and a guess would be worse than the untranslated number.
  #[test]
  fn a_frame_is_written_from_the_repository_root() {
    let trace = frame_for("scripts/helpers.ts", "7:3");

    assert_eq!(remap(&trace, &root()), "    at <anonymous> (scripts/helpers.ts:7:3)");
  }

  /// The bootstrap is rune's, not the user's, and it appears under sentences rune wrote.
  #[test]
  fn a_frame_from_runes_own_bootstrap_is_dropped() {
    let trace = format!("    at get (eval_script:83:27)\n{}", frame_for("rune.config.ts", "4:9"));

    assert_eq!(remap(&trace, &root()), "    at <anonymous> (rune.config.ts:4:9)");
  }

  /// The engine reports the start of the module when it will not say where. That is a
  /// real line belonging to an innocent statement, and a reader acts on a number.
  #[test]
  fn a_frame_at_the_start_of_the_module_is_dropped() {
    assert_eq!(remap(&frame_for("rune.config.ts", "1:1"), &root()), "");
  }

  /// A line that is not a frame is not rune's to rewrite.
  #[test]
  fn a_line_that_is_not_a_frame_is_left_alone() {
    let trace = "    at repeat (native)";

    assert_eq!(remap(trace, Path::new("/repo")), trace);
  }
}
