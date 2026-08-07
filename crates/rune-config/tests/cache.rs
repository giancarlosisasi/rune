//! The cache: proven read without any instrumentation in the shipping code, proven
//! content-addressed rather than mtime-based, and proven harmless when it cannot work.

use std::fs;
use std::path::{Path, PathBuf};

use rune_config::env::{Environment, PLATFORM};
use rune_config::inherit::Scope;
use rune_config::load::load;
use tempfile::TempDir;

const HELPER: &str = "export const COMMAND: string = 'vitest --run';\n";
const CONFIG: &str = "import { COMMAND } from './helper';\n\
                      export default { scripts: { test: { command: COMMAND } } };\n";

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

fn repo() -> TempDir {
  fixture(&[
    (".git/HEAD", "ref: refs/heads/main\n"),
    ("helper.ts", HELPER),
    ("rune.config.ts", CONFIG),
  ])
}

fn command_of(dir: &Path) -> String {
  command_with(dir, &Environment::default())
}

fn command_with(dir: &Path, environment: &Environment) -> String {
  let loaded = load(dir, environment).expect("config loads");
  let resolved =
    loaded.resolve("test", Scope::Nearest).expect("resolves").expect("`test` is defined");

  resolved.command.select(PLATFORM).to_owned()
}

/// What `test` resolves to from `dir`: the base command, and everything appended to it.
fn resolution(dir: &Path) -> (String, Vec<String>) {
  let loaded = load(dir, &Environment::default()).expect("config loads");
  let resolved =
    loaded.resolve("test", Scope::Nearest).expect("resolves").expect("`test` is defined");

  (resolved.command.select(PLATFORM).to_owned(), resolved.append_args.clone())
}

fn cache_dir(dir: &Path) -> PathBuf {
  dir.join("node_modules").join(".cache").join("rune")
}

fn cache_entries(dir: &Path) -> Vec<PathBuf> {
  let Ok(entries) = fs::read_dir(cache_dir(dir)) else {
    return Vec::new();
  };
  entries.filter_map(|entry| entry.ok().map(|entry| entry.path())).collect()
}

/// Replaces the resolved command inside the stored entry with a value no evaluation
/// could ever produce. If it comes back, the cache was read.
fn doctor(entry: &Path, sentinel: &str) {
  let stored = fs::read_to_string(entry).expect("read cache entry");
  let doctored = stored.replace("vitest --run", sentinel);
  assert_ne!(doctored, stored, "the cache entry did not contain the resolved command");
  fs::write(entry, doctored).expect("write doctored cache entry");
}

/// Test 2.16 — all three parts of the row, in the order that makes each one mean
/// something. No production code grew a test hook for this.
#[test]
fn the_cache_is_read_and_content_addressed() {
  const SENTINEL: &str = "SENTINEL-NOT-FROM-EVALUATION";

  let dir = repo();
  assert_eq!(command_of(dir.path()), "vitest --run");

  let entries = cache_entries(dir.path());
  assert_eq!(entries.len(), 1, "expected exactly one cache entry, got {entries:?}");
  let entry = &entries[0];

  // (a) the doctored value comes back, so the second run did not evaluate anything.
  doctor(entry, SENTINEL);
  assert_eq!(command_of(dir.path()), SENTINEL);

  // (b) editing an imported helper is a miss, so the real value returns.
  let original = fs::read(dir.path().join("helper.ts")).expect("read helper");
  fs::write(dir.path().join("helper.ts"), "export const COMMAND: string = 'jest';\n")
    .expect("edit helper");
  assert_eq!(command_of(dir.path()), "jest");

  // (c) restoring the exact original bytes reuses the *original* entry. An mtime-based
  // cache would have missed here — this is the assertion content addressing exists for.
  fs::write(dir.path().join("helper.ts"), &original).expect("restore helper");
  assert_eq!(command_of(dir.path()), SENTINEL);
}

/// Test 2.17 — a corrupt entry is a miss, never a user-facing error.
#[test]
fn a_corrupt_cache_entry_re_evaluates_silently() {
  let dir = repo();
  assert_eq!(command_of(dir.path()), "vitest --run");

  let entries = cache_entries(dir.path());
  fs::write(&entries[0], "{ this is not json").expect("corrupt the entry");

  assert_eq!(command_of(dir.path()), "vitest --run");
}

/// Test 2.18 — the portable oracle for "cannot write here": a file where the directory
/// belongs fails identically on Windows and POSIX, with no ACL or chmod branching.
#[test]
fn an_unwritable_cache_location_still_loads() {
  let dir = repo();
  let cache = cache_dir(dir.path());
  fs::create_dir_all(cache.parent().expect("has a parent")).expect("create .cache");
  fs::write(&cache, "a file where the directory belongs").expect("block the cache path");

  assert_eq!(command_of(dir.path()), "vitest --run");
  assert!(cache.is_file(), "the blocking file was replaced");
}

/// Test 2.19's second half: every specifier form must invalidate when its own target is
/// edited. A resolver used by the loader but not by the hasher would pass the first
/// assertion in this file and fail these.
#[test]
fn every_specifier_form_invalidates_its_own_target() {
  for (specifier, target) in [
    ("./h", "h.ts"),
    ("./h.ts", "h.ts"),
    ("./dir/index.ts", "dir/index.ts"),
    ("../outer", "outer.ts"),
  ] {
    let dir = fixture(&[
      (".git/HEAD", "ref: refs/heads/main\n"),
      ("outer.ts", "export const VALUE: string = 'first';\n"),
      ("nested/h.ts", "export const VALUE: string = 'first';\n"),
      ("nested/dir/index.ts", "export const VALUE: string = 'first';\n"),
      (
        "nested/rune.config.ts",
        &format!(
          "import {{ VALUE }} from '{specifier}';\n\
           export default {{ scripts: {{ test: {{ command: VALUE }} }} }};\n"
        ),
      ),
    ]);
    let start = dir.path().join("nested");
    let target = if target == "outer.ts" {
      dir.path().join(target)
    } else {
      dir.path().join("nested").join(target)
    };

    assert_eq!(command_of(&start), "first", "`{specifier}` did not resolve");

    fs::write(&target, "export const VALUE: string = 'second';\n").expect("edit the target");
    assert_eq!(command_of(&start), "second", "`{specifier}` served a stale cached result");
  }
}

/// A config that branches on `rune.isCI` must not be served a laptop result in CI.
///
/// `isCI` is derived from an environment variable, so it has to be recorded as a read
/// like any other. Baking the boolean in at setup time made this exact case return the
/// wrong command from cache.
#[test]
fn is_ci_invalidates_the_cache_like_any_other_variable() {
  let dir = fixture(&[
    (
      ".git/HEAD",
      "ref: refs/heads/main
",
    ),
    (
      "rune.config.ts",
      "export default { scripts: { test: { command: rune.isCI ? 'vitest --run' : 'vitest' } } };
",
    ),
  ]);

  let laptop = Environment::default();
  let ci = Environment::from_pairs([("CI", "1")]);

  assert_eq!(command_with(dir.path(), &laptop), "vitest");
  assert_eq!(command_with(dir.path(), &ci), "vitest --run");
  assert_eq!(command_with(dir.path(), &laptop), "vitest");
}

/// Test 4b.13 — one key over both import closures.
///
/// A key covering only the root config is the worst failure available to this project:
/// a package edits its own config, the next run reports success, and the command it ran
/// came from the definition the package had already replaced. All four files are edited
/// here — both configs and both of their imports — because the closure is what the key
/// covers, not the entry file.
#[test]
fn either_configs_closure_invalidates_the_entry() {
  let dir = fixture(&[
    (".git/HEAD", "ref: refs/heads/main\n"),
    ("root-helper.ts", "export const COMMAND: string = 'jest';\n"),
    (
      "rune.config.ts",
      "import { COMMAND } from './root-helper';\n\
       export default { scripts: { test: { command: COMMAND } } };\n",
    ),
    ("packages/legacy/package.json", "{ \"name\": \"legacy\" }\n"),
    ("packages/legacy/flag.ts", "export const FLAG: string = '--maxWorkers=1';\n"),
    (
      "packages/legacy/rune.config.ts",
      "import { FLAG } from './flag';\n\
       export default { scripts: { test: { extends: 'test', appendArgs: [FLAG] } } };\n",
    ),
  ]);
  let start = dir.path().join("packages/legacy");

  assert_eq!(resolution(&start), ("jest".to_owned(), vec!["--maxWorkers=1".to_owned()]));

  let cases = [
    ("root-helper.ts", "export const COMMAND: string = 'vitest';\n"),
    ("packages/legacy/flag.ts", "export const FLAG: string = '--maxWorkers=2';\n"),
    (
      "rune.config.ts",
      "import { COMMAND } from './root-helper';\n\
       export default { scripts: { test: { command: COMMAND + ' --run' } } };\n",
    ),
    (
      "packages/legacy/rune.config.ts",
      "import { FLAG } from './flag';\n\
       export default { scripts: { test: { extends: 'test', appendArgs: [FLAG, '--bail'] } } };\n",
    ),
  ];

  let expected = [
    ("vitest".to_owned(), vec!["--maxWorkers=1".to_owned()]),
    ("vitest".to_owned(), vec!["--maxWorkers=2".to_owned()]),
    ("vitest --run".to_owned(), vec!["--maxWorkers=2".to_owned()]),
    ("vitest --run".to_owned(), vec!["--maxWorkers=2".to_owned(), "--bail".to_owned()]),
  ];

  for ((relative, contents), expected) in cases.into_iter().zip(expected) {
    fs::write(dir.path().join(relative), contents).expect("edit the fixture file");

    assert_eq!(resolution(&start), expected, "editing `{relative}` served a stale result");
  }
}

/// A config that never mentions `isCI` must not be invalidated by `CI` changing.
#[test]
fn an_unread_variable_does_not_invalidate() {
  let dir = repo();

  assert_eq!(command_with(dir.path(), &Environment::default()), "vitest --run");
  let entries = cache_entries(dir.path());
  doctor(&entries[0], "SENTINEL");

  assert_eq!(command_with(dir.path(), &Environment::from_pairs([("CI", "1")])), "SENTINEL");
}
