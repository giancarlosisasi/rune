//! Evaluating a `rune.config.ts`: relative imports, the specifier forms that must all
//! reach the same file, and the imports that must be refused.

mod paths;

use std::fs;

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

/// Replaces the tempdir path so a snapshot does not encode where the test ran.
/// Test 2.2 — the headline of this change: a config is allowed to be more than one file.
#[test]
fn helper_import_is_inlined_into_the_command() {
  let dir = fixture(&[
    ("scripts/helpers.ts", "export const PORT: number = 4000;\n"),
    (
      "rune.config.ts",
      "import { PORT } from './scripts/helpers';\n\
       export default { scripts: { dev: { command: `vite --port ${PORT}` } } };\n",
    ),
  ]);

  let config = evaluate_config(&dir.path().join("rune.config.ts"), &Environment::default())
    .expect("config evaluates")
    .value;

  assert_eq!(config["scripts"]["dev"]["command"], "vite --port 4000");
}

/// Test 2.5 — the error has to explain the engine, not just report a missing module.
#[test]
fn npm_value_import_names_the_specifier_and_explains_the_engine() {
  let dir = fixture(&[(
    "rune.config.ts",
    "import { padStart } from 'lodash';\nexport default { value: padStart };\n",
  )]);

  let error = evaluate_config(&dir.path().join("rune.config.ts"), &Environment::default())
    .unwrap_err()
    .to_string();

  insta::with_settings!({ description => "npm value import" }, {
    insta::assert_snapshot!(redact(dir.path(), &error));
  });
}

/// Test 2.6 — the half that must keep working: `import type` never reaches the loader.
#[test]
fn type_only_npm_import_is_erased_and_the_config_loads() {
  let dir = fixture(&[(
    "rune.config.ts",
    "import type { Script } from '@gio-labs/rune';\n\
     const dev: Script = { command: 'vite' };\n\
     export default { scripts: { dev } };\n",
  )]);

  let config = evaluate_config(&dir.path().join("rune.config.ts"), &Environment::default())
    .expect("config evaluates")
    .value;

  assert_eq!(config["scripts"]["dev"]["command"], "vite");
}

/// The authoring style every guide and every published type is written for: `defineConfig`
/// comes from the package rune publishes, and the binary answers that import itself.
#[test]
fn define_config_is_supplied_by_the_binary_rather_than_by_node_modules() {
  let dir = fixture(&[(
    "rune.config.ts",
    "import { defineConfig } from '@gio-labs/rune';\n\
     export default defineConfig({ scripts: { dev: { command: 'vite' } } });\n",
  )]);

  let config = evaluate_config(&dir.path().join("rune.config.ts"), &Environment::default())
    .expect("config evaluates")
    .value;

  assert_eq!(config["scripts"]["dev"]["command"], "vite");
}

/// A config that reaches for anything beyond `defineConfig` and `rune` is told so, rather
/// than being handed an undefined value that fails somewhere further along. Widening the
/// module to carry the environment object must not widen it to carry everything.
#[test]
fn the_supplied_module_offers_nothing_beyond_define_config_and_rune() {
  let dir = fixture(&[(
    "rune.config.ts",
    "import { somethingElse } from '@gio-labs/rune';\n\
     export default { scripts: { dev: { command: somethingElse } } };\n",
  )]);

  let error = evaluate_config(&dir.path().join("rune.config.ts"), &Environment::default())
    .unwrap_err()
    .to_string();

  assert!(error.contains("somethingElse"), "{error}");
}

/// Test 2.3 — naming only the missing file leaves the user hunting for who imported it.
#[test]
fn missing_relative_import_names_the_file_and_its_importer() {
  let dir = fixture(&[("rune.config.ts", "import { x } from './nope';\nexport default { x };\n")]);

  let error = evaluate_config(&dir.path().join("rune.config.ts"), &Environment::default())
    .unwrap_err()
    .to_string();

  insta::with_settings!({ description => "missing relative import" }, {
    insta::assert_snapshot!(redact(dir.path(), &error));
  });
}

/// Test 2.4 — the oracle is that this test finishes at all. A hang is caught by the
/// nextest timeout, a stack overflow by the process dying.
#[test]
fn an_import_cycle_terminates() {
  let dir = fixture(&[
    ("a.ts", "import { b } from './b';\nexport const a = 'a';\nexport const fromB = () => b;\n"),
    ("b.ts", "import { a } from './a';\nexport const b = 'b';\nexport const fromA = () => a;\n"),
    (
      "rune.config.ts",
      "import { a } from './a';\nimport { b } from './b';\nexport default { a, b };\n",
    ),
  ]);

  let config = evaluate_config(&dir.path().join("rune.config.ts"), &Environment::default())
    .expect("a cycle still evaluates")
    .value;

  assert_eq!(config, serde_json::json!({ "a": "a", "b": "b" }));
}

/// Test 2.10 — refused for cache integrity, and the message has to say so.
#[test]
fn dynamic_import_is_rejected_with_the_cache_reason() {
  let dir = fixture(&[
    ("h.ts", "export const value = 1;\n"),
    ("rune.config.ts", "const m = import('./h');\nexport default { m };\n"),
  ]);

  let error = evaluate_config(&dir.path().join("rune.config.ts"), &Environment::default())
    .unwrap_err()
    .to_string();

  assert!(error.contains("dynamic `import()`"), "{error}");
  assert!(error.contains("stale"), "the cache-integrity reason is missing:\n{error}");
}

/// Test 2.19 — four specifier forms, one resolver. The cache half of this row lands
/// with the cache itself.
#[test]
fn every_specifier_form_resolves() {
  let dir = fixture(&[
    ("outer.ts", "export const outer = 'outer';\n"),
    ("nested/h.ts", "export const h = 'h';\n"),
    ("nested/dir/index.ts", "export const indexed = 'indexed';\n"),
    (
      "nested/rune.config.ts",
      "import { h } from './h';\n\
       import { h as explicit } from './h.ts';\n\
       import { indexed } from './dir/index.ts';\n\
       import { outer } from '../outer';\n\
       export default { h, explicit, indexed, outer };\n",
    ),
  ]);

  let config = evaluate_config(&dir.path().join("nested/rune.config.ts"), &Environment::default())
    .expect("config evaluates")
    .value;

  assert_eq!(
    config,
    serde_json::json!({ "h": "h", "explicit": "h", "indexed": "indexed", "outer": "outer" })
  );
}

/// Test 2.20 — the same file reached through both separators must be one module. If the
/// key split, the two imports would be distinct object identities.
#[test]
fn both_path_separators_reach_one_module_instance() {
  let dir = fixture(&[
    ("scripts/helper.ts", "export const box = { seen: true };\n"),
    (
      "rune.config.ts",
      "import { box as viaSlash } from './scripts/helper';\n\
       import { box as viaBackslash } from '.\\\\scripts\\\\helper';\n\
       export default { sameInstance: viaSlash === viaBackslash };\n",
    ),
  ]);

  let config = evaluate_config(&dir.path().join("rune.config.ts"), &Environment::default())
    .expect("config evaluates")
    .value;

  assert_eq!(config["sameInstance"], true);
}

/// Test 2.8 — the three things a config is allowed to know about its machine.
#[test]
fn the_imported_rune_object_reports_the_environment() {
  let dir = fixture(&[(
    "rune.config.ts",
    "import { rune } from '@gio-labs/rune';\n\
     export default { platform: rune.platform, isCI: rune.isCI, token: rune.env.MY_TOKEN };\n",
  )]);
  let environment = Environment::from_pairs([("CI", "1"), ("MY_TOKEN", "abc123")]);

  let evaluated =
    evaluate_config(&dir.path().join("rune.config.ts"), &environment).expect("config evaluates");

  let expected_platform = if cfg!(windows) {
    "win32"
  } else if cfg!(target_os = "macos") {
    "darwin"
  } else {
    "linux"
  };
  assert_eq!(evaluated.value["platform"], expected_platform);
  assert_eq!(evaluated.value["isCI"], true);
  assert_eq!(evaluated.value["token"], "abc123");
}

/// Only the variables the config actually read may enter the cache key. Hashing the
/// whole environment would miss the cache on every unrelated change.
#[test]
fn only_the_variables_the_config_read_are_observed() {
  let dir = fixture(&[(
    "rune.config.ts",
    "import { rune } from '@gio-labs/rune';\n\
     export default { a: rune.env.WANTED };\n",
  )]);
  let environment = Environment::from_pairs([("WANTED", "yes"), ("IGNORED", "no")]);

  let evaluated =
    evaluate_config(&dir.path().join("rune.config.ts"), &environment).expect("config evaluates");

  assert_eq!(evaluated.observed_env.keys().collect::<Vec<_>>(), vec!["WANTED"]);
}

/// Test 2.9 — the spec says frozen; this is what frozen has to mean in practice.
#[test]
fn the_imported_rune_object_cannot_be_reassigned() {
  let dir = fixture(&[(
    "rune.config.ts",
    "import { rune } from '@gio-labs/rune';\n\
     try { (rune as any).platform = 'hacked'; } catch (_) {}\n\
     try { (rune as any).env.CI = 'hacked'; } catch (_) {}\n\
     export default { platform: rune.platform, ci: rune.env.CI };\n",
  )]);
  let environment = Environment::from_pairs([("CI", "real")]);

  let config = evaluate_config(&dir.path().join("rune.config.ts"), &environment)
    .expect("config evaluates")
    .value;

  assert_ne!(config["platform"], "hacked");
  assert_eq!(config["ci"], "real");
}

/// Test 2.9b — a helper module is where most environment reads actually live, because
/// that is where the repeated fragments get factored to. The supplied module has to be
/// importable from any file in the graph, not only from the entry config.
#[test]
fn a_relative_import_can_read_the_environment_too() {
  let dir = fixture(&[
    (
      "scripts/helpers.ts",
      "import { rune } from '@gio-labs/rune';\n\
       export const reporter = rune.isCI ? 'github' : 'pretty';\n",
    ),
    (
      "rune.config.ts",
      "import { reporter } from './scripts/helpers';\n\
       export default { reporter };\n",
    ),
  ]);
  let environment = Environment::from_pairs([("CI", "1")]);

  let evaluated =
    evaluate_config(&dir.path().join("rune.config.ts"), &environment).expect("config evaluates");

  assert_eq!(evaluated.value["reporter"], "github");
}

/// Test 2.9a — the upgrade path for every config written against the old global. Reading
/// the bare name has to say what to write instead, not `rune is not defined`.
#[test]
fn reading_rune_without_importing_it_names_the_import() {
  let dir = fixture(&[("rune.config.ts", "export default { platform: rune.platform };\n")]);

  let error = evaluate_config(&dir.path().join("rune.config.ts"), &Environment::default())
    .unwrap_err()
    .to_string();

  assert!(error.contains("rune"), "the unavailable name is not reported:\n{error}");
  assert!(
    error.contains("import { rune } from \"@gio-labs/rune\""),
    "the message does not show the import that replaces the global:\n{error}"
  );
}

/// Test 2.7 — a bare `ReferenceError` would be correct and useless. The message has to
/// answer the question the user actually has.
#[test]
fn node_apis_fail_with_a_message_listing_what_is_available() {
  for api in ["require", "process", "fs"] {
    let dir = fixture(&[("rune.config.ts", &format!("export default {{ value: {api} }};\n"))]);

    let error = evaluate_config(&dir.path().join("rune.config.ts"), &Environment::default())
      .unwrap_err()
      .to_string();

    assert!(error.contains(api), "`{api}` is not named:\n{error}");
    assert!(error.contains("rune.env"), "`{api}` does not list what works:\n{error}");
    assert!(
      error.contains("import { rune } from \"@gio-labs/rune\""),
      "`{api}` lists the environment object without the import that provides it:\n{error}"
    );
  }
}

/// Test 2.1 — behavioral, not a snapshot: a snapshot of stripped enum output would
/// freeze codegen formatting and churn on every oxc bump. This is the row that catches
/// the semantic builder losing enum values.
#[test]
fn an_enum_member_value_reaches_the_resolved_command() {
  let dir = fixture(&[(
    "rune.config.ts",
    "enum Port { Dev = 5173, Preview = 4173 }\n\
     export default { scripts: { dev: { command: `vite --port ${Port.Dev}` } } };\n",
  )]);

  let config = evaluate_config(&dir.path().join("rune.config.ts"), &Environment::default())
    .expect("config evaluates")
    .value;

  assert_eq!(config["scripts"]["dev"]["command"], "vite --port 5173");
}

/// Test 2.11 — the most common authoring mistake. It must never be a panic.
#[test]
fn a_missing_or_non_object_default_export_names_the_file() {
  for source in
    ["export const scripts = {};\n", "export default 42;\n", "export default 'a string';\n"]
  {
    let dir = fixture(&[("rune.config.ts", source)]);

    let error = evaluate_config(&dir.path().join("rune.config.ts"), &Environment::default())
      .unwrap_err()
      .to_string();

    assert!(error.contains("rune.config.ts"), "the file is not named:\n{error}");
    assert!(error.contains("export default"), "no guidance given:\n{error}");
  }
}

/// Test 2.12 — decision D7 resolved to source-map remapping. The assertion is the exact
/// line the user wrote: the config's `boom()` call is on line 6 of the file, while the
/// stripped JavaScript puts it on line 3. An approximation would report the 3.
#[test]
fn a_throwing_config_reports_the_original_typescript_line() {
  let dir = fixture(&[
    (
      "helper.ts",
      "export const boom = (): string => {\n  throw new Error('helper exploded');\n};\n",
    ),
    (
      "rune.config.ts",
      "import { boom } from './helper';\n\
       \n\
       const unused: number = 1;\n\
       \n\
       export default {\n  \
       scripts: { dev: { command: boom() } },\n\
       };\n",
    ),
  ]);

  let error = evaluate_config(&dir.path().join("rune.config.ts"), &Environment::default())
    .unwrap_err()
    .to_string();

  assert!(error.contains("helper exploded"), "{error}");
  assert!(
    error.contains("rune.config.ts:6:"),
    "the config frame is not the line the user wrote:\n{error}"
  );
  assert!(error.contains("helper.ts:2:"), "the helper frame is wrong:\n{error}");
}
