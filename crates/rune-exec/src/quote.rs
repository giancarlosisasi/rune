//! Making one argument survive one shell.
//!
//! Both of the runners this project studied quote pass-through arguments with a POSIX
//! quoter whatever shell is about to read them — concurrently in
//! `command-parser/expand-arguments.ts`, just in its own sh-only `quote()`. That is what
//! happens when quoting is written once, against the shell the author's own machine
//! runs, so the shell is an argument here and the table that tests it runs everywhere.

use std::ffi::OsStr;
use std::path::Path;

use crate::shell::{self, ShellKind};

/// How many readers an argument has to survive on its way to the child.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reads {
  /// The shell reads the line, and the child's own startup code splits what it is handed.
  Once,
  /// A batch file re-reads its arguments through `%*` after `cmd.exe` has already read
  /// the line, so everything cmd acts on has to survive being read twice.
  Twice,
}

/// How many readers the arguments appended to `command` will pass through.
///
/// Decided from the file `PATH` resolution actually chooses, never from the command text:
/// a config writes `biome`, the file is `biome.CMD`, and nobody writes the extension. The
/// `PATH` to search is the child's own, `node_modules/.bin` directories included, because
/// that is the one the shell will search.
pub fn reads(
  command: &str,
  shell: ShellKind,
  path: Option<&OsStr>,
  path_extensions: Option<&OsStr>,
) -> Reads {
  if shell != ShellKind::Cmd {
    return Reads::Once;
  }

  let resolved = shell::leading_program(command)
    .and_then(|program| shell::locate(Path::new(program), path, path_extensions));

  match resolved {
    Some(file) if is_batch_file(&file) => Reads::Twice,
    _ => Reads::Once,
  }
}

fn is_batch_file(file: &Path) -> bool {
  file.extension().is_some_and(|extension| {
    extension.eq_ignore_ascii_case("cmd") || extension.eq_ignore_ascii_case("bat")
  })
}

/// `command` with `arguments` appended, each one quoted for `shell` and for `reads`.
///
/// Nothing is appended for an empty list, so a bare `--` leaves the command exactly as
/// the config wrote it.
pub fn command_line(command: &str, arguments: &[String], shell: ShellKind, reads: Reads) -> String {
  let mut line = command.to_owned();
  for argument in arguments {
    line.push(' ');
    line.push_str(&quote(argument, shell, reads));
  }

  line
}

/// One element, quoted so `shell` hands it to the child as a single argument with its
/// content intact.
pub fn quote(element: &str, shell: ShellKind, reads: Reads) -> String {
  match shell {
    ShellKind::Posix => posix(element),
    ShellKind::Cmd => cmd(element, reads),
    ShellKind::PowerShell => power_shell(element),
  }
}

/// Every character that stops a word from being one bare word to a POSIX shell.
const POSIX_SPECIAL: &[char] = &[
  '\t', '\n', '\r', ' ', '"', '#', '$', '&', '\'', '(', ')', '*', ';', '<', '>', '?', '\\', '`',
  '|', '~',
];

/// The characters `cmd.exe` acts on rather than passes along.
const CMD_SPECIAL: &[char] = &[' ', '!', '%', '^', '&', '(', ')', '<', '>', '|', '"'];

/// Single quotes, because they are the only construct a POSIX shell does not look inside.
///
/// A single quote cannot appear within them, so it leaves and returns: `'` closes the
/// run, `\'` is the literal character, and `'` opens the next run. The two cleanups after
/// that remove the empty runs this leaves at the edges — npm's quoter does the same, and
/// matching it is what keeps the parity suite honest.
fn posix(element: &str) -> String {
  if element.is_empty() {
    return "''".to_owned();
  }

  if !element.contains(POSIX_SPECIAL) {
    return element.to_owned();
  }

  let quoted = format!("'{}'", element.replace('\'', r"'\''"));
  drop_leading_empty_runs(&quoted).replace(r"\'''", r"\'")
}

/// Removes the `''` pairs a leading single quote produces, unless that would leave
/// nothing at all.
fn drop_leading_empty_runs(quoted: &str) -> &str {
  let mut start = 0;
  while start + 2 < quoted.len() && quoted[start..].starts_with("''") {
    start += 2;
  }

  &quoted[start..]
}

/// Two escapes, because two parsers read the line — and three when the child is a batch
/// file, because it re-reads its own arguments after cmd has finished with them.
///
/// The child's own startup code splits the command line by the MSVC rules — double
/// quotes group, backslashes only matter in front of a quote — and `cmd.exe` reads the
/// line before that, acting on its own metacharacters wherever they appear. So the
/// element is first quoted for the child, then every character cmd would act on is
/// prefixed with `^` for cmd.
///
/// `%*` inside a batch file expands to text that cmd then reads again, so a second `^`
/// round is what leaves the first one intact for that second reading. Without it an `&`
/// in a filename ends the command and runs whatever followed it.
///
/// A batch file that calls another batch file would need a fourth round. That is
/// unbounded, and no runner in this ecosystem attempts it.
fn cmd(element: &str, reads: Reads) -> String {
  if element.is_empty() {
    // A bare pair of quotes: cmd hands it on, and the child's own parser reads it as the
    // empty argument. Prefixing them with `^` would work too, and npm does not, so this
    // does not either — the parity suite compares these byte for byte.
    return "\"\"".to_owned();
  }

  let quoted = if element.contains([' ', '\t', '\n', '\u{b}', '"']) {
    quote_for_msvc(element)
  } else {
    element.to_owned()
  };

  let escaped = escape_for_cmd(&quoted);
  match reads {
    Reads::Once => escaped,
    Reads::Twice => escape_for_cmd(&escaped),
  }
}

/// Every character cmd acts on, prefixed with the one character that stops it acting.
fn escape_for_cmd(text: &str) -> String {
  let mut escaped = String::with_capacity(text.len());
  for character in text.chars() {
    if CMD_SPECIAL.contains(&character) {
      escaped.push('^');
    }
    escaped.push(character);
  }

  escaped
}

/// The MSVC command-line convention: a backslash run is literal unless a quote follows
/// it, in which case the run is doubled and the quote escaped.
fn quote_for_msvc(element: &str) -> String {
  let mut quoted = String::with_capacity(element.len() + 2);
  quoted.push('"');

  let mut backslashes = 0;
  for character in element.chars() {
    match character {
      '\\' => backslashes += 1,
      '"' => {
        for _ in 0..=backslashes * 2 {
          quoted.push('\\');
        }
        backslashes = 0;
        quoted.push('"');
      }
      _ => {
        for _ in 0..backslashes {
          quoted.push('\\');
        }
        backslashes = 0;
        quoted.push(character);
      }
    }
  }

  // A run touching the closing quote would escape it, so it is doubled too.
  for _ in 0..backslashes * 2 {
    quoted.push('\\');
  }
  quoted.push('"');

  quoted
}

/// Single quotes again, but PowerShell's own kind: it looks inside nothing there, and a
/// single quote is written twice.
///
/// Every element is quoted, with no bare case. PowerShell's metacharacter set is long
/// enough that a list of exceptions is a list of future bugs, and quoting a word that
/// did not need it costs nothing.
fn power_shell(element: &str) -> String {
  format!("'{}'", element.replace('\'', "''"))
}

#[cfg(test)]
mod tests {
  use std::ffi::OsString;

  use super::{Reads, ShellKind, command_line, quote, reads};

  /// Test 4a.6 — one element, three shells, three answers.
  ///
  /// Each expected string is what that shell has to be handed for the child to receive
  /// the element unchanged. Spelling them out is what lets a Linux machine hold rune to
  /// the `cmd.exe` rules, which is the only way the rule set that is wrong in both
  /// reference implementations stays right in this one.
  #[test]
  fn one_element_is_quoted_three_different_ways() {
    // element, posix, cmd, powershell
    let table = [
      ("--watch", "--watch", "--watch", "'--watch'"),
      ("a b", "'a b'", r#"^"a^ b^""#, "'a b'"),
      ("it's", r"'it'\''s'", "it's", "'it''s'"),
      ("say \"hi\"", r#"'say "hi"'"#, r#"^"say^ \^"hi\^"^""#, "'say \"hi\"'"),
      ("a&b", r"'a&b'", "a^&b", "'a&b'"),
      ("$HOME", "'$HOME'", "$HOME", "'$HOME'"),
      ("%PATH%", "%PATH%", "^%PATH^%", "'%PATH%'"),
      ("a|b", "'a|b'", "a^|b", "'a|b'"),
      ("--", "--", "--", "'--'"),
      ("", "''", "\"\"", "''"),
      ("'quoted'", r"\''quoted'\'", "'quoted'", "'''quoted'''"),
    ];

    for (element, posix, cmd, power_shell) in table {
      assert_eq!(quote(element, ShellKind::Posix, Reads::Once), posix, "posix: {element:?}");
      assert_eq!(quote(element, ShellKind::Cmd, Reads::Once), cmd, "cmd: {element:?}");
      assert_eq!(
        quote(element, ShellKind::PowerShell, Reads::Once),
        power_shell,
        "powershell: {element:?}"
      );
    }
  }

  /// A backslash run before a quote is the case the MSVC rules exist for: without
  /// doubling it, the child sees an escaped quote instead of a closing one.
  #[test]
  fn backslashes_before_a_quote_are_doubled_for_cmd() {
    assert_eq!(quote(r#"a\"b c"#, ShellKind::Cmd, Reads::Once), r#"^"a\\\^"b^ c^""#);
    assert_eq!(quote(r"a\b c", ShellKind::Cmd, Reads::Once), r#"^"a\b^ c^""#);
    assert_eq!(quote(r"trailing\", ShellKind::Cmd, Reads::Once), r"trailing\");
  }

  /// A second reader means a second round, over the result of the first. The `^` that
  /// protects the metacharacter is itself a metacharacter, so it is what the second round
  /// mostly protects.
  #[test]
  fn a_second_reader_gets_a_second_round() {
    let table = [
      ("--watch", "--watch"),
      ("a&b", "a^^^&b"),
      ("a|b", "a^^^|b"),
      ("a<b>c", "a^^^<b^^^>c"),
      ("a^b", "a^^^^b"),
      ("a b", r#"^^^"a^^^ b^^^""#),
      ("", "\"\""),
    ];

    for (element, expected) in table {
      assert_eq!(quote(element, ShellKind::Cmd, Reads::Twice), expected, "{element:?}");
    }
  }

  /// Only a batch child is read twice, and only cmd reads a line the way that matters.
  /// Everything else answers `Once`, which is the behaviour that was already correct.
  #[test]
  fn only_a_batch_file_child_is_read_twice() {
    let directory = tempfile::tempdir().expect("create tempdir");
    // `biome` beside `biome.CMD` is what a package manager actually writes: a shell script
    // no Windows process can start, and the batch file that runs instead of it.
    for name in ["biome", "biome.CMD", "legacy.bat", "real.exe"] {
      std::fs::write(directory.path().join(name), "").expect("write the fixture");
    }

    let path = OsString::from(directory.path());
    let extensions = OsString::from(".COM;.EXE;.BAT;.CMD");
    let of = |command: &str, shell| reads(command, shell, Some(&path), Some(&extensions));

    if cfg!(windows) {
      assert_eq!(of("biome lint .", ShellKind::Cmd), Reads::Twice);
      assert_eq!(of("legacy", ShellKind::Cmd), Reads::Twice);
      assert_eq!(of("real --flag", ShellKind::Cmd), Reads::Once, "a real executable reads once");
      assert_eq!(of("absent", ShellKind::Cmd), Reads::Once, "an unresolvable child reads once");
      assert_eq!(of("| biome", ShellKind::Cmd), Reads::Once, "an operator stops the reading");
    }

    assert_eq!(of("biome lint .", ShellKind::Posix), Reads::Once, "posix is one reader");
    assert_eq!(of("biome lint .", ShellKind::PowerShell), Reads::Once);
  }

  #[test]
  fn an_empty_argument_list_leaves_the_command_alone() {
    assert_eq!(command_line("vitest --run", &[], ShellKind::Posix, Reads::Once), "vitest --run");
  }

  #[test]
  fn arguments_are_appended_in_order_after_the_command() {
    let arguments = ["--reporter".to_owned(), "a b".to_owned()];

    assert_eq!(
      command_line("vitest", &arguments, ShellKind::Posix, Reads::Once),
      "vitest --reporter 'a b'"
    );
  }
}
