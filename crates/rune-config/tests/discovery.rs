//! What a failed discovery tells the user, and how it knows.
//!
//! The walk has always distinguished its two stopping conditions, because the boundary
//! rule requires it. Only the message collapsed them, and the two need opposite advice:
//! stopping at a repository means "your config is not in this repository", while running
//! out of filesystem means "you are not in a repository at all".
//!
//! The text is the deliverable here, so the three shapes are snapshots. Every path is
//! redacted, because a snapshot carrying a temporary directory asserts the machine.

use std::fs;
use std::path::Path;
use std::time::Instant;

use rune_config::discover::{CONFIG_FILE, StoppedAt, discover, nearest_package_json};
use tempfile::TempDir;

/// A tree with the given directories and files, and no config anywhere.
fn tree(files: &[&str]) -> TempDir {
  let dir = tempfile::tempdir().expect("create tempdir");

  for relative in files {
    let path = dir.path().join(relative);
    fs::create_dir_all(path.parent().expect("a fixture path has a parent"))
      .expect("create parents");
    fs::write(&path, "").expect("write fixture file");
  }

  dir
}

/// Replaces the tempdir with `[TMP]` and writes what is left with forward slashes, so one
/// snapshot serves every platform.
fn redact(dir: &Path, text: &str) -> String {
  let native = dir.to_string_lossy().into_owned();

  text.replace(&native, "[TMP]").replace(&native.replace('\\', "/"), "[TMP]").replace('\\', "/")
}

/// Nothing above a temporary directory may be a repository, or the walk stops there and
/// reports a boundary the test did not build. Stated rather than assumed: it is a fact
/// about the machine, and a machine where it is untrue would otherwise fail obscurely.
fn assert_no_repository_above(dir: &Path) {
  for ancestor in dir.ancestors().skip(1) {
    assert!(
      !ancestor.join(".git").exists(),
      "this machine holds a repository at {}, above the temporary directory",
      ancestor.display()
    );
  }
}

/// Test R8.1 — the walk really did stop at a repository, and the message names which one.
#[test]
fn a_walk_stopped_by_a_repository_names_it_and_offers_a_config_there() {
  let dir = tree(&[".git/HEAD", "packages/ui/package.json"]);
  let start = dir.path().join("packages/ui");

  let error = discover(&start).expect_err("no config exists anywhere in this tree");

  assert_eq!(error.stopped_at, StoppedAt::Repository(dir.path().to_path_buf()));

  insta::with_settings!({ description => "no config, and the walk stopped at a repository" }, {
    insta::assert_snapshot!(redact(dir.path(), &error.to_string()));
  });
}

/// Test R8.2 — the sentence the evidence caught: printed above `C:\`, where there is no
/// repository and never was one. The advice a user needs here is the opposite of R8.1's.
#[test]
fn a_walk_that_reaches_the_top_of_the_filesystem_says_so() {
  let dir = tree(&["work/package.json"]);
  assert_no_repository_above(dir.path());
  let start = dir.path().join("work");

  let error = discover(&start).expect_err("no config and no repository");

  assert_eq!(error.stopped_at, StoppedAt::FilesystemRoot);
  assert!(!error.to_string().contains("repository boundary"), "{error}");

  insta::with_settings!({ description => "no config, and no repository anywhere above" }, {
    insta::assert_snapshot!(redact(dir.path(), &error.to_string()));
  });
}

/// Test R8.3 — the config the upward walk cannot see. Without this the user is told to run
/// the command that created the file they are standing above.
#[test]
fn a_config_below_the_starting_directory_is_named() {
  let dir = tree(&[".git/HEAD", &format!("src/deep/{CONFIG_FILE}")]);

  let error = discover(dir.path()).expect_err("the config below is invisible to the walk");

  assert_eq!(
    error.found_below.as_deref(),
    Some(dir.path().join("src/deep").join(CONFIG_FILE).as_path())
  );

  insta::with_settings!({ description => "no config above, and one in a subfolder" }, {
    insta::assert_snapshot!(redact(dir.path(), &error.to_string()));
  });
}

/// Test R17.6 — the manifest walk describes its failure in the same vocabulary the config
/// walk does. Answering found-or-nothing is what left its caller with one sentence for two
/// situations, and picking the wrong one whenever the walk ran out of filesystem.
#[test]
fn a_manifest_walk_stopped_by_a_repository_reports_that_repository() {
  let dir = tree(&[".git/HEAD", "packages/ui/.keep"]);

  let stopped = nearest_package_json(&dir.path().join("packages/ui"))
    .expect_err("no package.json anywhere in this tree");

  assert_eq!(stopped, StoppedAt::Repository(dir.path().to_path_buf()));
}

/// The other half of R17.6, and the half every existing caller depends on: what the walk
/// answers when it succeeds is still the manifest itself.
#[test]
fn a_manifest_walk_that_finds_one_answers_with_it() {
  let dir = tree(&[".git/HEAD", "package.json", "packages/ui/.keep"]);

  let found = nearest_package_json(&dir.path().join("packages/ui"));

  assert_eq!(found, Ok(dir.path().join("package.json")));
}

/// Test R8.6 — the walk ends at the top of the filesystem instead of looping there.
///
/// The budget is deliberately loose: this asserts termination, not speed, and a tight
/// bound would report a busy machine as a defect.
#[test]
fn discovery_terminates_where_no_repository_exists() {
  let dir = tree(&["work/package.json"]);
  assert_no_repository_above(dir.path());

  let started = Instant::now();
  let error = discover(&dir.path().join("work")).expect_err("no config and no repository");

  assert_eq!(error.stopped_at, StoppedAt::FilesystemRoot);
  assert!(started.elapsed().as_secs() < 5, "the walk took {:?}", started.elapsed());
}
