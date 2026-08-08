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

use std::path::{Path, PathBuf};

/// Replaces every spelling of `dir` with `[TMP]`, then writes what is left with forward
/// slashes so one snapshot serves every platform.
pub fn redact(dir: &Path, text: &str) -> String {
  let mut spellings = vec![dir.to_path_buf()];
  if let Ok(resolved) = dunce::canonicalize(dir) {
    spellings.push(resolved);
  }

  replace_each(&spellings, text)
}

/// Longest spelling first, and that ordering is the whole of the correctness here.
///
/// On macOS one spelling *contains* the other: `/var/folders/x` resolves to
/// `/private/var/folders/x`. Replace the short one first and the long one is left as
/// `/private[TMP]`, which is neither the path nor the placeholder.
fn replace_each(spellings: &[PathBuf], text: &str) -> String {
  let mut forms: Vec<String> = spellings
    .iter()
    .flat_map(|path| {
      let native = path.to_string_lossy().into_owned();
      // A message may carry a path rune assembled itself, with the separators already
      // turned round.
      let slashed = native.replace('\\', "/");
      [native, slashed]
    })
    .collect();

  forms.sort_by(|left, right| right.len().cmp(&left.len()).then_with(|| left.cmp(right)));
  forms.dedup();

  let mut redacted = text.to_owned();
  for form in forms {
    redacted = redacted.replace(&form, "[TMP]");
  }

  redacted.replace('\\', "/")
}

/// The macOS case, written out so the ordering rule cannot be lost again. Both spellings
/// are supplied directly: what the filesystem would resolve to is not reachable from a
/// test that has to give the same answer on every platform.
#[test]
fn a_spelling_that_contains_another_is_still_replaced_whole() {
  let spellings = [
    PathBuf::from("/var/folders/df/T/.tmpAbCd"),
    PathBuf::from("/private/var/folders/df/T/.tmpAbCd"),
  ];

  let redacted = replace_each(
    &spellings,
    "cannot find `./nope` imported from /private/var/folders/df/T/.tmpAbCd/rune.config.ts",
  );

  assert_eq!(redacted, "cannot find `./nope` imported from [TMP]/rune.config.ts");
}

/// The other direction: a message carrying the spelling the test was handed, not the one
/// the filesystem resolved it to.
#[test]
fn either_spelling_on_its_own_is_replaced() {
  let spellings = [
    PathBuf::from("/var/folders/df/T/.tmpAbCd"),
    PathBuf::from("/private/var/folders/df/T/.tmpAbCd"),
  ];

  assert_eq!(
    replace_each(&spellings, "/var/folders/df/T/.tmpAbCd/rune.config.ts"),
    "[TMP]/rune.config.ts"
  );
}
