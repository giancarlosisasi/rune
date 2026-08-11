//! Which file an `envFile` names, and what happens when it cannot be used.
//!
//! Everything here is about the file itself rather than about the environment it builds:
//! where a relative path points, what its bytes are, and the ways a declared file can be
//! unusable. The layering those files feed is exercised in `rune-cli/tests/precedence.rs`.
//!
//! A file is read when a script that declares it is reached, so these go through the
//! reader rather than through `load`. A file only one script can reach is that script's
//! problem: it is not a reason for `rune list` to stop working.

use std::fs;

use rune_config::env::Environment;
use rune_config::inherit::Scope;
use rune_config::load::load;
use tempfile::TempDir;

/// A repository whose env file is written byte by byte.
///
/// The encoding is the subject of half this file, and a fixture written as a Rust string
/// is UTF-8 with no mark — it cannot express either thing being tested.
fn repo_with_bytes(config_scripts: &str, env_file: &[u8]) -> TempDir {
  let dir = repo(&[("rune.config.ts", &config(config_scripts))]);
  fs::write(dir.path().join(".env"), env_file).expect("write the env file");

  dir
}

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

/// The error the reader gives for the file `script` declares as `declared`, resolved
/// against the config at `config`.
fn rejection(files: &[(&str, &str)], config: &str, declared: &str) -> String {
  let dir = repo(files);

  rune_config::envfile::read(dir.path(), &dir.path().join(config), "test", declared)
    .expect_err("the file cannot be used")
    .to_string()
}

/// Test 4c.6 — settled as a hard error rather than warn-and-continue.
///
/// Continuing would put the child in front of a missing variable, and the failure would
/// arrive as whatever that script does with an undefined `API_URL`: a wrong default, a
/// connection refused, or a test that passed because it tested nothing.
#[test]
fn a_declared_env_file_that_does_not_exist_is_an_error() {
  let error = rejection(
    &[("rune.config.ts", &config(r#"{ test: { command: "vitest", envFile: ".env" } }"#))],
    "rune.config.ts",
    ".env",
  );

  assert!(error.contains("`test`"), "the message must name the script: {error}");
  assert!(error.contains(".env"), "the message must name the resolved path: {error}");
  assert!(error.contains("rune.config.ts"), "the message must name the config: {error}");

  insta::with_settings!({ description => "a script naming a file that is not there" }, {
    insta::assert_snapshot!(error);
  });
}

/// The message earns the config it names when there are two of them: a package declaring
/// `.env` and a root declaring `.env` mean different files, and only the declaring config
/// says which one was wanted.
#[test]
fn a_missing_file_names_the_config_that_declared_it() {
  let error = rejection(
    &[
      ("rune.config.ts", &config(r#"{ test: { command: "vitest" } }"#)),
      ("package.json", "{ \"name\": \"fixture-root\" }\n"),
      ("packages/legacy/package.json", "{ \"name\": \"legacy\" }\n"),
      (
        "packages/legacy/rune.config.ts",
        &config(r#"{ test: { extends: "test", envFile: "./.env" } }"#),
      ),
    ],
    "packages/legacy/rune.config.ts",
    "./.env",
  );

  assert!(
    error.contains("packages/legacy/.env"),
    "the path must resolve against the package, not the root: {error}"
  );
  assert!(
    error.contains("packages/legacy/rune.config.ts"),
    "the message must name the config that declared it: {error}"
  );
}

/// Test 4c.8 — `dotenvy` reports the text of the line it rejected and an offset inside
/// that text, never the line's position in the file, so rune counts the lines itself.
#[test]
fn a_malformed_assignment_names_the_file_and_the_line() {
  let error = rejection(
    &[
      ("rune.config.ts", &config(r#"{ test: { command: "vitest", envFile: ".env" } }"#)),
      (".env", "# the endpoint\nAPI_URL=https://example.test\nnot an assignment\n"),
    ],
    "rune.config.ts",
    ".env",
  );

  assert!(error.contains(".env"), "the message must name the file: {error}");
  assert!(error.contains("line 3"), "the message must name the line: {error}");

  insta::with_settings!({ description => "a line that is not an assignment" }, {
    insta::assert_snapshot!(error);
  });
}

/// The bytes a Windows editor writes in front of a file it saves as UTF-8.
const MARK: &[u8] = &[0xEF, 0xBB, 0xBF];

/// Reads `.env` for the one script the fixture defines, straight through the reader.
fn read_env_file(dir: &TempDir) -> Result<rune_config::envfile::EnvFile, String> {
  rune_config::envfile::read(dir.path(), &dir.path().join("rune.config.ts"), "test", ".env")
    .map_err(|error| error.to_string())
}

fn bytes(parts: &[&[u8]]) -> Vec<u8> {
  parts.concat()
}

/// Test R10.1 — the mark is what Windows tools write by default, so the file arrives
/// broken and nothing a user can see in an editor explains the refusal.
///
/// The assertion is on the parsed key. A message-level assertion passes while the
/// invisible byte rides along inside the name and reaches the child.
#[test]
fn a_byte_order_mark_is_stripped_before_parsing() {
  let dir = repo_with_bytes(
    r#"{ test: { command: "vitest", envFile: ".env" } }"#,
    &bytes(&[MARK, b"BOMKEY=1\nSECOND=2\n"]),
  );

  let file = read_env_file(&dir).expect("a file written by a Windows editor has to load");

  assert_eq!(file.assignments[0].name, "BOMKEY", "the mark must not survive into the name");
  assert_eq!(file.assignments[0].value, "1");
  assert_eq!(file.assignments[1].name, "SECOND", "the assignment after it must load too");
}

/// Test R10.5 — only a mark at the very start is a mark. The same bytes anywhere else
/// belong to the file, and a repair that deleted them everywhere would corrupt a value
/// its author wrote on purpose.
#[test]
fn the_same_bytes_anywhere_else_are_data() {
  let dir = repo_with_bytes(
    r#"{ test: { command: "vitest", envFile: ".env" } }"#,
    &bytes(&[b"A=one", MARK, b"two\n"]),
  );

  let file = read_env_file(&dir).expect("the file parses");

  assert_eq!(file.assignments[0].value, "one\u{feff}two");
}

/// Test R10.2 — the other thing a Windows editor does. `stream did not contain valid
/// UTF-8` is the standard library's sentence: it names neither the encoding that is there
/// nor the one rune reads, and it leaves the user with nothing to do next.
#[test]
fn a_utf_16_file_is_named_and_the_conversion_is_given() {
  let contents: Vec<u8> =
    bytes(&[&[0xFF, 0xFE], &"A=1\n".encode_utf16().flat_map(u16::to_le_bytes).collect::<Vec<_>>()]);
  let dir = repo_with_bytes(r#"{ test: { command: "vitest", envFile: ".env" } }"#, &contents);

  let error = read_env_file(&dir).expect_err("rune reads UTF-8");

  assert!(error.contains("UTF-16"), "the encoding that is there is not named: {error}");
  assert!(error.contains("UTF-8"), "the encoding rune reads is not named: {error}");

  insta::with_settings!({ description => "an env file saved as UTF-16" }, {
    insta::assert_snapshot!(error);
  });
}

/// Test R10.3 — the phrase the current message is made of, asserted over the reader's own
/// failures rather than by grepping the tree.
#[test]
fn the_standard_librarys_wording_reaches_nobody() {
  let utf16: Vec<u8> =
    bytes(&[&[0xFF, 0xFE], &"A=1\n".encode_utf16().flat_map(u16::to_le_bytes).collect::<Vec<_>>()]);

  for contents in [utf16, bytes(&[b"A=", &[0xC3, 0x28], b"\n"])] {
    let dir = repo_with_bytes(r#"{ test: { command: "vitest", envFile: ".env" } }"#, &contents);

    let error = read_env_file(&dir).expect_err("neither file is UTF-8");

    assert!(
      !error.contains("stream did not contain valid UTF-8"),
      "the standard library's wording reached the user: {error}"
    );
    assert!(error.contains("UTF-8"), "the encoding rune reads is not named: {error}");
  }
}

/// Test R10.7 — a line quoted back to the user is a line they can read. A raw control
/// byte written to a terminal is swallowed by it, so the proof of the failure disappears
/// exactly where it is needed.
#[test]
fn a_quoted_line_is_printable() {
  let dir = repo_with_bytes(
    r#"{ test: { command: "vitest", envFile: ".env" } }"#,
    &bytes(&[b"A=1\n", MARK, b"B=2\n"]),
  );

  let error = read_env_file(&dir).expect_err("a mark inside the file is not an assignment");

  assert!(!error.contains('\u{feff}'), "the byte was written to the terminal raw: {error}");
  assert!(error.contains("\\u{feff}"), "the byte is not shown at all: {error}");
  assert!(error.contains("line 2"), "the line is not named: {error}");
}

/// Test R11.1's crate half, and R11.5 — loading a config no longer depends on a file
/// only one script can reach.
///
/// The rule is that everything a *name* can reach is resolved before anything starts.
/// `lint` cannot reach the file `start:api` declares by any path, and it used to be
/// refused for it — as did listing, which is the command a stuck user runs to find out
/// what they can run at all.
#[test]
fn a_config_loads_when_a_file_only_one_script_can_reach_is_unusable() {
  for (name, contents) in [(".env", None), (".env", Some("not an assignment\n"))] {
    let mut files = vec![(
      "rune.config.ts",
      config(r#"{ lint: { command: "biome" }, test: { command: "vitest", envFile: ".env" } }"#),
    )];
    if let Some(contents) = contents {
      files.push((name, contents.to_owned()));
    }
    let borrowed: Vec<(&str, &str)> =
      files.iter().map(|(path, contents)| (*path, contents.as_str())).collect();
    let dir = repo(&borrowed);

    let loaded = load(dir.path(), &Environment::default())
      .expect("a file one script declares cannot stop the config loading");

    assert_eq!(loaded.names(Scope::Nearest).len(), 2, "every script is still listed");
  }
}

/// Test R11.6 — a file named by four members of one group is opened once.
///
/// Asserted on what the reader read, keyed by the path it resolved to. Counting opens
/// would need a hook in shipping code, which no test here may require.
#[test]
fn a_file_named_by_four_members_is_read_once() {
  let dir = repo(&[
    (
      "rune.config.ts",
      &config(
        r#"{
          a: { command: "vitest", envFile: ".env" },
          b: { command: "vitest", envFile: ".env" },
          c: { command: "vitest", envFile: "./.env" },
          d: { command: "vitest", envFile: ".env" },
          all: { serial: ["a", "b", "c", "d"] }
        }"#,
      ),
    ),
    (".env", "SHARED=1\n"),
  ]);
  let loaded = load(dir.path(), &Environment::default()).expect("the config loads");
  let mut files = rune_config::envfile::Files::default();

  for script in ["a", "b", "c", "d"] {
    let resolved = loaded.resolve(script, Scope::Nearest).expect("resolves").expect("defined");
    for declared in &resolved.env_files {
      files.read(&loaded.discovered.root, script, declared).expect("the file is there");
    }
  }

  assert_eq!(files.len(), 1, "four members naming one file must open it once");
}

/// Test R11.7 — the same relative path in two configs is two files, because a path in a
/// config means what it means relative to that config. Resolving against the working
/// directory would make the answer depend on where the user was standing.
///
/// Written against what a resolution *declares*, so the anchoring rule is pinned by an
/// assertion that does not move when the reading does.
#[test]
fn each_config_resolves_its_own_relative_path() {
  let dir = repo(&[
    ("rune.config.ts", &config(r#"{ test: { command: "vitest", envFile: "./.env" } }"#)),
    ("package.json", "{ \"name\": \"fixture-root\" }\n"),
    (".env", "FROM_ROOT=yes\n"),
    ("packages/legacy/package.json", "{ \"name\": \"legacy\" }\n"),
    (
      "packages/legacy/rune.config.ts",
      &config(r#"{ test: { extends: "test", envFile: "./.env" } }"#),
    ),
    ("packages/legacy/.env", "FROM_PACKAGE=yes\n"),
  ]);

  let loaded = load(&dir.path().join("packages/legacy"), &Environment::default())
    .expect("both files exist and parse");
  let resolved = loaded.resolve("test", Scope::Nearest).expect("resolves").expect("defined");

  let declaring: Vec<String> = resolved
    .env_files
    .iter()
    .map(|declared| {
      rune_config::paths::relative_to(&loaded.discovered.root, declared.source).replace('\\', "/")
    })
    .collect();
  assert_eq!(
    declaring,
    ["packages/legacy/rune.config.ts", "rune.config.ts"],
    "nearest first, and both levels took part"
  );

  let mut files = rune_config::envfile::Files::default();
  let read = |files: &mut rune_config::envfile::Files, index: usize| {
    files
      .read(&loaded.discovered.root, "test", &resolved.env_files[index])
      .expect("both files are there")
      .clone()
  };

  assert_eq!(read(&mut files, 0).assignments[0].name, "FROM_PACKAGE");
  assert_eq!(read(&mut files, 1).assignments[0].name, "FROM_ROOT");
}
