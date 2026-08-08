'use strict';

// Test 6a.1 — one table is the only place a platform is named.

const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');

const platforms = require('../rune/lib/platforms');
const { assertSnapshot } = require('./helpers/snapshot');

const meta = JSON.parse(
  fs.readFileSync(path.join(__dirname, '..', 'rune', 'package.json'), 'utf8'),
);

test('the six platform manifests render from the table', () => {
  const manifests = platforms.PLATFORMS.map((entry) => platforms.manifest(entry, '1.2.3'));

  assert.equal(manifests.length, 6);
  assertSnapshot('platform-manifests', manifests);
});

test('a platform package never says which C library the binary was linked against', () => {
  for (const entry of platforms.PLATFORMS) {
    const manifest = platforms.manifest(entry, '1.2.3');

    assert.equal(manifest.libc, undefined, `${manifest.name} declares a C library`);
    assert.ok(!manifest.name.includes('musl'), `${manifest.name} leaks the C library`);
  }
});

test('a platform package carries no bin entry of its own', () => {
  for (const entry of platforms.PLATFORMS) {
    assert.equal(platforms.manifest(entry, '1.2.3').bin, undefined);
  }
});

test('the wrapper resolves every supported platform to exactly one package', () => {
  const resolved = platforms.PLATFORMS.map((entry) => ({
    platform: `${entry.os}-${entry.cpu}`,
    package: platforms.entryFor(entry.os, entry.cpu).package,
    binary: platforms.entryFor(entry.os, entry.cpu).binary,
  }));

  assertSnapshot('resolution-switch', resolved);
});

test('an unsupported platform resolves to nothing rather than to a guess', () => {
  assert.equal(platforms.entryFor('freebsd', 'x64'), undefined);
  assert.equal(platforms.entryFor('linux', 'riscv64'), undefined);
});

test('the release matrix is one build per target triple', () => {
  const matrix = platforms.releaseMatrix();

  // Six packages, five builds: the Windows arm64 package ships the x64 binary.
  assert.equal(matrix.length, 5);
  assertSnapshot('release-matrix', matrix);
});

test('the meta manifest declares none of the four footguns', () => {
  assert.equal(meta.engines, undefined, 'engines would keep the diagnostics off old Node');
  assert.equal(meta.os, undefined, 'os would keep the package off the platforms that need it');
  assert.equal(meta.cpu, undefined, 'cpu would do the same');

  for (const hook of ['preinstall', 'install', 'postinstall', 'prepare']) {
    assert.equal(meta.scripts?.[hook], undefined, `${hook} makes installing do work`);
  }
});

test('the meta manifest ships the wrapper and the types, and nothing else', () => {
  assert.deepEqual(meta.bin, { rune: './bin/rune' });
  assert.deepEqual(meta.files.toSorted(), ['bin', 'index.d.ts', 'index.js', 'lib']);
});
