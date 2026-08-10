//! `rune init` through the real binary.
//!
//! Every test that generates a config loads it again with the real loader. A generated
//! file that reads beautifully and does not parse is the one failure this command can
//! produce on its own, and a snapshot cannot tell the two apart.

mod harness;

use harness::{Test, assert_same_path};

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

/// The manifest measured in the field: a repository part-way through adopting rune, where
/// every npm script has already been rewritten to call it. Sixteen name themselves, and
/// four name something else — three of those four targets live in a rune config the seeder
/// never reads, because it reads `package.json` only.
const ADOPTING_PACKAGE_JSON: &str = r#"
    {
      "name": "fixture",
      "scripts": {
        "build": "rune run build",
        "build:apps": "rune run build:apps",
        "build:packages": "rune run build:packages",
        "check": "rune run check",
        "ci": "rune run ci",
        "clean": "rune run clean:all",
        "dev": "rune run dev",
        "dev:api": "rune run dev:api",
        "dev:packages": "rune run dev:packages",
        "dev:web": "rune run dev:web",
        "format": "rune run format:fix",
        "format:check": "rune run format",
        "lint": "rune run lint",
        "lint:fix": "rune run lint:fix",
        "smoke": "rune run smoke",
        "start": "rune run start",
        "test": "rune run test",
        "test:coverage": "rune run test:coverage",
        "test:watch": "rune run test:watch",
        "typecheck": "rune run typecheck:all"
      }
    }
"#;

/// The sixteen entries of `ADOPTING_PACKAGE_JSON` that name themselves.
const SELF_NAMING: [&str; 16] = [
  "build",
  "build:apps",
  "build:packages",
  "check",
  "ci",
  "dev",
  "dev:api",
  "dev:packages",
  "dev:web",
  "lint",
  "lint:fix",
  "smoke",
  "start",
  "test",
  "test:coverage",
  "test:watch",
];

fn stdout_of(output: &std::process::Output) -> String {
  String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n")
}

/// The path `init` reported writing, read off its own diagnostic.
fn created_path(output: &std::process::Output) -> String {
  let stderr = String::from_utf8_lossy(&output.stderr).replace("\r\n", "\n");

  stderr
    .lines()
    .find_map(|line| line.strip_prefix("created "))
    .expect("init reports the path it wrote")
    .to_owned()
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

/// Test R4.1 — the file belongs to the project, not to the terminal. A config written
/// three levels down cannot be seen by the walk that looks for one, so this is where the
/// loop starts: `init` succeeds, and every later command still says there is no config.
#[test]
fn init_from_a_subfolder_writes_beside_the_nearest_package_json() {
  let test = Test::new()
    .file("package.json", PACKAGE_JSON)
    .args(["init"])
    .stdout("")
    .stderr_regex(r"rune\.config\.ts")
    .status(0);

  let output = test.run_in("src/deep");

  assert_same_path(
    Some(&created_path(&output)),
    &test.dir().join(CONFIG_FILE),
    "init reported a path that is not the one beside the manifest",
  );
  assert!(test.dir().join(CONFIG_FILE).is_file(), "no config beside the manifest");
  assert!(
    !test.dir().join("src/deep").join(CONFIG_FILE).exists(),
    "a config was left where the terminal happened to be open"
  );
}

/// Test R4.2 — the other half of R4.1, and the half the user feels. Writing to the right
/// place only matters because the tool can then see it.
#[test]
fn the_config_written_from_a_subfolder_is_the_one_every_command_finds() {
  let test = Test::new()
    .file("package.json", PACKAGE_JSON)
    .args(["init"])
    .stdout("")
    .stderr_regex(r"rune\.config\.ts")
    .status(0);

  test.run_in("src/deep");

  assert!(
    loaded_scripts(&test).contains("hello"),
    "listing from the project root did not find the config init had just written"
  );
}

/// Test R4.3 — an empty directory has no better anchor than itself, which is the case a
/// user starting from nothing is in. Unchanged behaviour, and the row that stops the
/// anchor rule going further than it should.
#[test]
fn with_no_manifest_above_the_config_lands_where_the_user_is_standing() {
  let test = Test::new().args(["init"]).stdout("").stderr_regex(r"rune\.config\.ts").status(0);

  test.run_in("src/deep");

  assert!(
    test.dir().join("src/deep").join(CONFIG_FILE).is_file(),
    "with nothing above it, the config belongs where the command was run"
  );
  assert!(!test.dir().join(CONFIG_FILE).exists(), "nothing asked for a config at the root");
}

/// Test R4.4 — the refusal has to be judged where the file would land. Judging it where
/// the user is standing writes a second config beside the first one it refused to touch.
#[test]
fn the_refusal_to_overwrite_is_judged_at_the_resolved_path() {
  let test = Test::new()
    .file("package.json", PACKAGE_JSON)
    .config("export default { scripts: { mine: { command: \"vitest\" } } };\n")
    .args(["init"])
    .stdout("")
    .stderr_regex(r"(?s)already exists")
    .status(1);

  let path = test.dir().join(CONFIG_FILE);
  let before = std::fs::read(&path).expect("read the hand-written config");

  test.run_in("src/deep");

  let after = std::fs::read(&path).expect("read the config back");
  assert_eq!(before, after, "init changed a config it had no permission to touch");
  assert!(
    !test.dir().join("src/deep").join(CONFIG_FILE).exists(),
    "the refusal was judged in the wrong directory and wrote a second config"
  );
}

/// Test R4.5 — the generated guide is the whole documentation a new user has in front of
/// them, and the sentence introducing the kinds reads as a complete list. Leaving one out
/// tells a careful reader there are three, and the missing one is the half that replaces
/// `concurrently`.
#[test]
fn the_starter_names_every_script_kind_and_shows_each_one() {
  let test = Test::new().args(["init"]).stdout("").stderr_regex(r"rune\.config\.ts").status(0);

  test.run();

  let generated =
    std::fs::read_to_string(test.dir().join(CONFIG_FILE)).expect("init wrote a config");

  for kind in ["command", "extends", "serial", "parallel"] {
    assert!(
      generated.contains(&format!("`{kind}`")),
      "the guide never names `{kind}`:\n{generated}"
    );
    assert!(
      generated.contains(&format!("{kind}: ")),
      "the guide names `{kind}` without showing one:\n{generated}"
    );
  }
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

/// Tests R4.6 and R4.9 — the whole point of the seeder, measured against the manifest that
/// broke it. Copying these commands one for one wrote twenty scripts that all re-enter
/// rune: sixteen reaching themselves, four reaching names nothing defines.
///
/// The four that name something else stay, as the aliases they are. Three of their targets
/// are defined nowhere here, so the header says so and the loader says so again the moment
/// anything is listed — which is the answer arriving where the user can act on it, rather
/// than a file that looks finished.
#[test]
fn seeding_a_manifest_that_already_calls_rune_writes_no_command_that_calls_rune() {
  let test = Test::new()
    .file("package.json", ADOPTING_PACKAGE_JSON)
    .args(["init", "--from-package-json"])
    .stdout("")
    .stderr_regex(r"rune\.config\.ts")
    .status(0);

  test.run();

  let generated =
    std::fs::read_to_string(test.dir().join(CONFIG_FILE)).expect("init wrote a config");

  insta::with_settings!({ description => "a manifest whose every script already calls rune: sixteen naming themselves, four naming something else" }, {
    insta::assert_snapshot!(generated);
  });

  let (header, config) =
    generated.split_once("export default").expect("the generated file declares a config");
  let (notes, _) = header.split_once("// A script runs").expect("the guide follows the header");

  assert!(!config.contains("rune run"), "a generated command still calls rune:\n{config}");

  for name in SELF_NAMING {
    assert!(notes.contains(name), "`{name}` was dropped without a word:\n{notes}");
  }

  assert_eq!(config.matches("extends:").count(), 4, "the four redirections:\n{config}");
  assert_eq!(config.matches("command:").count(), 0, "nothing here is a command:\n{config}");
  for target in ["clean:all", "format:fix", "format", "typecheck:all"] {
    assert!(
      config.contains(&format!("extends: \"{target}\"")),
      "no alias of `{target}`:\n{config}"
    );
  }

  for unseen in ["clean:all", "format:fix", "typecheck:all"] {
    assert!(notes.contains(unseen), "`{unseen}` is defined nowhere and unmentioned:\n{notes}");
  }

  // R4.9 for this file. Reaching resolution at all proves it parsed; what it reports is
  // the dangling alias the header already named, in rune's own words.
  let listed = test.then_run(&["list"]);
  let reported = String::from_utf8_lossy(&listed.stderr);

  assert!(!listed.status.success(), "an alias defined nowhere must not list as ordinary");
  assert!(
    reported.contains("which no script defines"),
    "the file must load, leaving only the missing name to report:\n{reported}"
  );
}
