//! Overrides, `--root` and `rune inspect` through the real binary.
//!
//! The unit tests in `rune-config` hold the resolution rules. What only the binary can
//! show is that standing in one package changes the answer, that `--root` changes it
//! back, and that a user can be told which of the two happened.

mod harness;

use harness::{Test, monorepo};

/// The fake tool the fixture's commands call, so a test can read the argv a child
/// actually received rather than the command line rune wrote.
const TOOL: &str = "faketool.exe";

/// The root's shared definition, and the one package that needs an extra flag.
fn shared_test_script() -> String {
  format!(r#"{{ test: {{ command: "{TOOL} report-env", description: "run the unit tests" }} }}"#)
}

const LEGACY_OVERRIDE: &str = r#"{ test: { extends: "test", appendArgs: ["--maxWorkers=1"] } }"#;

/// The fixture with the fake tool installed at the root, ready to report its argv.
fn fixture(root: &str, legacy: &str) -> Test {
  monorepo(root, legacy).tool(&format!("node_modules/.bin/{TOOL}"))
}

fn argv(output: &std::process::Output) -> Vec<String> {
  let report: serde_json::Value =
    serde_json::from_slice(&output.stdout).expect("rune-testkit reports JSON");

  report["argv"]
    .as_array()
    .expect("the report carries an argv")
    .iter()
    .map(|argument| argument.as_str().expect("an argument is a string").to_owned())
    .collect()
}

/// Test 4b.7 — the story the whole change exists for: one package needs one extra flag,
/// and gets it without a `package.json` anywhere being touched.
#[test]
fn a_package_override_appends_its_flag_to_the_root_command() {
  let test = fixture(&shared_test_script(), LEGACY_OVERRIDE)
    .args(["run", "test"])
    .stdout_regex(r"(?s)^\{.*\}\n$")
    .status(0);

  let manifests =
    ["package.json", "packages/legacy/package.json"].map(|relative| test.dir().join(relative));
  let before = manifests.clone().map(|path| std::fs::read(path).expect("read the manifest"));

  let output = test.run_in("packages/legacy");

  assert_eq!(argv(&output), ["report-env", "--maxWorkers=1"]);

  let after = manifests.map(|path| std::fs::read(path).expect("read the manifest"));
  assert_eq!(after, before, "a package.json changed");
}

/// Test 4b.9 — the other half of the same guarantee. A package that overrides nothing
/// must be unaffected by a package that does.
#[test]
fn a_package_without_a_config_runs_the_root_command_unmodified() {
  let output = fixture(&shared_test_script(), LEGACY_OVERRIDE)
    .args(["run", "test"])
    .stdout_regex(r"(?s)^\{.*\}\n$")
    .status(0)
    .run_in("packages/modern");

  assert_eq!(argv(&output), ["report-env"]);
}

/// Test 4b.10 — `--root` resolves as if the run had started at the repository root, from
/// inside the very package that overrides the script.
#[test]
fn the_root_flag_resolves_against_the_root_config_alone() {
  let output = fixture(&shared_test_script(), LEGACY_OVERRIDE)
    .args(["run", "--root", "test"])
    .stdout_regex(r"(?s)^\{.*\}\n$")
    .status(0)
    .run_in("packages/legacy");

  assert_eq!(argv(&output), ["report-env"], "the package's flag must not be added");
}

/// A script only the package defines does not exist at the root, so `--root` says so
/// rather than quietly falling back to the package's own definition.
#[test]
fn the_root_flag_cannot_reach_a_package_only_script() {
  let legacy = format!(
    r#"{{ test: {{ extends: "test", appendArgs: ["--maxWorkers=1"] }},
          codegen: {{ command: "{TOOL} emit generating" }} }}"#
  );

  fixture(&shared_test_script(), &legacy)
    .args(["run", "--root", "codegen"])
    .stdout("")
    .stderr_regex(r"(?s)no script named `codegen`")
    .status(1)
    .run_in("packages/legacy");
}

/// Test 4b.14 — the seam between this change and pass-through arguments.
///
/// Both features are individually correct and their interaction is tested nowhere else:
/// inherited arguments are part of the command the config defines, so they come first,
/// and what the user typed on the command line comes last.
#[test]
fn command_line_arguments_come_after_inherited_ones() {
  let output = fixture(&shared_test_script(), LEGACY_OVERRIDE)
    .args(["run", "test", "--", "--watch", "a b"])
    .stdout_regex(r"(?s)^\{.*\}\n$")
    .status(0)
    .run_in("packages/legacy");

  assert_eq!(argv(&output), ["report-env", "--maxWorkers=1", "--watch", "a b"]);
}

/// The same ordering across a chain that lives entirely in the root config, so the rule
/// is not an accident of where the definitions happened to sit.
#[test]
fn a_three_level_chain_orders_its_arguments_from_the_base_outward() {
  let root = format!(
    r#"{{
      test: {{ command: "{TOOL} report-env" }},
      "test:ci": {{ extends: "test", appendArgs: ["--run"] }},
      "test:ci:dot": {{ extends: "test:ci", appendArgs: ["--reporter=dot"] }}
    }}"#
  );

  let output = fixture(&root, LEGACY_OVERRIDE)
    .args(["run", "test:ci:dot", "--", "--bail=1"])
    .stdout_regex(r"(?s)^\{.*\}\n$")
    .status(0)
    .run();

  assert_eq!(argv(&output), ["report-env", "--run", "--reporter=dot", "--bail=1"]);
}

/// Test 4b.11 — the chain is explained, and nothing runs.
///
/// The probe is concrete on purpose: the fixture's command names a tool that writes a
/// marker file when it executes, and the assertion is that the file does not exist. A
/// test that only checked the output would pass just as happily if `inspect` had spawned
/// the command and thrown the result away.
#[test]
fn inspect_renders_the_chain_and_spawns_nothing() {
  let marker = "inspect-must-not-run.txt";
  let root =
    format!(r#"{{ test: {{ command: "{TOOL} touch {marker}", env: {{ NODE_ENV: "test" }} }} }}"#);

  let test = monorepo(&root, LEGACY_OVERRIDE)
    .tool(&format!("node_modules/.bin/{TOOL}"))
    .args(["inspect", "test"])
    .stdout_regex(r"(?s)resolved through")
    .status(0);

  let output = test.run_in("packages/legacy");

  assert!(
    !test.dir().join("packages/legacy").join(marker).exists() && !test.dir().join(marker).exists(),
    "inspect executed the command it was asked to explain"
  );

  insta::with_settings!({ description => "a script a package overrides by extending it" }, {
    insta::assert_snapshot!(String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n"));
  });
}

/// The plain case: no package config, no chain to speak of. It still names the config the
/// definition came from, because "where did this come from" is the question `inspect`
/// answers even when the answer is short.
#[test]
fn inspect_explains_a_script_that_extends_nothing() {
  let output = Test::new()
    .config("export default { scripts: { build: { command: \"tsc -b\" } } };\n")
    .args(["inspect", "build"])
    .stdout_regex(r"(?s)resolved through")
    .status(0)
    .run();

  insta::with_settings!({ description => "a root script with no inheritance" }, {
    insta::assert_snapshot!(String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n"));
  });
}

/// The third shape in the fixture: a script only the package defines. Its chain names
/// the package config and nothing else, which is what tells a reader that this one does
/// not exist anywhere else in the repository.
#[test]
fn inspect_explains_a_script_the_package_defines_for_itself() {
  let legacy = r#"{
    test: { extends: "test", appendArgs: ["--maxWorkers=1"] },
    codegen: { command: "echo generating", cwd: "src" }
  }"#;

  let output = monorepo(&shared_test_script(), legacy)
    .args(["inspect", "codegen"])
    .stdout_regex(r"(?s)resolved through")
    .status(0)
    .run_in("packages/legacy");

  insta::with_settings!({ description => "a script only the package defines" }, {
    insta::assert_snapshot!(String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n"));
  });
}

/// `--root` changes what `run` would do, so it has to change what `inspect` reports —
/// otherwise the command that exists to explain `run` would explain the wrong run.
#[test]
fn inspect_honors_the_root_flag() {
  let output = fixture(&shared_test_script(), LEGACY_OVERRIDE)
    .args(["inspect", "--root", "test"])
    .stdout_regex(r"(?s)resolved through")
    .status(0)
    .run_in("packages/legacy");

  let report = String::from_utf8_lossy(&output.stdout);
  assert!(!report.contains("--maxWorkers=1"), "the package's addition must be absent:\n{report}");
  assert!(
    !report.contains("packages/legacy/rune.config.ts"),
    "the package's config must not appear in the chain:\n{report}"
  );
}

/// Inspecting a name nothing defines is the third place a script name can be missed, and
/// it gets the same answer as the other two.
#[test]
fn inspect_of_an_unknown_name_suggests_the_closest() {
  Test::new()
    .config("export default { scripts: { build: { command: \"tsc -b\" } } };\n")
    .args(["inspect", "biuld"])
    .stdout("")
    .stderr_regex(r"(?s)no script named `biuld`.*did you mean `build`\?")
    .status(1)
    .run();
}

/// Test 4b.12 and R7.1 — the annotations that stop a developer reading one list and
/// assuming it means the same thing in every directory, in the words of the rule they
/// describe. A package narrows a shared script and can never replace it, so a listing
/// that calls the result an override teaches the model the loader refuses.
#[test]
fn list_annotates_what_the_package_changed() {
  let root = r#"{
    build: { command: "tsc -b", description: "compile the workspace" },
    test: { command: "jest", description: "run the unit tests" }
  }"#;
  let legacy = r#"{
    test: { extends: "test", appendArgs: ["--maxWorkers=1"] },
    codegen: { command: "echo generating", description: "regenerate the client" }
  }"#;

  let output = monorepo(root, legacy)
    .args(["list"])
    .stdout_regex(r"(?s)build")
    .status(0)
    .run_in("packages/legacy");

  let listing = String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n");

  assert!(listing.contains("(narrowed here)"), "{listing}");
  assert!(
    !listing.contains("overrid"),
    "the listing must not call a narrowing an override:\n{listing}"
  );

  insta::with_settings!({ description => "listed from a package that narrows one script and defines one" }, {
    insta::assert_snapshot!(listing);
  });
}

/// The unannotated half of the same row: a directory with no config of its own produces
/// the same bytes as the repository root, which is what makes the annotations meaningful
/// where they do appear.
#[test]
fn list_is_unannotated_where_no_package_config_exists() {
  let root = r#"{
    build: { command: "tsc -b", description: "compile the workspace" },
    test: { command: "jest", description: "run the unit tests" }
  }"#;

  let expected = "
    build  compile the workspace
    test   run the unit tests
    ";

  let test = monorepo(root, LEGACY_OVERRIDE).args(["list"]).stdout(expected).status(0);

  test.run();
  test.run_in("packages/modern");
}
