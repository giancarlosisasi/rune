'use strict';

// Test 6a.2 — pins are derived from the platform table, never maintained by hand.

const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const test = require('node:test');

const platforms = require('../rune/lib/platforms');
const { bump, withDerivedPins } = require('../scripts/bump-version');

const META = path.join(__dirname, '..', 'rune', 'package.json');
const meta = JSON.parse(fs.readFileSync(META, 'utf8'));

test('a bump pins every platform package at exactly the new version', () => {
  const bumped = withDerivedPins(meta, '9.9.9');

  assert.equal(bumped.version, '9.9.9');
  assert.deepEqual(
    Object.keys(bumped.optionalDependencies),
    platforms.PLATFORMS.map((entry) => entry.package),
  );
  for (const [name, pin] of Object.entries(bumped.optionalDependencies)) {
    assert.equal(pin, '9.9.9', `${name} is not pinned at the new version`);
  }
});

test('a hand-edited pin does not survive a bump', () => {
  const drifted = {
    ...meta,
    optionalDependencies: { '@giancarlosio/rune-linux-x64': '^0.0.1', 'left-pad': '1.0.0' },
  };

  assert.deepEqual(withDerivedPins(drifted, '2.0.0'), withDerivedPins(meta, '2.0.0'));
});

test('the committed manifest is what a bump would write', () => {
  assert.deepEqual(withDerivedPins(meta, meta.version), meta);
});

// A bump into a throwaway copy of the three files it writes.
function bumpInto(t, version) {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'rune-bump-'));
  t.after(() => fs.rmSync(directory, { recursive: true, force: true }));

  const paths = {
    manifestPath: path.join(directory, 'package.json'),
    versionPath: path.join(directory, 'version.txt'),
    cargoPath: path.join(directory, 'Cargo.toml'),
  };
  fs.writeFileSync(paths.manifestPath, JSON.stringify({ ...meta, version }, null, 2));
  fs.copyFileSync(path.join(__dirname, '..', '..', 'Cargo.toml'), paths.cargoPath);

  bump(paths);
  return paths;
}

test('a bump writes the version file the binary embeds', (t) => {
  const { manifestPath, versionPath } = bumpInto(t, '3.1.4');

  assert.equal(fs.readFileSync(versionPath, 'utf8'), '3.1.4\n');
  assert.equal(JSON.parse(fs.readFileSync(manifestPath, 'utf8')).version, '3.1.4');
  assert.equal(
    JSON.parse(fs.readFileSync(manifestPath, 'utf8')).optionalDependencies[
      '@giancarlosio/rune-darwin-arm64'
    ],
    '3.1.4',
  );
});

// The config cache keys on the version compiled into the binary. Left behind by a bump,
// that key stops moving, and an upgrade that changes how a config is read would serve the
// previous version's answer out of the cache.
test('a bump moves the version the crates are built with', (t) => {
  const { cargoPath } = bumpInto(t, '3.1.4');
  const cargo = fs.readFileSync(cargoPath, 'utf8');

  assert.match(cargo, /\[workspace\.package\][\s\S]*?\nversion = "3\.1\.4"/);
});

test('a bump leaves the dependency versions alone', (t) => {
  const { cargoPath } = bumpInto(t, '3.1.4');
  const before = fs.readFileSync(path.join(__dirname, '..', '..', 'Cargo.toml'), 'utf8');
  const after = fs.readFileSync(cargoPath, 'utf8');

  const dependencies = (text) => text.slice(text.indexOf('[workspace.dependencies]'));
  assert.equal(dependencies(after), dependencies(before));
});
