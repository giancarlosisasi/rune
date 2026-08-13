//! The shape a resolved config must have.
//!
//! Dispatch is manual and explicit. `#[serde(untagged)]` was the obvious choice and is
//! the wrong one: it does not compose with `deny_unknown_fields`, and its failure reads
//! "data did not match any variant", which tells a user nothing about the mistake they
//! made. Here the discriminant keys are inspected by name, so the error can say which
//! script is wrong and which word in it is the problem.

use std::collections::BTreeMap;
use std::time::Duration;

use serde::Deserialize;
use thiserror::Error;

use crate::suggest::{closest, did_you_mean};

/// The keys that decide which kind of script an entry is. Each later change adds one.
const DISCRIMINANTS: &[&str] = &["command", "extends", "serial", "parallel"];

/// The discriminants that name other scripts rather than running a command.
const GROUPS: &[&str] = &["serial", "parallel"];

/// Fields any script may carry, whatever its discriminant.
const COMMON_FIELDS: &[&str] = &["description", "cwd", "env", "envFile"];

/// The arguments an extending script adds to what it inherits.
const APPEND_ARGS: &str = "appendArgs";

/// The scripts that run before a script's own command.
const DEPENDS_ON: &str = "dependsOn";

/// Makes a group run every member instead of stopping at the first failure.
const CONTINUE_ON_ERROR: &str = "continueOnError";

/// Which member of a parallel group the group takes its result from.
const SUCCESS_POLICY: &str = "successPolicy";

/// Keeps one script on rune's own terminal while its siblings are piped.
const INTERACTIVE: &str = "interactive";

/// How long one run of a script may take before its process tree is terminated.
const TIMEOUT: &str = "timeout";

/// How many further attempts a failing script gets.
const RETRIES: &str = "retries";

/// The wait between those attempts.
const RETRY_DELAY: &str = "retryDelay";

/// What a process tree is asked to end with.
const KILL_SIGNAL: &str = "killSignal";

/// How long that request is given before the tree is ended for it.
const KILL_TIMEOUT: &str = "killTimeout";

/// How a script is run rather than what it runs. Every one of these describes a single
/// process, so they are legal on command and extends scripts and refused on groups.
const LIFECYCLE: &[&str] = &[TIMEOUT, RETRIES, RETRY_DELAY, KILL_SIGNAL, KILL_TIMEOUT];

/// The value of `retryDelay` that grows the wait instead of repeating it.
const EXPONENTIAL: &str = "exponential";

/// The keys a per-OS `command` object may hold. The names are Node's `process.platform`
/// values, so a config reading `rune.platform` and a config using this object spell the
/// same operating system the same way.
const PER_OS_KEYS: &[&str] = &["default", "win32", "darwin", "linux"];

/// The key every per-OS object must have.
const FALLBACK_KEY: &str = "default";

#[derive(Debug, Error)]
pub enum SchemaError {
  #[error(
    "the config has no `scripts`\n\n\
     every script rune can run is an entry in it:\n\n  \
     export default {{\n    \
     scripts: {{\n      \
     dev: {{ command: \"vite\" }},\n    \
     }},\n  \
     }};"
  )]
  NoScripts,

  #[error(
    "the config's `scripts` must be an object; found {found}\n\n\
     `scripts` maps each name to what that name runs:\n\n  \
     scripts: {{\n    \
     dev: {{ command: \"vite\" }},\n  \
     }}"
  )]
  ScriptsShape { found: String },

  #[error("script `{script}` must be an object; found {found}")]
  NotAScript { script: String, found: String },

  #[error("script `{script}` has no command\n\nevery script needs one of: {}", list(DISCRIMINANTS))]
  NoDiscriminant { script: String },

  #[error(
    "script `{script}` sets both `{first}` and `{second}`\n\n\
     a script is exactly one kind, so keep the one you meant and remove the other:\n  \
     `{first}` {}\n  `{second}` {}",
    meaning(.first),
    meaning(.second)
  )]
  ManyDiscriminants { script: String, first: String, second: String },

  #[error(
    "script `{script}` sets `{APPEND_ARGS}` but does not extend anything\n\n\
     `{APPEND_ARGS}` adds arguments to a command a script inherits, so it only means \
     something\nnext to `extends`. A script with a `command` of its own writes its \
     arguments into it."
  )]
  AppendArgsWithoutExtends { script: String },

  #[error(
    "script `{script}` has an unknown field `{field}`\n\nallowed here: {allowed}{}",
    did_you_mean(.suggestion.as_deref())
  )]
  UnknownField { script: String, field: String, allowed: String, suggestion: Option<String> },

  #[error(
    "script `{script}` has a `command` that is {found}\n\n\
     a command is either a string, or an object naming one command per operating \
     system: {}",
    list(PER_OS_KEYS)
  )]
  CommandShape { script: String, found: String },

  #[error(
    "script `{script}` has a per-operating-system `command` with no `{FALLBACK_KEY}`\n\n\
     `{FALLBACK_KEY}` is what runs on every system without an entry of its own.\n\
     Without it this script would exist on some machines and not on others."
  )]
  NoFallback { script: String },

  #[error("script `{script}` extends {found}\n\n`extends` is the name of another script")]
  ExtendsShape { script: String, found: String },

  #[error("script `{script}` has a `{key}` that is {found}\n\n{explanation}")]
  NotAList { script: String, key: &'static str, found: String, explanation: &'static str },

  #[error("script `{script}` has `{key}` element {position} that is {found}\n\n{explanation}")]
  NotAListElement {
    script: String,
    key: &'static str,
    position: usize,
    found: String,
    explanation: &'static str,
  },

  #[error(
    "script `{script}` sets `{DEPENDS_ON}` beside `{group}`\n\n\
     a group runs the scripts it names and nothing before them. To run something first, \
     make\nit the first member of a `serial` group with this one after it."
  )]
  DependsOnGroup { script: String, group: String },

  #[error(
    "script `{script}` has a `{CONTINUE_ON_ERROR}` that is {found}\n\n\
     `{CONTINUE_ON_ERROR}` is true or false. Set, every member runs even after one fails, \
     and\nthe group still ends with the first failure's exit code."
  )]
  ContinueOnErrorShape { script: String, found: String },

  #[error(
    "script `{script}` has a `{SUCCESS_POLICY}` of `{found}`\n\n\
     `{SUCCESS_POLICY}` is one of: {}",
    list(SuccessPolicy::PERMITTED)
  )]
  SuccessPolicyValue { script: String, found: String },

  #[error(
    "script `{script}` sets `{SUCCESS_POLICY}` on a `serial` group\n\n\
     `{SUCCESS_POLICY}` picks between members by the time they exited, and a serial group \
     runs\nthem one at a time — first and last are already its member list. The option \
     applies\nto `parallel` groups."
  )]
  SuccessPolicyOnSerial { script: String },

  #[error(
    "script `{script}` sets `{INTERACTIVE}` on a `{group}` group\n\n\
     `{INTERACTIVE}` describes one process's relationship with the terminal. A group is \
     not a\nprocess, and only one member of a group can own the terminal. Put it on the \
     member\nthat needs it."
  )]
  InteractiveOnGroup { script: String, group: String },

  #[error(
    "script `{script}` has an `{INTERACTIVE}` that is {found}\n\n\
     `{INTERACTIVE}` is true or false. Set, the script keeps rune's own terminal even \
     while\nits siblings are piped and prefixed."
  )]
  InteractiveShape { script: String, found: String },

  #[error(
    "script `{script}` sets `{option}` on a `{group}` group\n\n\
     `{option}` describes how one process is run, and a group is not a process — it names \
     the\nscripts it runs. Put it on the members it should apply to."
  )]
  LifecycleOnGroup { script: String, group: String, option: String },

  #[error(
    "script `{script}` has a `{key}` that is {found}\n\n\
     `{key}` is a whole number of milliseconds."
  )]
  NotADuration { script: String, key: &'static str, found: String },

  #[error(
    "script `{script}` has a `{RETRIES}` that is {found}\n\n\
     `{RETRIES}` is how many further attempts a failing script gets, written as a whole \
     number."
  )]
  NotACount { script: String, found: String },

  #[error(
    "script `{script}` has a `{RETRY_DELAY}` that is {found}\n\n\
     `{RETRY_DELAY}` is either a whole number of milliseconds waited before every \
     attempt, or\n`{EXPONENTIAL}`, which waits 2^attempt seconds and so grows with each one."
  )]
  RetryDelayValue { script: String, found: String },

  #[error(
    "script `{script}` sets `{RETRY_DELAY}` but no `{RETRIES}`\n\n\
     `{RETRY_DELAY}` is the wait between attempts, and without `{RETRIES}` there is no \
     second attempt\nfor it to come before."
  )]
  RetryDelayWithoutRetries { script: String },

  #[error(
    "script `{script}` has a `{KILL_SIGNAL}` that is {found}\n\n\
     `{KILL_SIGNAL}` is one of: {}",
    list(KillSignal::PERMITTED)
  )]
  KillSignalValue { script: String, found: String },

  #[error("script `{script}`: {source}")]
  Invalid { script: String, source: serde_json::Error },
}

/// What a list-valued key holds, printed when the value is not a list of strings.
///
/// The pairs sit together because the two messages a user can meet for one key have to
/// agree with each other about what that key is for.
const APPEND_ARGS_LIST: (&str, &str) = (
  "`appendArgs` is a list of arguments, one element per argument:\n\n  \
   appendArgs: [\"--maxWorkers=1\"]",
  "every element is one argument, written as a string",
);

const SERIAL_LIST: (&str, &str) = (
  "`serial` is a list of script names, run one at a time in the order written:\n\n  \
   serial: [\"lint\", \"typecheck\", \"test\"]",
  "every element is the name of another script, written as a string",
);

const PARALLEL_LIST: (&str, &str) = (
  "`parallel` is a list of script names, all started at once:\n\n  \
   parallel: [\"dev:server\", \"dev:watch\"]",
  "every element is the name of another script, written as a string",
);

const DEPENDS_ON_LIST: (&str, &str) = (
  "`dependsOn` is a list of script names, run to completion before this script's own \
   command:\n\n  dependsOn: [\"clean\"]",
  "every element is the name of another script, written as a string",
);

fn list(names: &[&str]) -> String {
  names.iter().map(|name| format!("`{name}`")).collect::<Vec<_>>().join(", ")
}

/// What each discriminant makes a script do, in one line.
///
/// The conflict error prints this for both keys it found, so a user picks the one they
/// meant without opening the documentation. Every variant a later change adds gets a
/// line here, and the conflict message keeps working without being rewritten.
fn meaning(discriminant: &str) -> &'static str {
  match discriminant {
    "command" => "runs a command of its own",
    "extends" => "builds on another script and adds to it",
    "serial" => "runs other scripts, one after another",
    "parallel" => "runs other scripts, all at the same time",
    other => unreachable!("`{other}` is listed as a discriminant but has no meaning"),
  }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
  /// Sorted by name, because `rune list` prints them in this order and a map with a
  /// random iteration order would make that output differ between runs.
  pub scripts: BTreeMap<String, Script>,
}

/// One entry of the `scripts` object: the fields every script may carry, plus the one
/// thing that makes it the kind of script it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Script {
  pub description: Option<String>,
  /// Where the script runs. A relative value is resolved against the invoking package.
  pub cwd: Option<String>,
  /// Variables the script sets for its own child, which win over inherited values.
  pub env: BTreeMap<String, String>,
  /// A file of variables that fill the gaps the process environment leaves. Resolved
  /// against the config that declares it, never against the working directory.
  pub env_file: Option<String>,
  /// Scripts that run to completion before this one's own command.
  ///
  /// `None` means the entry said nothing, so an extending script inherits what it builds
  /// on; an empty list means it said "nothing runs first" and that wins. A group never
  /// carries one — it runs the scripts it names and nothing before them.
  pub depends_on: Option<Vec<String>>,
  /// Keeps this script on rune's own terminal even as a member of a group.
  ///
  /// `None` means the entry said nothing, so an extending script inherits the answer.
  pub interactive: Option<bool>,
  /// How the script is run: its timeout, its retries, and how its tree is ended. Never
  /// carried by a group.
  pub lifecycle: Lifecycle,
  pub kind: Kind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
  /// Runs a command of its own.
  Command(Command),
  /// Runs another script's command, with more arguments after it.
  Extends { target: String, append_args: Vec<String> },
  /// Runs other scripts, one at a time, in the order written.
  Serial { members: Vec<String>, continue_on_error: bool },
  /// Runs other scripts all at once, and waits for every one of them.
  Parallel { members: Vec<String>, continue_on_error: bool, policy: SuccessPolicy },
}

/// Which member of a parallel group the group takes its result from.
///
/// `First` and `Last` are settled by exit time, never by position in the member list: a
/// member's place in a list says nothing about when it finishes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SuccessPolicy {
  /// The group succeeds only if every member succeeded.
  #[default]
  All,
  /// The group takes the result of the member that exited first in time.
  First,
  /// The group takes the result of the member that exited last in time.
  Last,
}

impl SuccessPolicy {
  /// The values a config may write, in the order the error message lists them.
  pub const PERMITTED: &'static [&'static str] = &["all", "first", "last"];

  fn parse(value: &str) -> Option<Self> {
    match value {
      "all" => Some(Self::All),
      "first" => Some(Self::First),
      "last" => Some(Self::Last),
      _ => None,
    }
  }
}

/// The lifecycle options one script declared.
///
/// Every field is optional because absence is meaningful: an extending script inherits
/// each option it does not declare, and declaring one overrides only that one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Lifecycle {
  /// How long one attempt may run before its whole process tree is terminated.
  pub timeout: Option<Duration>,
  /// How many further attempts a failing script gets.
  pub retries: Option<u32>,
  pub retry_delay: Option<RetryDelay>,
  pub kill_signal: Option<KillSignal>,
  /// How long a tree is given to act on `kill_signal` before it is ended for it.
  pub kill_timeout: Option<Duration>,
}

impl Lifecycle {
  /// Takes every option `later` declares and keeps the rest — the per-key rule `env`
  /// follows, so an extending script overrides one option without discarding the others.
  pub fn absorb(&mut self, later: Self) {
    self.timeout = later.timeout.or(self.timeout);
    self.retries = later.retries.or(self.retries);
    self.retry_delay = later.retry_delay.or(self.retry_delay);
    self.kill_signal = later.kill_signal.or(self.kill_signal);
    self.kill_timeout = later.kill_timeout.or(self.kill_timeout);
  }
}

/// How long rune waits before running a failing script again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryDelay {
  /// The same wait before every attempt.
  Fixed(Duration),
  /// `2^attempt` seconds, growing with each attempt.
  Exponential,
}

/// The signal a script's process tree is asked to end with.
///
/// A closed set rather than whatever the platform happens to name: a config written on one
/// operating system has to load on every other, and `Signal::from_str` would accept a
/// different set of names on each.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KillSignal {
  Hup,
  Int,
  Quit,
  Term,
  Kill,
}

impl KillSignal {
  /// The values a config may write, in the order the error message lists them.
  pub const PERMITTED: &'static [&'static str] =
    &["SIGHUP", "SIGINT", "SIGQUIT", "SIGTERM", "SIGKILL"];

  fn parse(value: &str) -> Option<Self> {
    match value {
      "SIGHUP" => Some(Self::Hup),
      "SIGINT" => Some(Self::Int),
      "SIGQUIT" => Some(Self::Quit),
      "SIGTERM" => Some(Self::Term),
      "SIGKILL" => Some(Self::Kill),
      _ => None,
    }
  }
}

/// What a script runs: one command everywhere, or one per operating system.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
  Everywhere(String),
  PerOs(PerOsCommand),
}

impl Command {
  /// The command string for `platform`, named the way `process.platform` names it.
  pub fn select(&self, platform: &str) -> &str {
    match self {
      Self::Everywhere(command) => command,
      Self::PerOs(per_os) => per_os.select(platform),
    }
  }
}

/// `rm -rf dist` and `rmdir /s /q dist` are one intent with two spellings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PerOsCommand {
  /// What a system with no entry of its own runs. Required, so that a config cannot
  /// define a script which silently does not exist on somebody else's machine.
  pub default: String,
  pub win32: Option<String>,
  pub darwin: Option<String>,
  pub linux: Option<String>,
}

impl PerOsCommand {
  /// The entry matching `platform`, or `default`.
  ///
  /// The platform is an argument rather than something read from `cfg!` here: that is
  /// what lets one machine test the choice every other machine would make.
  pub fn select(&self, platform: &str) -> &str {
    let matched = match platform {
      "win32" => self.win32.as_deref(),
      "darwin" => self.darwin.as_deref(),
      "linux" => self.linux.as_deref(),
      _ => None,
    };

    matched.unwrap_or(&self.default)
  }
}

/// The fields a script carries whatever kind it is. Deserialized apart from the
/// discriminant so that the discriminant's own errors can name the script.
#[derive(Debug, Deserialize)]
struct Common {
  #[serde(default)]
  description: Option<String>,
  #[serde(default)]
  cwd: Option<String>,
  #[serde(default)]
  env: BTreeMap<String, String>,
  #[serde(default, rename = "envFile")]
  env_file: Option<String>,
}

/// Turns an evaluated config into the typed shape, or says exactly what is wrong with it.
pub fn parse(value: &serde_json::Value) -> Result<Config, SchemaError> {
  // Two unrelated mistakes, each with its own repair: the key was never written, or it
  // holds something a script map cannot be. One sentence for both described the config the
  // same sentence had just asked for, and reported finding an object when it wanted one.
  let scripts = value.get("scripts").ok_or(SchemaError::NoScripts)?;
  let scripts =
    scripts.as_object().ok_or_else(|| SchemaError::ScriptsShape { found: describe(scripts) })?;

  let mut parsed = BTreeMap::new();
  for (name, entry) in scripts {
    parsed.insert(name.clone(), parse_script(name, entry)?);
  }

  Ok(Config { scripts: parsed })
}

fn parse_script(name: &str, entry: &serde_json::Value) -> Result<Script, SchemaError> {
  let object = entry
    .as_object()
    .ok_or_else(|| SchemaError::NotAScript { script: name.to_owned(), found: describe(entry) })?;

  let mut present = DISCRIMINANTS.iter().filter(|key| object.contains_key(**key));
  let found = present.next();
  if let (Some(first), Some(second)) = (found, present.next()) {
    return Err(SchemaError::ManyDiscriminants {
      script: name.to_owned(),
      first: (*first).to_owned(),
      second: (*second).to_owned(),
    });
  }

  // What a message prints stays specific to the entry: the variant's own set once the kind
  // can be read, the kind selectors and the common fields while it cannot.
  let allowed = found.map_or_else(allowed_without_a_kind, |kind| allowed_for(kind));

  // Ahead of every rule that depends on the kind, because losing the field that names the
  // kind makes the entry's remaining correct fields illegal for the kind rune then infers.
  // Reporting one of those names a field the user must not change and never the misspelled
  // one, and this is the only pass that can still tell the difference.
  reject_foreign_fields(name, object, &allowed)?;

  // Before the discriminant dispatch, so that the message is about the mistake the user
  // made rather than about a field that happens to be unknown to the arm they landed in.
  if object.contains_key(APPEND_ARGS) && !object.contains_key("extends") {
    return Err(SchemaError::AppendArgsWithoutExtends { script: name.to_owned() });
  }

  // Checked ahead of the arms so the message is about the option a user reached for
  // rather than about a field the arm they landed in happens not to know.
  if let Some(group) = found.filter(|key| GROUPS.contains(key)) {
    if object.contains_key(DEPENDS_ON) {
      return Err(SchemaError::DependsOnGroup {
        script: name.to_owned(),
        group: (*group).to_owned(),
      });
    }

    if object.contains_key(INTERACTIVE) {
      return Err(SchemaError::InteractiveOnGroup {
        script: name.to_owned(),
        group: (*group).to_owned(),
      });
    }

    if let Some(option) = LIFECYCLE.iter().find(|key| object.contains_key(**key)) {
      return Err(SchemaError::LifecycleOnGroup {
        script: name.to_owned(),
        group: (*group).to_owned(),
        option: (*option).to_owned(),
      });
    }
  }

  if found == Some(&"serial") && object.contains_key(SUCCESS_POLICY) {
    return Err(SchemaError::SuccessPolicyOnSerial { script: name.to_owned() });
  }

  // What is left is a field some other kind allows, which is a fact about the kind rather
  // than about the field. Checked before deserializing so the message can name the script:
  // serde's own unknown-field error knows the field but not which entry it came from.
  reject_unknown_fields(name, object, &allowed)?;

  let Some(discriminant) = found else {
    return Err(SchemaError::NoDiscriminant { script: name.to_owned() });
  };

  let kind = match *discriminant {
    "command" => Kind::Command(parse_command(name, &object["command"])?),
    "extends" => parse_extends(name, object)?,
    "serial" => Kind::Serial {
      members: string_list(name, "serial", &object["serial"], SERIAL_LIST)?,
      continue_on_error: parse_continue_on_error(name, object)?,
    },
    "parallel" => Kind::Parallel {
      members: string_list(name, "parallel", &object["parallel"], PARALLEL_LIST)?,
      continue_on_error: parse_continue_on_error(name, object)?,
      policy: parse_success_policy(name, object)?,
    },
    other => unreachable!("`{other}` is listed as a discriminant but has no arm"),
  };

  let depends_on = match object.get(DEPENDS_ON) {
    None => None,
    Some(value) => Some(string_list(name, DEPENDS_ON, value, DEPENDS_ON_LIST)?),
  };

  let interactive = match object.get(INTERACTIVE) {
    None => None,
    Some(value) => Some(value.as_bool().ok_or_else(|| SchemaError::InteractiveShape {
      script: name.to_owned(),
      found: describe(value),
    })?),
  };

  let common: Common = serde_json::from_value(entry.clone())
    .map_err(|source| SchemaError::Invalid { script: name.to_owned(), source })?;

  Ok(Script {
    description: common.description,
    cwd: common.cwd,
    env: common.env,
    env_file: common.env_file,
    depends_on,
    interactive,
    lifecycle: parse_lifecycle(name, object)?,
    kind,
  })
}

/// The five options that describe how a script is run, or the reason one of them cannot be
/// read. Only ever reached for a script that runs a command of its own.
fn parse_lifecycle(
  script: &str,
  object: &serde_json::Map<String, serde_json::Value>,
) -> Result<Lifecycle, SchemaError> {
  let retries = match object.get(RETRIES) {
    None => None,
    Some(value) => {
      Some(whole(value).and_then(|count| u32::try_from(count).ok()).ok_or_else(|| {
        SchemaError::NotACount { script: script.to_owned(), found: literal(value) }
      })?)
    }
  };

  let retry_delay = parse_retry_delay(script, object)?;
  if retry_delay.is_some() && retries.is_none() {
    return Err(SchemaError::RetryDelayWithoutRetries { script: script.to_owned() });
  }

  let kill_signal = match object.get(KILL_SIGNAL) {
    None => None,
    Some(value) => Some(value.as_str().and_then(KillSignal::parse).ok_or_else(|| {
      SchemaError::KillSignalValue { script: script.to_owned(), found: literal(value) }
    })?),
  };

  Ok(Lifecycle {
    timeout: parse_duration(script, object, TIMEOUT)?,
    retries,
    retry_delay,
    kill_signal,
    kill_timeout: parse_duration(script, object, KILL_TIMEOUT)?,
  })
}

fn parse_retry_delay(
  script: &str,
  object: &serde_json::Map<String, serde_json::Value>,
) -> Result<Option<RetryDelay>, SchemaError> {
  let Some(value) = object.get(RETRY_DELAY) else {
    return Ok(None);
  };

  if value.as_str() == Some(EXPONENTIAL) {
    return Ok(Some(RetryDelay::Exponential));
  }

  let millis = whole(value).ok_or_else(|| SchemaError::RetryDelayValue {
    script: script.to_owned(),
    found: literal(value),
  })?;

  Ok(Some(RetryDelay::Fixed(Duration::from_millis(millis))))
}

/// A number of milliseconds, or the reason it is not one — negative and fractional being
/// the two ways a duration is written wrong.
fn parse_duration(
  script: &str,
  object: &serde_json::Map<String, serde_json::Value>,
  key: &'static str,
) -> Result<Option<Duration>, SchemaError> {
  let Some(value) = object.get(key) else {
    return Ok(None);
  };

  let millis = whole(value).ok_or_else(|| SchemaError::NotADuration {
    script: script.to_owned(),
    key,
    found: literal(value),
  })?;

  Ok(Some(Duration::from_millis(millis)))
}

/// A number that counts something, however the engine spelled it.
///
/// A config is evaluated as JavaScript, where every number is a double: `1000` comes back
/// as `1000.0`. Reading only integers here would reject every duration a user can write.
/// Above `2^53` a double no longer carries whole numbers exactly, so a value that large is
/// refused rather than silently rounded to one the config never said.
fn whole(value: &serde_json::Value) -> Option<u64> {
  /// The largest whole number a double represents exactly.
  const EXACT: f64 = 9_007_199_254_740_992.0;

  if let Some(counted) = value.as_u64() {
    return Some(counted);
  }

  let number = value.as_f64()?;
  if number.fract() != 0.0 || !(0.0..=EXACT).contains(&number) {
    return None;
  }

  #[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "whole, positive and within the exactly representable range, checked above"
  )]
  Some(number as u64)
}

/// The value as the user wrote it, for the messages where naming its type explains
/// nothing: `-5` and `1000` are both "a number", and only one of them is the mistake.
fn literal(value: &serde_json::Value) -> String {
  match value {
    serde_json::Value::Number(number) => format!("`{number}`"),
    serde_json::Value::String(text) => format!("`{text}`"),
    other => describe(other),
  }
}

fn parse_continue_on_error(
  script: &str,
  object: &serde_json::Map<String, serde_json::Value>,
) -> Result<bool, SchemaError> {
  let Some(value) = object.get(CONTINUE_ON_ERROR) else {
    return Ok(false);
  };

  value.as_bool().ok_or_else(|| SchemaError::ContinueOnErrorShape {
    script: script.to_owned(),
    found: describe(value),
  })
}

fn parse_success_policy(
  script: &str,
  object: &serde_json::Map<String, serde_json::Value>,
) -> Result<SuccessPolicy, SchemaError> {
  let Some(value) = object.get(SUCCESS_POLICY) else {
    return Ok(SuccessPolicy::default());
  };

  value.as_str().and_then(SuccessPolicy::parse).ok_or_else(|| SchemaError::SuccessPolicyValue {
    script: script.to_owned(),
    found: value.as_str().map_or_else(|| describe(value), str::to_owned),
  })
}

/// A list of strings, or the reason `value` is not one.
///
/// `explanation` carries what the key is for, once for the whole value and once for a
/// single bad element, so three keys share the checking without sharing their wording.
fn string_list(
  script: &str,
  key: &'static str,
  value: &serde_json::Value,
  explanation: (&'static str, &'static str),
) -> Result<Vec<String>, SchemaError> {
  let (whole, element) = explanation;

  let items = value.as_array().ok_or_else(|| SchemaError::NotAList {
    script: script.to_owned(),
    key,
    found: describe(value),
    explanation: whole,
  })?;

  items
    .iter()
    .enumerate()
    .map(|(position, item)| {
      item.as_str().map(str::to_owned).ok_or_else(|| SchemaError::NotAListElement {
        script: script.to_owned(),
        key,
        position,
        found: describe(item),
        explanation: element,
      })
    })
    .collect()
}

fn parse_extends(
  script: &str,
  object: &serde_json::Map<String, serde_json::Value>,
) -> Result<Kind, SchemaError> {
  let value = &object["extends"];
  let target = value
    .as_str()
    .ok_or_else(|| SchemaError::ExtendsShape { script: script.to_owned(), found: describe(value) })?
    .to_owned();

  let Some(arguments) = object.get(APPEND_ARGS) else {
    return Ok(Kind::Extends { target, append_args: Vec::new() });
  };

  let append_args = string_list(script, APPEND_ARGS, arguments, APPEND_ARGS_LIST)?;

  Ok(Kind::Extends { target, append_args })
}

fn parse_command(script: &str, value: &serde_json::Value) -> Result<Command, SchemaError> {
  if let Some(command) = value.as_str() {
    return Ok(Command::Everywhere(command.to_owned()));
  }

  let Some(object) = value.as_object() else {
    return Err(SchemaError::CommandShape { script: script.to_owned(), found: describe(value) });
  };

  if let Some(key) = object.keys().find(|key| !PER_OS_KEYS.contains(&key.as_str())) {
    return Err(unknown_field(script, key, PER_OS_KEYS));
  }

  let entry = |key: &str| -> Result<Option<String>, SchemaError> {
    match object.get(key) {
      None => Ok(None),
      Some(value) => value.as_str().map(|command| Some(command.to_owned())).ok_or_else(|| {
        SchemaError::CommandShape { script: script.to_owned(), found: describe(value) }
      }),
    }
  };

  let default =
    entry(FALLBACK_KEY)?.ok_or_else(|| SchemaError::NoFallback { script: script.to_owned() })?;

  Ok(Command::PerOs(PerOsCommand {
    default,
    win32: entry("win32")?,
    darwin: entry("darwin")?,
    linux: entry("linux")?,
  }))
}

/// The fields legal for one variant: its own discriminant plus everything shared.
fn allowed_with(specific: &[&'static str]) -> Vec<&'static str> {
  specific.iter().copied().chain(COMMON_FIELDS.iter().copied()).collect()
}

/// The same, for a variant that runs a command of its own and so may say how it is run.
fn allowed_to_run(specific: &[&'static str]) -> Vec<&'static str> {
  specific
    .iter()
    .copied()
    .chain(LIFECYCLE.iter().copied())
    .chain(COMMON_FIELDS.iter().copied())
    .collect()
}

/// The fields one kind of script may carry, named by the discriminant that decides it.
fn allowed_for(discriminant: &str) -> Vec<&'static str> {
  match discriminant {
    "command" => allowed_to_run(&["command", DEPENDS_ON, INTERACTIVE]),
    "extends" => allowed_to_run(&["extends", APPEND_ARGS, DEPENDS_ON, INTERACTIVE]),
    "serial" => allowed_with(&["serial", CONTINUE_ON_ERROR]),
    "parallel" => allowed_with(&["parallel", CONTINUE_ON_ERROR, SUCCESS_POLICY]),
    other => unreachable!("`{other}` is listed as a discriminant but has no fields"),
  }
}

/// The fields legal while the kind cannot be read: the selectors themselves, and what is
/// legal beside any of them.
///
/// `dependsOn` belongs here because it is legal next to every variant without being one, so
/// an entry carrying only that is missing a kind rather than holding an unknown field.
fn allowed_without_a_kind() -> Vec<&'static str> {
  let mut selectors = DISCRIMINANTS.to_vec();
  selectors.push(DEPENDS_ON);
  allowed_with(&selectors)
}

/// Every field some kind of script may carry.
///
/// Joined from the sets above rather than written out, so a field a later variant adds is
/// covered here without being listed twice — and so this can never refuse a field one of
/// those sets prints as allowed.
fn any_field() -> Vec<&'static str> {
  DISCRIMINANTS
    .iter()
    .flat_map(|discriminant| allowed_for(discriminant))
    .chain(allowed_without_a_kind())
    .collect()
}

/// A field no kind of script may carry, refused before the kind is read.
///
/// "In no rule set" is a fact about the entry and "not allowed here" is a fact about the
/// kind rune inferred, and only the first can be stated when the misspelled field is the one
/// that names the kind.
fn reject_foreign_fields(
  script: &str,
  object: &serde_json::Map<String, serde_json::Value>,
  allowed: &[&'static str],
) -> Result<(), SchemaError> {
  let known = any_field();

  match object.keys().find(|field| !known.contains(&field.as_str())) {
    Some(field) => Err(unknown_field(script, field, allowed)),
    None => Ok(()),
  }
}

fn reject_unknown_fields(
  script: &str,
  object: &serde_json::Map<String, serde_json::Value>,
  allowed: &[&'static str],
) -> Result<(), SchemaError> {
  match object.keys().find(|field| !allowed.contains(&field.as_str())) {
    Some(field) => Err(unknown_field(script, field, allowed)),
    None => Ok(()),
  }
}

/// The suggestion is drawn from the list the message is about to print, so the two cannot
/// disagree about what is legal in this position.
fn unknown_field(script: &str, field: &str, allowed: &[&'static str]) -> SchemaError {
  SchemaError::UnknownField {
    script: script.to_owned(),
    field: field.to_owned(),
    allowed: list(allowed),
    suggestion: closest(field, allowed.iter().copied()).map(str::to_owned),
  }
}

fn describe(value: &serde_json::Value) -> String {
  match value {
    serde_json::Value::Null => "null".to_owned(),
    serde_json::Value::Bool(_) => "a boolean".to_owned(),
    serde_json::Value::Number(_) => "a number".to_owned(),
    serde_json::Value::String(_) => "a string".to_owned(),
    serde_json::Value::Array(_) => "an array".to_owned(),
    serde_json::Value::Object(_) => "an object".to_owned(),
  }
}

#[cfg(test)]
mod tests {
  use std::time::Duration;

  use serde_json::json;

  use super::{KillSignal, Kind, PerOsCommand, RetryDelay, SuccessPolicy, parse};

  /// The command of a script that has one, for the tests that only care about that.
  fn command_of(config: &super::Config, name: &str, platform: &str) -> String {
    match &config.scripts[name].kind {
      Kind::Command(command) => command.select(platform).to_owned(),
      _ => panic!("`{name}` does not run a command of its own"),
    }
  }

  #[test]
  fn a_command_script_parses() {
    let config = parse(&json!({
      "scripts": { "dev": { "command": "vite", "description": "start the dev server" } }
    }))
    .expect("parses");

    assert_eq!(command_of(&config, "dev", "linux"), "vite");
    assert_eq!(config.scripts["dev"].description.as_deref(), Some("start the dev server"));
  }

  /// The `extends` half of the discriminant dispatch, with both of its fields.
  #[test]
  fn an_extending_script_parses() {
    let config = parse(&json!({
      "scripts": {
        "test": { "command": "vitest" },
        "test:ci": { "extends": "test", "appendArgs": ["--run", "--reporter=dot"] },
      }
    }))
    .expect("parses");

    let Kind::Extends { target, append_args } = &config.scripts["test:ci"].kind else {
      panic!("`test:ci` did not parse as an extending script");
    };
    assert_eq!(target, "test");
    assert_eq!(append_args, &["--run".to_owned(), "--reporter=dot".to_owned()]);
  }

  /// `appendArgs` is optional: extending a script without adding anything is how a
  /// package renames a shared command or changes only its `env`.
  #[test]
  fn an_extending_script_without_append_args_parses() {
    let config = parse(&json!({
      "scripts": { "a": { "command": "vitest" }, "b": { "extends": "a" } }
    }))
    .expect("parses");

    let Kind::Extends { append_args, .. } = &config.scripts["b"].kind else {
      panic!("`b` did not parse as an extending script");
    };
    assert!(append_args.is_empty());
  }

  #[test]
  fn an_extends_that_is_not_a_name_is_rejected() {
    let error =
      parse(&json!({ "scripts": { "b": { "extends": ["a"] } } })).unwrap_err().to_string();

    assert!(error.contains("`b`"), "{error}");
  }

  #[test]
  fn append_args_must_be_a_list_of_strings() {
    let error = parse(&json!({ "scripts": { "b": { "extends": "a", "appendArgs": "--run" } } }))
      .unwrap_err()
      .to_string();
    assert!(error.contains("appendArgs"), "{error}");

    let error = parse(&json!({ "scripts": { "b": { "extends": "a", "appendArgs": [1] } } }))
      .unwrap_err()
      .to_string();
    assert!(error.contains("appendArgs"), "{error}");
  }

  /// Test 4a.3 — all four branches, on every machine.
  ///
  /// The platform is an argument precisely so this table runs everywhere. Reading `cfg!`
  /// inside `select` would leave a Linux runner exercising one branch out of four, and
  /// the fallback branch exercised on no machine at all.
  #[test]
  fn per_os_selection_covers_every_branch() {
    let per_os = PerOsCommand {
      default: "make build".to_owned(),
      win32: Some("build.cmd".to_owned()),
      darwin: Some("make build-mac".to_owned()),
      linux: Some("make build-linux".to_owned()),
    };

    for (platform, expected) in [
      ("win32", "build.cmd"),
      ("darwin", "make build-mac"),
      ("linux", "make build-linux"),
      ("freebsd", "make build"),
    ] {
      assert_eq!(per_os.select(platform), expected, "on `{platform}`");
    }
  }

  /// A system the object names no entry for is the same case as a system rune has never
  /// heard of: `default` is what both get.
  #[test]
  fn an_absent_entry_falls_back_the_same_way_an_unknown_system_does() {
    let per_os = PerOsCommand {
      default: "rm -rf dist".to_owned(),
      win32: Some("rmdir /s /q dist".to_owned()),
      darwin: None,
      linux: None,
    };

    assert_eq!(per_os.select("win32"), "rmdir /s /q dist");
    assert_eq!(per_os.select("darwin"), "rm -rf dist");
  }

  #[test]
  fn a_per_os_command_parses() {
    let config = parse(&json!({
      "scripts": { "clean": { "command": { "default": "rm -rf dist", "win32": "rmdir /s /q dist" } } }
    }))
    .expect("parses");

    assert_eq!(command_of(&config, "clean", "win32"), "rmdir /s /q dist");
    assert_eq!(command_of(&config, "clean", "linux"), "rm -rf dist");
  }

  /// An object with only `default` is legal, and is the string form written the long way.
  #[test]
  fn a_per_os_command_with_only_default_behaves_like_a_string() {
    let config = parse(&json!({ "scripts": { "build": { "command": { "default": "tsc" } } } }))
      .expect("parses");

    assert_eq!(command_of(&config, "build", "win32"), "tsc");
  }

  #[test]
  fn a_per_os_command_rejects_a_key_that_is_not_an_operating_system() {
    let error = parse(&json!({
      "scripts": { "clean": { "command": { "default": "rm -rf dist", "windows": "rmdir" } } }
    }))
    .unwrap_err()
    .to_string();

    assert!(error.contains("`clean`"), "{error}");
    assert!(error.contains("`windows`"), "{error}");
  }

  #[test]
  fn a_command_that_is_neither_a_string_nor_an_object_is_rejected() {
    let error =
      parse(&json!({ "scripts": { "build": { "command": ["tsc"] } } })).unwrap_err().to_string();

    assert!(error.contains("`build`"), "{error}");
  }

  /// Test 2.13 — the typo case. Naming the field without naming the script leaves the
  /// user searching a config that may have thirty entries.
  #[test]
  fn an_unknown_field_names_the_script_and_the_field() {
    let error =
      parse(&json!({ "scripts": { "test": { "comand": "vitest" } } })).unwrap_err().to_string();

    assert!(error.contains("`test`"), "{error}");
    assert!(error.contains("`comand`"), "{error}");
  }

  /// Test R24.1 — the field naming the kind, misspelled, and everything else correct.
  ///
  /// Losing that field drops the entry into the rule set for a kind rune could not read,
  /// where the fields that are still right genuinely are not allowed. Naming one of those
  /// sends the reader to a line they must not change, so what the message leaves out is
  /// asserted beside what it says.
  #[test]
  fn a_misspelled_kind_is_named_and_the_correct_fields_are_not() {
    let error = parse(&json!({
      "scripts": {
        "check": {
          "paralell": ["lint", "format", "typecheck"],
          "continueOnError": true,
          "description": "lint, format and types at once"
        }
      }
    }))
    .unwrap_err()
    .to_string();

    assert!(!error.contains("continueOnError"), "a field the user must not change: {error}");

    insta::with_settings!({ description => "a group whose `parallel` is misspelled" }, {
      insta::assert_snapshot!(error);
    });
  }

  /// Test R24.2 — the same mistake where the field left behind has a rule of its own.
  ///
  /// `appendArgs` without `extends` is a real rule and a good message, and it is the wrong
  /// answer here: it describes the kind rune inferred rather than the word the user typed.
  #[test]
  fn a_misspelled_extends_is_named_rather_than_the_arguments_beside_it() {
    let error = parse(&json!({
      "scripts": { "test:ci": { "extnds": "test", "appendArgs": ["--run"] } }
    }))
    .unwrap_err()
    .to_string();

    assert!(error.contains("`extnds`"), "{error}");
    assert!(!error.contains("appendArgs"), "{error}");
  }

  /// Test R24.3 — one transposition, in the one place the crate's matcher was not wired up.
  #[test]
  fn a_misspelled_field_is_offered_its_spelling() {
    let error =
      parse(&json!({ "scripts": { "test": { "commnd": "vitest" } } })).unwrap_err().to_string();

    assert!(error.contains("did you mean `command`?"), "{error}");
  }

  /// Test R24.4 — a suggestion that is wrong costs more than no suggestion at all.
  #[test]
  fn a_field_close_to_nothing_legal_is_offered_nothing() {
    let error = parse(&json!({ "scripts": { "test": { "command": "vitest", "webpack": 1 } } }))
      .unwrap_err()
      .to_string();

    assert!(error.contains("`webpack`"), "{error}");
    assert!(!error.contains("did you mean"), "{error}");
  }

  /// Test R24.8 — the set a field is checked against is every field any kind allows; the
  /// set the message prints stays the one that applies where the field was written.
  ///
  /// Printing the union would tell a reader a group may carry `retries`, which is the
  /// opposite mistake to the one this change repairs.
  #[test]
  fn the_allowed_list_stays_specific_to_the_entry() {
    let running = parse(&json!({ "scripts": { "a": { "command": "x", "webpack": 1 } } }))
      .unwrap_err()
      .to_string();
    assert!(running.contains("`timeout`"), "a command script may say how it is run: {running}");

    let group = parse(&json!({ "scripts": { "a": { "serial": ["b"], "webpack": 1 } } }))
      .unwrap_err()
      .to_string();
    assert!(group.contains("`continueOnError`"), "a group's own field is missing: {group}");
    assert!(!group.contains("`timeout`"), "a group is not a process: {group}");

    let unreadable =
      parse(&json!({ "scripts": { "a": { "webpack": 1 } } })).unwrap_err().to_string();
    assert!(unreadable.contains("`parallel`"), "the kind selectors are missing: {unreadable}");
    assert!(!unreadable.contains("`timeout`"), "the kind is not known: {unreadable}");
  }

  /// Test R24.10 — the one way a check placed ahead of the kind dispatch breaks a config
  /// that works today.
  #[test]
  fn every_legal_variant_still_parses() {
    parse(&json!({
      "scripts": {
        "test": { "command": "vitest", "dependsOn": ["build"], "timeout": 1000, "interactive": true, "cwd": "apps/api", "env": { "CI": "1" }, "envFile": ".env", "description": "d" },
        "test:ci": { "extends": "test", "appendArgs": ["--run"], "retries": 2, "killSignal": "SIGINT" },
        "ci": { "serial": ["test"], "continueOnError": true, "description": "d" },
        "dev": { "parallel": ["test"], "continueOnError": true, "successPolicy": "first" },
      }
    }))
    .expect("every legal variant parses");
  }

  /// Test R24.5 — one sentence answered two unrelated mistakes, and contradicted itself
  /// doing it: what it reported having found was the config it had just asked for.
  #[test]
  fn a_missing_scripts_key_and_an_unusable_one_read_differently() {
    let missing = parse(&json!({ "name": "x" })).unwrap_err().to_string();
    let unusable = parse(&json!({ "scripts": [] })).unwrap_err().to_string();

    assert_ne!(missing, unusable);
    for message in [&missing, &unusable] {
      assert!(!message.contains("found an object"), "{message}");
    }

    insta::with_settings!({ description => "no `scripts` key, and a `scripts` that is a list" }, {
      insta::assert_snapshot!(format!("{missing}\n\n---\n\n{unusable}"));
    });
  }

  /// Test R24.6 — the reported value is the one that is wrong, and four ways of writing it
  /// wrong read as four different things.
  #[test]
  fn the_kind_of_value_under_scripts_is_named() {
    let messages: Vec<String> = [json!([]), json!("hello"), json!(42), json!(null)]
      .into_iter()
      .map(|value| parse(&json!({ "scripts": value })).unwrap_err().to_string())
      .collect();

    for message in &messages {
      assert!(message.contains("`scripts`"), "{message}");
    }

    let distinct: std::collections::BTreeSet<&String> = messages.iter().collect();
    assert_eq!(distinct.len(), messages.len(), "{messages:?}");
  }

  /// Test 2.14 — listing the legal discriminants matters more as later changes add them.
  #[test]
  fn a_script_with_no_discriminant_lists_the_legal_ones() {
    let error = parse(&json!({ "scripts": { "empty": {} } })).unwrap_err().to_string();

    assert!(error.contains("`empty`"), "{error}");
    assert!(error.contains("`command`"), "{error}");
    assert!(error.contains("`extends`"), "{error}");
  }

  /// The `serial` half of the dispatch, with both of its fields.
  #[test]
  fn a_serial_group_parses() {
    let config = parse(&json!({
      "scripts": {
        "ci": { "serial": ["lint", "test"], "continueOnError": true },
      }
    }))
    .expect("parses");

    let Kind::Serial { members, continue_on_error } = &config.scripts["ci"].kind else {
      panic!("`ci` did not parse as a group");
    };
    assert_eq!(members, &["lint".to_owned(), "test".to_owned()]);
    assert!(*continue_on_error);
  }

  /// Stopping at the first failure is the default, so a group that says nothing about it
  /// must not quietly run everything.
  #[test]
  fn a_group_stops_at_the_first_failure_unless_it_says_otherwise() {
    let config = parse(&json!({ "scripts": { "ci": { "serial": ["lint"] } } })).expect("parses");

    let Kind::Serial { continue_on_error, .. } = &config.scripts["ci"].kind else {
      panic!("`ci` did not parse as a group");
    };
    assert!(!*continue_on_error);
  }

  #[test]
  fn a_serial_that_is_not_a_list_of_names_is_rejected() {
    let error =
      parse(&json!({ "scripts": { "ci": { "serial": "lint" } } })).unwrap_err().to_string();
    assert!(error.contains("`ci`"), "{error}");
    assert!(error.contains("`serial`"), "{error}");

    let error = parse(&json!({ "scripts": { "ci": { "serial": [1] } } })).unwrap_err().to_string();
    assert!(error.contains("element 0"), "{error}");
  }

  #[test]
  fn a_continue_on_error_that_is_not_a_boolean_is_rejected() {
    let error =
      parse(&json!({ "scripts": { "ci": { "serial": ["a"], "continueOnError": "yes" } } }))
        .unwrap_err()
        .to_string();

    assert!(error.contains("`ci`"), "{error}");
    assert!(error.contains("`continueOnError`"), "{error}");
  }

  /// Test 5a's schema half — a group also declaring a command has no meaning rune could
  /// pick, and the conflict message has to name both words.
  #[test]
  fn a_group_that_also_declares_a_command_is_rejected() {
    let error = parse(&json!({ "scripts": { "ci": { "serial": ["a"], "command": "vitest" } } }))
      .unwrap_err()
      .to_string();

    assert!(error.contains("`ci`"), "{error}");
    assert!(error.contains("`command`"), "{error}");
    assert!(error.contains("`serial`"), "{error}");
  }

  #[test]
  fn a_group_that_also_extends_is_rejected() {
    let error = parse(&json!({ "scripts": { "ci": { "serial": ["a"], "extends": "b" } } }))
      .unwrap_err()
      .to_string();

    assert!(error.contains("`extends`"), "{error}");
    assert!(error.contains("`serial`"), "{error}");
  }

  /// `appendArgs` on a group is caught by the rule that catches it anywhere else: there
  /// is no inherited command for the arguments to join.
  #[test]
  fn a_group_that_appends_arguments_is_rejected() {
    let error = parse(&json!({ "scripts": { "ci": { "serial": ["a"], "appendArgs": ["--x"] } } }))
      .unwrap_err()
      .to_string();

    assert!(error.contains("`ci`"), "{error}");
    assert!(error.contains("appendArgs"), "{error}");
  }

  #[test]
  fn a_command_script_carries_its_prerequisites() {
    let config = parse(&json!({
      "scripts": { "build": { "command": "tsc -b", "dependsOn": ["clean"] } }
    }))
    .expect("parses");

    assert_eq!(
      config.scripts["build"].depends_on.as_deref(),
      Some(["clean".to_owned()].as_slice())
    );
  }

  /// A group already says what runs and in which order. A second ordering on the same
  /// entry would have no unambiguous meaning, so it is refused rather than picked between.
  #[test]
  fn depends_on_beside_serial_is_rejected() {
    let error = parse(&json!({ "scripts": { "ci": { "serial": ["a"], "dependsOn": ["b"] } } }))
      .unwrap_err()
      .to_string();

    assert!(error.contains("`ci`"), "{error}");
    assert!(error.contains("`dependsOn`"), "{error}");
    assert!(error.contains("`serial`"), "{error}");
  }

  /// `dependsOn` says when a script runs, never what it runs, so it cannot stand on its
  /// own. The message has to be the missing-discriminant one, not "unknown field".
  #[test]
  fn depends_on_alone_is_not_a_variant() {
    let error = parse(&json!({ "scripts": { "build": { "dependsOn": ["clean"] } } }))
      .unwrap_err()
      .to_string();

    assert!(error.contains("`build`"), "{error}");
    assert!(error.contains("no command"), "{error}");
    assert!(error.contains("`serial`"), "{error}");
  }

  #[test]
  fn a_depends_on_that_is_not_a_list_of_names_is_rejected() {
    let error = parse(&json!({ "scripts": { "b": { "command": "x", "dependsOn": "a" } } }))
      .unwrap_err()
      .to_string();

    assert!(error.contains("`dependsOn`"), "{error}");
  }

  /// The `parallel` half of the dispatch, with all three of its fields.
  #[test]
  fn a_parallel_group_parses_with_its_options() {
    let config = parse(&json!({
      "scripts": {
        "dev": { "parallel": ["server", "watch"], "continueOnError": true, "successPolicy": "first" },
      }
    }))
    .expect("parses");

    let Kind::Parallel { members, continue_on_error, policy } = &config.scripts["dev"].kind else {
      panic!("`dev` did not parse as a parallel group");
    };
    assert_eq!(members, &["server".to_owned(), "watch".to_owned()]);
    assert!(*continue_on_error);
    assert_eq!(*policy, SuccessPolicy::First);
  }

  /// Requiring every member to succeed is the default, so a group that says nothing about
  /// it must not quietly take one member's answer for the whole group.
  #[test]
  fn a_parallel_group_requires_every_member_unless_it_says_otherwise() {
    let config = parse(&json!({ "scripts": { "dev": { "parallel": ["a"] } } })).expect("parses");

    let Kind::Parallel { policy, continue_on_error, .. } = &config.scripts["dev"].kind else {
      panic!("`dev` did not parse as a parallel group");
    };
    assert_eq!(*policy, SuccessPolicy::All);
    assert!(!*continue_on_error);
  }

  /// The value a user wrote and the values they could have written, both named. A message
  /// listing neither leaves them guessing at a closed set.
  #[test]
  fn a_success_policy_outside_the_permitted_set_names_the_script_and_the_choices() {
    let error =
      parse(&json!({ "scripts": { "dev": { "parallel": ["a"], "successPolicy": "any" } } }))
        .unwrap_err()
        .to_string();

    assert!(error.contains("`dev`"), "{error}");
    assert!(error.contains("`any`"), "{error}");
    for permitted in SuccessPolicy::PERMITTED {
      assert!(error.contains(permitted), "{error}");
    }
  }

  /// A serial group runs its members one at a time, so first and last are already its
  /// member list. The option is refused where it would mean nothing, and the message says
  /// where it does belong.
  #[test]
  fn a_success_policy_on_a_serial_group_is_rejected() {
    let error =
      parse(&json!({ "scripts": { "ci": { "serial": ["a"], "successPolicy": "first" } } }))
        .unwrap_err()
        .to_string();

    assert!(error.contains("`ci`"), "{error}");
    assert!(error.contains("successPolicy"), "{error}");
    assert!(error.contains("parallel"), "{error}");
  }

  #[test]
  fn a_parallel_that_is_not_a_list_of_names_is_rejected() {
    let error =
      parse(&json!({ "scripts": { "dev": { "parallel": "server" } } })).unwrap_err().to_string();
    assert!(error.contains("`dev`"), "{error}");
    assert!(error.contains("`parallel`"), "{error}");
  }

  #[test]
  fn a_parallel_group_that_also_declares_a_command_is_rejected() {
    let error = parse(&json!({ "scripts": { "dev": { "parallel": ["a"], "command": "vite" } } }))
      .unwrap_err()
      .to_string();

    assert!(error.contains("`command`"), "{error}");
    assert!(error.contains("`parallel`"), "{error}");
  }

  /// `interactive` describes one process's relationship with the terminal, which is
  /// exactly what a command script is.
  #[test]
  fn interactive_parses_on_a_command_script() {
    let config =
      parse(&json!({ "scripts": { "dev": { "command": "vite", "interactive": true } } }))
        .expect("parses");

    assert_eq!(config.scripts["dev"].interactive, Some(true));
  }

  /// A group is not a process, and only one member of a group can own the terminal. Both
  /// kinds of group refuse it, and the message says where it belongs.
  #[test]
  fn interactive_on_a_group_is_rejected_and_says_where_it_belongs() {
    for group in ["serial", "parallel"] {
      let error = parse(&json!({
        "scripts": { "dev": { group: ["a"], "interactive": true } }
      }))
      .unwrap_err()
      .to_string();

      assert!(error.contains("`dev`"), "{group}: {error}");
      assert!(error.contains("interactive"), "{group}: {error}");
      assert!(error.contains("member"), "{group}: {error}");
    }
  }

  #[test]
  fn an_interactive_that_is_not_a_boolean_is_rejected() {
    let error =
      parse(&json!({ "scripts": { "dev": { "command": "vite", "interactive": "yes" } } }))
        .unwrap_err()
        .to_string();

    assert!(error.contains("`dev`"), "{error}");
    assert!(error.contains("interactive"), "{error}");
  }

  /// The three options a script most often carries together, all read back.
  #[test]
  fn lifecycle_options_parse_on_a_command_script() {
    let config = parse(&json!({
      "scripts": {
        "e2e": { "command": "playwright test", "timeout": 30_000, "retries": 2, "retryDelay": "exponential" },
      }
    }))
    .expect("parses");

    let lifecycle = config.scripts["e2e"].lifecycle;
    assert_eq!(lifecycle.timeout, Some(Duration::from_secs(30)));
    assert_eq!(lifecycle.retries, Some(2));
    assert_eq!(lifecycle.retry_delay, Some(RetryDelay::Exponential));
  }

  #[test]
  fn a_numeric_retry_delay_is_milliseconds() {
    let config = parse(&json!({
      "scripts": { "flaky": { "command": "vitest", "retries": 1, "retryDelay": 250 } }
    }))
    .expect("parses");

    assert_eq!(
      config.scripts["flaky"].lifecycle.retry_delay,
      Some(RetryDelay::Fixed(Duration::from_millis(250)))
    );
  }

  #[test]
  fn a_kill_signal_and_timeout_parse_on_an_extending_script() {
    let config = parse(&json!({
      "scripts": {
        "dev": { "command": "node server.js" },
        "dev:api": { "extends": "dev", "killSignal": "SIGINT", "killTimeout": 2000 },
      }
    }))
    .expect("parses");

    let lifecycle = config.scripts["dev:api"].lifecycle;
    assert_eq!(lifecycle.kill_signal, Some(KillSignal::Int));
    assert_eq!(lifecycle.kill_timeout, Some(Duration::from_secs(2)));
  }

  /// Test 5d.7's unit half — every option, on both kinds of group. A group is not a
  /// process, so the message has to point at the members rather than merely refuse.
  #[test]
  fn a_lifecycle_option_on_a_group_is_rejected_and_says_where_it_belongs() {
    for group in ["serial", "parallel"] {
      for (option, value) in [
        ("timeout", json!(1000)),
        ("retries", json!(2)),
        ("retryDelay", json!(500)),
        ("killSignal", json!("SIGINT")),
        ("killTimeout", json!(1000)),
      ] {
        let error = parse(&json!({
          "scripts": { "ci": { group: ["a"], option: value } }
        }))
        .unwrap_err()
        .to_string();

        assert!(error.contains("`ci`"), "{group}/{option}: {error}");
        assert!(error.contains(option), "{group}/{option}: {error}");
        assert!(error.contains("members"), "{group}/{option}: {error}");
      }
    }
  }

  /// A delay between attempts that will never happen is a mistake rather than a
  /// preference, so it is refused instead of quietly ignored.
  #[test]
  fn a_retry_delay_without_retries_is_rejected() {
    let error = parse(&json!({ "scripts": { "a": { "command": "x", "retryDelay": 500 } } }))
      .unwrap_err()
      .to_string();

    assert!(error.contains("`a`"), "{error}");
    assert!(error.contains("retryDelay"), "{error}");
    assert!(error.contains("retries"), "{error}");
  }

  /// The value written and the values accepted, both named. Neither alone turns the
  /// message into an edit.
  #[test]
  fn a_retry_delay_that_is_neither_a_duration_nor_exponential_is_rejected() {
    for written in [json!("fast"), json!(-5), json!(true)] {
      let error = parse(
        &json!({ "scripts": { "a": { "command": "x", "retries": 1, "retryDelay": written } } }),
      )
      .unwrap_err()
      .to_string();

      assert!(error.contains("`a`"), "{written}: {error}");
      assert!(error.contains("retryDelay"), "{written}: {error}");
      assert!(error.contains("exponential"), "{written}: {error}");
    }
  }

  #[test]
  fn an_unknown_kill_signal_names_the_value_and_the_permitted_ones() {
    let error = parse(&json!({ "scripts": { "a": { "command": "x", "killSignal": "SIGSTOP" } } }))
      .unwrap_err()
      .to_string();

    assert!(error.contains("`a`"), "{error}");
    assert!(error.contains("SIGSTOP"), "{error}");
    for permitted in KillSignal::PERMITTED {
      assert!(error.contains(permitted), "{error}");
    }
  }

  /// A duration is a count of milliseconds, so the two ways of writing one wrong — below
  /// zero and between two whole numbers — are both refused, naming the value.
  #[test]
  fn a_duration_that_is_not_a_whole_number_of_milliseconds_is_rejected() {
    for key in ["timeout", "killTimeout"] {
      for written in [json!(-1), json!(1.5), json!("30s")] {
        let error = parse(&json!({ "scripts": { "a": { "command": "x", key: written } } }))
          .unwrap_err()
          .to_string();

        assert!(error.contains("`a`"), "{key} = {written}: {error}");
        assert!(error.contains(key), "{key} = {written}: {error}");
        assert!(error.contains("milliseconds"), "{key} = {written}: {error}");
      }
    }
  }

  #[test]
  fn a_retries_count_that_is_not_a_whole_number_is_rejected() {
    let error = parse(&json!({ "scripts": { "a": { "command": "x", "retries": -1 } } }))
      .unwrap_err()
      .to_string();

    assert!(error.contains("`a`"), "{error}");
    assert!(error.contains("retries"), "{error}");
  }

  /// A script that says nothing carries nothing, so execution applies its own defaults
  /// rather than inheriting a value the config never wrote.
  #[test]
  fn a_script_that_declares_nothing_carries_no_lifecycle_options() {
    let config = parse(&json!({ "scripts": { "a": { "command": "x" } } })).expect("parses");

    assert_eq!(config.scripts["a"].lifecycle, super::Lifecycle::default());
  }

  #[test]
  fn a_config_without_scripts_is_rejected() {
    assert!(parse(&json!({ "other": {} })).is_err());
  }

  #[test]
  fn a_script_that_is_not_an_object_is_rejected() {
    let error = parse(&json!({ "scripts": { "dev": "vite" } })).unwrap_err().to_string();
    assert!(error.contains("`dev`"), "{error}");
  }

  #[test]
  fn scripts_come_back_sorted() {
    let config = parse(&json!({ "scripts": { "z": { "command": "z" }, "a": { "command": "a" } } }))
      .expect("parses");

    assert_eq!(config.scripts.keys().collect::<Vec<_>>(), vec!["a", "z"]);
  }
}
