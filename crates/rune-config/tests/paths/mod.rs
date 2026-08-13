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
//!
//! The same rule covers positions inside rune's own bootstrap: a snapshot is here for the
//! sentence, and a line number in a file no user can open is noise that moves whenever
//! that file is edited.

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

/// Frames from rune's own setup keep their shape and lose their position.
///
/// The sentence is what these snapshots exist for. Pinning the line numbers of a file no
/// user can open would make every edit to it look like a change to the message.
pub fn without_internal_positions(text: &str) -> String {
  const FRAME: &str = "eval_script:";

  let mut kept = String::with_capacity(text.len());
  let mut rest = text;

  while let Some(start) = rest.find(FRAME) {
    kept.push_str(&rest[..start]);
    kept.push_str("eval_script");

    let position = &rest[start + FRAME.len()..];
    let end = position.find(|c: char| !c.is_ascii_digit() && c != ':').unwrap_or(position.len());
    rest = &position[end..];
  }
  kept.push_str(rest);

  kept
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

/// The frame keeps its name, so a message that gained or lost one still shows in a
/// snapshot. Only the position — which moves whenever the bootstrap is edited — goes.
#[test]
fn an_internal_frame_keeps_its_name_and_loses_its_position() {
  assert_eq!(
    without_internal_positions("thrown\n    at get (eval_script:83:27)\n    at x (a.ts:1:1)"),
    "thrown\n    at get (eval_script)\n    at x (a.ts:1:1)"
  );
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
