//! `rune run <name>`.

use std::path::Path;

use rune_config::env::PLATFORM;
use rune_config::inherit::Scope;
use rune_exec::{Completion, ExecRequest};

use crate::script::{env_files, load_here, unknown};

/// Resolves `name` and runs it with the child owning the terminal.
///
/// `arguments` are what the user typed after `--`. They go last, after everything the
/// configuration contributed along the extension chain, because the config is the default
/// and the command line is the override.
pub fn run(name: &str, arguments: &[String], scope: Scope) -> Result<Completion, String> {
  let loaded = load_here()?;
  let resolved =
    loaded.resolve(name, scope).map_err(stringify)?.ok_or_else(|| unknown(name, &loaded, scope))?;

  let mut all_arguments = resolved.append_args.clone();
  all_arguments.extend_from_slice(arguments);
  let files = env_files(&resolved);

  let request = ExecRequest {
    script_name: name,
    // `PLATFORM` is the same constant a config reads as `rune.platform`, so a config
    // branching by hand and a per-OS object cannot disagree about which system this is.
    command: resolved.command.select(PLATFORM),
    arguments: &all_arguments,
    root: &loaded.discovered.root,
    package_dir: &loaded.discovered.package_dir,
    cwd: resolved.cwd.map(Path::new),
    env: &resolved.env,
    env_files: &files,
  };

  rune_exec::run(&request).map_err(stringify)
}

fn stringify(error: impl std::fmt::Display) -> String {
  error.to_string()
}
