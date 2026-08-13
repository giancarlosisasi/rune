//! `envFile` through the real binary: what reaches the child, and what is refused.
//!
//! The layering rules themselves are unit-tested in `rune-exec`. What only the binary can
//! show is that a real child process received the environment those rules describe, and
//! that the warnings a user depends on to notice a dropped assignment actually appear on
//! stderr.

mod harness;

use harness::{Test, assert_same_path};

/// The fake tool every fixture calls, so the environment is read out of a real child
/// rather than out of rune's own idea of what it built.
const TOOL: &str = "faketool.exe";

fn config(scripts: &str) -> String {
  format!("export default {{ scripts: {scripts} }};\n")
}

/// A repository whose one script reports the environment it was given.
///
/// `fields` are spliced in after the command, which is how each test adds the `envFile`
/// or `env` it is about.
fn fixture(fields: &str) -> Test {
  Test::new()
    .config(&config(&format!(r#"{{ probe: {{ command: "{TOOL} report-env", {fields} }} }}"#)))
    .tool(&format!("node_modules/.bin/{TOOL}"))
    .args(["run", "probe"])
    .stdout_regex(r"(?s)^\{.*\}\n$")
    .status(0)
}

fn env_of(output: &std::process::Output) -> serde_json::Value {
  let report: serde_json::Value =
    serde_json::from_slice(&output.stdout).expect("rune-testkit reports JSON");

  report["env"].clone()
}

fn stderr_of(output: &std::process::Output) -> String {
  String::from_utf8_lossy(&output.stderr).replace("\r\n", "\n")
}

/// Test 4c.2 — the rule the whole change exists to enforce.
///
/// CI exports `CI=true`, a committed file says `CI=false`, and the build has to behave
/// like CI. The warning is half the row: a rule nobody can observe is a rule that gets
/// reported as "rune ignores my .env file".
#[test]
fn an_env_file_never_overrides_the_process_environment() {
  let output = fixture(r#"envFile: ".env""#)
    .file(".env", "CI=false\n")
    .env("CI", "true")
    .stderr_regex(r"^warning: ")
    .run();

  assert_eq!(env_of(&output)["CI"], "true", "the process environment must win");
  assert!(
    !String::from_utf8_lossy(&output.stdout).contains("warning"),
    "stdout belongs to the child, so everything rune says about itself goes to stderr"
  );

  insta::with_settings!({ description => "a file assigning a variable the environment already has" }, {
    insta::assert_snapshot!(stderr_of(&output));
  });
}

/// Test 4c.1 — the feature itself. An assignment that was applied says nothing, which is
/// what keeps the warnings worth reading.
#[test]
fn an_env_file_fills_a_variable_the_environment_lacks() {
  let output = fixture(r#"envFile: ".env""#)
    .file(".env", "API_URL=https://example.test\n")
    .env_remove("API_URL")
    .run();

  assert_eq!(env_of(&output)["API_URL"], "https://example.test");
}

/// Test 4c.3 — the `env` map is the config's own last word, so it wins even where the
/// process environment has nothing to say and the file was free to fill the gap.
#[test]
fn the_env_map_beats_the_env_file() {
  let output = fixture(r#"envFile: ".env", env: { FOO: "map" }"#)
    .file(".env", "FOO=file\n")
    .env_remove("FOO")
    .stderr_regex(
      r"^warning: `FOO` on line 1 of `\.env` was ignored: this script's `env` already sets it",
    )
    .run();

  assert_eq!(env_of(&output)["FOO"], "map");
}

/// Test 4c.4 — the prefix rune keeps for itself, refused from both places a config can
/// write a variable. The mistake is the same either way, so the treatment is too.
///
/// The values are what makes the row matter: a script told it is running at a root it is
/// not would mislead every tool downstream that reads these.
#[test]
fn the_rune_prefix_is_refused_from_a_file_and_from_an_env_map() {
  let test = fixture(r#"envFile: ".env", env: { RUNE_ROOT: "somewhere-else" }"#)
    .file(".env", "RUNE_SCRIPT_NAME=another-script\n")
    .stderr_regex(r"(?s)^warning: .*\nwarning: ");

  let root = test.dir().to_path_buf();
  let output = test.run();
  let env = env_of(&output);

  assert_eq!(env["RUNE_SCRIPT_NAME"], "probe", "rune's own value must survive");
  assert_same_path(env["RUNE_ROOT"].as_str(), &root, "rune's own value must survive");

  insta::with_settings!({ description => "the reserved prefix, attempted from both sources" }, {
    insta::assert_snapshot!(stderr_of(&output));
  });
}

/// Test R19.5 — the count of how deep rune has started rune is refused from a config in
/// exactly the way the other reserved names are.
///
/// It is the one reserved value a config would gain something by writing: a config that
/// could reset the count could keep a runaway going for ever. The prefix already covers
/// the name, and this is what says so rather than leaving it to be assumed.
#[test]
fn the_depth_cannot_be_reset_from_a_file_or_from_an_env_map() {
  let output = fixture(r#"envFile: ".env", env: { RUNE_DEPTH: "0" }"#)
    .file(".env", "RUNE_DEPTH=0\n")
    .env("RUNE_DEPTH", "4")
    .stderr_regex(r"(?s)^warning: .*\nwarning: ")
    .run();

  assert_eq!(env_of(&output)["RUNE_DEPTH"], "5", "rune's own count must survive");

  insta::with_settings!({ description => "the count, attempted from both sources" }, {
    insta::assert_snapshot!(stderr_of(&output));
  });
}

/// A repository where one script names a file that is not there, which is what cloning a
/// repository whose `.env` is in `.gitignore` gives you.
fn with_a_missing_file() -> Test {
  Test::new()
    .config(&config(&format!(
      r#"{{
        lint: {{ command: "{TOOL} touch linted.txt" }},
        start: {{ command: "{TOOL} report-env", envFile: ".env" }},
      }}"#
    )))
    .tool(&format!("node_modules/.bin/{TOOL}"))
}

/// Test R11.1 — listing is the command a stuck user runs to find out what they can run,
/// and it was the first thing to stop working.
#[test]
fn listing_survives_a_file_that_is_not_there() {
  let output =
    with_a_missing_file().args(["list"]).stdout_regex(r"(?s)lint.*start").status(0).run();

  assert!(String::from_utf8_lossy(&output.stdout).contains("start"), "every script is listed");
}

/// Test R11.2 — `lint` cannot reach `start`'s file by any path, so nothing about that
/// file is `lint`'s business.
#[test]
fn an_unrelated_script_still_runs() {
  let test = with_a_missing_file().args(["run", "lint"]).status(0);
  let directory = test.dir().to_path_buf();

  test.run();

  assert!(directory.join("linted.txt").exists(), "`lint` must run");
}

/// Test R11.3 — the message itself is one of the best in the product, and nothing about
/// it changes. Only its reach does.
#[test]
fn the_declaring_script_refuses_exactly_as_it_did() {
  let test = with_a_missing_file().args(["run", "start"]).status(1).stderr_regex(r"(?s)envFile");
  let directory = test.dir().to_path_buf();

  let output = test.run();

  insta::with_settings!({ description => "the script that declares the missing file" }, {
    insta::assert_snapshot!(harness::redact(&directory, &stderr_of(&output)));
  });
}

/// Test R11.4 — the rule this change must not lose: everything a name can reach is read
/// before anything starts.
///
/// The oracle is the marker the first member would have written. An assertion on the
/// message alone passes while the first two members have already run and had effects,
/// which is the failure the eager read existed to prevent.
#[test]
fn a_group_refuses_before_its_first_member_spawns() {
  let test = Test::new()
    .config(&config(&format!(
      r#"{{
        first: {{ command: "{TOOL} touch first-ran.txt" }},
        second: {{ command: "{TOOL} touch second-ran.txt" }},
        third: {{ command: "{TOOL} report-env", envFile: ".env" }},
        all: {{ serial: ["first", "second", "third"] }},
      }}"#
    )))
    .tool(&format!("node_modules/.bin/{TOOL}"))
    .args(["run", "all"])
    .status(1)
    .stderr_regex(r"(?s)`third`");
  let directory = test.dir().to_path_buf();

  test.run();

  assert!(!directory.join("first-ran.txt").exists(), "the first member must never start");
  assert!(!directory.join("second-ran.txt").exists(), "nor the second");
}

/// Test R10.4 — the same name twice in one file. The report named one file as both the
/// winner and the loser, with no line numbers, so the only thing it told a user was that
/// they had to go and find both occurrences themselves.
#[test]
fn a_key_repeated_in_one_file_names_both_lines() {
  let output = fixture(r#"envFile: ".env""#)
    .file(".env", "DUP=first\nOTHER=1\nDUP=second\n")
    .env_remove("DUP")
    .stderr_regex(r"^warning: ")
    .run();

  assert_eq!(env_of(&output)["DUP"], "first", "the first assignment still wins");

  insta::with_settings!({ description => "the same name assigned twice in one file" }, {
    insta::assert_snapshot!(stderr_of(&output));
  });
}

/// Test 4c.9 — the precedence rule made visible on demand rather than only in passing.
///
/// A delta that showed only what got through would leave a user who just edited `.env`
/// unable to tell that the edit did nothing. Nothing is warned here: `inspect` reports,
/// and the report is the whole product of the command.
#[test]
fn inspect_lists_the_environment_delta_and_what_was_dropped() {
  let script = r#"{ probe: {
    command: "vitest",
    envFile: ".env",
    env: { FOO: "map", RUNE_ROOT: "somewhere-else" }
  } }"#;

  let output = Test::new()
    .config(&config(script))
    .file(".env", "API_URL=https://example.test\nCI=false\nFOO=file\n")
    .env("CI", "true")
    .env_remove("FOO")
    .env_remove("API_URL")
    .args(["inspect", "probe"])
    .stdout_regex(r"(?s)ignored")
    .status(0)
    .run();

  insta::with_settings!({ description => "a script whose file is half applied and half refused" }, {
    insta::assert_snapshot!(String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n"));
  });
}
