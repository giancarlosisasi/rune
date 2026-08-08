'use strict';

// The README's platform matrix, rendered from the platform table.
//
// A README is written once and then rots. This part of it cannot: the same table that
// names the packages, generates their manifests and drives the build matrix writes the
// rows a user reads, and a test fails when the file stops matching.

const fs = require('node:fs');
const path = require('node:path');

const platforms = require('../rune/lib/platforms');

const README = path.join(__dirname, '..', '..', 'README.md');
const START = '<!-- platforms:start -->';
const END = '<!-- platforms:end -->';

const OPERATING_SYSTEM = { win32: 'Windows', darwin: 'macOS', linux: 'Linux' };

function row(entry) {
  // An arm64 package built from an x64 triple ships the x64 binary and leans on the
  // operating system's emulation. Today that is Windows on ARM.
  const note = entry.cpu === 'arm64' && entry.target.startsWith('x86_64')
    ? 'ships the x64 binary, run under emulation'
    : 'native';

  return `| ${OPERATING_SYSTEM[entry.os]} | ${entry.cpu} | \`${entry.package}\` | ${note} |`;
}

function table() {
  return [
    '| System | Architecture | Package | Binary |',
    '| --- | --- | --- | --- |',
    ...platforms.PLATFORMS.map(row),
  ].join('\n');
}

function render(readme) {
  const before = readme.indexOf(START);
  const after = readme.indexOf(END);

  if (before === -1 || after === -1) {
    throw new Error(`README.md has no ${START} … ${END} block`);
  }
  return `${readme.slice(0, before + START.length)}\n\n${table()}\n\n${readme.slice(after)}`;
}

if (require.main === module) {
  fs.writeFileSync(README, render(fs.readFileSync(README, 'utf8')));
  process.stdout.write('README.md: the platform matrix is up to date\n');
}

module.exports = { README, render, table };
