//! Keeping the machine that ran the test out of the snapshot.
//!
//! A temporary directory has more than one spelling. macOS hands one out as
//! `/var/folders/…` and resolves it to `/private/var/folders/…`. A Windows runner's
//! `TEMP` is the 8.3 short name `C:\Users\RUNNER~1\…`, while canonicalizing it writes the
//! account name out in full. Whichever spelling a message carries depends on who built
//! the path, so a redaction that knows only one of them passes on the machine that wrote
//! the snapshot and fails on every other one.
//!
//! This lived as four near-identical copies until all four failed on macOS at once.

use std::path::Path;

/// Replaces every spelling of `dir` with `[TMP]`, then writes what is left with forward
/// slashes so one snapshot serves every platform.
pub fn redact(dir: &Path, text: &str) -> String {
  let mut spellings = vec![dir.to_path_buf()];
  if let Ok(resolved) = dunce::canonicalize(dir) {
    spellings.push(resolved);
  }

  let mut redacted = text.to_owned();
  for spelling in spellings {
    let native = spelling.to_string_lossy().into_owned();

    redacted = redacted.replace(&native, "[TMP]");
    // A message may carry a path rune assembled itself, with the separators already
    // turned round.
    redacted = redacted.replace(&native.replace('\\', "/"), "[TMP]");
  }

  redacted.replace('\\', "/")
}
