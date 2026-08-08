'use strict';

// Turning the build matrix's artifacts into the tarballs that get published.
//
// Every binary arrives in its own downloaded directory, named after the target triple it
// was built for. Which package each one fills is the platform table's answer, not this
// script's — two packages share the Windows binary, and that is the table's business.

const fs = require('node:fs');
const path = require('node:path');

const platforms = require('../rune/lib/platforms');
const { packRelease } = require('./dist');
const { META, validatePins } = require('./release-plan');

function binaryIn(artifacts, entry) {
  const file = path.join(artifacts, `binary-${entry.target}`, entry.binary);

  if (!fs.existsSync(file)) {
    throw new Error(`${entry.package} has no binary: nothing at ${file}`);
  }
  return file;
}

function main() {
  const [artifacts] = process.argv.slice(2);
  if (!artifacts) {
    throw new Error('usage: assemble-release.js <downloaded-artifacts-directory>');
  }

  const release = packRelease({ binaryFor: (entry) => binaryIn(artifacts, entry) });

  // Checked here, where the built set is a fact rather than a claim, so a release that
  // cannot describe itself stops before the smoke tests spend twenty minutes on it.
  const manifest = JSON.parse(
    fs.readFileSync(path.join(__dirname, '..', 'rune', 'package.json'), 'utf8'),
  );
  const problems = validatePins({
    manifest,
    built: release.packed.filter((one) => one.name !== META),
    version: release.version,
  });
  if (problems.length > 0) {
    throw new Error(`the release does not describe itself:\n  ${problems.join('\n  ')}`);
  }

  process.stdout.write(
    `${release.version}: packed ${release.packed.length} packages into ${release.directory}\n`,
  );
  for (const one of release.packed) {
    process.stdout.write(`  ${one.name} -> ${one.tarball}\n`);
  }
}

if (require.main === module) {
  main();
}

module.exports = { binaryIn };
