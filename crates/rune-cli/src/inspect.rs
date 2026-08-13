//! `rune inspect <name>` — what would happen, without making it happen.
//!
//! Inheritance is the first feature that makes a script name ambiguous: the answer now
//! depends on where the user is standing and how far the chain goes. This command is what
//! keeps that explainable, so it is a product surface rather than a debugging aid. A
//! confusing explanation is as severe a defect as a wrong execution, because from the
//! outside the two look the same.
//!
//! One invocation answers the whole question. A group is explained to the bottom, every
//! value on screen says what put it there, and a config the walk went past without using
//! is named — because a file that is silently ignored is the one mistake nothing else here
//! can report.
//!
//! Nothing here spawns a process. Not the command, not the shell — the shell is only
//! consulted for its name, because that is what decides how arguments are quoted.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use rune_config::env::PLATFORM;
use rune_config::envfile::Files;
use rune_config::inherit::{Link, Resolved, Runs, Scope};
use rune_config::load::Loaded;
use rune_config::paths::relative_to;
use rune_config::schema::{Command, KillSignal, Lifecycle, RetryDelay, SuccessPolicy};
use rune_exec::environment::{self, ChildEnvironment, Descriptor, FileLayer, Layering, Origin};
use rune_exec::quote::{self, command_line};
use rune_exec::shell::{self, SHELL_VARIABLE, Shell};

use crate::script::{self, env_files, load_here, unknown};

/// Prints the resolution of `name`: what runs, where, with what, and how that was reached.
pub fn run(name: &str, scope: Scope) -> Result<(), String> {
  let loaded = load_here()?;
  let resolved = loaded
    .resolve(name, scope)
    .map_err(|error| error.to_string())?
    .ok_or_else(|| unknown(name, &loaded, scope))?;

  // One reader for the whole report, exactly as a run uses one: a file four members name
  // is opened once, and the report cannot describe an environment built from a file rune
  // could not open.
  let mut files = Files::default();
  let here = directory(&resolved, &loaded);
  let facts = facts(name, &resolved, &loaded, &mut files, Position::Entry)?;
  let members = members(&resolved, &loaded, scope, &mut files, &here, &mut Vec::new())?;

  rune_out::line(&render(name, &facts, &members, &resolved, &loaded));

  Ok(())
}

/// The width of the label column, so a value that spans lines stays under its own value.
const LABEL: usize = 11;

/// One thing a script declared, and the word the report files it under.
///
/// A label is written once per run of lines that share it, so four lifecycle options read
/// as one block rather than as four repetitions of the word.
type Detail = (&'static str, String);

/// Everything one script declared about running, worked out without running it.
struct Facts {
  /// What this name runs: a command line, or the scripts it stands for.
  runs: String,
  /// Everything else: how it fails, what runs first, its lifecycle, its directory, its
  /// environment, what it asked for and will not get, and the files that took part.
  detail: Vec<Detail>,
}

/// One script inside a group, and whatever it runs in turn.
struct Node {
  name: String,
  facts: Facts,
  members: Vec<Node>,
}

fn render(
  name: &str,
  facts: &Facts,
  members: &[Node],
  resolved: &Resolved<'_>,
  loaded: &Loaded,
) -> String {
  let mut report = format!("{name}\n\n");

  let _ = writeln!(report, "{:<LABEL$}  {}", heading(resolved), facts.runs);
  write_detail(&mut report, "", &facts.detail);

  if !members.is_empty() {
    report.push_str("\nmembers\n");
    let width = column_width(members, 2);
    for member in members {
      write_node(&mut report, member, 2, width);
    }
  }

  report.push_str("\nresolved through\n");
  write_chain(&mut report, resolved, loaded);

  report.trim_end().to_owned()
}

/// The word the first line takes, which is what the name stands for.
fn heading(resolved: &Resolved<'_>) -> &'static str {
  match resolved.runs {
    Runs::Command(_) => "command",
    Runs::Serial { .. } => "runs",
    Runs::Parallel { .. } => "all at once",
  }
}

/// The detail block, each label written once for the run of lines that share it.
fn write_detail(report: &mut String, prefix: &str, detail: &[Detail]) {
  let mut last = "";
  for (label, text) in detail {
    let shown = if *label == last {
      ""
    } else {
      last = label;
      *label
    };
    let _ = writeln!(report, "{prefix}{shown:<LABEL$}  {text}");
  }
}

/// How wide the name column has to be for every node in the tree to line up.
fn column_width(nodes: &[Node], indent: usize) -> usize {
  nodes
    .iter()
    .map(|node| (indent + node.name.len()).max(column_width(&node.members, indent + 2)))
    .max()
    .unwrap_or(0)
}

/// One member, its own facts under it, and its members under those.
fn write_node(report: &mut String, node: &Node, indent: usize, width: usize) {
  let name = format!("{:indent$}{}", "", node.name);
  let _ = writeln!(report, "{name:<width$}  {}", node.facts.runs);

  write_detail(report, &format!("{:width$}  ", ""), &node.facts.detail);

  for member in &node.members {
    write_node(report, member, indent + 2, width);
  }
}

/// The chain, then the configs the walk went past and did not use.
fn write_chain(report: &mut String, resolved: &Resolved<'_>, loaded: &Loaded) {
  let root = &loaded.discovered.root;
  let unused: Vec<String> =
    loaded.discovered.unused.iter().map(|path| relative_to(root, path)).collect();
  let sources: Vec<String> =
    resolved.chain.iter().map(|link| relative_to(root, link.source)).collect();

  let source_width = sources.iter().chain(&unused).map(String::len).max().unwrap_or(0);
  let name_width = resolved.chain.iter().map(|link| link.name.len()).max().unwrap_or(0);

  for (link, source) in resolved.chain.iter().zip(&sources) {
    let contribution = contribution(link, resolved);

    let _ =
      writeln!(report, "  {source:<source_width$}  {:<name_width$}  {contribution}", link.name);
  }

  for source in &unused {
    let _ = writeln!(
      report,
      "  {source:<source_width$}  {:<name_width$}  \
       took no part — only the nearest config and the root are read",
      ""
    );
  }
}

/// What one step of the chain put into the answer.
///
/// Every field the merge decided is read back off the declaration that level wrote, so a
/// level that only set a variable says so instead of repeating a command it never touched.
/// The one at the base is the one that decides what actually runs, and it says that too,
/// because something has to.
fn contribution(link: &Link<'_>, resolved: &Resolved<'_>) -> String {
  let mut parts = Vec::new();
  let declared = link.declared;

  if link.defines_what_runs() {
    parts.push(match resolved.runs {
      Runs::Command(command) => format!("runs `{}`", command.select(PLATFORM)),
      Runs::Serial { members, .. } => format!("runs {}", members.join(", ")),
      Runs::Parallel { members, .. } => format!("runs {} at once", members.join(", ")),
    });
  }

  let appended = link.append_args();
  if !appended.is_empty() {
    parts.push(format!("appends `{}`", appended.join(" ")));
  }

  if !declared.env.is_empty() {
    let names: Vec<&str> = declared.env.keys().map(String::as_str).collect();
    parts.push(format!("sets {}", names.join(", ")));
  }

  if let Some(file) = &declared.env_file {
    parts.push(format!("reads `{file}`"));
  }

  if let Some(prerequisites) = &declared.depends_on {
    parts.push(if prerequisites.is_empty() {
      // A level saying "nothing runs first" is a decision, and one the block above cannot
      // show: what it removed leaves no line behind.
      "depends on nothing".to_owned()
    } else {
      format!("depends on {}", prerequisites.join(", "))
    });
  }

  if let Some(directory) = &declared.cwd {
    parts.push(format!("runs in `{directory}`"));
  }

  let options = lifecycle_keys(&declared.lifecycle);
  if !options.is_empty() {
    parts.push(format!("declares {}", options.join(", ")));
  }

  if parts.is_empty() {
    return "adds nothing".to_owned();
  }

  parts.join(", ")
}

/// Where in the report a script's facts are being written.
///
/// The two differ in what they may leave out. The script asked about states everything;
/// a member states what it chose for itself, because a column repeating what the whole
/// group already said buries the one member that chose otherwise.
#[derive(Clone, Copy)]
enum Position<'a> {
  Entry,
  Member { alongside: &'a Path },
}

/// Everything `run` would work out about one script, without running it.
fn facts(
  name: &str,
  resolved: &Resolved<'_>,
  loaded: &Loaded,
  files: &mut Files,
  position: Position<'_>,
) -> Result<Facts, String> {
  // Read before anything is rendered: a report that describes an environment built from a
  // file rune could not open would be an explanation of a run that cannot happen.
  let read = env_files(resolved, name, loaded, files)?;
  let layering = layered(name, resolved, loaded, &read);
  let entry = matches!(position, Position::Entry);

  let mut detail = Vec::new();

  if let Runs::Command(command) = resolved.runs {
    let starts_rune = starts_rune(command.select(PLATFORM), name);
    detail.extend(starts_rune.into_iter().map(|line| ("runs rune", line)));
  }

  if let Runs::Serial { continue_on_error: true, .. } = resolved.runs {
    detail.push(("on failure", "keeps going, then reports the first".to_owned()));
  }
  if let Runs::Parallel { continue_on_error, policy, .. } = resolved.runs {
    if entry || policy != SuccessPolicy::All {
      detail.push(("succeeds if", succeeds_if(policy).to_owned()));
    }
    if continue_on_error {
      detail.push(("on failure", "lets the others finish, then reports".to_owned()));
    }
  }

  if !resolved.depends_on.is_empty() {
    detail.push(("runs first", resolved.depends_on.join(" → ")));
  }

  detail.extend(lifecycle(&resolved.lifecycle).into_iter().map(|line| ("lifecycle", line)));

  let here = directory(resolved, loaded);
  let elsewhere = match position {
    Position::Entry => true,
    Position::Member { alongside } => alongside != here,
  };
  if elsewhere {
    detail.push(("directory", relative_to(&loaded.discovered.root, &here)));
  }

  detail.extend(environment_lines(&layering).into_iter().map(|line| ("environment", line)));
  // Separately, and always: a delta showing only what got through cannot tell a user that
  // the file they just edited had no effect.
  detail.extend(layering.ignored.iter().map(|one| ("ignored", one.to_string())));
  detail.extend(file_lines(&read, &layering).into_iter().map(|line| ("env files", line)));

  Ok(Facts { runs: runs(resolved, &layering.environment, entry), detail })
}

/// What a command that starts rune again deserves to have said about it, and nothing at
/// all for a command that does not.
///
/// A command that runs rune looks like every other command, and the only thing telling a
/// useful nested call apart from a script that runs itself without end is which name
/// follows. The leading token is read exactly as it is read to identify a child for
/// quoting, and nothing else is parsed: whether the script's own name appears after it
/// decides which of the two is printed, and a command opening with an operator or a
/// variable yields nothing.
fn starts_rune(command: &str, script: &str) -> Vec<String> {
  let Some(program) = shell::leading_program(command).filter(is_rune) else {
    return Vec::new();
  };

  let after = command.trim_start().trim_start_matches('"');
  let arguments = after.strip_prefix(program).unwrap_or_default();

  if !arguments.split_whitespace().any(|word| word == script) {
    return vec!["starts rune again".to_owned()];
  }

  vec![
    "starts rune again, on this same script".to_owned(),
    "a script whose command runs itself calls itself without end".to_owned(),
  ]
}

/// Whether a program name is rune, however it was spelled or found.
fn is_rune(program: &&str) -> bool {
  Path::new(program).file_stem().is_some_and(|stem| stem.eq_ignore_ascii_case("rune"))
}

/// What this name runs, in one line: a command line, or the scripts it stands for.
///
/// A member says which way its own members run, because nothing else on its line does.
/// The script the report is about says it in the label column instead.
fn runs(resolved: &Resolved<'_>, environment: &ChildEnvironment, entry: bool) -> String {
  match resolved.runs {
    Runs::Command(command) => assembled_command(resolved, command, environment),
    Runs::Serial { members, .. } if entry => members.join(" → "),
    Runs::Parallel { members, .. } if entry => members.join(", "),
    Runs::Serial { members, .. } => format!("runs {}", members.join(" → ")),
    Runs::Parallel { members, .. } => format!("runs {} at once", members.join(", ")),
  }
}

/// Every value the child will see that the config had something to say about, with what
/// put it there.
///
/// The source is what the whole block is for. `LOG_LEVEL=info` from a file loses to the
/// real environment and `LOG_LEVEL=info` from an `env` map beats it — opposite ends of the
/// precedence rule, and without the third column they print identically.
fn environment_lines(layering: &Layering) -> Vec<String> {
  /// How far the source column is allowed to be pushed out. One shortened value carries
  /// its true length with it, and aligning to that would move every other source off the
  /// screen to keep one row tidy.
  const WIDEST: usize = 36;

  let assignments: Vec<(String, &Origin)> = layering
    .applied
    .iter()
    .map(|(name, applied)| (format!("{name}={}", printable(&applied.value)), &applied.source))
    .collect();

  let width = assignments.iter().map(|(shown, _)| shown.len()).max().unwrap_or(0).min(WIDEST);

  assignments.iter().map(|(shown, source)| format!("{shown:<width$}  {source}")).collect()
}

/// Each file that took part, named once with what it supplied and what it lost.
fn file_lines(read: &[FileLayer], layering: &Layering) -> Vec<String> {
  read
    .iter()
    .map(|file| {
      let supplied =
        layering.applied.values().filter(|applied| applied.source.is_file(&file.source)).count();
      let lost = layering.ignored.iter().filter(|one| one.source.is_file(&file.source)).count();

      format!(
        "`{}`  {supplied} {} used, {lost} ignored",
        file.source,
        plural(supplied, "assignment")
      )
    })
    .collect()
}

/// A value as the report prints it, which is never how the child receives it.
///
/// A value spanning lines would otherwise leave its column and read as another key with
/// no name, and one of a megabyte would scroll the rest of the report away. The child
/// still gets the exact bytes: this is the report only.
fn printable(value: &str) -> String {
  /// Wide enough for a realistic value, narrow enough that one long one cannot bury the
  /// report on an ordinary terminal.
  const LIMIT: usize = 48;

  let mut shown = String::with_capacity(value.len());
  for character in value.chars() {
    match character {
      '\n' => shown.push_str("\\n"),
      '\r' => shown.push_str("\\r"),
      '\t' => shown.push_str("\\t"),
      other if other.is_control() => {
        let _ = write!(shown, "\\u{{{:x}}}", other as u32);
      }
      other => shown.push(other),
    }
  }

  if shown.chars().count() <= LIMIT {
    return shown;
  }

  let kept: String = shown.chars().take(LIMIT).collect();
  format!("{kept}… ({} characters)", value.chars().count())
}

/// Every member of a group, resolved for its own facts, to the bottom.
///
/// A member is looked up with the same call `run` makes for it, so what the report says
/// about a member and what running it would do come from one answer. A name already on the
/// path is not expanded again — the loader refuses a config that could produce one, and a
/// renderer that trusted that would recurse forever the day it stopped being true.
fn members(
  resolved: &Resolved<'_>,
  loaded: &Loaded,
  scope: Scope,
  files: &mut Files,
  alongside: &Path,
  path: &mut Vec<String>,
) -> Result<Vec<Node>, String> {
  let names = match resolved.runs {
    Runs::Command(_) => return Ok(Vec::new()),
    Runs::Serial { members, .. } | Runs::Parallel { members, .. } => members,
  };

  names
    .iter()
    .map(|name| {
      let member = loaded
        .resolve(name, scope)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| unknown(name, loaded, scope))?;

      let facts = facts(name, &member, loaded, files, Position::Member { alongside })?;
      let members = if path.iter().any(|seen| seen == name) {
        Vec::new()
      } else {
        path.push(name.clone());
        let nested = members(&member, loaded, scope, files, alongside, path);
        path.pop();
        nested?
      };

      Ok(Node { name: name.clone(), facts, members })
    })
    .collect()
}

/// The lifecycle options a script declared, one line each, and nothing for the ones it
/// left alone.
///
/// Defaults are deliberately absent: a report that listed `SIGTERM after 5000 ms` for
/// every script would bury the one line that was a decision under four that were not.
fn lifecycle(declared: &Lifecycle) -> Vec<String> {
  let mut lines = Vec::new();

  if let Some(timeout) = declared.timeout {
    lines.push(format!("ends the whole tree after {} ms", timeout.as_millis()));
  }

  if let Some(retries) = declared.retries {
    let wait = match declared.retry_delay {
      None => String::new(),
      Some(RetryDelay::Fixed(delay)) => format!(", waiting {} ms between them", delay.as_millis()),
      Some(RetryDelay::Exponential) => ", waiting 2^attempt seconds between them".to_owned(),
    };
    lines.push(format!(
      "retries {retries} more {} on failure{wait}",
      plural(retries as usize, "time")
    ));
  }

  if let Some(signal) = declared.kill_signal {
    lines.push(format!("ends with {}", named(signal)));
  }

  if let Some(timeout) = declared.kill_timeout {
    lines.push(format!("waits {} ms before SIGKILL", timeout.as_millis()));
  }

  lines
}

/// The lifecycle keys a level wrote, spelled as the config spells them.
///
/// A chain line is a summary, so it names the keys rather than repeating the sentences the
/// block above already carries.
fn lifecycle_keys(declared: &Lifecycle) -> Vec<&'static str> {
  [
    declared.timeout.is_some().then_some("timeout"),
    declared.retries.is_some().then_some("retries"),
    declared.retry_delay.is_some().then_some("retryDelay"),
    declared.kill_signal.is_some().then_some("killSignal"),
    declared.kill_timeout.is_some().then_some("killTimeout"),
  ]
  .into_iter()
  .flatten()
  .collect()
}

fn plural(count: usize, word: &str) -> String {
  if count == 1 { word.to_owned() } else { format!("{word}s") }
}

/// The signal spelled the way the config writes it.
fn named(signal: KillSignal) -> &'static str {
  match signal {
    KillSignal::Hup => "SIGHUP",
    KillSignal::Int => "SIGINT",
    KillSignal::Quit => "SIGQUIT",
    KillSignal::Term => "SIGTERM",
    KillSignal::Kill => "SIGKILL",
  }
}

/// What the success policy means, said rather than named. `first` is the value a user
/// writes; "the member that finishes first" is what they need to know it does.
fn succeeds_if(policy: SuccessPolicy) -> &'static str {
  match policy {
    SuccessPolicy::All => "every member succeeds",
    SuccessPolicy::First => "the member that finishes first succeeds",
    SuccessPolicy::Last => "the member that finishes last succeeds",
  }
}

/// The environment `run` would build, worked out without running anything.
///
/// The same function `run` uses, against the same process environment, so what the report
/// calls ignored is exactly what the run would drop.
fn layered(name: &str, resolved: &Resolved<'_>, loaded: &Loaded, files: &[FileLayer]) -> Layering {
  environment::build(
    std::env::vars_os(),
    &Descriptor {
      script_name: name,
      root: &loaded.discovered.root,
      package_dir: &loaded.discovered.package_dir,
      env: &resolved.env,
      env_files: files,
    },
  )
}

/// The command line the shell would be handed, quoted the way that shell reads it.
///
/// The shell is identified by name only. Locating it on disk is what `run` does, and
/// doing it here would make an explanation fail on a machine where the tool is missing.
/// The child is looked for, because how many times its arguments get read decides how they
/// are escaped — but a child that cannot be found only means the explanation says what a
/// single reader would receive, which is what a run would then do too.
fn assembled_command(
  resolved: &Resolved<'_>,
  command: &Command,
  environment: &ChildEnvironment,
) -> String {
  let configured = std::env::var_os(SHELL_VARIABLE);
  let shell = Shell::select(configured.as_deref());
  let selected = command.select(PLATFORM);
  let reads =
    quote::reads(selected, shell.kind, environment.get("PATH"), environment.get("PATHEXT"));

  command_line(selected, &resolved.append_args, shell.kind, reads)
}

/// Where the script would run: literally the function `run` calls, so the explanation and
/// the run cannot describe two different directories.
fn directory(resolved: &Resolved<'_>, loaded: &Loaded) -> PathBuf {
  script::directory(resolved.cwd, &loaded.discovered.package_dir)
}

#[cfg(test)]
mod tests {
  use super::printable;

  /// Test R13.6's unit half — a value spanning lines is one row, and the escape is what
  /// keeps it under its own column instead of reading as another key with no name.
  #[test]
  fn a_value_spanning_lines_becomes_one_row() {
    assert_eq!(printable("one\ntwo"), "one\\ntwo");
  }

  /// The length reported is the value's, not the shortened form's. Reporting the shown
  /// length would tell a user their megabyte was 48 characters long.
  #[test]
  fn a_long_value_is_cut_and_states_its_true_length() {
    let shortened = printable(&"x".repeat(1_048_576));

    assert!(shortened.ends_with("… (1048576 characters)"), "{shortened}");
    assert!(shortened.len() < 100, "{shortened}");
  }

  #[test]
  fn an_ordinary_value_is_printed_as_it_is() {
    assert_eq!(printable("https://example.test"), "https://example.test");
  }
}
