//! Reference errors between scripts, and the words used to report them.
//!
//! These go through `load` and snapshot what a user actually reads. A group is the first
//! feature where a name can be wrong in a script nobody is running, so the message has to
//! carry enough to find it: which script holds the bad name, which name it is, and — for
//! a circle — the path that closed it.

mod paths;

use std::fs;

use paths::redact;
use rune_config::env::Environment;
use rune_config::load::load;
use tempfile::TempDir;

fn repo(config: &str) -> TempDir {
  let dir = tempfile::tempdir().expect("create tempdir");
  fs::create_dir_all(dir.path().join(".git")).expect("create the boundary directory");
  fs::write(dir.path().join(".git/HEAD"), "ref: refs/heads/main\n").expect("write the boundary");
  fs::write(dir.path().join("rune.config.ts"), config).expect("write the config");
  dir
}

fn rejection(scripts: &str) -> String {
  let dir = repo(&format!("export default {{ scripts: {scripts} }};\n"));
  let error = load(dir.path(), &Environment::default()).unwrap_err().to_string();

  redact(dir.path(), &error)
}

/// Test 5a.5 — two groups that name each other.
///
/// Found when the config loads, not when the group runs, and rendered as the path rather
/// than as the fact. "cycle detected" would leave the user rebuilding their own config
/// graph by hand to find which scripts were involved.
#[test]
fn a_circle_between_two_groups_is_rejected_at_load_time_with_the_path() {
  let error = rejection(
    r"{
      a: { serial: ['b'] },
      b: { serial: ['a'] },
    }",
  );

  assert!(error.contains("a → b → a"), "the message must render the path: {error}");

  insta::with_settings!({ description => "two groups naming each other" }, {
    insta::assert_snapshot!(error);
  });
}

/// Test 5a.6 — a member no script defines.
///
/// Three things have to be in the message. The group, because the mistake is not in the
/// script being run. The member, because a group may have a dozen. And the closest
/// defined name, because the mistake is nearly always a typo.
#[test]
fn a_member_no_script_defines_names_the_group_the_member_and_the_closest_name() {
  let error = rejection(
    r"{
      lint: { command: 'eslint .' },
      test: { command: 'vitest' },
      ci: { serial: ['lint', 'tset'] },
    }",
  );

  assert!(error.contains("`ci`"), "the message must name the group: {error}");
  assert!(error.contains("`tset`"), "the message must name the member: {error}");
  assert!(error.contains("`test`"), "the message must offer the closest name: {error}");

  insta::with_settings!({ description => "a group member no script defines" }, {
    insta::assert_snapshot!(error);
  });
}

/// The same check reaches prerequisites, and says which script wanted the missing one.
#[test]
fn a_prerequisite_no_script_defines_names_the_script_that_wanted_it() {
  let error = rejection(
    r"{
      clean: { command: 'rm -rf dist' },
      build: { command: 'tsc -b', dependsOn: ['clen'] },
    }",
  );

  assert!(error.contains("`build`"), "the message must name the script: {error}");
  assert!(error.contains("`clen`"), "the message must name the prerequisite: {error}");

  insta::with_settings!({ description => "a prerequisite no script defines" }, {
    insta::assert_snapshot!(error);
  });
}

/// A circle can run through a member on one side and a prerequisite on the other, which
/// is why the walk covers both in one pass instead of checking them separately.
#[test]
fn a_circle_through_a_prerequisite_and_a_member_is_rejected() {
  let error = rejection(
    r"{
      ci: { serial: ['build'] },
      build: { command: 'tsc -b', dependsOn: ['ci'] },
    }",
  );

  assert!(error.contains("build → ci"), "the message must render the path: {error}");

  insta::with_settings!({ description => "a circle closed through dependsOn" }, {
    insta::assert_snapshot!(error);
  });
}

/// A group already carries its order in its member list, so a second one on the same
/// entry is refused rather than silently picked between.
#[test]
fn depends_on_beside_serial_is_rejected() {
  let error = rejection(
    r"{
      lint: { command: 'eslint .' },
      ci: { serial: ['lint'], dependsOn: ['lint'] },
    }",
  );

  assert!(error.contains("`ci`"), "the message must name the script: {error}");

  insta::with_settings!({ description => "dependsOn on a group" }, {
    insta::assert_snapshot!(error);
  });
}
