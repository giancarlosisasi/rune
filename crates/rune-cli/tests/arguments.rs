//! What Rune says when a name or an argument does not belong where it was typed.
//!
//! `--root` is the flag a user learns on one command and then types everywhere. Both
//! things that happen next are teaching moments, and both used to be spent saying
//! something the user could not act on.

mod harness;

use harness::{Test, monorepo};

/// The root defines two scripts; the package defines one of its own that the root has
/// never heard of. That last one is what `--root` cannot see.
fn repository() -> Test {
  monorepo(
    r#"{
      build: { command: "tsc -b" },
      test: { command: "vitest run" }
    }"#,
    r#"{ routes: { command: "node routes.mjs" } }"#,
  )
}

fn stderr(output: &std::process::Output) -> String {
  String::from_utf8_lossy(&output.stderr).replace("\r\n", "\n")
}

/// Test R14.1 — the resolution is right and the sentence is not.
///
/// `here` is true without the flag and false with it, and nothing on screen mentions
/// `--root`. So the first conclusion a user draws is that Rune is not reading their
/// config, which sends them to check the filename, the export and the install — everything
/// except the flag they typed.
#[test]
fn a_package_script_asked_for_at_the_root_scope_names_both() {
  let test = repository()
    .args(["inspect", "routes", "--root"])
    .status(1)
    .stderr_regex(r"(?s)repository root");

  let output = test.run_in("packages/legacy");

  insta::with_settings!({ description => "a script only the package defines, asked for with --root" }, {
    insta::assert_snapshot!(stderr(&output));
  });
}

/// Test R14.2 — `run` and `inspect` come through one function, and this is what holds
/// them there.
///
/// The flag comes before the script name on `run`: everything after the name belongs to
/// the command, so `rune run routes --root` would hand `--root` to the child. And each
/// command needs its own fixture, because both have to be run from inside the package —
/// the sentence they are being compared for is the one only that directory produces.
#[test]
fn run_and_inspect_give_the_same_miss() {
  let inspected = repository()
    .args(["inspect", "routes", "--root"])
    .status(1)
    .stderr_regex(r"(?s)repository root")
    .run_in("packages/legacy");

  let run = repository()
    .args(["run", "--root", "routes"])
    .status(1)
    .stderr_regex(r"(?s)repository root")
    .run_in("packages/legacy");

  assert_eq!(
    stderr(&inspected),
    stderr(&run),
    "`run` and `inspect` describe the same miss differently"
  );
}

/// Test R14.3 — naming the scope must not cost the suggestion. A user who mistyped still
/// needs the closest name, and it has to be the closest name *at the scope they asked
/// for*.
#[test]
fn a_name_nothing_defines_names_the_scope_and_still_suggests() {
  let test =
    repository().args(["inspect", "biuld", "--root"]).status(1).stderr_regex(r"(?s)did you mean");

  let output = test.run_in("packages/legacy");

  insta::with_settings!({ description => "a misspelling asked for with --root" }, {
    insta::assert_snapshot!(stderr(&output));
  });
}

/// Test R14.4 — the common case, word for word what it was. The rare case is what this
/// change repairs, and repairing it must not rewrite the message everybody else reads.
#[test]
fn the_default_scope_is_unchanged() {
  let output = repository()
    .args(["inspect", "biuld"])
    .status(1)
    .stderr_regex(r"(?s)did you mean")
    .run_in("packages/legacy");

  insta::with_settings!({ description => "a misspelling at the default scope" }, {
    insta::assert_snapshot!(stderr(&output));
  });
}

/// Test R14.5 — a flag that works on two of five commands gets typed on the other three,
/// and the refusal is the only teaching moment there is.
///
/// `rune init` is the worst of them: it says options exist and names none, so the one
/// command of the three that really has a flag is the one that hides it.
#[test]
fn a_flag_typed_on_a_command_that_does_not_take_it_names_both_sides() {
  let refused =
    repository().args(["init", "--root"]).status(2).stderr_regex(r"(?s)--from-package-json").run();

  insta::with_settings!({ description => "--root typed on a command that has its own options" }, {
    insta::assert_snapshot!(stderr(&refused));
  });

  let no_options =
    repository().args(["list", "--root"]).status(2).stderr_regex(r"(?s)rune run").run();

  insta::with_settings!({ description => "--root typed on a command with no options at all" }, {
    insta::assert_snapshot!(stderr(&no_options));
  });
}

/// Test R14.6 — writing the message ourselves must not quietly turn "you typed it wrong"
/// into "it went wrong". A script reads that code.
#[test]
fn a_usage_error_keeps_its_code_and_its_stream() {
  let output = repository().args(["init", "--root"]).status(2).stderr_regex(r"(?s)init").run();

  assert!(output.stdout.is_empty(), "a usage error wrote to stdout");
}

/// Test R14.7 — one error kind moves. Everything else the parser refuses is what it was,
/// which is the boundary this change lives inside.
#[test]
fn every_other_parsing_failure_is_untouched() {
  let missing = repository().args(["run"]).status(2).stderr_regex(r"(?s)required").run();
  let unknown =
    repository().args(["nonsense"]).status(2).stderr_regex(r"(?s)unrecognized|unexpected").run();
  let help = repository().args(["--help"]).status(0).stdout_regex(r"(?s)Commands:").run();

  insta::with_settings!({ description => "a required argument that is missing" }, {
    insta::assert_snapshot!(stderr(&missing));
  });
  insta::with_settings!({ description => "a subcommand that does not exist" }, {
    insta::assert_snapshot!(stderr(&unknown));
  });

  assert!(help.stderr.is_empty(), "help wrote to stderr");
}
