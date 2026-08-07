//! Config shapes that must be refused, and the words used to refuse them.
//!
//! These go through `load` rather than through the schema function directly: a shape
//! rejection is only useful if it happens when a user loads their config, and the path
//! from a file on disk to a message is what a user actually meets.

use std::fs;
use std::path::Path;

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

/// Keeps the tempdir out of the snapshot.
fn redact(dir: &Path, text: &str) -> String {
  let root = dunce::canonicalize(dir).expect("canonicalize tempdir");
  text.replace(&root.to_string_lossy().to_string(), "[TMP]").replace('\\', "/")
}

fn rejection(config: &str) -> String {
  let dir = repo(config);
  let error = load(dir.path(), &Environment::default()).unwrap_err().to_string();

  redact(dir.path(), &error)
}

/// Test 4a.5 — a per-OS object without `default`.
///
/// The mistake is invisible on the machine that made it: a config declaring only `win32`
/// works all day for its author and defines nothing for everyone else. Rejecting it when
/// the config is loaded means whoever wrote it is the one who reads the message.
#[test]
fn a_per_os_command_without_default_is_rejected_at_load_time() {
  let error = rejection(
    "export default {\n  scripts: {\n    clean: { command: { win32: 'rmdir /s /q dist' } },\n  },\n};\n",
  );

  assert!(error.contains("`clean`"), "the message must name the script: {error}");
  assert!(error.contains("`default`"), "the message must name the missing key: {error}");

  insta::with_settings!({ description => "per-OS command with no default" }, {
    insta::assert_snapshot!(error);
  });
}

/// Test 4b.5 — a script that both runs a command and extends another one.
///
/// The two answer the same question, so a script carrying both has no meaning rune could
/// pick without guessing. Naming the script and both words is what turns a rejected
/// config into a one-line edit.
#[test]
fn a_script_with_both_command_and_extends_is_rejected() {
  let error = rejection(
    "export default {\n  scripts: {\n    \
     base: { command: 'vitest' },\n    \
     ci: { command: 'vitest --run', extends: 'base' },\n  \
     },\n};\n",
  );

  assert!(error.contains("`ci`"), "the message must name the script: {error}");
  assert!(error.contains("`command`"), "the message must name the first key: {error}");
  assert!(error.contains("`extends`"), "the message must name the second key: {error}");

  insta::with_settings!({ description => "a script declaring command and extends together" }, {
    insta::assert_snapshot!(error);
  });
}

/// Test 4b.6 — `appendArgs` on a script that extends nothing.
///
/// The arguments would have nothing to be appended to. Silently ignoring them is the
/// failure worth avoiding: the script runs, looks right, and quietly drops a flag.
#[test]
fn append_args_without_extends_is_rejected() {
  let error = rejection(
    "export default {\n  scripts: {\n    \
     test: { command: 'vitest', appendArgs: ['--run'] },\n  \
     },\n};\n",
  );

  assert!(error.contains("`test`"), "the message must name the script: {error}");
  assert!(error.contains("`appendArgs`"), "the message must name the key: {error}");
  assert!(error.contains("`extends`"), "the message must name what it needs: {error}");

  insta::with_settings!({ description => "appendArgs on a script that extends nothing" }, {
    insta::assert_snapshot!(error);
  });
}
