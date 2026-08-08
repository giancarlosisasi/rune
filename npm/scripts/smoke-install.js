'use strict';

// Test 6b.2 — the packed tarballs, installed the way a user installs them, on the
// operating system a user has.
//
// Every other test in this repository reaches rune directly. This one goes through the
// package manager twice: once to install, and once for every script it runs. It is the
// only check that can catch a broken `bin` entry, a missing file in `files`, a mode bit
// that never made it, or a wrapper that resolves to nothing on this platform.
//
// It runs against a real release build, so it lives here rather than in the suite that
// runs on every commit.

const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');

const platforms = require('../rune/lib/platforms');
const { PACKED, tarballSpec } = require('./dist');
const { runNpm } = require('./npm-cli');
const { META } = require('./release-plan');

const FIXTURE = path.join(__dirname, '..', 'test', 'fixtures', 'monorepo');

function output(result) {
  return String(result).trim();
}

function script(name, cwd) {
  return output(runNpm(['run', '--silent', name], { cwd }));
}

// The meta package plus this machine's platform package, and nothing else. The pins name
// versions no registry carries yet, so the optional dependencies are omitted and the one
// package that matters is installed from its own tarball — which is also what proves the
// wrapper finds a package it was not told about.
function install(project, tarballs, entry) {
  const { packed } = JSON.parse(fs.readFileSync(path.join(tarballs, PACKED), 'utf8'));
  const wanted = packed.filter((one) => one.name === entry.package || one.name === META);

  assert.equal(wanted.length, 2, `expected the meta package and ${entry.package}`);

  runNpm(
    [
      'install',
      '--no-audit',
      '--no-fund',
      '--omit=optional',
      ...wanted.map((one) => tarballSpec(tarballs, one.tarball)),
    ],
    { cwd: project },
  );
}

function smoke(tarballs) {
  const entry = platforms.entryFor(process.platform, process.arch);
  if (!entry) {
    throw new Error(`rune ships no package for ${process.platform} ${process.arch}`);
  }

  const { version } = JSON.parse(fs.readFileSync(path.join(tarballs, PACKED), 'utf8'));
  const project = fs.mkdtempSync(path.join(os.tmpdir(), 'rune-smoke-'));

  try {
    fs.cpSync(FIXTURE, project, { recursive: true });
    install(project, tarballs, entry);

    assert.equal(
      script('reported-version', project),
      `rune ${version}`,
      'the installed rune reports another version',
    );

    const listed = script('visible-scripts', project);
    for (const name of ['greet', 'check']) {
      assert.ok(listed.includes(name), `\`rune list\` does not mention ${name}:\n${listed}`);
    }

    // The same script, defined once at the root, run from the root and from a package
    // that defines nothing of its own. Both go through the package manager.
    for (const from of ['.', path.join('packages', 'api'), path.join('packages', 'web')]) {
      const ran = script('check', path.join(project, from));

      assert.match(ran, /v\d+\.\d+\.\d+/, `the script produced nothing from ${from}:\n${ran}`);
    }

    process.stdout.write(`rune ${version} installs and runs on ${process.platform} ${process.arch}\n`);
  } finally {
    fs.rmSync(project, { recursive: true, force: true });
  }
}

if (require.main === module) {
  const [tarballs = path.join(__dirname, '..', 'dist', 'tarballs')] = process.argv.slice(2);
  smoke(tarballs);
}

module.exports = { smoke };
