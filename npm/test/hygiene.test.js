'use strict';

// Tests R6.1, R6.2 and R6.4 — what a person reading the published artifact finds before
// they let their company install it.
//
// Neither of these costs anything at run time, and both stop adoption quietly: MIT
// requires the text to travel with every copy, and a lifecycle script running node on a
// file that is not in the tarball has to be proven harmless before anyone can move on.
//
// Asserted over the packed tarball as well as the assembled directory, because a check
// over the source tree passes while the assembly step drops the file.

const assert = require('node:assert/strict');
const { execFileSync } = require('node:child_process');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const test = require('node:test');

const platforms = require('../rune/lib/platforms');
const { LICENSE_FILE, assembleMeta, assemblePlatform, packRelease } = require('../scripts/dist');

const REPOSITORY_LICENSE = path.join(__dirname, '..', '..', LICENSE_FILE);

function workspace(t) {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'rune-hygiene-'));
  t.after(() => fs.rmSync(directory, { recursive: true, force: true }));
  return directory;
}

function assembleEverything(work) {
  const out = path.join(work, 'out');
  const assembled = platforms.PLATFORMS.map((entry) =>
    assemblePlatform(out, entry, process.execPath, '1.2.3'),
  );

  return [...assembled, assembleMeta(out)];
}

function manifestIn(directory) {
  return JSON.parse(fs.readFileSync(path.join(directory, 'package.json'), 'utf8'));
}

test('R6.1 — every assembled package carries the repository licence', (t) => {
  const expected = fs.readFileSync(REPOSITORY_LICENSE);

  for (const assembled of assembleEverything(workspace(t))) {
    const licence = path.join(assembled, LICENSE_FILE);

    assert.ok(fs.existsSync(licence), `${manifestIn(assembled).name} claims a licence it omits`);
    assert.deepEqual(fs.readFileSync(licence), expected, 'a second copy has drifted');
  }
});

test('R6.2 — no assembled manifest declares a script', (t) => {
  for (const assembled of assembleEverything(workspace(t))) {
    const manifest = manifestIn(assembled);

    assert.equal(
      manifest.scripts,
      undefined,
      `${manifest.name} publishes a script block that cannot run where it lands`,
    );
  }
});

test('the committed manifest keeps its scripts, so a local bump still works', () => {
  const committed = manifestIn(path.join(__dirname, '..', 'rune'));

  assert.ok(committed.scripts, 'stripping happens on the way out, not in the repository');
});

test('R6.4 — the licence and the empty script block survive into the tarball', (t) => {
  const work = workspace(t);
  const entry = platforms.entryFor(process.platform, process.arch);

  // node stands in for the native binary, as everywhere else in this suite.
  const release = packRelease({
    outDirectory: work,
    binaryFor: () => process.execPath,
    entries: [entry],
  });

  const expected = fs.readFileSync(REPOSITORY_LICENSE, 'utf8').replace(/\r\n/gu, '\n');

  for (const one of release.packed) {
    const unpacked = path.join(work, 'unpacked', path.basename(one.tarball, '.tgz'));
    fs.mkdirSync(unpacked, { recursive: true });

    // Copied in and extracted from there: a Windows path carries a drive letter, and tar
    // reads everything before a colon as the name of a remote host.
    fs.copyFileSync(path.join(release.directory, one.tarball), path.join(unpacked, one.tarball));
    execFileSync('tar', ['-xzf', one.tarball], { cwd: unpacked });

    const root = path.join(unpacked, 'package');
    const licence = fs.readFileSync(path.join(root, LICENSE_FILE), 'utf8').replace(/\r\n/gu, '\n');

    assert.equal(licence, expected, `${one.name} ships no licence a scanner can read`);
    assert.equal(manifestIn(root).scripts, undefined, `${one.name} ships a script block`);
  }
});
