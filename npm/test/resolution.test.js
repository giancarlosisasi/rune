'use strict';

// Test 6a.9 and the order the wrapper looks in. Resolution takes the platform, the
// architecture, the environment and the resolver as arguments, so an arm64 machine can
// be asked about without being one.

const assert = require('node:assert/strict');
const path = require('node:path');
const test = require('node:test');

const platforms = require('../rune/lib/platforms');
const { resolveBinary } = require('../rune/lib/resolve');

// A resolver that knows about exactly the packages a case says are installed.
function installed(...specifiers) {
  const known = new Set(specifiers);
  return (specifier) => {
    if (!known.has(specifier)) {
      throw new Error(`Cannot find module '${specifier}'`);
    }
    return `/node_modules/${specifier}`;
  };
}

const nothingExists = () => false;
const everythingExists = () => true;

// A filesystem holding exactly the listed paths, spelled the way this machine spells them.
function tree(...paths) {
  const known = new Set(paths.map((one) => path.resolve(one)));
  return (candidate) => known.has(path.resolve(candidate));
}

// The entry point an install of the meta package presents, under `root`.
function entryPointIn(root) {
  return path.join(root, 'node_modules', '@gio-labs', 'rune', 'bin', 'rune');
}

const REPOSITORY = path.resolve('/work/repo');
const ABOVE = path.resolve('/work');
const asItself = (one) => one;

test('the platform package is used when it is there', () => {
  const entry = platforms.entryFor('linux', 'x64');

  const resolution = resolveBinary({
    platform: 'linux',
    arch: 'x64',
    env: {},
    resolve: installed(platforms.specifier(entry)),
    exists: nothingExists,
  });

  assert.equal(resolution.path, `/node_modules/${platforms.specifier(entry)}`);
  assert.equal(resolution.warning, undefined);
});

test('6a.9 — an arm64 host with only the x64 sibling runs it, and says so', () => {
  const sibling = platforms.entryFor('win32', 'x64');

  const resolution = resolveBinary({
    platform: 'win32',
    arch: 'arm64',
    env: {},
    resolve: installed(platforms.specifier(sibling)),
    exists: nothingExists,
  });

  assert.equal(resolution.path, `/node_modules/${platforms.specifier(sibling)}`);
  assert.equal(resolution.warning, 'no arm64 binary found, running the x64 build under emulation');
});

test('an x64 host has no sibling to fall back to', () => {
  const resolution = resolveBinary({
    platform: 'linux',
    arch: 'x64',
    env: {},
    resolve: installed(),
    exists: nothingExists,
  });

  assert.equal(resolution.failure.kind, 'missing');
  assert.equal(resolution.failure.package, '@gio-labs/rune-linux-x64');
  assert.deepEqual(resolution.failure.tried, ['@gio-labs/rune-linux-x64/bin/rune']);
});

test('an arm64 host that has neither package reports both attempts', () => {
  const resolution = resolveBinary({
    platform: 'darwin',
    arch: 'arm64',
    env: {},
    resolve: installed(),
    exists: nothingExists,
  });

  assert.deepEqual(resolution.failure.tried, [
    '@gio-labs/rune-darwin-arm64/bin/rune',
    '@gio-labs/rune-darwin-x64/bin/rune',
  ]);
});

test('a platform outside the table is not guessed at', () => {
  const resolution = resolveBinary({
    platform: 'freebsd',
    arch: 'x64',
    env: {},
    resolve: installed(),
    exists: nothingExists,
  });

  assert.deepEqual(resolution.failure, { kind: 'unsupported', platform: 'freebsd', arch: 'x64' });
});

test('the override is looked at before anything is resolved', () => {
  const resolution = resolveBinary({
    platform: 'linux',
    arch: 'x64',
    env: { RUNE_BINARY_PATH: '/home/dev/rune/target/debug/rune' },
    resolve: () => assert.fail('resolution must not be reached'),
    exists: everythingExists,
  });

  assert.equal(resolution.path, '/home/dev/rune/target/debug/rune');
});

test('an override naming nothing never falls through to the installed release', () => {
  const resolution = resolveBinary({
    platform: 'linux',
    arch: 'x64',
    env: { RUNE_BINARY_PATH: '/home/dev/rune/target/debug/rune' },
    resolve: () => assert.fail('resolution must not be reached'),
    exists: nothingExists,
  });

  assert.equal(resolution.failure.kind, 'override-missing');
  assert.equal(resolution.failure.path, '/home/dev/rune/target/debug/rune');
});

// R5.2, R5.3 and R5.4 — the repository-local step. It sits between the override and the
// platform package, so everything below it has to stay exactly as it was.

test('R5.2 — the install found is the copy already running, so there is nothing to hand over to', () => {
  const self = entryPointIn(REPOSITORY);
  const entry = platforms.entryFor('linux', 'x64');

  const resolution = resolveBinary({
    platform: 'linux',
    arch: 'x64',
    env: {},
    cwd: path.join(REPOSITORY, 'packages', 'ui'),
    self,
    resolve: installed(platforms.specifier(entry)),
    exists: tree(self),
    realpath: asItself,
  });

  assert.equal(resolution.handover, undefined, 'handing over to itself never terminates');
  assert.equal(resolution.path, `/node_modules/${platforms.specifier(entry)}`);
});

test('a copy reached from outside the repository hands over to the pinned one', () => {
  const local = entryPointIn(REPOSITORY);

  const resolution = resolveBinary({
    platform: 'linux',
    arch: 'x64',
    env: {},
    cwd: path.join(REPOSITORY, 'packages', 'ui'),
    self: path.resolve('/usr/lib/node_modules/@gio-labs/rune/bin/rune'),
    resolve: () => assert.fail('a handover resolves nothing itself'),
    exists: tree(local),
    realpath: asItself,
  });

  assert.equal(resolution.handover, local);
});

test('the nearest install wins, the way the config walk works', () => {
  const nearest = entryPointIn(path.join(REPOSITORY, 'packages', 'ui'));

  const resolution = resolveBinary({
    platform: 'linux',
    arch: 'x64',
    env: {},
    cwd: path.join(REPOSITORY, 'packages', 'ui', 'src'),
    self: path.resolve('/usr/lib/node_modules/@gio-labs/rune/bin/rune'),
    resolve: () => assert.fail('a handover resolves nothing itself'),
    exists: tree(nearest, entryPointIn(REPOSITORY)),
    realpath: asItself,
  });

  assert.equal(resolution.handover, nearest);
});

test('R5.4 — an install above the repository boundary is not reached into', () => {
  const entry = platforms.entryFor('linux', 'x64');

  const resolution = resolveBinary({
    platform: 'linux',
    arch: 'x64',
    env: {},
    cwd: path.join(REPOSITORY, 'packages', 'ui'),
    self: path.resolve('/usr/lib/node_modules/@gio-labs/rune/bin/rune'),
    resolve: installed(platforms.specifier(entry)),
    exists: tree(path.join(REPOSITORY, '.git'), entryPointIn(ABOVE)),
    realpath: asItself,
  });

  assert.equal(resolution.handover, undefined, 'the walk left the repository');
  assert.equal(resolution.path, `/node_modules/${platforms.specifier(entry)}`);
});

test('an install at the repository root is found, boundary or not', () => {
  const local = entryPointIn(REPOSITORY);

  const resolution = resolveBinary({
    platform: 'linux',
    arch: 'x64',
    env: {},
    cwd: REPOSITORY,
    self: path.resolve('/usr/lib/node_modules/@gio-labs/rune/bin/rune'),
    resolve: () => assert.fail('a handover resolves nothing itself'),
    exists: tree(local, path.join(REPOSITORY, '.git')),
    realpath: asItself,
  });

  assert.equal(resolution.handover, local, 'a repository root normally holds both');
});

test('R5.3 — the override outranks a repository-local install', () => {
  const resolution = resolveBinary({
    platform: 'linux',
    arch: 'x64',
    env: { RUNE_BINARY_PATH: '/home/dev/rune/target/debug/rune' },
    cwd: REPOSITORY,
    self: path.resolve('/usr/lib/node_modules/@gio-labs/rune/bin/rune'),
    resolve: () => assert.fail('resolution must not be reached'),
    exists: everythingExists,
    realpath: asItself,
  });

  assert.equal(resolution.path, '/home/dev/rune/target/debug/rune');
  assert.equal(resolution.handover, undefined);
});

test('a symlinked install and the running copy are one file, so nothing is handed over', () => {
  const linked = entryPointIn(REPOSITORY);
  const store = path.resolve('/store/@gio-labs+rune@0.1.2/node_modules/@gio-labs/rune/bin/rune');
  const entry = platforms.entryFor('linux', 'x64');

  const resolution = resolveBinary({
    platform: 'linux',
    arch: 'x64',
    env: {},
    cwd: REPOSITORY,
    self: store,
    resolve: installed(platforms.specifier(entry)),
    exists: tree(linked),
    // What a package manager that links from a content-addressed store hands back.
    realpath: (one) => (path.resolve(one) === linked ? store : one),
  });

  assert.equal(resolution.handover, undefined, 'the link and its target are the same install');
});
