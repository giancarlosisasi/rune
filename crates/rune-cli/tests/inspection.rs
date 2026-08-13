//! What `rune inspect` explains: the whole tree, where every value came from, and the
//! configs that took no part.
//!
//! The report is the deliverable of this suite, so most of it is snapshots. The two rows
//! that are not are the ones a snapshot cannot decide: what the child actually receives,
//! and what a failure does.

mod harness;

use harness::Test;

/// The fake tool the fixtures call. It reports its own environment, which is the only
/// oracle that can say what a child really got.
const TOOL: &str = "faketool.exe";

/// A repository whose `ci` is the shape the evidence came from: a serial group whose
/// first member is a parallel group, whose second brings a prerequisite and a timeout,
/// and whose third sets two variables.
fn pipeline() -> Test {
  Test::new().config(
    r#"
    export default {
      scripts: {
        lint: { command: "biome lint ." },
        typecheck: { command: "tsc --noEmit" },
        format: { command: "biome format ." },
        check: { parallel: ["lint", "typecheck", "format"], continueOnError: true },
        "clean:all": { command: "rimraf dist" },
        build: { command: "tsc -b", dependsOn: ["clean:all"], timeout: 600000 },
        test: { command: "vitest run", env: { NODE_ENV: "test", TZ: "UTC" } },
        ci: { serial: ["check", "build", "test"] },
      },
    };
    "#,
  )
}

fn reported(output: &std::process::Output) -> String {
  String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n")
}

/// Test R13.1 — eight invocations to understand one script, and no flag that shortens it.
///
/// Every fact here is one `inspect` already holds: asking it about `check` prints all of
/// them. The defect is that asking about `ci` does not.
#[test]
fn a_serial_group_holding_a_parallel_group_reports_both_levels() {
  let output = pipeline().args(["inspect", "ci"]).stdout_regex(r"(?s)resolved through").run();

  insta::with_settings!({ description => "a serial group whose first member is a parallel group" }, {
    insta::assert_snapshot!(reported(&output));
  });
}

/// Test R13.2 — a member's prerequisite, timeout and variables belong on its line.
///
/// The same fixture as R13.1, asked about the member directly, so the two reports can be
/// compared: what `inspect build` says is what `inspect ci` has to say about `build`.
#[test]
fn a_members_own_facts_travel_with_it() {
  let output = pipeline().args(["inspect", "build"]).stdout_regex(r"(?s)resolved through").run();

  insta::with_settings!({ description => "a member with a prerequisite and a timeout" }, {
    insta::assert_snapshot!(reported(&output));
  });
}

/// Test R13.9 — four levels, and one invocation.
#[test]
fn a_four_level_nest_reports_its_whole_shape() {
  let output = Test::new()
    .config(
      r#"
      export default {
        scripts: {
          deepest: { command: "echo deepest" },
          inner: { parallel: ["deepest"] },
          middle: { serial: ["inner"] },
          outer: { parallel: ["middle"] },
          top: { serial: ["outer"] },
        },
      };
      "#,
    )
    .args(["inspect", "top"])
    .stdout_regex(r"(?s)resolved through")
    .run();

  let report = reported(&output);

  for name in ["outer", "middle", "inner", "deepest", "echo deepest"] {
    assert!(report.contains(name), "`{name}` is missing from the report:\n{report}");
  }
}

/// Test R13.11 — it runs twice, so it is reported twice. rune orders scripts and does
/// not deduplicate work, and a report that collapsed the second one would be describing
/// a different run.
#[test]
fn a_script_named_twice_is_reported_twice() {
  let output = Test::new()
    .config(
      r#"
      export default {
        scripts: {
          mark: { command: "echo mark" },
          both: { serial: ["mark", "mark"] },
        },
      };
      "#,
    )
    .args(["inspect", "both"])
    .stdout_regex(r"(?s)resolved through")
    .run();

  let report = reported(&output);
  let members = report.lines().filter(|line| line.trim_start().starts_with("mark ")).count();

  assert_eq!(members, 2, "`mark` runs twice and is reported once:\n{report}");
}

/// Test R13.10 — `inspect` exists to say what `run` would do, so it may not succeed
/// where `run` refuses.
#[test]
fn a_group_whose_distant_member_is_broken_fails_as_run_does() {
  let broken = Test::new()
    .config(
      r#"
      export default {
        scripts: {
          first: { command: "echo one" },
          second: { command: "echo two" },
          ci: { serial: ["first", "second", "nothing-defines-this"] },
        },
      };
      "#,
    )
    .args(["inspect", "ci"])
    .status(1)
    .stderr_regex(r"(?s)nothing-defines-this");

  let inspected = broken.run();

  assert!(inspected.stdout.is_empty(), "a refusal wrote to stdout");

  let run = broken.then_run(&["run", "ci"]);
  assert_eq!(
    String::from_utf8_lossy(&inspected.stderr),
    String::from_utf8_lossy(&run.stderr),
    "`inspect` and `run` disagree about a broken member"
  );
}

/// Test R13.3 — four levels that only change the environment produced four identical
/// lines, none of which named the variable that level contributed.
#[test]
fn a_chain_that_only_changes_the_environment_names_what_each_level_set() {
  let output = Test::new()
    .config(
      r#"
      export default {
        scripts: {
          "probe:1": { command: "node probe.mjs", env: { TZ: "UTC", LEVEL: "one" } },
          "probe:2": { extends: "probe:1", env: { LEVEL: "two" } },
          "probe:3": { extends: "probe:2", env: { NODE_ENV: "test" } },
          "probe:4": { extends: "probe:3", env: { TZ: "CET" } },
        },
      };
      "#,
    )
    .args(["inspect", "probe:4"])
    .stdout_regex(r"(?s)resolved through")
    .run();

  insta::with_settings!({ description => "a chain where every level sets a variable" }, {
    insta::assert_snapshot!(reported(&output));
  });
}

/// Test R13.4 — one value loses to the real environment and one beats it, and the two
/// printed identically. The file's name appeared nowhere at all.
#[test]
fn a_file_value_and_a_map_value_are_told_apart() {
  let test = Test::new()
    .config(
      r#"
      export default {
        scripts: {
          "start:api": { command: "node server.mjs", envFile: ".env.shared" },
          "dev:api": { command: "node server.mjs", env: { LOG_LEVEL: "info" } },
        },
      };
      "#,
    )
    .file(".env.shared", "LOG_LEVEL=info\n")
    .stdout_regex(r"(?s)resolved through")
    .args(["inspect", "start:api"]);

  let from_file = test.run();
  let from_map = test.then_run(&["inspect", "dev:api"]);

  insta::with_settings!({ description => "the value comes from an env file" }, {
    insta::assert_snapshot!(reported(&from_file));
  });
  insta::with_settings!({ description => "the value comes from the script's own env map" }, {
    insta::assert_snapshot!(reported(&from_map));
  });
}

/// Test R13.5 — the row used to vanish, so the user was told what was dropped and never
/// told what the child gets instead.
#[test]
fn where_the_process_environment_wins_the_childs_value_is_shown() {
  let output = Test::new()
    .config(
      r#"
      export default {
        scripts: { probe: { command: "node probe.mjs", envFile: ".env" } },
      };
      "#,
    )
    .file(".env", "LOG_LEVEL=from-the-file\n")
    .env("LOG_LEVEL", "from-the-environment")
    .args(["inspect", "probe"])
    .stdout_regex(r"(?s)resolved through")
    .run();

  let report = reported(&output);

  assert!(
    report.contains("LOG_LEVEL=from-the-environment"),
    "the value the child will get is missing:\n{report}"
  );
  assert!(
    report.contains("the process environment"),
    "the winning value is not attributed:\n{report}"
  );
  assert!(report.contains("was ignored"), "the assignment that lost is missing:\n{report}");
}

/// A value holding a newline, and one of a megabyte. Both reach the child correctly; it
/// is the report that could not survive them.
fn awkward_values() -> Test {
  Test::new()
    .config(&format!(
      r#"
      export default {{
        scripts: {{ probe: {{ command: "{TOOL} report-env", envFile: ".env" }} }},
      }};
      "#
    ))
    .tool(&format!("node_modules/.bin/{TOOL}"))
    .file(".env", &format!("LINES=\"one\ntwo\"\nBIG={}\n", "x".repeat(1_048_576)))
}

/// Test R13.6 — a two-line value left its column and read as a third key with no name; a
/// one-megabyte value put the whole report off the screen.
#[test]
fn a_value_cannot_break_the_report() {
  let output =
    awkward_values().args(["inspect", "probe"]).stdout_regex(r"(?s)resolved through").run();

  let report = reported(&output);

  assert!(report.len() < 2000, "one value buried the report: {} bytes", report.len());
  assert!(report.contains("1048576 characters"), "the true length is missing:\n{report}");

  insta::with_settings!({ description => "values the report has to survive" }, {
    insta::assert_snapshot!(report);
  });
}

/// Test R13.7 — the row the rest of this file rests on. Every snapshot above would pass
/// just as well if the shortening reached the child, so the oracle here is the child's
/// own report of the environment it was given.
#[test]
fn the_child_still_receives_what_the_report_shortened() {
  let output = awkward_values().args(["run", "probe"]).stdout_regex(r"(?s)^\{.*\}\n$").run();

  let report: serde_json::Value =
    serde_json::from_slice(&output.stdout).expect("rune-testkit reports JSON");
  let environment = &report["env"];

  assert_eq!(environment["LINES"], "one\ntwo");
  assert_eq!(
    environment["BIG"].as_str().map(str::len),
    Some(1_048_576),
    "the child received a shortened value"
  );
}

/// Test R13.8 — the only silent wrong answer in the whole session.
///
/// `apps/rune.config.ts` is the obvious place for something two applications share. It
/// works perfectly from `apps`, which is where its author tests it, and does nothing from
/// the two directories where the work happens.
#[test]
fn a_config_that_took_no_part_is_named() {
  let test = Test::new()
    .config("export default { scripts: { lint: { command: \"biome lint .\" } } };\n")
    .file("package.json", "{ \"name\": \"root\", \"private\": true }\n")
    .file(
      "apps/rune.config.ts",
      "export default { scripts: { shared: { command: \"echo shared\" } } };\n",
    )
    .file("apps/api/package.json", "{ \"name\": \"api\" }\n")
    .file(
      "apps/api/rune.config.ts",
      "export default { scripts: { routes: { command: \"echo routes\" } } };\n",
    )
    .args(["inspect", "lint"])
    .stdout_regex(r"(?s)resolved through");

  let passed_over = test.run_in("apps/api");

  insta::with_settings!({ description => "a config between the package and the root" }, {
    insta::assert_snapshot!(reported(&passed_over));
  });

  let nothing_between = Test::new()
    .config("export default { scripts: { lint: { command: \"biome lint .\" } } };\n")
    .args(["inspect", "lint"])
    .stdout_regex(r"(?s)resolved through")
    .run();

  assert!(
    !reported(&nothing_between).contains("took no part"),
    "a report said something about configs when there were none:\n{}",
    reported(&nothing_between)
  );
}

/// Test R16.1 — the stream a command's product lands on is the promise a pipe is built
/// on, and the only way to learn it today is to run it and guess.
///
/// `inspect` spawns nothing, so no child owns stdout, and its report exists to be piped,
/// grepped and pasted into a pull request.
#[test]
fn a_successful_inspection_puts_its_whole_report_on_stdout() {
  let output = pipeline().args(["inspect", "build"]).stdout_regex(r"(?s)tsc -b").status(0).run();

  assert!(!output.stdout.is_empty(), "the report must be on stdout");
  assert!(
    output.stderr.is_empty(),
    "a successful query wrote to stderr:\n{}",
    String::from_utf8_lossy(&output.stderr)
  );
}

/// Test R16.2 — the half that stops the rule collapsing into "inspect writes to stdout".
///
/// What lands on stdout is the product. A complaint is not a product, and a script
/// reading the stream must never receive one.
#[test]
fn a_failed_inspection_is_a_diagnostic_and_leaves_stdout_empty() {
  let output = pipeline()
    .args(["inspect", "nothing-defines-this"])
    .stderr_regex(r"(?s)nothing-defines-this")
    .status(1)
    .run();

  assert!(output.stdout.is_empty(), "a refusal wrote to stdout");
}

/// Test R19.6 — the one thing about a command a reader cannot judge from the command
/// text: whether it starts rune again, and whether the script it starts is this one.
///
/// The three cases only mean something beside each other. A nested call is useful and a
/// script that runs itself is a defect, and they look identical on the `command` line.
#[test]
fn a_command_that_runs_rune_is_named_as_one() {
  let repository = Test::new().config(
    r#"
    export default {
      scripts: {
        release: { command: "rune run build" },
        loop: { command: "rune run loop" },
        build: { command: "tsc -b" },
      },
    };
    "#,
  );

  let nested = repository.then_run(&["inspect", "release"]);
  let itself = repository.then_run(&["inspect", "loop"]);
  let ordinary = repository.then_run(&["inspect", "build"]);

  for output in [&nested, &itself, &ordinary] {
    assert!(output.status.success(), "an inspection failed:\n{}", reported(output));
  }

  insta::with_settings!({ description => "a command that runs rune on another script" }, {
    insta::assert_snapshot!(reported(&nested));
  });
  insta::with_settings!({ description => "a command that runs rune on this same script" }, {
    insta::assert_snapshot!(reported(&itself));
  });

  assert!(
    !reported(&ordinary).contains("runs rune"),
    "an ordinary command was reported as running rune:\n{}",
    reported(&ordinary)
  );
}
