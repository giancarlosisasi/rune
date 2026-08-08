'use strict';

// Finding the binary, in a fixed order, with every failure carrying the reason it failed.
//
// Nothing here touches the process it is running in: the platform, the architecture, the
// environment and the resolver all arrive as arguments. That is what lets a test ask
// what an arm64 machine would do without being one.

const platforms = require('./platforms');

// Set and present wins over everything. Set and absent is a hard error rather than a
// fall-through: a developer pointing this at a stale build has to be told, not quietly
// handed the installed release and left wondering why their changes do nothing.
const OVERRIDE = 'RUNE_BINARY_PATH';

function resolveBinary({ platform, arch, env, resolve, exists }) {
  const override = env[OVERRIDE];
  if (override) {
    return exists(override)
      ? { path: override }
      : { failure: { kind: 'override-missing', variable: OVERRIDE, path: override } };
  }

  const entry = platforms.entryFor(platform, arch);
  if (!entry) {
    return { failure: { kind: 'unsupported', platform, arch } };
  }

  const tried = [];
  const direct = attempt(resolve, platforms.specifier(entry), tried);
  if (direct) {
    return { path: direct };
  }

  const sibling = platforms.emulatedFallbackFor(platform, arch);
  if (sibling) {
    const emulated = attempt(resolve, platforms.specifier(sibling), tried);
    if (emulated) {
      return {
        path: emulated,
        warning: `no ${arch} binary found, running the ${sibling.cpu} build under emulation`,
      };
    }
  }

  return { failure: { kind: 'missing', platform, arch, package: entry.package, tried } };
}

function attempt(resolve, specifier, tried) {
  tried.push(specifier);
  try {
    return resolve(specifier);
  } catch {
    return undefined;
  }
}

module.exports = { OVERRIDE, resolveBinary };
