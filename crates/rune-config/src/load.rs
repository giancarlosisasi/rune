//! The whole path from a working directory to a set of resolvable scripts.
//!
//! Discover, evaluate, validate — with the cache consulted in the middle. Callers get one
//! function so the order can never differ between `list`, `run` and `inspect`.
//!
//! Validation happens here rather than at run time. A cycle in a script nobody ran today
//! is still a broken config, and finding it when the config is loaded means the person
//! who wrote it is the one who reads the message.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::cache::{self, Cache};
use crate::compose::{self, ComposeError, Plan};
use crate::discover::{Discovered, NotFound, discover};
use crate::env::Environment;
use crate::envfile::EnvFileError;
use crate::eval::{EvalError, Evaluated, evaluate_config};
use crate::inherit::{self, InheritError, Layer, Resolved, Scope};
use crate::paths::relative_to;
use crate::schema::{Kind, SchemaError, parse};

#[derive(Debug, Error)]
pub enum LoadError {
  #[error(transparent)]
  NotFound(#[from] NotFound),

  #[error(transparent)]
  Eval(#[from] EvalError),

  #[error("{} is not a valid rune config\n\n{source}", .path_display)]
  Schema { path_display: String, source: SchemaError },

  #[error(
    "{config} redefines the root script `{script}` with `{discriminant}`\n\n\
     a package may extend a script the root defines, never replace it:\n\n  \
     {script}: {{ extends: \"{script}\", appendArgs: [\"--your-flag\"] }}\n\n\
     replacing it would leave this package behind the next time the root definition \
     changes,\nand nothing would say so. A genuinely different command deserves a name \
     of its own."
  )]
  Override { config: String, script: String, discriminant: String },

  #[error(transparent)]
  Compose(#[from] ComposeError),

  #[error(transparent)]
  EnvFile(#[from] EnvFileError),
}

/// Where a script's definition came from, seen from the directory rune was run in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provenance {
  /// The root config defines it and nothing here changes it.
  Root,
  /// This package extends the root's definition.
  Overridden,
  /// This package defines it and the root does not.
  Package,
}

/// The configs in play, parsed and validated.
#[derive(Debug, Clone)]
pub struct Loaded {
  pub discovered: Discovered,
  /// Nearest first: the package config when there is one, then the root config.
  pub layers: Vec<Layer>,
}

impl Loaded {
  /// What `name` resolves to, or `None` when no config in scope defines it.
  pub fn resolve(&self, name: &str, scope: Scope) -> Result<Option<Resolved<'_>>, InheritError> {
    inherit::resolve(&self.layers, name, scope)
  }

  /// Everything running `name` would run, in order, or `None` when nothing defines it.
  pub fn plan(&self, name: &str, scope: Scope) -> Result<Option<Plan>, ComposeError> {
    compose::plan(&self.layers, name, scope)
  }

  /// Every script name in scope, sorted and without duplicates.
  pub fn names(&self, scope: Scope) -> BTreeSet<&str> {
    inherit::names(&self.layers, scope).collect()
  }

  pub fn provenance(&self, name: &str) -> Provenance {
    let Some(package) = self.package_layer() else {
      return Provenance::Root;
    };

    if !package.config.scripts.contains_key(name) {
      return Provenance::Root;
    }

    if self.root_layer().config.scripts.contains_key(name) {
      Provenance::Overridden
    } else {
      Provenance::Package
    }
  }

  /// The config belonging to the package rune was run from, when it has one of its own.
  pub fn package_layer(&self) -> Option<&Layer> {
    (self.layers.len() > 1).then(|| &self.layers[0])
  }

  fn root_layer(&self) -> &Layer {
    self.layers.last().expect("discovery always yields a root config")
  }

  /// A package may narrow a shared script, never replace it.
  fn check_overrides(&self) -> Result<(), LoadError> {
    let Some(package) = self.package_layer() else {
      return Ok(());
    };

    for (name, script) in &package.config.scripts {
      let collides = self.root_layer().config.scripts.contains_key(name);
      if collides && !matches!(script.kind, Kind::Extends { .. }) {
        return Err(LoadError::Override {
          config: relative_to(&self.discovered.root, &package.source),
          script: name.clone(),
          discriminant: discriminant_of(&script.kind).to_owned(),
        });
      }
    }

    Ok(())
  }

  /// Every name plans, so a cycle, a missing target or a missing member is found now
  /// rather than by whoever happens to run that one script first.
  fn check_composition(&self) -> Result<(), LoadError> {
    Ok(compose::validate(&self.layers, Scope::Nearest)?)
  }
}

/// Evaluates every config that takes part and records them under one cache key.
fn evaluate_all(
  cache: &Cache,
  entries: &[PathBuf],
  environment: &Environment,
) -> Result<Vec<serde_json::Value>, EvalError> {
  let evaluated = entries
    .iter()
    .map(|entry| evaluate_config(entry, environment))
    .collect::<Result<Vec<Evaluated>, EvalError>>()?;

  cache::store(cache, entries, &evaluated);

  Ok(evaluated.into_iter().map(|one| one.value).collect())
}

fn discriminant_of(kind: &Kind) -> &'static str {
  match kind {
    Kind::Command(_) => "command",
    Kind::Extends { .. } => "extends",
    Kind::Serial { .. } => "serial",
  }
}

/// Finds the configs for `start`, evaluates them, and validates what they say together.
pub fn load(start: &Path, environment: &Environment) -> Result<Loaded, LoadError> {
  let discovered = discover(start)?;
  let entries = discovered.configs();
  let cache = Cache::for_root(&discovered.root);

  let values = match cache::lookup(&cache, &entries, environment) {
    Some(cached) => cached,
    None => evaluate_all(&cache, &entries, environment)?,
  };

  let layers = entries
    .iter()
    .zip(values)
    .map(|(source, value)| {
      let config = parse(&value).map_err(|error| LoadError::Schema {
        path_display: source.display().to_string(),
        source: error,
      })?;

      Ok(Layer::new(&discovered.root, source.clone(), config)?)
    })
    .collect::<Result<Vec<Layer>, LoadError>>()?;

  let loaded = Loaded { discovered, layers };
  loaded.check_overrides()?;
  loaded.check_composition()?;

  Ok(loaded)
}
