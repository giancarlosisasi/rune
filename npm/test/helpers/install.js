'use strict';

const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');

const platforms = require('../../rune/lib/platforms');

const META_SOURCE = path.join(__dirname, '..', '..', 'rune');
const STUBS = path.join(__dirname, '..', 'stubs');

function currentPlatform() {
  return platforms.entryFor(process.platform, process.arch);
}

// A throwaway node_modules holding the wrapper and whichever platform packages a test
// wants present.
//
// The stand-in for the native binary is node itself, under the name the wrapper looks
// for. That keeps one fixture working on every operating system — Windows will not run a
// shell script and POSIX will not run an `.exe` — and it costs the tests only this: the
// script the binary should run is the first argument the test passes.
function fakeInstall({ present = [currentPlatform()] } = {}) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'rune-install-'));
  const scope = path.join(root, 'node_modules', '@gio-labs');

  fs.mkdirSync(scope, { recursive: true });
  fs.cpSync(META_SOURCE, path.join(scope, 'rune'), { recursive: true });

  for (const entry of present) {
    const directory = path.join(scope, path.basename(entry.package));
    fs.mkdirSync(path.join(directory, platforms.BINARY_DIRECTORY), { recursive: true });
    fs.writeFileSync(
      path.join(directory, 'package.json'),
      `${JSON.stringify(platforms.manifest(entry, '0.0.0'), null, 2)}\n`,
    );
    placeNode(path.join(directory, platforms.BINARY_DIRECTORY, entry.binary));
  }

  return { root, shim: path.join(scope, 'rune', 'bin', 'rune') };
}

// A hard link where the filesystem allows one: node is large, and every test wants a copy.
function placeNode(target) {
  try {
    fs.linkSync(process.execPath, target);
  } catch {
    fs.copyFileSync(process.execPath, target);
  }
  fs.chmodSync(target, 0o755);
}

function stub(name) {
  return path.join(STUBS, name);
}

function remove(root) {
  fs.rmSync(root, { recursive: true, force: true });
}

module.exports = { currentPlatform, fakeInstall, remove, stub };
