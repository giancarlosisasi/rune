//! The environment object a config imports, and the refusal for everything else.
//!
//! A bare `ReferenceError: fs is not defined` is technically correct and useless. The
//! user's real question is "then what *can* I use?", so every refused name answers it.

use std::collections::BTreeMap;

use rquickjs::Ctx;
use rquickjs::function::Func;

use crate::env::{ObservedEnvironment, PLATFORM};

/// Builds the environment object and installs the refusals. Runs once per context, before
/// any module loads, so that `__rune` is waiting when the supplied module evaluates.
pub fn install(ctx: &Ctx<'_>, env: &ObservedEnvironment) -> rquickjs::Result<()> {
  let globals = ctx.globals();
  let reader = env.clone();
  let ci_reader = env.clone();

  let lister = env.clone();

  globals.set("__runeEnvRead", Func::from(move |name: String| reader.read(&name)))?;
  globals.set("__runeEnvNames", Func::from(move || lister.names()))?;
  globals.set("__runeIsCI", Func::from(move || ci_reader.is_ci()))?;
  globals.set("__runePlatform", PLATFORM)?;
  globals.set("__runeUnavailable", refusals())?;

  ctx.eval::<(), _>(BOOTSTRAP)
}

/// Every name a config can reach for and will not find.
///
/// Published so that the rule — no name here reaches a user as the engine's own `is not
/// defined` — can be asserted over the set the product installs rather than over a copy
/// of it kept in a test.
pub fn unavailable_names() -> impl Iterator<Item = &'static str> {
  UNAVAILABLE.iter().map(|entry| entry.name)
}

/// A name the config environment does not offer, and why.
struct Unavailable {
  name: &'static str,
  answer: Answer,
}

/// What a refusal says after it has named the name.
///
/// Most names need only the one explanation. The two a user types on purpose need the
/// answer to the question that made them type it, which "this is not Node" is not.
#[derive(Clone, Copy)]
enum Answer {
  NotNode,
  Printing,
  Deferring,
}

/// One table: adding a name and deciding whether it deserves words of its own are the
/// same edit.
///
/// The first group is Node's; the second is what a user reaches for while debugging a
/// config that produced the wrong string.
const UNAVAILABLE: &[Unavailable] = &[
  Unavailable { name: "require", answer: Answer::NotNode },
  Unavailable { name: "process", answer: Answer::NotNode },
  Unavailable { name: "module", answer: Answer::NotNode },
  Unavailable { name: "exports", answer: Answer::NotNode },
  Unavailable { name: "Buffer", answer: Answer::NotNode },
  Unavailable { name: "global", answer: Answer::NotNode },
  Unavailable { name: "__dirname", answer: Answer::NotNode },
  Unavailable { name: "__filename", answer: Answer::NotNode },
  Unavailable { name: "fs", answer: Answer::NotNode },
  Unavailable { name: "path", answer: Answer::NotNode },
  Unavailable { name: "os", answer: Answer::NotNode },
  Unavailable { name: "url", answer: Answer::NotNode },
  Unavailable { name: "util", answer: Answer::NotNode },
  Unavailable { name: "child_process", answer: Answer::NotNode },
  Unavailable { name: "crypto", answer: Answer::NotNode },
  Unavailable { name: "console", answer: Answer::Printing },
  Unavailable { name: "alert", answer: Answer::NotNode },
  Unavailable { name: "fetch", answer: Answer::NotNode },
  Unavailable { name: "setTimeout", answer: Answer::Deferring },
  Unavailable { name: "setInterval", answer: Answer::Deferring },
  Unavailable { name: "queueMicrotask", answer: Answer::Deferring },
];

/// What a config may use, which is what somebody who just found a name missing wants next.
const AVAILABLE: &str = "\
  \n  - `import { rune } from \"@gio-labs/rune\"` — then `rune.env`, `rune.platform`, `rune.isCI`\
  \n  - relative imports of other files in this repository, such as `./scripts/helpers.ts`\
  \n  - `import { defineConfig } from \"@gio-labs/rune\"` — the one package rune supplies itself\
  \n  - `import type` from any npm package — type-only imports are erased before evaluation";

/// The command that answers what a print or a log was reaching for.
const INSPECT: &str = "to see what a script resolved to, run:\n  rune inspect <name>";

/// Each refused name against the sentence it throws, for the bootstrap to install.
fn refusals() -> BTreeMap<&'static str, String> {
  UNAVAILABLE.iter().map(|entry| (entry.name, refusal(entry.name, entry.answer))).collect()
}

fn refusal(name: &str, answer: Answer) -> String {
  let body = match answer {
    Answer::NotNode => format!(
      "rune evaluates this file with an embedded JavaScript engine, not Node.js, so Node \
       globals and modules do not exist here.\n\nwhat does work:{AVAILABLE}"
    ),
    Answer::Printing => format!(
      "rune evaluates this file to produce a result, not to run it, so there is nothing to \
       print to.\n\n{INSPECT}"
    ),
    // Not "this is not Node": the engine defines `queueMicrotask`, and rune is the one
    // taking it away. A message blaming the engine for that would state something rune
    // has not established.
    Answer::Deferring => format!(
      "rune evaluates this file to produce a result, not to run it. Work scheduled for later \
       would happen while a result is being computed and not when the same result comes from \
       the cache, so nothing here defers.\n\n{INSPECT}"
    ),
  };

  format!("`{name}` is not available in a rune config.\n\n{body}")
}

/// `env` is a proxy rather than a plain object so that reads can be recorded: the cache
/// key covers the variables a config actually asked for, and nothing else. `rune` is a
/// proxy for a different reason — a frozen object cannot be given words, and every
/// refusal here has to be a sentence naming the property and the rule.
///
/// Modules are strict-mode code, so a throwing trap and a silent `false` both end an
/// assignment; only the throwing one gets to say why.
const BOOTSTRAP: &str = r#"
(() => {
  const read = __runeEnvRead;
  const names = __runeEnvNames;

  const readOnly = (property) => new TypeError(
    "`" + property + "` is read-only.\n\n" +
    "`rune` describes the machine this config is being evaluated on. Changing it would " +
    "change what the config resolves with and not what is true."
  );

  const named = (owner, property) =>
    typeof property === "string" ? owner + "." + property : owner;

  const env = new Proxy(Object.create(null), {
    get(_target, name) {
      return typeof name === "string" ? read(name) : undefined;
    },
    has(_target, name) {
      return typeof name === "string" && read(name) !== undefined;
    },
    ownKeys() { return names(); },
    // The target holds none of these keys, so both flags have to say so: an engine
    // checks that a proxy is not claiming a key is fixed when nothing holds it, and
    // throws where the answer would otherwise merely be wrong.
    getOwnPropertyDescriptor(_target, name) {
      if (typeof name !== "string") { return undefined; }
      const value = read(name);
      return value === undefined
        ? undefined
        : { value, writable: false, enumerable: true, configurable: true };
    },
    set(_target, name) { throw readOnly(named("rune.env", name)); },
    deleteProperty(_target, name) { throw readOnly(named("rune.env", name)); },
    defineProperty(_target, name) { throw readOnly(named("rune.env", name)); },
  });

  // `isCI` is a getter, not a value: it is derived from an environment variable, and
  // reading it has to be recorded so the cache knows the config depended on it. Baking
  // the boolean in at setup time would make that read invisible and the cache stale.
  // `Object.freeze` leaves an accessor working while still rejecting assignment.
  const members = {};
  Object.defineProperty(members, "env", { value: env, enumerable: true });
  Object.defineProperty(members, "platform", { value: __runePlatform, enumerable: true });
  Object.defineProperty(members, "isCI", { get: __runeIsCI, enumerable: true });
  Object.freeze(members);

  // Reads pass straight through, so `Object.keys(rune)` is still the three members and
  // `isCI` is still the accessor. Only the three ways of changing it are intercepted.
  const rune = new Proxy(members, {
    set(_target, name) { throw readOnly(named("rune", name)); },
    deleteProperty(_target, name) { throw readOnly(named("rune", name)); },
    defineProperty(_target, name) { throw readOnly(named("rune", name)); },
  });

  // The supplied module exports this. It is the seam between the bootstrap, which can
  // still reach the native handles, and the module, which evaluates after they are gone.
  Object.defineProperty(globalThis, "__rune", { value: rune });

  // Reading `rune` without importing it used to work. Answering with the import is what
  // turns an upgrade into a one-line fix instead of a puzzle.
  Object.defineProperty(globalThis, "rune", {
    configurable: false,
    get() {
      throw new ReferenceError(
        "`rune` is not a global in a rune config.\n\n" +
        "it is an export of the module rune supplies:\n\n" +
        "  import { rune } from \"@gio-labs/rune\";\n\n" +
        "the object itself is unchanged — `rune.env`, `rune.platform` and `rune.isCI` " +
        "all work once it is imported."
      );
    },
    set() { throw readOnly("rune"); },
  });

  // The names and their sentences come from rune, so that adding a name and deciding
  // what it says are one edit in one language. This loop is only the mechanism.
  for (const name of Object.keys(__runeUnavailable)) {
    const message = __runeUnavailable[name];
    try {
      Object.defineProperty(globalThis, name, {
        configurable: false,
        get() { throw new ReferenceError(message); },
      });
    } catch (_) {
      // A name the engine already defines as non-configurable stays as it is; there is
      // nothing better to do about it and failing here would break every config.
    }
  }

  delete globalThis.__runeEnvRead;
  delete globalThis.__runeEnvNames;
  delete globalThis.__runePlatform;
  delete globalThis.__runeIsCI;
  delete globalThis.__runeUnavailable;
})();
"#;
