'use strict';

// What the machine can be asked about a failed install, so the message can name a cause
// instead of a symptom.

const fs = require('node:fs');
const path = require('node:path');

const platforms = require('./platforms');

// Lockfiles, and the manager each one belongs to. Order matters only in the rare repo
// that carries two.
const LOCKFILES = [
  { file: 'pnpm-lock.yaml', manager: 'pnpm' },
  { file: 'package-lock.json', manager: 'npm' },
  { file: 'yarn.lock', manager: 'yarn' },
  { file: 'bun.lock', manager: 'bun' },
  { file: 'bun.lockb', manager: 'bun' },
];

// Far enough to leave a hoisted node_modules and reach a workspace root, and short
// enough to stop at a home directory rather than walk to the filesystem root.
const WALK_LIMIT = 10;

function findLockfile(startDirectory) {
  let directory = startDirectory;

  for (let step = 0; step < WALK_LIMIT; step += 1) {
    for (const { file, manager } of LOCKFILES) {
      const candidate = path.join(directory, file);
      if (fs.existsSync(candidate)) {
        return { path: candidate, manager };
      }
    }

    const parent = path.dirname(directory);
    if (parent === directory) {
      return undefined;
    }
    directory = parent;
  }

  return undefined;
}

// Whether a lockfile knows about a package at all. Every lockfile format writes package
// names in plain text, including the binary one, so reading bytes answers all of them.
function lockfileMentions(lockfilePath, packageName) {
  try {
    return fs.readFileSync(lockfilePath, 'latin1').includes(packageName);
  } catch {
    return false;
  }
}

// Platform packages installed for some other platform. Their presence is the signature
// of one node_modules shared between two systems.
function foreignPackages(startDirectory) {
  const installed = new Set();

  let directory = startDirectory;
  for (let step = 0; step < WALK_LIMIT; step += 1) {
    const scope = path.join(directory, 'node_modules', '@giancarlosio');
    for (const entry of readDirectory(scope)) {
      installed.add(`@giancarlosio/${entry}`);
    }

    const parent = path.dirname(directory);
    if (parent === directory) {
      break;
    }
    directory = parent;
  }

  return platforms.PLATFORMS.map((entry) => entry.package).filter((name) => installed.has(name));
}

function readDirectory(directory) {
  try {
    return fs.readdirSync(directory);
  } catch {
    return [];
  }
}

// Which package manager is running, from the environment it sets for its own scripts.
// A repair command for the wrong manager is worse than no repair command.
function detectPackageManager(env) {
  const agent = env.npm_config_user_agent;
  if (!agent) {
    return undefined;
  }

  const [name, version = ''] = agent.split(' ')[0].split('/');
  return { name, major: Number.parseInt(version, 10) || undefined };
}

module.exports = { detectPackageManager, findLockfile, foreignPackages, lockfileMentions };
