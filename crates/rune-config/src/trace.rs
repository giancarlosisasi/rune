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

/// Rewrites every `at name (file:line:col)` frame to the original `.ts` position.
///
/// A frame that cannot be remapped is left exactly as it was. A half-translated trace
/// is still more useful than none, and guessing would be worse than both.
pub fn remap(trace: &str) -> String {
  let mut remapped = String::with_capacity(trace.len());

  for (index, line) in trace.lines().enumerate() {
    if index > 0 {
      remapped.push('\n');
    }
    remapped.push_str(&remap_frame(line).unwrap_or_else(|| line.to_owned()));
  }

  if trace.ends_with('\n') {
    remapped.push('\n');
  }
  remapped
}

fn remap_frame(line: &str) -> Option<String> {
  let frame = frame(line)?;
  let (row, column) = original_position(Path::new(frame.path), frame.row, frame.column)?;

  let position = frame.position;
  Some(format!("{}{}:{row}:{column}{}", &line[..position.start], frame.path, &line[position.end..]))
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
