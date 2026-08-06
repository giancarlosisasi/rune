//! Turning an engine stack trace back into positions in the TypeScript the user wrote.
//!
//! Stripping is transform-then-reprint, not an in-place edit, so the engine's line
//! numbers belong to generated JavaScript. A config is code its author is expected to
//! debug, and "roughly line 14" in a file full of template strings is not good enough.
//!
//! The map is built here, on the failure path only. A config that does not throw never
//! pays for it.

use std::path::Path;

use crate::strip::strip_types_with_map;

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
  let open = line.find('(')?;
  let close = line.rfind(')')?;
  let location = line.get(open + 1..close)?;

  // Split from the right: a Windows path carries its own colon in `D:\`.
  let (rest, column) = location.rsplit_once(':')?;
  let (path, row) = rest.rsplit_once(':')?;
  let row: u32 = row.parse().ok()?;
  let column: u32 = column.parse().ok()?;

  let (source_row, source_column) = original_position(Path::new(path), row, column)?;

  Some(format!("{}({path}:{source_row}:{source_column}){}", &line[..open], &line[close + 1..]))
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
