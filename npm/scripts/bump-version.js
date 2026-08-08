'use strict';

// The version bump, and the only thing that writes a version anywhere.
//
// `npm version <x.y.z>` inside the meta package writes the new number into its manifest
// and then runs this as its `postversion` hook. From there everything else is derived:
// the six pins come from the platform table at exactly that version, and `version.txt`
// — the file the Rust binary embeds at compile time — is rewritten to match. A pin can
// therefore never drift, and the binary can never report a version the package does not
// carry.

const fs = require('node:fs');
const path = require('node:path');

const { optionalDependencies } = require('../rune/lib/platforms');

const MANIFEST = path.join(__dirname, '..', 'rune', 'package.json');
const VERSION_FILE = path.join(__dirname, '..', '..', 'version.txt');

function withDerivedPins(manifest, version) {
  return { ...manifest, version, optionalDependencies: optionalDependencies(version) };
}

function bump({ manifestPath = MANIFEST, versionPath = VERSION_FILE } = {}) {
  const manifest = JSON.parse(fs.readFileSync(manifestPath, 'utf8'));
  const bumped = withDerivedPins(manifest, manifest.version);

  fs.writeFileSync(manifestPath, `${JSON.stringify(bumped, null, 2)}\n`);
  fs.writeFileSync(versionPath, `${bumped.version}\n`);

  return bumped;
}

if (require.main === module) {
  const bumped = bump();
  process.stdout.write(`rune ${bumped.version}: pins and version.txt regenerated\n`);
}

module.exports = { bump, withDerivedPins };
