//! What evaluating a config may spend, through the real binary.
//!
//! Every fixture here names a limit of its own in the environment. Waiting for rune's own
//! default would cost seconds per row for nothing, and a row that names a limit also shows
//! that the variable is read — which is the escape hatch a config that genuinely needs
//! longer depends on.
//!
//! None of these could be written before the ceilings existed. A row describing what rune
//! did with an endless config would not fail; it would run until the test runner killed
//! it, which reports a defect in the test.

mod harness;

use std::process::Output;

use harness::{Test, redact};

const TIME_LIMIT: &str = "RUNE_CONFIG_TIME_LIMIT_MS";
const MEMORY_LIMIT: &str = "RUNE_CONFIG_MEMORY_LIMIT_MB";

/// Far above any scheduling hiccup on a shared runner, and far below what a row can afford
/// to wait for.
const SHORT_TIME: &str = "250";

/// Filled in milliseconds by a config that does not stop, and far above the megabyte a
/// runtime holds with the whole config graph loaded.
const SMALL_HEAP: &str = "32";

const ENDLESS: &str = r#"
    while (true) {}
    export default { scripts: { dev: { command: "vite" } } };
"#;

const ENDLESS_ALLOCATION: &str = r#"
    const growing: string[] = [];
    while (true) {
      growing.push("x".repeat(1024));
    }
    export default { scripts: { dev: { command: "vite" } } };
"#;

fn stderr_of(output: &Output) -> String {
  String::from_utf8_lossy(&output.stderr).replace("\r\n", "\n")
}

/// A repository whose config never finishes being evaluated, asked the cheapest question
/// there is.
fn never_finishes() -> Test {
  Test::new()
    .config(ENDLESS)
    .env(TIME_LIMIT, SHORT_TIME)
    .args(["list"])
    .stdout("")
    .stderr_regex(r"did not finish being evaluated within \d+ ms")
    .status(1)
}

/// Test R25.1 — the runaway ends by itself and says which file to open.
///
/// Before this, one core sat at 100 % with nothing on either stream for as long as anyone
/// was willing to wait, and every rune command in the repository was dead while it lasted.
#[test]
fn a_config_that_never_finishes_is_stopped_and_named() {
  let test = never_finishes();
  let output = test.run();

  insta::with_settings!({ description => "a config holding a loop that never ends" }, {
    insta::assert_snapshot!(redact(test.dir(), &stderr_of(&output)));
  });
}

/// Test R25.2 — the file named is the file the engine was actually in.
///
/// The entry config is what a message with no mechanism behind it would name every time,
/// so this is the row that tells the two apart.
#[test]
fn the_file_that_did_not_finish_is_the_one_named() {
  let test = Test::new()
    .file("scripts/helpers.ts", "export function port(): number {\n  while (true) {}\n}\n")
    .config(
      r#"
        import { port } from "./scripts/helpers";
        export default { scripts: { dev: { command: `vite --port ${port()}` } } };
        "#,
    )
    .env(TIME_LIMIT, SHORT_TIME)
    .args(["list"])
    .stdout("")
    .stderr_regex(r"did not finish being evaluated within \d+ ms")
    .status(1);

  let reported = redact(test.dir(), &stderr_of(&test.run()));

  assert!(
    reported.trim_start().starts_with("scripts/helpers.ts"),
    "the helper is where the engine was, and it is what the message must name:\n{reported}"
  );
}

/// Test R25.3 — an allocation that never stops is rune's to refuse.
///
/// The exit code is half the oracle. Left to the operating system this ends as a kill —
/// 137 under a container ceiling, and on Windows the machine's commit limit stops it with
/// nothing written at all. Either would pass a text comparison by never reaching it.
#[test]
fn a_config_that_allocates_without_stopping_is_stopped_and_named() {
  let test = Test::new()
    .config(ENDLESS_ALLOCATION)
    .env(MEMORY_LIMIT, SMALL_HEAP)
    .args(["list"])
    .stdout("")
    .stderr_regex(r"asked for more memory than a config may use")
    .status(1);

  let output = test.run();

  insta::with_settings!({ description => "a config allocating without end" }, {
    insta::assert_snapshot!(redact(test.dir(), &stderr_of(&output)));
  });
}

/// Test R25.4 — the way out, in both directions.
///
/// One config that takes a known time to evaluate, refused under a limit below it and
/// loaded under a limit above it. A variable that is read and a variable that is obeyed
/// are different claims, and one run can only make the first.
#[test]
fn the_time_limit_is_adjustable_in_both_directions() {
  let slow = r#"
      const until: number = Date.now() + 400;
      while (Date.now() < until) {}
      export default { scripts: { dev: { command: "vite" } } };
  "#;

  let test = Test::new()
    .config(slow)
    .env(TIME_LIMIT, "100")
    .args(["list"])
    .stdout("")
    .stderr_regex(r"did not finish being evaluated within 100 ms")
    .status(1);
  test.run();

  Test::new().config(slow).env(TIME_LIMIT, "20000").args(["list"]).stdout("dev\n").status(0).run();
}

/// Test R25.5 — the config is evaluated before any command does its own work, so all three
/// have to answer the same way.
///
/// A ceiling that only `run` applied would leave a stuck user with no command to run to
/// find out why.
#[test]
fn every_command_reports_the_ceiling_the_same_way() {
  let test = never_finishes();
  let listed = test.run();

  for arguments in [["inspect", "dev"], ["run", "dev"]] {
    let other = test.then_run(&arguments);

    assert_eq!(other.status.code(), Some(1), "rune {}", arguments.join(" "));
    assert!(other.stdout.is_empty(), "rune {} wrote to stdout", arguments.join(" "));
    assert_eq!(stderr_of(&other), stderr_of(&listed), "rune {}", arguments.join(" "));
  }
}

/// Test R25.6 — what neither ceiling may catch.
///
/// A config that awaits something that settles is legal and finishes. The oracle is the
/// child, not the report: the awaited value has to reach the command that runs.
#[test]
fn a_config_that_awaits_something_that_settles_still_loads() {
  const TOOL: &str = "faketool.exe";

  Test::new()
    .file(
      "scripts/helpers.ts",
      "export async function port(): Promise<number> {\n  return await Promise.resolve(4000);\n}\n",
    )
    .config(&format!(
      r#"
      import {{ port }} from "./scripts/helpers";
      const resolved: number = await port();
      export default {{ scripts: {{ dev: {{ command: `{TOOL} emit ${{resolved}}` }} }} }};
      "#
    ))
    .tool(&format!("node_modules/.bin/{TOOL}"))
    .args(["run", "dev"])
    .stdout("4000")
    .status(0)
    .run();
}

/// Test R25.8 — a ceiling is a property of the machine and the moment.
///
/// Caching one would serve a refusal to a machine that had the time or the memory to
/// succeed, which is the one thing the cache is never allowed to cost.
#[test]
fn neither_ceiling_leaves_a_cache_entry() {
  for test in [
    never_finishes(),
    Test::new()
      .config(ENDLESS_ALLOCATION)
      .env(MEMORY_LIMIT, SMALL_HEAP)
      .args(["list"])
      .stdout("")
      .stderr_regex(r"asked for more memory than a config may use")
      .status(1),
  ] {
    test.run();

    let cache = test.dir().join("node_modules/.cache/rune");
    let entries = std::fs::read_dir(&cache).map_or(0, Iterator::count);

    assert_eq!(entries, 0, "a refused evaluation left something at {}", cache.display());
  }
}
