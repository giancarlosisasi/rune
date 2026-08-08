'use strict';

// Test 6a.9 and the order the wrapper looks in. Resolution takes the platform, the
// architecture, the environment and the resolver as arguments, so an arm64 machine can
// be asked about without being one.

const assert = require('node:assert/strict');
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
  assert.equal(resolution.failure.package, '@giancarlosio/rune-linux-x64');
  assert.deepEqual(resolution.failure.tried, ['@giancarlosio/rune-linux-x64/bin/rune']);
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
    '@giancarlosio/rune-darwin-arm64/bin/rune',
    '@giancarlosio/rune-darwin-x64/bin/rune',
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
