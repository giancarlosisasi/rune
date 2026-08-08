'use strict';

// The version bump, and the only thing that writes a version anywhere.
//
// `npm version <x.y.z>` inside the meta package writes the new number into its manifest
// and then runs this as its `postversion` hook. From there everything else is derived:
// the six pins come from the platform table at exactly that version, `version.txt` — the
// file the Rust binary embeds at compile time — is rewritten to match, and the workspace
// crates are moved to the same number. A pin can therefore never drift, the binary can
// never report a version its package does not carry, and the config cache — which keys on
// the compiled-in crate version — cannot go on serving the previous version's answers.

const fs = require('node:fs');
const path = require('node:path');

const { optionalDependencies } = require('../rune/lib/platforms');

const WORKSPACE = path.join(__dirname, '..', '..');
const MANIFEST = path.join(__dirname, '..', 'rune', 'package.json');
const VERSION_FILE = path.join(WORKSPACE, 'version.txt');
const CARGO_FILE = path.join(WORKSPACE, 'Cargo.toml');

// The one version under `[workspace.package]`, and nothing that looks like it. Every
// crate inherits from there, and the dependency versions right below must not move.
const WORKSPACE_VERSION = /(\[workspace\.package\][^[]*?\nversion\s*=\s*")[^"]*(")/;

function withDerivedPins(manifest, version) {
  return { ...manifest, version, optionalDependencies: optionalDependencies(version) };
}

function withCrateVersion(cargo, version) {
  if (!WORKSPACE_VERSION.test(cargo)) {
    throw new Error('Cargo.toml has no version under [workspace.package]');
  }
  return cargo.replace(WORKSPACE_VERSION, `$1${version}$2`);
}

function bump({
  manifestPath = MANIFEST,
  versionPath = VERSION_FILE,
  cargoPath = CARGO_FILE,
} = {}) {
  const manifest = JSON.parse(fs.readFileSync(manifestPath, 'utf8'));
  const bumped = withDerivedPins(manifest, manifest.version);

  fs.writeFileSync(manifestPath, `${JSON.stringify(bumped, null, 2)}\n`);
  fs.writeFileSync(versionPath, `${bumped.version}\n`);
  fs.writeFileSync(
    cargoPath,
    withCrateVersion(fs.readFileSync(cargoPath, 'utf8'), bumped.version),
  );

  return bumped;
}

if (require.main === module) {
  const bumped = bump();
  process.stdout.write(`rune ${bumped.version}: pins, version.txt and Cargo.toml regenerated\n`);
}

module.exports = { bump, withCrateVersion, withDerivedPins };
