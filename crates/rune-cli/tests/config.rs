//! `rune list` and `rune cache clear` through the real binary.
//!
//! Every fixture here writes a `.git` marker. Without it the upward walk escapes the
//! temporary directory and finds whatever config happens to sit above it on the machine
//! running the test, and the result stops being the same for everyone.

mod harness;

use harness::Test;

const CONFIG: &str = r#"
    const shared: string = "vitest";

    export default {
      scripts: {
        dev: { command: "vite", description: "start the dev server" },
        test: { command: shared, description: "run the unit tests" },
        build: { command: "tsc -b" },
      },
    };
"#;

/// Test 2.23 — a snapshot, because the exact text is the deliverable.
#[test]
fn list_prints_sorted_scripts_with_descriptions() {
  Test::new()
    .file(".git/HEAD", "ref: refs/heads/main\n")
    .config(CONFIG)
    .args(["list"])
    .stdout(
      "
      build
      dev    start the dev server
      test   run the unit tests
      ",
    )
    .status(0)
    .run();
}

/// The same row's second half: byte-identical from a nested package directory.
#[test]
fn list_is_identical_from_a_nested_directory() {
  Test::new()
    .file(".git/HEAD", "ref: refs/heads/main\n")
    .config(CONFIG)
    .file("packages/foo/package.json", "{}\n")
    .args(["list"])
    .stdout(
      "
      build
      dev    start the dev server
      test   run the unit tests
      ",
    )
    .status(0)
    .run_in("packages/foo");
}

/// Test 2.22 — the walk must stop at the boundary and say where it looked.
#[test]
fn no_config_anywhere_names_the_directory_and_suggests_init() {
  Test::new()
    .file(".git/HEAD", "ref: refs/heads/main\n")
    .args(["list"])
    .stdout("")
    .stderr_regex(r"(?s)no rune\.config\.ts found.*rune init")
    .status(1)
    .run();
}

/// Test 2.15 — on any load failure stdout stays empty. Once `run` spawns children,
/// stdout belongs to them, and a diagnostic printed there would be indistinguishable
/// from a script's own output.
#[test]
fn a_broken_config_reports_on_stderr_and_leaves_stdout_empty() {
  Test::new()
    .file(".git/HEAD", "ref: refs/heads/main\n")
    .config("export default { scripts: { test: { comand: \"vitest\" } } };\n")
    .args(["list"])
    .stdout("")
    .stderr_regex(r"(?s)`test`.*`comand`")
    .status(1)
    .run();
}

#[test]
fn a_config_that_throws_reports_on_stderr_and_leaves_stdout_empty() {
  Test::new()
    .file(".git/HEAD", "ref: refs/heads/main\n")
    .config("throw new Error('config exploded');\n")
    .args(["list"])
    .stdout("")
    .stderr_regex(r"config exploded")
    .status(1)
    .run();
}

/// Test 2.24 — removing the directory, and succeeding when there is nothing to remove.
#[test]
fn cache_clear_removes_the_directory_and_is_idempotent() {
  let test = Test::new().file(".git/HEAD", "ref: refs/heads/main\n").config(CONFIG);
  let root = test.dir().to_path_buf();

  // Populate the cache by loading once.
  Test::new()
    .file(".git/HEAD", "ref: refs/heads/main\n")
    .config(CONFIG)
    .args(["list"])
    .stdout_regex(r"(?s)build")
    .status(0)
    .run();

  test.args(["cache", "clear"]).stdout("").stderr("").status(0).run();
  assert!(!root.join("node_modules/.cache/rune").exists(), "the cache directory survived");

  Test::new()
    .file(".git/HEAD", "ref: refs/heads/main\n")
    .config(CONFIG)
    .args(["cache", "clear"])
    .stdout("")
    .stderr("")
    .status(0)
    .run();
}
