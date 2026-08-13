//! Evaluating a config with an embedded JavaScript engine.
//!
//! Every file, entry or import, goes through the same pipeline: read, strip its
//! TypeScript, hand the JavaScript to QuickJS as a module keyed by its canonical path.
//! Specifiers are turned into those keys by [`crate::resolve`] and nothing else.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use rquickjs::loader::{ImportAttributes, Loader, Resolver};
use rquickjs::module::Declared;
use rquickjs::{CatchResultExt, CaughtError, Context, Ctx, Module, Runtime, Value};
use thiserror::Error;

use crate::builtin;
use crate::env::{Environment, Observations, ObservedEnvironment};
use crate::globals::install;
use crate::limits::{self, Limits};
use crate::paths::Shown;
use crate::resolve::{ResolveError, canonical, resolve};
use crate::strip::{StripError, specifiers_importing, strip_types};

/// Everything that can stop a config being loaded.
///
/// Every variant names a file, and none of them hands on the words of the engine, the
/// parser, the standard library or the operating system as the whole of what is printed.
/// These are read by someone who wrote a config, not by someone who wrote rune.
#[derive(Debug, Error)]
pub enum EvalError {
  #[error("{source}{}", format_chain(.chain))]
  Resolve { source: Box<ResolveError>, chain: Vec<Shown> },

  #[error("cannot read {path}: {source}")]
  Unreadable { path: Shown, source: std::io::Error },

  #[error("{}", format_strip_errors(.path, .errors))]
  Strip { path: Shown, errors: Vec<StripError> },

  #[error(
    "{importer} imports `{name}` from `{specifier}`, which does not export it{}",
    format_chain(.chain)
  )]
  MissingExport { importer: Shown, specifier: String, name: String, chain: Vec<Shown> },

  #[error("{}", format_runtime(.path, .message, .trace))]
  Runtime { path: Shown, message: String, trace: String },

  #[error(
    "{path} did not finish being evaluated within {} ms\n\n\
     rune evaluates a config to completion before any script starts, so work that waits or \
     loops without end never finishes and no rune command in this repository can run.\n\n\
     set {} higher for a config that genuinely needs longer.",
    .limit.as_millis(),
    limits::TIME_VARIABLE
  )]
  TimeLimit { path: Shown, limit: Duration },

  #[error(
    "{path} asked for more memory than a config may use while it is evaluated: the limit is \
     {limit_mb} MB\n\n\
     rune stops here itself so that the operating system does not stop it instead, which ends \
     the process with nothing on screen.\n\n\
     set {} higher for a config that genuinely needs more.",
    limits::MEMORY_VARIABLE
  )]
  MemoryLimit { path: Shown, limit_mb: u64 },

  #[error("{path}: {message}")]
  Shape { path: Shown, message: String },
}

impl EvalError {
  /// Whether a ceiling stopped the evaluation, rather than anything the config said.
  fn is_ceiling(&self) -> bool {
    matches!(self, Self::TimeLimit { .. } | Self::MemoryLimit { .. })
  }
}

/// One block per problem, each headed by the position editors and terminals both linkify.
fn format_strip_errors(path: &Shown, errors: &[StripError]) -> String {
  errors
    .iter()
    .map(|error| match &error.position {
      Some(at) => format!(
        "{path}:{}:{}\n  {}\n\n  {} | {}",
        at.line, at.column, error.message, at.line, at.text
      ),
      None => format!("{path}:\n  {}", error.message),
    })
    .collect::<Vec<_>>()
    .join("\n\n")
}

/// The engine's sentence, with rune's around it.
///
/// The inner sentence is often the most informative part — `cannot read property 'f' of
/// undefined` says exactly what happened — and it is useless on its own because it names
/// no file.
fn format_runtime(path: &Shown, message: &str, trace: &str) -> String {
  let mut text =
    format!("{path} failed while it was being evaluated as a rune config\n\n{message}");

  if !trace.trim().is_empty() {
    text.push_str("\n\n");
    text.push_str(trace.trim_end());
  }
  text
}

/// The imports followed to reach the file the message is about, when that took more than
/// the config itself.
fn format_chain(chain: &[Shown]) -> String {
  if chain.len() < 2 {
    return String::new();
  }

  let mut listed = String::from("\n\nimports followed to get here:");
  for step in chain {
    listed.push_str("\n  ");
    listed.push_str(&step.to_string());
  }
  listed
}

/// Carries a rich Rust error past QuickJS.
///
/// A loader failure reaches us again as a JavaScript `ReferenceError`, which flattens
/// the error into a string and loses its structure. Recording it here keeps the real
/// error, and the JavaScript exception becomes a fallback rather than the only copy.
type ErrorSlot = Rc<RefCell<Option<EvalError>>>;

/// One `import`, as the resolver saw it: who wrote it, what they wrote, and the module key
/// it became.
///
/// The engine keeps none of this. It is what lets a failure name the file that wrote the
/// import rather than the module that refused it.
struct Edge {
  importer: PathBuf,
  specifier: String,
  key: String,
}

type Imports = Rc<RefCell<Vec<Edge>>>;

struct PathResolver {
  slot: ErrorSlot,
  imports: Imports,
  entry: Shown,
}

impl PathResolver {
  fn record(&self, base: &str, specifier: &str, key: &str) {
    self.imports.borrow_mut().push(Edge {
      importer: PathBuf::from(base),
      specifier: specifier.to_owned(),
      key: key.to_owned(),
    });
  }
}

impl Resolver for PathResolver {
  fn resolve(
    &mut self,
    _ctx: &Ctx<'_>,
    base: &str,
    name: &str,
    _attributes: Option<ImportAttributes<'_>>,
  ) -> rquickjs::Result<String> {
    // The one specifier that is its own key: there is no file behind it to canonicalize.
    if builtin::is_builtin(name) {
      self.record(base, name, name);
      return Ok(name.to_owned());
    }

    match resolve(self.entry.root(), Path::new(base), name) {
      Ok(path) => {
        let key = path.to_string_lossy().into_owned();
        self.record(base, name, &key);
        Ok(key)
      }
      Err(error) => {
        let message = error.to_string();
        let chain = chain_to(&self.imports.borrow(), base, &self.entry);
        self.slot.borrow_mut().replace(EvalError::Resolve { source: Box::new(error), chain });
        Err(rquickjs::Error::new_resolving_message(base, name, message))
      }
    }
  }
}

struct StrippingLoader {
  slot: ErrorSlot,
  entry: Shown,
}

impl Loader for StrippingLoader {
  fn load<'js>(
    &mut self,
    ctx: &Ctx<'js>,
    name: &str,
    _attributes: Option<ImportAttributes<'js>>,
  ) -> rquickjs::Result<Module<'js, Declared>> {
    if builtin::is_builtin(name) {
      return Module::declare(ctx.clone(), name, builtin::SOURCE);
    }

    let path = Path::new(name);
    match read_and_strip(self.entry.root(), path) {
      Ok(code) => Module::declare(ctx.clone(), name, code),
      Err(error) => {
        let message = error.to_string();
        self.slot.borrow_mut().replace(error);
        Err(rquickjs::Error::new_loading_message(name, message))
      }
    }
  }
}

/// The imports followed from the entry config down to `key`, entry first.
///
/// A config graph may hold cycles, which are legal, so the walk stops at a file it has
/// already passed rather than following the circle.
fn chain_to(edges: &[Edge], key: &str, entry: &Shown) -> Vec<Shown> {
  let mut walked = vec![key.to_owned()];

  while let Some(edge) = edges.iter().find(|edge| edge.key == walked[walked.len() - 1]) {
    let importer = edge.importer.to_string_lossy().into_owned();
    if walked.contains(&importer) {
      break;
    }
    walked.push(importer);
  }

  walked.iter().rev().map(|step| entry.sibling(Path::new(step))).collect()
}

fn read_and_strip(root: &Path, path: &Path) -> Result<String, EvalError> {
  let source = std::fs::read_to_string(path)
    .map_err(|source| EvalError::Unreadable { path: Shown::new(root, path), source })?;

  strip_types(&source, path)
    .map(|stripped| stripped.code)
    .map_err(|errors| EvalError::Strip { path: Shown::new(root, path), errors })
}

/// What one evaluation produced: the config itself, plus what it read of the environment
/// on the way. The cache key is built from the second part as much as the first.
#[derive(Debug)]
pub struct Evaluated {
  pub value: serde_json::Value,
  pub observed: Observations,
}

/// Evaluates `entry` and returns its default export as JSON.
///
/// Relative imports are followed recursively through the same pipeline. Non-relative
/// specifiers used for a runtime value are an error: there is no npm resolution here.
/// `root` is the repository the config belongs to, and decides only how the files a
/// failure names are spelled.
pub fn evaluate_config(
  root: &Path,
  entry: &Path,
  environment: &Environment,
) -> Result<Evaluated, EvalError> {
  let entry = canonical(root, entry)
    .map_err(|source| EvalError::Resolve { source: Box::new(source), chain: Vec::new() })?;
  let shown = Shown::new(root, &entry);
  let code = read_and_strip(root, &entry)?;
  let observed = ObservedEnvironment::new(environment.clone());

  let slot: ErrorSlot = Rc::new(RefCell::new(None));
  let imports: Imports = Rc::new(RefCell::new(Vec::new()));
  let runtime = Runtime::new().map_err(|error| runtime_error(&error, &shown))?;
  runtime.set_loader(
    PathResolver { slot: Rc::clone(&slot), imports: Rc::clone(&imports), entry: shown.clone() },
    StrippingLoader { slot: Rc::clone(&slot), entry: shown.clone() },
  );

  // A config that imports itself in a cycle would otherwise recurse until the stack
  // runs out. QuickJS unwinds a memory limit as a normal exception instead.
  runtime.set_max_stack_size(STACK_LIMIT);

  let ceilings = Ceilings::new(Limits::from_environment(environment), shown.clone());
  runtime.set_memory_limit(ceilings.limits.memory_bytes());
  runtime.set_interrupt_handler(Some(ceilings.watch()));

  let context = Context::full(&runtime).map_err(|error| runtime_error(&error, &shown))?;
  let key = entry.to_string_lossy().into_owned();

  let outcome = context.with(|ctx| {
    install(&ctx, &observed).map_err(|error| runtime_error(&error, &shown))?;

    let value = match evaluate_entry(&ctx, &key, code, &shown, &ceilings, &imports) {
      // A promise is an object, so nothing downstream can tell one apart: it becomes an
      // empty config and is reported as an object, which is the one word that does not
      // lead anybody to the missing `await`.
      Ok(value) if value.is_promise() => return Err(unawaited_default_export(&shown)),
      Ok(value) => to_json(&value, &shown)?,
      // A ceiling is why this evaluation ended, whatever it was doing at the time.
      Err(caught) if caught.is_ceiling() => return Err(caught),
      // The slot holds the real error whenever a loader or resolver rejected the
      // module; the JavaScript exception is only the flattened copy of it.
      Err(caught) => return Err(slot.borrow_mut().take().unwrap_or(caught)),
    };

    if !value.is_object() {
      return Err(missing_default_export(&shown));
    }

    Ok(Evaluated { value, observed: observed.observations() })
  });

  outcome.map_err(|error| ceilings.or_out_of_memory(&runtime, error))
}

/// QuickJS unwinds this as a catchable exception, so a cyclic or runaway import graph
/// surfaces as an error rather than as a stack overflow that takes the process with it.
const STACK_LIMIT: usize = 1 << 20;

/// The ceilings one evaluation runs under, and what it takes to report meeting one.
struct Ceilings {
  limits: Limits,
  /// Set by the interrupt handler at the moment it decides to stop the engine. The engine
  /// says only `interrupted`, which is a word about itself; this is what knows rune set a
  /// deadline and that the deadline is what passed.
  overran: Rc<Cell<bool>>,
  /// Set when the engine ended by refusing an allocation. Half of the memory answer, kept
  /// until the other half can be read.
  refused: Cell<bool>,
  /// Where to point when a stack cannot say which file the engine was in.
  entry: Shown,
}

impl Ceilings {
  fn new(limits: Limits, entry: Shown) -> Self {
    Self { limits, overran: Rc::new(Cell::new(false)), refused: Cell::new(false), entry }
  }

  /// The handler QuickJS calls while it runs. Once it has stopped the engine it keeps
  /// saying so, because the unwind is not finished until control comes back to rune.
  fn watch(&self) -> Box<dyn FnMut() -> bool> {
    let overran = Rc::clone(&self.overran);
    let limit = self.limits.time;
    let started = Instant::now();

    Box::new(move || {
      if !overran.get() && started.elapsed() >= limit {
        overran.set(true);
      }

      overran.get()
    })
  }

  /// Records how the engine ended, and answers with the time ceiling when that is what
  /// ended it.
  ///
  /// Memory is not decided here. What the engine threw is only half of that answer, and
  /// the other half is the runtime's own accounting, which cannot be read while a context
  /// is open.
  fn reached(&self, error: &CaughtError<'_>) -> Option<EvalError> {
    self.refused.set(refused_an_allocation(error));

    self
      .overran
      .get()
      .then(|| EvalError::TimeLimit { path: self.evaluating(error), limit: self.limits.time })
  }

  /// Names the memory ceiling as the cause when the engine refused an allocation with the
  /// heap it was given already full.
  fn or_out_of_memory(&self, runtime: &Runtime, error: EvalError) -> EvalError {
    if error.is_ceiling() || !self.refused.get() || !heap_is_full(runtime) {
      return error;
    }

    EvalError::MemoryLimit { path: self.entry.clone(), limit_mb: self.limits.memory_mb }
  }

  /// The file the engine was reading when it was stopped.
  ///
  /// A loop inside an imported helper is that helper's, and the stack is the only place
  /// that says so. Frames naming rune's own bootstrap or the module rune supplies point at
  /// nothing a user can open, so the answer is the first frame that is a file on disk.
  fn evaluating(&self, error: &CaughtError<'_>) -> Shown {
    let CaughtError::Exception(exception) = error else {
      return self.entry.clone();
    };

    exception
      .stack()
      .and_then(|stack| stack.lines().find_map(readable_file))
      .map_or_else(|| self.entry.clone(), |path| self.entry.sibling(&path))
  }
}

fn readable_file(frame: &str) -> Option<PathBuf> {
  let path = PathBuf::from(crate::trace::frame(frame)?.path);

  path.is_file().then_some(path)
}

/// The engine's own marker for an allocation it could not make.
///
/// QuickJS throws a bare `null` when it runs out of memory, because building the error
/// object it would rather throw needs memory of its own. When enough was freed on the way
/// out it manages the object, and then the failure arrives as an ordinary exception.
fn refused_an_allocation(error: &CaughtError<'_>) -> bool {
  match error {
    CaughtError::Value(value) => value.is_null(),
    CaughtError::Exception(exception) => exception.message().as_deref() == Some("out of memory"),
    CaughtError::Error(_) => false,
  }
}

/// Whether the heap the engine was given is at least half full.
///
/// A config that threw a `null` of its own is the one other thing that looks like the
/// engine refusing an allocation, and a config that threw has filled nothing: a runtime
/// with the whole config graph loaded holds about a megabyte.
fn heap_is_full(runtime: &Runtime) -> bool {
  let usage = runtime.memory_usage();

  usage.malloc_size.saturating_mul(2) >= usage.malloc_limit
}

fn evaluate_entry<'js>(
  ctx: &Ctx<'js>,
  key: &str,
  code: String,
  entry: &Shown,
  ceilings: &Ceilings,
  imports: &Imports,
) -> Result<Value<'js>, EvalError> {
  let caught = |error: &CaughtError<'_>| caught_error(error, ceilings, imports);

  let declared = Module::declare(ctx.clone(), key, code).catch(ctx).map_err(|e| caught(&e))?;
  let (module, promise) = declared.eval().catch(ctx).map_err(|e| caught(&e))?;
  promise.finish::<()>().catch(ctx).map_err(|e| caught(&e))?;

  // A module that exports no `default` has nothing else this could be, so the generic
  // engine error is replaced by the one thing the user needs to be told.
  module.get::<_, Value<'js>>("default").map_err(|_| missing_default_export(entry))
}

/// The most common authoring mistake, and the one an engine-level error explains worst.
fn missing_default_export(entry: &Shown) -> EvalError {
  EvalError::Shape {
    path: entry.clone(),
    message: "a rune config must default-export an object\n\n\
              for example:\n\n  \
              export default {\n    \
              scripts: {\n      \
              dev: { command: \"vite\" },\n    \
              },\n  \
              };"
      .to_owned(),
  }
}

/// A config that exported the work instead of what the work produced.
///
/// The repair is one word, and naming the promise is what leads a reader to it. A config is
/// evaluated to completion before any script starts, so `await` at the top level is
/// supported and is the whole of the fix.
fn unawaited_default_export(entry: &Shown) -> EvalError {
  EvalError::Shape {
    path: entry.clone(),
    message: "a rune config must default-export an object; this one exports a promise\n\n\
              rune evaluates a config to completion before any script starts, so a config \
              that computes its scripts waits for that work itself:\n\n  \
              export default await buildScripts();"
      .to_owned(),
  }
}

fn caught_error(error: &CaughtError<'_>, ceilings: &Ceilings, imports: &Imports) -> EvalError {
  if let Some(reached) = ceilings.reached(error) {
    return reached;
  }
  if let Some(unexported) = missing_export(error, imports, &ceilings.entry) {
    return unexported;
  }

  let path = ceilings.evaluating(error);
  let (message, stack) = engine_words(error);

  // The engine reports positions in generated JavaScript. The user wrote TypeScript.
  EvalError::Runtime { message, trace: crate::trace::remap(&stack, path.root()), path }
}

/// What the engine said, split from where it says it happened.
fn engine_words(error: &CaughtError<'_>) -> (String, String) {
  match error {
    CaughtError::Exception(exception) => (
      exception.message().unwrap_or_else(|| "the engine gave no reason".to_owned()),
      exception.stack().unwrap_or_default(),
    ),
    // The one engine failure with no sentence worth passing on: it is written about the
    // engine's own scheduling, and rune already knows what it means for a config.
    CaughtError::Error(rquickjs::Error::WouldBlock) => (NEVER_SETTLES.to_owned(), String::new()),
    CaughtError::Error(error) => (error.to_string(), String::new()),
    CaughtError::Value(value) => (format!("{value:?} was thrown"), String::new()),
  }
}

/// A config awaiting something nothing will ever resolve.
///
/// `await` works and is meant to; what does not work is waiting for a promise that is not
/// already on its way to settling, because there is nothing left to run that could settle
/// it.
const NEVER_SETTLES: &str = "it is waiting for a promise that nothing will settle\n\n\
   rune evaluates a config to completion before any script starts, and no work is \
   scheduled for later, so a promise waiting on a timer, on input or on another process \
   never resolves.";

/// An import of a name the module it names does not provide.
///
/// The engine answers this one with an exception carrying no stack at all, so nothing in
/// what it says names the file that wrote the import. Its two names are read out of it and
/// the file is found in rune's own record of what resolved from where; none of the
/// engine's words are printed. A phrasing this does not recognise falls through to the
/// wrap, which still names the file being evaluated.
fn missing_export(error: &CaughtError<'_>, imports: &Imports, entry: &Shown) -> Option<EvalError> {
  let CaughtError::Exception(exception) = error else {
    return None;
  };
  if exception.stack().is_some_and(|stack| !stack.trim().is_empty()) {
    return None;
  }

  let (name, module) = read_missing_export(&exception.message()?)?;
  let edges = imports.borrow();
  let candidates: Vec<&Edge> = edges.iter().filter(|edge| edge.key == module).collect();

  // With one importer there is nothing to choose between. With several, the file that
  // asked for this name is the one to name, and the file itself is what says so.
  let edge = match candidates.as_slice() {
    [] => return None,
    [only] => only,
    several => several.iter().find(|edge| asked_for(edge, &name)).or(several.first())?,
  };

  Some(EvalError::MissingExport {
    importer: entry.sibling(&edge.importer),
    specifier: edge.specifier.clone(),
    name,
    chain: chain_to(&edges, &edge.importer.to_string_lossy(), entry),
  })
}

fn read_missing_export(message: &str) -> Option<(String, String)> {
  let rest = message.strip_prefix("Could not find export '")?;
  let (name, module) = rest.split_once("' in module '")?;

  Some((name.to_owned(), module.strip_suffix('\'')?.to_owned()))
}

fn asked_for(edge: &Edge, name: &str) -> bool {
  let Ok(source) = std::fs::read_to_string(&edge.importer) else {
    return false;
  };

  specifiers_importing(&source, name).contains(&edge.specifier)
}

fn runtime_error(error: &rquickjs::Error, path: &Shown) -> EvalError {
  EvalError::Runtime { path: path.clone(), message: error.to_string(), trace: String::new() }
}

/// Converts an evaluated JavaScript value into JSON.
///
/// This is the boundary where a config stops being a program and becomes data. Anything
/// with no JSON meaning — a function, a symbol — is rejected by name rather than dropped,
/// because silently losing a field is the worst way to tell a user their config is wrong.
fn to_json(value: &Value<'_>, path: &Shown) -> Result<serde_json::Value, EvalError> {
  if value.is_undefined() || value.is_null() {
    return Ok(serde_json::Value::Null);
  }

  if let Some(boolean) = value.as_bool() {
    return Ok(serde_json::Value::Bool(boolean));
  }

  if let Some(number) = value.as_number() {
    return serde_json::Number::from_f64(number).map(serde_json::Value::Number).ok_or_else(|| {
      EvalError::Shape {
        path: path.clone(),
        message: format!("`{number}` has no JSON representation"),
      }
    });
  }

  if let Some(string) = value.as_string() {
    let string = string.to_string().map_err(|error| runtime_error(&error, path))?;
    return Ok(serde_json::Value::String(string));
  }

  if let Some(array) = value.as_array() {
    let mut items = Vec::with_capacity(array.len());
    for item in array.iter::<Value<'_>>() {
      items.push(to_json(&item.map_err(|error| runtime_error(&error, path))?, path)?);
    }
    return Ok(serde_json::Value::Array(items));
  }

  if value.is_function() {
    return Err(EvalError::Shape {
      path: path.clone(),
      message: "a config may not contain functions; it must be plain data".to_owned(),
    });
  }

  if let Some(object) = value.as_object() {
    let mut map = serde_json::Map::new();
    for entry in object.props::<String, Value<'_>>() {
      let (key, item) = entry.map_err(|error| runtime_error(&error, path))?;
      map.insert(key, to_json(&item, path)?);
    }
    return Ok(serde_json::Value::Object(map));
  }

  Err(EvalError::Shape {
    path: path.clone(),
    message: format!("value of type `{}` has no JSON representation", value.type_of().as_str()),
  })
}

#[cfg(test)]
mod tests {
  use std::io::{Error as IoError, ErrorKind};
  use std::path::Path;
  use std::time::Duration;

  use super::EvalError;
  use crate::paths::Shown;
  use crate::resolve::ResolveError;
  use crate::strip::{Position, StripError};

  const ROOT: &str = if cfg!(windows) { r"C:\repo" } else { "/repo" };

  fn shown(name: &str) -> Shown {
    let root = Path::new(ROOT);
    Shown::new(root, &root.join(name))
  }

  /// Every way a specifier can fail to become a file.
  ///
  /// The match is the checker: a variant added later leaves it non-exhaustive and this
  /// crate stops compiling until the new kind has a sample here.
  fn every_resolve_failure() -> Vec<ResolveError> {
    let failures = vec![
      ResolveError::NotRelative {
        specifier: "lodash".to_owned(),
        importer: shown("rune.config.ts"),
      },
      ResolveError::NotFound {
        specifier: "./nope".to_owned(),
        importer: shown("rune.config.ts"),
        tried: vec![shown("nope.ts")],
      },
      ResolveError::Unreadable {
        path: shown("rune.config.ts"),
        source: IoError::new(ErrorKind::PermissionDenied, "denied"),
      },
    ];

    for failure in &failures {
      match failure {
        ResolveError::NotRelative { .. }
        | ResolveError::NotFound { .. }
        | ResolveError::Unreadable { .. } => {}
      }
    }

    failures
  }

  /// Every way loading a config can fail, checked the same way.
  fn every_failure() -> Vec<EvalError> {
    let mut failures: Vec<EvalError> = every_resolve_failure()
      .into_iter()
      .map(|source| EvalError::Resolve { source: Box::new(source), chain: Vec::new() })
      .collect();

    failures.extend([
      EvalError::Unreadable {
        path: shown("rune.config.ts"),
        source: IoError::new(ErrorKind::PermissionDenied, "denied"),
      },
      EvalError::Strip {
        path: shown("rune.config.ts"),
        errors: vec![StripError {
          message: "Unexpected token".to_owned(),
          position: Some(Position { line: 3, column: 24, text: "const x = {;".to_owned() }),
        }],
      },
      EvalError::MissingExport {
        importer: shown("rune.config.ts"),
        specifier: crate::builtin::MODULE.to_owned(),
        name: "nope".to_owned(),
        chain: Vec::new(),
      },
      EvalError::Runtime {
        path: shown("rune.config.ts"),
        message: "cannot read property 'f' of undefined".to_owned(),
        trace: String::new(),
      },
      EvalError::TimeLimit { path: shown("rune.config.ts"), limit: Duration::from_millis(250) },
      EvalError::MemoryLimit { path: shown("rune.config.ts"), limit_mb: 256 },
      EvalError::Shape { path: shown("rune.config.ts"), message: "not an object".to_owned() },
    ]);

    for failure in &failures {
      match failure {
        EvalError::Resolve { .. }
        | EvalError::Unreadable { .. }
        | EvalError::Strip { .. }
        | EvalError::MissingExport { .. }
        | EvalError::Runtime { .. }
        | EvalError::TimeLimit { .. }
        | EvalError::MemoryLimit { .. }
        | EvalError::Shape { .. } => {}
      }
    }

    failures
  }

  /// Test R23.11 — the rule, over the type rather than over a list of messages somebody
  /// remembered to keep up to date.
  ///
  /// The first line naming a file is what a foreign sentence cannot do: the parser, the
  /// engine and the standard library all describe what went wrong and none of them knows
  /// which file the user has to open. A message that leads with one of their sentences is
  /// that layer talking to the user directly.
  #[test]
  fn every_failure_leads_with_the_file_it_is_about() {
    for failure in every_failure() {
      let message = failure.to_string();
      let first = message.lines().next().unwrap_or_default();

      assert!(first.contains("rune.config.ts"), "the first line names no file:\n{message}");
    }
  }

  /// The repository the file sits in is the reader's own working directory, and printing
  /// it back to them costs most of a terminal line.
  #[test]
  fn no_failure_prints_the_path_to_the_repository() {
    for failure in every_failure() {
      let message = failure.to_string();

      assert!(!message.contains(ROOT), "an absolute path survived:\n{message}");
    }
  }
}
