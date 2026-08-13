//! What a config-load failure says: where it happened, in whose words, and spelled how.
//!
//! One line used to divide every message rune produces. What rune composed explained
//! itself; what it passed on from the parser, the engine or the operating system was that
//! layer talking to somebody who had never heard of it. These rows are that boundary.

mod paths;

use std::fs;
use std::path::Path;

use paths::redact;
use rune_config::env::Environment;
use rune_config::eval::evaluate_config;
use tempfile::TempDir;

fn fixture(files: &[(&str, &str)]) -> TempDir {
  let dir = tempfile::tempdir().expect("create tempdir");
  for (relative, contents) in files {
    let path = dir.path().join(relative);
    fs::create_dir_all(path.parent().expect("a fixture path has a parent"))
      .expect("create parents");
    fs::write(&path, contents).expect("write fixture file");
  }
  dir
}

/// The message loading this fixture's config produces.
fn failure(dir: &TempDir) -> String {
  let error =
    evaluate_config(dir.path(), &dir.path().join("rune.config.ts"), &Environment::default())
      .expect_err("the config must fail to load")
      .to_string();

  redact(dir.path(), &error)
}

/// Ten lines that survive stripping, so a reported line is unambiguous and the mistake
/// below them is nowhere near the top of the file.
const PADDING: &str = "const a1: number = 1;\nconst a2: number = 2;\nconst a3: number = 3;\n\
                       const a4: number = 4;\nconst a5: number = 5;\nconst a6: number = 6;\n\
                       const a7: number = 7;\nconst a8: number = 8;\nconst a9: number = 9;\n\
                       const a10: number = 10;\n";

/// Four imports that stripping erases, which is what moves a generated line away from the
/// line the user wrote.
const ERASED: &str = "import type { A } from './types';\nimport type { B } from './types';\n\
                      import type { C } from './types';\nimport type { D } from './types';\n";

const TYPES: (&str, &str) =
  ("types.ts", "export type A = 1;\nexport type B = 1;\nexport type C = 1;\nexport type D = 1;\n");

/// Whether the text points at a line in a `.ts` file.
fn names_a_line(text: &str) -> bool {
  text.match_indices(".ts:").any(|(at, _)| {
    text[at + ".ts:".len()..].starts_with(|character: char| character.is_ascii_digit())
  })
}

/// Test R23.1 — the brace is on a known line, and `Unexpected token` sends the reader to
/// look for it.
///
/// The parser holds the span and drops it one line after it arrives. `tsc` answers the
/// same file with a line and a column, and a 300-line config is where a person meets this.
#[test]
fn a_syntax_error_names_the_line_and_shows_it() {
  let dir = fixture(&[
    ("scripts/helpers.ts", "export const a = 1;\nexport const b = 2;\nexport const broken = {;\n"),
    ("rune.config.ts", "import { a } from './scripts/helpers';\nexport default { a };\n"),
  ]);

  insta::with_settings!({ description => "a stray brace in an imported helper" }, {
    insta::assert_snapshot!(failure(&dir));
  });
}

/// Test R23.2 — an absolute path spends most of a terminal line on directories the reader
/// is already standing in, and `rune list` has never printed one.
///
/// The oracle is the redaction: anything still absolute under the fixture comes back as
/// the placeholder, whatever spelling the message carried.
#[test]
fn no_config_load_error_carries_an_absolute_path() {
  let kinds: [(&str, &[(&str, &str)]); 6] = [
    (
      "a syntax error",
      &[
        ("scripts/helpers.ts", "export const broken = {;\n"),
        (
          "rune.config.ts",
          "import { broken } from './scripts/helpers';\nexport default { broken };\n",
        ),
      ],
    ),
    (
      "a missing file",
      &[("rune.config.ts", "import { x } from './scripts/nope';\nexport default { x };\n")],
    ),
    (
      "an npm import",
      &[("rune.config.ts", "import { x } from 'lodash';\nexport default { x };\n")],
    ),
    ("a config that throws", &[("rune.config.ts", "throw new Error('boom');\n")]),
    ("a refused global", &[("rune.config.ts", "export default { v: console };\n")]),
    ("a default export that is not an object", &[("rune.config.ts", "export default 42;\n")]),
  ];

  for (kind, files) in kinds {
    let dir = fixture(files);
    let message = failure(&dir);

    assert!(!message.contains("[TMP]"), "{kind} carries an absolute path:\n{message}");
  }
}

/// Test R23.3 — the module and the export are named and the file is not, so the reader
/// opens every file in the graph to find the import.
#[test]
fn a_missing_export_names_the_file_that_wrote_it() {
  let dir = fixture(&[
    ("scripts/helpers.ts", "import { nope } from '@gio-labs/rune';\nexport const value = nope;\n"),
    ("rune.config.ts", "import { value } from './scripts/helpers';\nexport default { value };\n"),
  ]);

  insta::with_settings!({ description => "an export the supplied module does not have" }, {
    insta::assert_snapshot!(failure(&dir));
  });
}

/// Test R23.4 — every one of these is thrown by rune's own bootstrap, so every one of them
/// used to carry a frame naming it. `eval_script` is a file the user did not write and
/// cannot open.
#[test]
fn no_refusal_carries_a_frame_from_runes_own_bootstrap() {
  for name in rune_config::globals::unavailable_names() {
    let dir = fixture(&[("rune.config.ts", &format!("export default {{ value: {name} }};\n"))]);
    let message = failure(&dir);

    assert!(!message.contains("eval_script"), "`{name}` shows rune's insides:\n{message}");
  }
}

/// Test R23.5 — the engine's sentence says exactly what happened and names no file, which
/// is the whole of the message today.
#[test]
fn an_engine_error_is_wrapped_in_runes_own_sentence() {
  let dir = fixture(&[(
    "rune.config.ts",
    &format!(
      "{PADDING}const bad = undefined as unknown as {{ f: string }};\nexport default {{ v: bad.f }};\n"
    ),
  )]);

  insta::with_settings!({ description => "an error the engine raised" }, {
    insta::assert_snapshot!(failure(&dir));
  });
}

/// Test R23.6 — the half that already works, and the half a change teaching rune to
/// withhold positions is most likely to take away.
///
/// The `throw` is on line 15 of the file the user wrote and on line 11 of the JavaScript
/// the engine ran, because the four type-only imports above it are erased.
#[test]
fn a_statement_the_config_threw_from_keeps_its_line() {
  let dir =
    fixture(&[TYPES, ("rune.config.ts", &format!("{ERASED}{PADDING}throw new Error('boom');\n"))]);

  let message = failure(&dir);

  assert!(message.contains("rune.config.ts:15:"), "the thrown line is not reported:\n{message}");
}

/// Test R23.7 — the same file, the same line, an error the engine raises rather than one
/// the config throws.
///
/// It reports the start of the module, which lands on a real line belonging to an innocent
/// statement. A reader acts on a number, so no number is better than that one.
#[test]
fn a_position_the_engine_cannot_place_is_not_printed() {
  let dir = fixture(&[
    TYPES,
    (
      "rune.config.ts",
      &format!(
        "{ERASED}{PADDING}const bad = undefined as unknown as {{ f: string }};\nexport default {{ v: bad.f }};\n"
      ),
    ),
  ]);

  let message = failure(&dir);

  assert!(message.contains("rune.config.ts"), "the file is not named:\n{message}");
  assert!(!names_a_line(&message), "a position was invented:\n{message}");
}

/// Test R23.8 — a config that waits for something nothing will settle. The engine answers
/// with a sentence about its own internals.
#[test]
fn a_promise_that_never_settles_names_the_file() {
  let dir = fixture(&[("rune.config.ts", "export default await new Promise<never>(() => {});\n")]);

  insta::with_settings!({ description => "awaiting a promise that never settles" }, {
    insta::assert_snapshot!(failure(&dir));
  });
}

/// Test R23.9 — a limit of the engine's, reached by a config that asked for something the
/// engine cannot build.
#[test]
fn an_engine_limit_is_reported_in_runes_words() {
  let dir = fixture(&[("rune.config.ts", "export default { v: 'x'.repeat(2 ** 31) };\n")]);

  insta::with_settings!({ description => "a string longer than the engine can build" }, {
    insta::assert_snapshot!(failure(&dir));
  });
}

/// Test R23.10 — reading a `const` above the line that declares it. The engine raises it,
/// so it is also a case where no position may be printed.
#[test]
fn a_use_before_declaration_is_in_runes_words() {
  let dir = fixture(&[(
    "rune.config.ts",
    &format!("{PADDING}export default {{ v: later }};\nconst later: number = 1;\n"),
  )]);

  let message = failure(&dir);

  assert!(message.contains("rune.config.ts"), "the file is not named:\n{message}");
  assert!(
    message.contains("failed while it was being evaluated"),
    "the engine's sentence is the whole message:\n{message}"
  );
  assert!(!names_a_line(&message), "a position was invented:\n{message}");
}

/// Test R23.12 — the file half of a syntax error, which already works and is the half most
/// easily lost. A broken helper sends the user to the helper, however deep it is.
#[test]
fn the_failing_file_is_named_at_every_import_depth() {
  let broken = "export const broken = {;\n";
  let depths: [(&str, &[(&str, &str)]); 3] = [
    ("the config itself", &[("rune.config.ts", broken)]),
    (
      "one import down",
      &[
        ("scripts/helpers.ts", broken),
        (
          "rune.config.ts",
          "import { broken } from './scripts/helpers';\nexport default { broken };\n",
        ),
      ],
    ),
    (
      "two imports down",
      &[
        ("scripts/inner.ts", broken),
        ("scripts/helpers.ts", "export { broken } from './inner';\n"),
        (
          "rune.config.ts",
          "import { broken } from './scripts/helpers';\nexport default { broken };\n",
        ),
      ],
    ),
  ];

  let expected = ["rune.config.ts:", "scripts/helpers.ts:", "scripts/inner.ts:"];

  for ((depth, files), file) in depths.into_iter().zip(expected) {
    let dir = fixture(files);
    let message = failure(&dir);

    assert!(message.starts_with(file), "{depth} names the wrong file:\n{message}");
  }
}

/// Test R23.13 — a specifier that resolves to nothing, written two imports below the
/// config. Naming the file it was written in is half the answer; the other half is how a
/// reader gets to that file from the one they know about.
#[test]
fn a_failed_resolution_names_the_chain_that_reached_it() {
  let dir = fixture(&[
    ("scripts/inner.ts", "import { x } from './nowhere';\nexport const inner = x;\n"),
    ("scripts/helpers.ts", "export { inner } from './inner';\n"),
    ("rune.config.ts", "import { inner } from './scripts/helpers';\nexport default { inner };\n"),
  ]);

  insta::with_settings!({ description => "a specifier that resolves to nothing" }, {
    insta::assert_snapshot!(failure(&dir));
  });
}

/// Test R23.14 — a refusal rune raises about the file as a whole points at no span, and a
/// position invented for it would point at nothing.
#[test]
fn a_refusal_with_no_span_keeps_its_shape() {
  let dir = fixture(&[
    ("h.ts", "export const value = 1;\n"),
    ("rune.config.ts", "const m = import('./h');\nexport default { m };\n"),
  ]);

  let message = failure(&dir);

  assert!(message.starts_with("rune.config.ts:\n"), "the file is not named:\n{message}");
  assert!(message.contains("stale"), "the reason is gone:\n{message}");
  assert!(!names_a_line(&message), "a position was invented:\n{message}");
}

/// Test R24.7 — a config that exports the work rather than what the work produced.
///
/// A promise is an object to everything downstream, so it becomes an empty config and is
/// reported as an object. "A promise" is the one word that leads a reader to the `await`
/// they left out.
#[test]
fn a_default_export_that_is_a_promise_says_so() {
  let dir = fixture(&[("rune.config.ts", "export default Promise.resolve({ scripts: {} });\n")]);

  insta::with_settings!({ description => "a config exporting a promise of its scripts" }, {
    insta::assert_snapshot!(failure(&dir));
  });
}

/// Test R24.9 — the sibling message, which is right already and sits one branch away from
/// everything the promise case changes.
#[test]
fn a_default_export_that_is_not_an_object_keeps_its_message() {
  for (kind, config) in
    [("no default export", "export const x = 1;\n"), ("a number", "export default 42;\n")]
  {
    let dir = fixture(&[("rune.config.ts", config)]);
    let message = failure(&dir);

    assert!(message.contains("must default-export an object"), "{kind}: {message}");
    assert!(message.contains("dev: { command: \"vite\" }"), "{kind} lost the example: {message}");
  }
}

/// The redaction has to be able to fail, or every row above it passes by not looking.
#[test]
fn the_absolute_path_oracle_can_fail() {
  let dir = tempfile::tempdir().expect("create tempdir");
  let absolute = dir.path().join("rune.config.ts");

  assert!(redact(dir.path(), &absolute.to_string_lossy()).contains("[TMP]"));
  assert!(!redact(dir.path(), "rune.config.ts").contains("[TMP]"));
  assert!(Path::new(&absolute).is_absolute());
}
