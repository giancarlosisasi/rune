//! Making one argument survive one shell.
//!
//! Both of the runners this project studied quote pass-through arguments with a POSIX
//! quoter whatever shell is about to read them — concurrently in
//! `command-parser/expand-arguments.ts`, just in its own sh-only `quote()`. That is what
//! happens when quoting is written once, against the shell the author's own machine
//! runs, so the shell is an argument here and the table that tests it runs everywhere.

use crate::shell::ShellKind;

/// `command` with `arguments` appended, each one quoted for `shell`.
///
/// Nothing is appended for an empty list, so a bare `--` leaves the command exactly as
/// the config wrote it.
pub fn command_line(command: &str, arguments: &[String], shell: ShellKind) -> String {
  let mut line = command.to_owned();
  for argument in arguments {
    line.push(' ');
    line.push_str(&quote(argument, shell));
  }

  line
}

/// One element, quoted so `shell` hands it to the child as a single argument with its
/// content intact.
pub fn quote(element: &str, shell: ShellKind) -> String {
  match shell {
    ShellKind::Posix => posix(element),
    ShellKind::Cmd => cmd(element),
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

/// Two escapes, because two parsers read the line.
///
/// The child's own startup code splits the command line by the MSVC rules — double
/// quotes group, backslashes only matter in front of a quote — and `cmd.exe` reads the
/// line before that, acting on its own metacharacters wherever they appear. So the
/// element is first quoted for the child, then every character cmd would act on is
/// prefixed with `^` for cmd.
///
/// A batch file re-reads its arguments a third time, which needs a third round. That is
/// not done here: knowing a command ends in `.cmd` means parsing the command string to
/// find where it starts, and this change does not tokenize the user's command.
fn cmd(element: &str) -> String {
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

  let mut escaped = String::with_capacity(quoted.len());
  for character in quoted.chars() {
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
  use super::{ShellKind, command_line, quote};

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
      assert_eq!(quote(element, ShellKind::Posix), posix, "posix: {element:?}");
      assert_eq!(quote(element, ShellKind::Cmd), cmd, "cmd: {element:?}");
      assert_eq!(quote(element, ShellKind::PowerShell), power_shell, "powershell: {element:?}");
    }
  }

  /// A backslash run before a quote is the case the MSVC rules exist for: without
  /// doubling it, the child sees an escaped quote instead of a closing one.
  #[test]
  fn backslashes_before_a_quote_are_doubled_for_cmd() {
    assert_eq!(quote(r#"a\"b c"#, ShellKind::Cmd), r#"^"a\\\^"b^ c^""#);
    assert_eq!(quote(r"a\b c", ShellKind::Cmd), r#"^"a\b^ c^""#);
    assert_eq!(quote(r"trailing\", ShellKind::Cmd), r"trailing\");
  }

  #[test]
  fn an_empty_argument_list_leaves_the_command_alone() {
    assert_eq!(command_line("vitest --run", &[], ShellKind::Posix), "vitest --run");
  }

  #[test]
  fn arguments_are_appended_in_order_after_the_command() {
    let arguments = ["--reporter".to_owned(), "a b".to_owned()];

    assert_eq!(command_line("vitest", &arguments, ShellKind::Posix), "vitest --reporter 'a b'");
  }
}
