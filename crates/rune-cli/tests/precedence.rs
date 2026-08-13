//! Test 4c.7 — the whole environment precedence table, across an extends chain.
//!
//! A matrix rather than a row per rule, because the bugs here are interactions. Each rule
//! on its own is easy to implement correctly and easy to get wrong in combination, and
//! the combination that matters most is a file at two levels of a chain: "fills gaps
//! only" has to mean gaps in what the layers below already accumulated, not gaps in the
//! process environment alone.
//!
//! Nothing spawns here. The two crates that answer the question are composed exactly as
//! `run` composes them — `rune-config` decides which files take part and in what order,
//! `rune-exec` layers them — and the process environment is supplied by the test, so the
//! table means the same thing on every machine.

use std::ffi::{OsStr, OsString};
use std::fs;

use rune_config::env::Environment;
use rune_config::envfile::Files;
use rune_config::inherit::Scope;
use rune_config::load::load;
use rune_exec::environment::{self, Assignment, Descriptor, FileLayer, Layering};
use tempfile::TempDir;

/// The root's shared definition. Its `env` covers the three cases a map takes part in:
/// a variable only it sets, one the package's map also sets, and one a file also sets.
const ROOT_CONFIG: &str = r#"export default {
  scripts: {
    test: {
      command: "vitest",
      envFile: "./.env",
      env: {
        ONLY_ROOT_MAP: "root-map",
        MAP_OVERRIDES_MAP: "root-map",
        MAP_BEATS_FILE: "map",
        MAP_BEATS_PROCESS: "map"
      }
    }
  }
};
"#;

/// The package narrows the shared script and brings a file and a map of its own, which is
/// the arrangement the whole change has to hold up under.
const PACKAGE_CONFIG: &str = r#"export default {
  scripts: {
    test: {
      extends: "test",
      envFile: "./.env",
      env: { MAP_OVERRIDES_MAP: "package-map", RUNE_ROOT: "hijacked" }
    }
  }
};
"#;

const ROOT_ENV_FILE: &str = "ONLY_ROOT_FILE=root-file\n\
                             BOTH_FILES=root-file\n\
                             PROCESS_AND_FILES=root-file\n\
                             MAP_BEATS_FILE=file\n\
                             RUNE_SCRIPT_NAME=hijacked\n";

const PACKAGE_ENV_FILE: &str = "ONLY_PACKAGE_FILE=package-file\n\
                                BOTH_FILES=package-file\n\
                                PROCESS_AND_FILES=package-file\n";

fn repo() -> TempDir {
  let dir = tempfile::tempdir().expect("create tempdir");
  let files = [
    (".git/HEAD", "ref: refs/heads/main\n"),
    ("package.json", "{ \"name\": \"fixture-root\" }\n"),
    ("rune.config.ts", ROOT_CONFIG),
    (".env", ROOT_ENV_FILE),
    ("packages/legacy/package.json", "{ \"name\": \"legacy\" }\n"),
    ("packages/legacy/rune.config.ts", PACKAGE_CONFIG),
    ("packages/legacy/.env", PACKAGE_ENV_FILE),
  ];

  for (relative, contents) in files {
    let path = dir.path().join(relative);
    fs::create_dir_all(path.parent().expect("a fixture path has a parent"))
      .expect("create parents");
    fs::write(&path, contents).expect("write fixture file");
  }

  dir
}

/// The environment rune is pretending to have been started with.
fn process_environment() -> Vec<(OsString, OsString)> {
  [("ONLY_PROCESS", "process"), ("PROCESS_AND_FILES", "process"), ("MAP_BEATS_PROCESS", "process")]
    .iter()
    .map(|(name, value)| (OsString::from(name), OsString::from(value)))
    .collect()
}

#[test]
fn the_precedence_table_holds_across_an_extends_chain() {
  let dir = repo();
  let loaded = load(&dir.path().join("packages/legacy"), &Environment::default())
    .expect("both configs and both files load");
  let resolved = loaded.resolve("test", Scope::Nearest).expect("resolves").expect("defined");

  let mut read = Files::default();
  let files: Vec<FileLayer> = resolved
    .env_files
    .iter()
    .map(|declared| {
      let file =
        read.read(&loaded.discovered.root, "test", declared).expect("both files are there");

      FileLayer {
        source: file.source.clone(),
        assignments: file
          .assignments
          .iter()
          .map(|assignment| Assignment {
            name: assignment.name.clone(),
            value: assignment.value.clone(),
            line: assignment.line,
          })
          .collect(),
      }
    })
    .collect();

  let layering: Layering = environment::build(
    process_environment(),
    &Descriptor {
      script_name: "test",
      root: &loaded.discovered.root,
      package_dir: &loaded.discovered.package_dir,
      env: &resolved.env,
      env_files: &files,
    },
  );

  // One row per combination of the sources a variable can come from.
  let expected = [
    ("ONLY_PROCESS", "process"),
    ("ONLY_ROOT_FILE", "root-file"),
    ("ONLY_PACKAGE_FILE", "package-file"),
    // Two files, no other claim: the nearer config wins, as it does for every other field.
    ("BOTH_FILES", "package-file"),
    // The rule the change exists for, and it holds for both levels of the chain at once.
    ("PROCESS_AND_FILES", "process"),
    ("ONLY_ROOT_MAP", "root-map"),
    ("MAP_OVERRIDES_MAP", "package-map"),
    ("MAP_BEATS_FILE", "map"),
    ("MAP_BEATS_PROCESS", "map"),
    // Reserved, so neither the file nor the map that tried to set one reached the child.
    ("RUNE_SCRIPT_NAME", "test"),
  ];

  for (name, value) in expected {
    assert_eq!(layering.environment.get(name), Some(OsStr::new(value)), "`{name}`");
  }

  assert_eq!(layering.environment.get("RUNE_ROOT"), Some(loaded.discovered.root.as_os_str()));
  assert_eq!(
    layering.environment.get("RUNE_PACKAGE_DIR"),
    Some(loaded.discovered.package_dir.as_os_str())
  );

  // The delta is every name the config had something to say about, each with the value
  // the child will see and what put it there. A variable that was merely inherited is not
  // in it. `PROCESS_AND_FILES` is: the config assigned it and lost, and a delta that
  // dropped the row would say what was ignored and never what the child gets.
  let applied: Vec<(&str, &str, String)> = layering
    .applied
    .iter()
    .map(|(name, applied)| (name.as_str(), applied.value.as_str(), applied.source.to_string()))
    .collect();
  assert_eq!(
    applied,
    [
      ("BOTH_FILES", "package-file", "`packages/legacy/.env`".to_owned()),
      ("MAP_BEATS_FILE", "map", "this script's `env`".to_owned()),
      ("MAP_BEATS_PROCESS", "map", "this script's `env`".to_owned()),
      ("MAP_OVERRIDES_MAP", "package-map", "this script's `env`".to_owned()),
      ("ONLY_PACKAGE_FILE", "package-file", "`packages/legacy/.env`".to_owned()),
      ("ONLY_ROOT_FILE", "root-file", "`.env`".to_owned()),
      ("ONLY_ROOT_MAP", "root-map", "this script's `env`".to_owned()),
      ("PROCESS_AND_FILES", "process", "the process environment".to_owned()),
    ]
  );

  // Every way an assignment can be refused, in one artifact: beaten by the process
  // environment from either level, beaten by the nearer file, beaten by the `env` map,
  // and blocked by the reserved prefix from a file and from a map alike.
  let reported: Vec<String> =
    layering.ignored.iter().map(std::string::ToString::to_string).collect();

  insta::with_settings!({ description => "a chain where every layer claims something" }, {
    insta::assert_snapshot!(reported.join("\n"));
  });
}
