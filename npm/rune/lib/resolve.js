'use strict';

// Finding the binary, in a fixed order, with every failure carrying the reason it failed.
//
// Nothing here touches the process it is running in: the platform, the architecture, the
// environment and the resolver all arrive as arguments. That is what lets a test ask
// what an arm64 machine would do without being one.

const path = require('node:path');

const platforms = require('./platforms');

// Set and present wins over everything. Set and absent is a hard error rather than a
// fall-through: a developer pointing this at a stale build has to be told, not quietly
// handed the installed release and left wondering why their changes do nothing.
const OVERRIDE = 'RUNE_BINARY_PATH';

// What a repository-local install presents, relative to the directory it was installed in.
const LOCAL_ENTRY = ['node_modules', '@gio-labs', 'rune', 'bin', 'rune'];

// The entry that ends the upward walk, so a run in a scratch directory cannot reach into
// an unrelated repository elsewhere on the machine.
const BOUNDARY = '.git';

function resolveBinary({ platform, arch, env, cwd, self, resolve, exists, realpath }) {
  const override = env[OVERRIDE];
  if (override) {
    return exists(override)
      ? { path: override }
      : { failure: { kind: 'override-missing', variable: OVERRIDE, path: override } };
  }

  // A version pinned by a repository is a statement about how that repository is built,
  // and typing a command inside it does not revoke that. Whichever copy was reached, the
  // pinned one runs.
  const local = localInstall({ platform, cwd, self, exists, realpath });
  if (local) {
    return { handover: local };
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

// The entry point of the nearest repository-local install, when there is one and it is not
// the copy already running.
//
// The same shape of walk the binary makes for a config: upward from the working directory,
// the first hit wins, and a repository boundary ends it. The install is looked for before
// the boundary is tested, because a repository root normally holds both.
function localInstall({ platform, cwd, self, exists, realpath }) {
  if (!cwd) {
    return undefined;
  }

  for (const directory of ancestors(cwd)) {
    const entry = path.join(directory, ...LOCAL_ENTRY);
    if (exists(entry)) {
      // Identity, not a marker in the environment: an environment flag leaks into every
      // child and survives into processes that have nothing to do with this. Two names
      // for one file mean there is nothing to hand over to, and handing over to itself
      // would never terminate.
      return sameFile(platform, entry, self, realpath) ? undefined : entry;
    }

    if (exists(path.join(directory, BOUNDARY))) {
      return undefined;
    }
  }

  return undefined;
}

function* ancestors(from) {
  let directory = path.resolve(from);

  for (;;) {
    yield directory;

    const parent = path.dirname(directory);
    if (parent === directory) {
      return;
    }
    directory = parent;
  }
}

// A package manager that links from a content-addressed store gives one file two names, so
// the links are followed before the comparison. Windows paths name the same file whatever
// their case.
function sameFile(platform, left, right, realpath) {
  if (!right) {
    return false;
  }

  const spelling = (one) => {
    let resolved = one;
    try {
      resolved = realpath(one);
    } catch {
      // Nothing to follow. The path as written is the best answer available.
    }

    const absolute = path.resolve(resolved);
    return platform === 'win32' ? absolute.toLowerCase() : absolute;
  };

  return spelling(left) === spelling(right);
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
