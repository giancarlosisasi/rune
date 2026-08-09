//! Which config a relative path in a script belongs to.
//!
//! One rule covers `cwd` and `envFile` alike: a relative path is relative to the config
//! that wrote it. This is the crate-level half of it. Resolution has to carry the
//! declaring config out together with the value, because a layer narrows a script one key
//! at a time and nothing downstream can work out afterwards which layer won.

use std::fs;
use std::path::Path;

use rune_config::env::Environment;
use rune_config::inherit::Scope;
use rune_config::load::load;
use rune_config::paths::relative_to;
use tempfile::TempDir;

fn repo(files: &[(&str, &str)]) -> TempDir {
  let dir = tempfile::tempdir().expect("create tempdir");
  fs::create_dir_all(dir.path().join(".git")).expect("create the boundary directory");
  fs::write(dir.path().join(".git/HEAD"), "ref: refs/heads/main\n").expect("write the boundary");

  for (relative, contents) in files {
    let path = dir.path().join(relative);
    fs::create_dir_all(path.parent().expect("a fixture path has a parent"))
      .expect("create parents");
    fs::write(&path, contents).expect("write fixture file");
  }

  dir
}

fn config(scripts: &str) -> String {
  format!("export default {{ scripts: {scripts} }};\n")
}

/// The `cwd` a name resolves to, and the config that wrote it.
fn declared_cwd(dir: &Path, from: &str, name: &str) -> (String, String) {
  let loaded = load(&dir.join(from), &Environment::default()).expect("the configs load");
  let resolved = loaded.resolve(name, Scope::Nearest).expect("resolves").expect("defined");
  let cwd = resolved.cwd.expect("the script declares a cwd");

  (cwd.value.to_owned(), relative_to(&loaded.discovered.root, cwd.source))
}

/// Test R2.3 — the three layerings a `cwd` can arrive through, in one fixture.
///
/// A package narrows a shared script one key at a time, so the value and the file that
/// wrote it have to travel together. Deriving the file afterwards means deciding a second
/// time which layer won, and two derivations can disagree.
#[test]
fn a_relative_cwd_belongs_to_the_config_that_wrote_it() {
  let dir = repo(&[
    ("package.json", "{ \"name\": \"fixture-root\", \"private\": true }\n"),
    ("rune.config.ts", &config(r#"{ api: { command: "node server.js", cwd: "apps/api" } }"#)),
    ("packages/ui/package.json", "{ \"name\": \"ui\" }\n"),
    ("packages/ui/rune.config.ts", &config(r#"{ api: { extends: "api" } }"#)),
    ("packages/web/package.json", "{ \"name\": \"web\" }\n"),
    ("packages/web/rune.config.ts", &config(r#"{ api: { extends: "api", cwd: "server" } }"#)),
  ]);

  assert_eq!(
    declared_cwd(dir.path(), ".", "api"),
    ("apps/api".to_owned(), "rune.config.ts".to_owned()),
    "a script that declares its own value belongs to its own config"
  );

  assert_eq!(
    declared_cwd(dir.path(), "packages/ui", "api"),
    ("apps/api".to_owned(), "rune.config.ts".to_owned()),
    "a package that narrows without a cwd inherits the root's value and the root's anchor"
  );

  assert_eq!(
    declared_cwd(dir.path(), "packages/web", "api"),
    ("server".to_owned(), "packages/web/rune.config.ts".to_owned()),
    "a package that declares its own cwd anchors on itself"
  );
}
