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

test('a bump writes the version file the binary embeds', () => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'rune-bump-'));
  const manifest = path.join(directory, 'package.json');
  const version = path.join(directory, 'version.txt');
  fs.writeFileSync(manifest, JSON.stringify({ ...meta, version: '3.1.4' }, null, 2));

  bump({ manifestPath: manifest, versionPath: version });

  assert.equal(fs.readFileSync(version, 'utf8'), '3.1.4\n');
  assert.equal(JSON.parse(fs.readFileSync(manifest, 'utf8')).version, '3.1.4');
  assert.equal(
    JSON.parse(fs.readFileSync(manifest, 'utf8')).optionalDependencies[
      '@giancarlosio/rune-darwin-arm64'
    ],
    '3.1.4',
  );

  fs.rmSync(directory, { recursive: true, force: true });
});
