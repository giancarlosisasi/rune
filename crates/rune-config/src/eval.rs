//! Evaluating a config with an embedded JavaScript engine.
//!
//! Every file, entry or import, goes through the same pipeline: read, strip its
//! TypeScript, hand the JavaScript to QuickJS as a module keyed by its canonical path.
//! Specifiers are turned into those keys by [`crate::resolve`] and nothing else.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use rquickjs::loader::{ImportAttributes, Loader, Resolver};
use rquickjs::module::Declared;
use rquickjs::{CatchResultExt, CaughtError, Context, Ctx, Module, Runtime, Value};
use thiserror::Error;

use crate::builtin;
use crate::env::{Environment, ObservedEnvironment};
use crate::globals::install;
use crate::resolve::{ResolveError, canonical, resolve};
use crate::strip::{StripError, strip_types};

#[derive(Debug, Error)]
pub enum EvalError {
  #[error("{0}")]
  Resolve(#[from] ResolveError),

  #[error("cannot read {}: {source}", .path.display())]
  Unreadable { path: PathBuf, source: std::io::Error },

  #[error("{}:\n{}", .path.display(), format_strip_errors(.errors))]
  Strip { path: PathBuf, errors: Vec<StripError> },

  #[error("{message}")]
  Runtime { message: String },

  #[error("{}: {message}", .path.display())]
  Shape { path: PathBuf, message: String },
}

fn format_strip_errors(errors: &[StripError]) -> String {
  let mut listed = String::new();
  for error in errors {
    listed.push_str("  ");
    listed.push_str(&error.message);
    listed.push('\n');
  }
  listed
}

/// Carries a rich Rust error past QuickJS.
///
/// A loader failure reaches us again as a JavaScript `ReferenceError`, which flattens
/// the error into a string and loses its structure. Recording it here keeps the real
/// error, and the JavaScript exception becomes a fallback rather than the only copy.
type ErrorSlot = Rc<RefCell<Option<EvalError>>>;

struct PathResolver {
  slot: ErrorSlot,
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
      return Ok(name.to_owned());
    }

    match resolve(Path::new(base), name) {
      Ok(path) => Ok(path.to_string_lossy().into_owned()),
      Err(error) => {
        let message = error.to_string();
        self.slot.borrow_mut().replace(EvalError::Resolve(error));
        Err(rquickjs::Error::new_resolving_message(base, name, message))
      }
    }
  }
}

struct StrippingLoader {
  slot: ErrorSlot,
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
    match read_and_strip(path) {
      Ok(code) => Module::declare(ctx.clone(), name, code),
      Err(error) => {
        let message = error.to_string();
        self.slot.borrow_mut().replace(error);
        Err(rquickjs::Error::new_loading_message(name, message))
      }
    }
  }
}

fn read_and_strip(path: &Path) -> Result<String, EvalError> {
  let source = std::fs::read_to_string(path)
    .map_err(|source| EvalError::Unreadable { path: path.to_owned(), source })?;

  strip_types(&source, path)
    .map(|stripped| stripped.code)
    .map_err(|errors| EvalError::Strip { path: path.to_owned(), errors })
}

/// What one evaluation produced: the config itself, plus the environment variables it
/// read on the way. The cache key is built from the second part as much as the first.
#[derive(Debug)]
pub struct Evaluated {
  pub value: serde_json::Value,
  pub observed_env: BTreeMap<String, Option<String>>,
}

/// Evaluates `entry` and returns its default export as JSON.
///
/// Relative imports are followed recursively through the same pipeline. Non-relative
/// specifiers used for a runtime value are an error: there is no npm resolution here.
pub fn evaluate_config(entry: &Path, environment: &Environment) -> Result<Evaluated, EvalError> {
  let entry = canonical(entry)?;
  let code = read_and_strip(&entry)?;
  let observed = ObservedEnvironment::new(environment.clone());

  let slot: ErrorSlot = Rc::new(RefCell::new(None));
  let runtime = Runtime::new().map_err(|error| runtime_error(&error))?;
  runtime.set_loader(
    PathResolver { slot: Rc::clone(&slot) },
    StrippingLoader { slot: Rc::clone(&slot) },
  );

  // A config that imports itself in a cycle would otherwise recurse until the stack
  // runs out. QuickJS unwinds a memory limit as a normal exception instead.
  runtime.set_max_stack_size(STACK_LIMIT);

  let context = Context::full(&runtime).map_err(|error| runtime_error(&error))?;
  let key = entry.to_string_lossy().into_owned();

  context.with(|ctx| {
    install(&ctx, &observed).map_err(|error| runtime_error(&error))?;

    let value = match evaluate_entry(&ctx, &key, code, &entry) {
      Ok(value) => to_json(&value, &entry)?,
      // The slot holds the real error whenever a loader or resolver rejected the
      // module; the JavaScript exception is only the flattened copy of it.
      Err(caught) => return Err(slot.borrow_mut().take().unwrap_or(caught)),
    };

    if !value.is_object() {
      return Err(missing_default_export(&entry));
    }

    Ok(Evaluated { value, observed_env: observed.observations() })
  })
}

/// QuickJS unwinds this as a catchable exception, so a cyclic or runaway import graph
/// surfaces as an error rather than as a stack overflow that takes the process with it.
const STACK_LIMIT: usize = 1 << 20;

fn evaluate_entry<'js>(
  ctx: &Ctx<'js>,
  key: &str,
  code: String,
  entry: &Path,
) -> Result<Value<'js>, EvalError> {
  let declared =
    Module::declare(ctx.clone(), key, code).catch(ctx).map_err(|error| caught_error(&error))?;
  let (module, promise) = declared.eval().catch(ctx).map_err(|error| caught_error(&error))?;
  promise.finish::<()>().catch(ctx).map_err(|error| caught_error(&error))?;

  // A module that exports no `default` has nothing else this could be, so the generic
  // engine error is replaced by the one thing the user needs to be told.
  module.get::<_, Value<'js>>("default").map_err(|_| missing_default_export(entry))
}

/// The most common authoring mistake, and the one an engine-level error explains worst.
fn missing_default_export(entry: &Path) -> EvalError {
  EvalError::Shape {
    path: entry.to_owned(),
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

fn caught_error(error: &CaughtError<'_>) -> EvalError {
  // The engine reports positions in generated JavaScript. The user wrote TypeScript.
  EvalError::Runtime { message: crate::trace::remap(&error.to_string()) }
}

fn runtime_error(error: &rquickjs::Error) -> EvalError {
  EvalError::Runtime { message: error.to_string() }
}

/// Converts an evaluated JavaScript value into JSON.
///
/// This is the boundary where a config stops being a program and becomes data. Anything
/// with no JSON meaning — a function, a symbol — is rejected by name rather than dropped,
/// because silently losing a field is the worst way to tell a user their config is wrong.
fn to_json(value: &Value<'_>, path: &Path) -> Result<serde_json::Value, EvalError> {
  if value.is_undefined() || value.is_null() {
    return Ok(serde_json::Value::Null);
  }

  if let Some(boolean) = value.as_bool() {
    return Ok(serde_json::Value::Bool(boolean));
  }

  if let Some(number) = value.as_number() {
    return serde_json::Number::from_f64(number).map(serde_json::Value::Number).ok_or_else(|| {
      EvalError::Shape {
        path: path.to_owned(),
        message: format!("`{number}` has no JSON representation"),
      }
    });
  }

  if let Some(string) = value.as_string() {
    let string = string.to_string().map_err(|error| runtime_error(&error))?;
    return Ok(serde_json::Value::String(string));
  }

  if let Some(array) = value.as_array() {
    let mut items = Vec::with_capacity(array.len());
    for item in array.iter::<Value<'_>>() {
      items.push(to_json(&item.map_err(|error| runtime_error(&error))?, path)?);
    }
    return Ok(serde_json::Value::Array(items));
  }

  if value.is_function() {
    return Err(EvalError::Shape {
      path: path.to_owned(),
      message: "a config may not contain functions; it must be plain data".to_owned(),
    });
  }

  if let Some(object) = value.as_object() {
    let mut map = serde_json::Map::new();
    for entry in object.props::<String, Value<'_>>() {
      let (key, item) = entry.map_err(|error| runtime_error(&error))?;
      map.insert(key, to_json(&item, path)?);
    }
    return Ok(serde_json::Value::Object(map));
  }

  Err(EvalError::Shape {
    path: path.to_owned(),
    message: format!("value of type `{}` has no JSON representation", value.type_of().as_str()),
  })
}
