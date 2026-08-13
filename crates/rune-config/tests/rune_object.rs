//! The object a config imports: what it answers, what it enumerates, and what it refuses.
//!
//! Every assertion here is measured against the environment the config is evaluated with,
//! never against a fixed expectation. A row asserting that `PATH` is present passes on a
//! machine that happens to spell it that way and says nothing about the rule.

mod paths;

use std::fs;

use paths::redact;
use rune_config::env::Environment;
use rune_config::eval::evaluate_config;
use tempfile::TempDir;

fn fixture(source: &str) -> TempDir {
  let dir = tempfile::tempdir().expect("create tempdir");
  fs::write(dir.path().join("rune.config.ts"), source).expect("write the config");
  dir
}

const IMPORT: &str = "import { rune } from '@gio-labs/rune';\n";

/// The default export of a config that evaluates.
fn exported(source: &str, environment: &Environment) -> serde_json::Value {
  let dir = fixture(&format!("{IMPORT}{source}"));

  evaluate_config(dir.path(), &dir.path().join("rune.config.ts"), environment)
    .expect("the config evaluates")
    .value
}

/// The message a config that cannot evaluate produces, with the machine taken out of it.
fn refusal(source: &str) -> String {
  let dir = fixture(&format!("{IMPORT}{source}"));

  let error =
    evaluate_config(dir.path(), &dir.path().join("rune.config.ts"), &Environment::default())
      .expect_err("the mutation must be refused")
      .to_string();

  redact(dir.path(), &error)
}

/// Test R9.1 — the name a config spells has to find the variable the platform stores.
///
/// Windows keeps the search path as `Path`, and every other reader on the platform hides
/// that. A config reading `rune.env.PATH` therefore works on two systems and silently
/// builds the wrong command on the third, which is the one-config-three-systems promise
/// defeated by a casing nobody chose. POSIX is the other half of the same row: two names
/// differing only in case are two variables there, and folding would make one unreachable.
#[test]
fn a_name_is_read_the_way_the_platform_stores_it() {
  let environment = Environment::from_pairs([("Path", "one"), ("OTHER", "two")]);

  let value = exported(
    "export default { upper: rune.env.PATH ?? null, exact: rune.env.Path ?? null, \
     lower: rune.env.path ?? null };\n",
    &environment,
  );

  assert_eq!(value["exact"], "one", "the spelling the environment holds must always answer");

  if cfg!(windows) {
    assert_eq!(value["upper"], "one", "`PATH` is how every cross-platform config spells it");
    assert_eq!(value["lower"], "one");
  } else {
    assert_eq!(value["upper"], serde_json::Value::Null, "case is significant away from Windows");
    assert_eq!(value["lower"], serde_json::Value::Null);
  }
}

/// Test R9.2 — `{ ...rune.env, EXTRA: "1" }` is the ordinary way to write "everything I
/// have plus one more". It produced `{ EXTRA: "1" }`, at load, with nothing on screen.
#[test]
fn the_environment_enumerates() {
  let environment = Environment::from_pairs([("ALPHA", "1"), ("BETA", "2"), ("GAMMA", "3")]);

  let value = exported(
    "export default {\n  \
       keys: Object.keys(rune.env),\n  \
       entries: Object.entries(rune.env).length,\n  \
       spread: { ...rune.env },\n  \
       json: JSON.parse(JSON.stringify(rune.env)),\n  \
       has: Object.prototype.hasOwnProperty.call(rune.env, 'BETA'),\n  \
       present: 'BETA' in rune.env,\n\
     };\n",
    &environment,
  );

  assert_eq!(value["keys"], serde_json::json!(["ALPHA", "BETA", "GAMMA"]));
  assert_eq!(value["entries"], 3.0);
  assert_eq!(value["spread"], serde_json::json!({ "ALPHA": "1", "BETA": "2", "GAMMA": "3" }));
  assert_eq!(value["json"], serde_json::json!({ "ALPHA": "1", "BETA": "2", "GAMMA": "3" }));
  assert_eq!(value["has"], true, "`hasOwnProperty` must agree with `in`");
  assert_eq!(value["present"], true);
}

/// Test R9.6 — wrapping the object must not change what is on it.
#[test]
fn the_object_still_carries_exactly_three_members() {
  let dir =
    fixture(&format!("{IMPORT}export default {{ keys: Object.keys(rune), ci: rune.isCI }};\n"));

  let evaluated = evaluate_config(
    dir.path(),
    &dir.path().join("rune.config.ts"),
    &Environment::from_pairs([("CI", "1")]),
  )
  .expect("the config evaluates");

  assert_eq!(evaluated.value["keys"], serde_json::json!(["env", "platform", "isCI"]));
  assert_eq!(evaluated.value["ci"], true);
  assert!(
    evaluated.observed.values.contains_key("CI"),
    "reading `isCI` must still be observed as a read of the variable it derives from"
  );
}

/// Test R9.3 — four refusals, each in Rune's own voice.
///
/// The words are the deliverable: every one of these was the engine's, naming neither
/// `rune`, nor the property, nor the rule. Reading a bare `rune` in the same product
/// already answers with the import and explains itself.
#[test]
fn assigning_into_the_environment_is_refused_in_runes_words() {
  insta::assert_snapshot!(refusal("rune.env.NEW = '1';\nexport default {};\n"));
}

#[test]
fn deleting_a_member_is_refused_in_runes_words() {
  insta::assert_snapshot!(refusal("delete (rune as any).platform;\nexport default {};\n"));
}

#[test]
fn assigning_to_a_member_is_refused_in_runes_words() {
  insta::assert_snapshot!(refusal("(rune as any).platform = 'linux';\nexport default {};\n"));
}

#[test]
fn assigning_to_the_global_name_is_refused_in_runes_words() {
  insta::assert_snapshot!(refusal("(globalThis as any).rune = {};\nexport default {};\n"));
}

/// Whatever the engine writes about the four above, none of it may reach a user.
#[test]
fn no_refusal_is_left_in_the_engines_words() {
  let engine = ["proxy:", "could not delete property", "no setter for property"];

  for source in [
    "rune.env.NEW = '1';\nexport default {};\n",
    "delete (rune as any).platform;\nexport default {};\n",
    "(rune as any).platform = 'linux';\nexport default {};\n",
    "(globalThis as any).rune = {};\nexport default {};\n",
  ] {
    let message = refusal(source);

    for phrase in engine {
      assert!(!message.contains(phrase), "`{phrase}` reached the user:\n{message}");
    }
    assert!(message.contains("read-only"), "the rule is not stated:\n{message}");
  }
}

/// The fifth attempt, and the one no trap can reach: rebinding the imported name is a
/// rule about a binding rather than an operation on Rune's object, so the engine answers
/// it. The row exists to say that this is known, and that it still fails.
#[test]
fn rebinding_the_import_is_refused_by_the_engine() {
  let dir = fixture(&format!("{IMPORT}rune = {{}} as any;\nexport default {{}};\n"));

  let error =
    evaluate_config(dir.path(), &dir.path().join("rune.config.ts"), &Environment::default())
      .expect_err("rebinding an import must be refused");

  assert!(error.to_string().contains("rune"), "the refusal must name the binding: {error}");
}
