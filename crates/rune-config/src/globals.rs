//! The one ambient object a config gets, and the refusal for everything else.
//!
//! A bare `ReferenceError: fs is not defined` is technically correct and useless. The
//! user's real question is "then what *can* I use?", so every refused name answers it.

use rquickjs::function::Func;
use rquickjs::{Ctx, Object};

use crate::env::{ObservedEnvironment, PLATFORM};

/// Installs `rune` and the refusals. Runs once per context, before any module loads.
pub fn install(ctx: &Ctx<'_>, env: &ObservedEnvironment) -> rquickjs::Result<()> {
  let globals = ctx.globals();
  let reader = env.clone();
  let ci_reader = env.clone();

  globals.set("__runeEnvRead", Func::from(move |name: String| reader.read(&name)))?;
  globals.set("__runeIsCI", Func::from(move || ci_reader.is_ci()))?;
  globals.set("__runePlatform", PLATFORM)?;

  ctx.eval::<(), _>(BOOTSTRAP)
}

/// Reads a value back out of an evaluated context, for tests and diagnostics.
pub fn rune_object<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<Object<'js>> {
  ctx.globals().get("rune")
}

/// `env` is a proxy rather than a plain object so that reads can be recorded: the cache
/// key covers the variables a config actually asked for, and nothing else.
///
/// Modules are strict-mode code, so the rejecting `set` trap and `Object.freeze` both
/// turn an assignment into a `TypeError` instead of a silent no-op.
const BOOTSTRAP: &str = r#"
(() => {
  const read = __runeEnvRead;

  const env = new Proxy(Object.create(null), {
    get(_target, name) {
      return typeof name === "string" ? read(name) : undefined;
    },
    has(_target, name) {
      return typeof name === "string" && read(name) !== undefined;
    },
    set() { return false; },
    deleteProperty() { return false; },
    ownKeys() { return []; },
  });

  // `isCI` is a getter, not a value: it is derived from an environment variable, and
  // reading it has to be recorded so the cache knows the config depended on it. Baking
  // the boolean in at setup time would make that read invisible and the cache stale.
  // `Object.freeze` leaves an accessor working while still rejecting assignment.
  const rune = {};
  Object.defineProperty(rune, "env", { value: env, enumerable: true });
  Object.defineProperty(rune, "platform", { value: __runePlatform, enumerable: true });
  Object.defineProperty(rune, "isCI", { get: __runeIsCI, enumerable: true });
  globalThis.rune = Object.freeze(rune);

  const AVAILABLE = [
    "  - `rune.env`, `rune.platform`, `rune.isCI`",
    "  - relative imports of other files in this repository, such as `./scripts/helpers.ts`",
    "  - `import { defineConfig } from \"@giancarlosio/rune\"` — the one package rune supplies itself",
    "  - `import type` from any npm package — type-only imports are erased before evaluation",
  ].join("\n");

  const UNAVAILABLE = [
    "require", "process", "module", "exports", "Buffer", "global",
    "__dirname", "__filename",
    "fs", "path", "os", "url", "util", "child_process", "crypto",
  ];

  for (const name of UNAVAILABLE) {
    try {
      Object.defineProperty(globalThis, name, {
        configurable: false,
        get() {
          throw new ReferenceError(
            "`" + name + "` is not available in a rune config.\n\n" +
            "rune evaluates this file with an embedded JavaScript engine, not Node.js, " +
            "so Node globals and modules do not exist here.\n\n" +
            "what does work:\n" + AVAILABLE
          );
        },
      });
    } catch (_) {
      // A name the engine already defines as non-configurable stays as it is; there is
      // nothing better to do about it and failing here would break every config.
    }
  }

  delete globalThis.__runeEnvRead;
  delete globalThis.__runePlatform;
  delete globalThis.__runeIsCI;
})();
"#;
