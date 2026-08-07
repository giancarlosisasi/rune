//! `rune init` through the real binary.
//!
//! Every test that generates a config loads it again with the real loader. A generated
//! file that reads beautifully and does not parse is the one failure this command can
//! produce on its own, and a snapshot cannot tell the two apart.

mod harness;

use harness::Test;

const CONFIG_FILE: &str = "rune.config.ts";

const PACKAGE_JSON: &str = r#"
    {
      "name": "fixture",
      "scripts": {
        "build": "tsc -b",
        "test:ci": "vitest run --reporter=dot",
        "lint": "eslint \"src/**/*.ts\""
      }
    }
"#;

fn stdout_of(output: &std::process::Output) -> String {
  String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n")
}

/// Every script the generated config defines, as the loader itself reports them.
fn loaded_scripts(test: &Test) -> String {
  let listed = test.then_run(&["list"]);

  assert!(
    listed.status.success(),
    "the generated config did not load:\n{}",
    String::from_utf8_lossy(&listed.stderr)
  );

  stdout_of(&listed)
}

/// Test 4d.1 — the round trip: written, then read back by the pipeline that has to
/// accept it. The starter is also snapshotted, because it is the first rune config most
/// users read and a change to it should be looked at rather than noticed later.
#[test]
fn a_fresh_init_writes_a_config_the_loader_accepts() {
  let test = Test::new().args(["init"]).stdout("").stderr_regex(r"rune\.config\.ts").status(0);

  test.run();

  let generated =
    std::fs::read_to_string(test.dir().join(CONFIG_FILE)).expect("init wrote a config");

  insta::with_settings!({ description => "the starter config, written into an empty directory" }, {
    insta::assert_snapshot!(generated);
  });

  assert!(loaded_scripts(&test).contains("hello"), "the starter's own script must be listed");
}

/// The loop this command closes: with no config anywhere, rune already told users to run
/// `init`. Until now it named a command that did not exist.
#[test]
fn the_command_the_missing_config_error_suggests_is_the_one_that_fixes_it() {
  let test = Test::new()
    .args(["list"])
    .stdout("")
    .stderr_regex(r"(?s)no rune\.config\.ts found.*rune init")
    .status(1);

  test.run();

  let created = test.then_run(&["init"]);
  assert!(created.status.success(), "the suggested command failed");

  assert!(!loaded_scripts(&test).is_empty(), "listing after init must find something");
}

/// Test 4d.2 — refusing is not enough. An implementation that truncates the file and
/// then fails would pass an exit-code assertion, so the bytes are compared instead.
#[test]
fn init_refuses_to_overwrite_and_leaves_the_file_untouched() {
  let test = Test::new()
    .config("export default { scripts: { mine: { command: \"vitest\" } } };\n")
    .args(["init"])
    .stdout("")
    .stderr_regex(r"(?s)already exists")
    .status(1);

  let path = test.dir().join(CONFIG_FILE);
  let before = std::fs::read(&path).expect("read the hand-written config");

  test.run();

  let after = std::fs::read(&path).expect("read the config back");
  assert_eq!(before, after, "init changed a config it had no permission to touch");
}

/// Test 4d.3 — the mechanical translation, snapshotted and then loaded. `test:ci` is not
/// an identifier and `lint` carries quotes: both have to survive into valid TypeScript.
#[test]
fn seeding_turns_every_npm_script_into_a_command_script() {
  let test = Test::new()
    .file("package.json", PACKAGE_JSON)
    .args(["init", "--from-package-json"])
    .stdout("")
    .stderr_regex(r"rune\.config\.ts")
    .status(0);

  test.run();

  let generated =
    std::fs::read_to_string(test.dir().join(CONFIG_FILE)).expect("init wrote a config");

  insta::with_settings!({ description => "three npm scripts: a plain one, a name that is not an identifier, and a command carrying quotes" }, {
    insta::assert_snapshot!(generated);
  });

  let listed = loaded_scripts(&test);
  for name in ["build", "lint", "test:ci"] {
    assert!(listed.contains(name), "`{name}` did not survive into the loaded config:\n{listed}");
  }
}

/// Test 4d.4 — the flag with nothing to seed from. The message names where it looked,
/// like the discovery error it is modelled on, and nothing is left behind.
#[test]
fn seeding_without_a_package_json_names_the_directory_it_searched() {
  let test = Test::new()
    .args(["init", "--from-package-json"])
    .stdout("")
    .stderr_regex(r"(?s)no package\.json found.*searched upward from")
    .status(1);

  let output = test.run();

  // The leaf rather than the whole path: a temporary directory reaches the child through
  // the operating system's own idea of where it is, which on macOS gains a `/private`.
  let leaf = test.dir().file_name().expect("a temporary directory has a name").to_string_lossy();
  let stderr = String::from_utf8_lossy(&output.stderr);

  assert!(stderr.contains(leaf.as_ref()), "the message must name where it looked:\n{stderr}");
  assert!(!test.dir().join(CONFIG_FILE).exists(), "a failed seeding left a config behind");
}
