//! The whole path from a working directory to a typed config.
//!
//! Discover, evaluate, validate — and, once it exists, consult the cache in the middle.
//! Callers get one function so the order can never differ between `list`, `run` and
//! `inspect`.

use std::path::Path;

use thiserror::Error;

use crate::cache::{self, Cache};
use crate::discover::{Discovered, NotFound, discover};
use crate::env::Environment;
use crate::eval::{EvalError, evaluate_config};
use crate::schema::{Config, SchemaError, parse};

#[derive(Debug, Error)]
pub enum LoadError {
  #[error(transparent)]
  NotFound(#[from] NotFound),

  #[error(transparent)]
  Eval(#[from] EvalError),

  #[error("{} is not a valid rune config\n\n{source}", .path_display)]
  Schema { path_display: String, source: SchemaError },
}

#[derive(Debug, Clone)]
pub struct Loaded {
  pub config: Config,
  pub discovered: Discovered,
}

/// Finds the config for `start`, evaluates it, and validates its shape.
pub fn load(start: &Path, environment: &Environment) -> Result<Loaded, LoadError> {
  let discovered = discover(start)?;
  let cache = Cache::for_root(&discovered.root);

  let value = if let Some(cached) = cache::lookup(&cache, &discovered.config, environment) {
    cached
  } else {
    let evaluated = evaluate_config(&discovered.config, environment)?;
    cache::store(&cache, &discovered.config, environment, &evaluated);
    evaluated.value
  };

  let config = parse(&value).map_err(|source| LoadError::Schema {
    path_display: discovered.config.display().to_string(),
    source,
  })?;

  Ok(Loaded { config, discovered })
}
