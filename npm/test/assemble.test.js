'use strict';

// Test 6b.5 — the executable bit is put back on, and the packed set is written down.
//
// Asserted on the assembled directory rather than inside a workflow run: an upload and a
// download of a build artifact strip mode bits, so what arrives at the packaging step is
// always a plain file. Restoring the bit is the packaging step's job, and this is where
// it can be checked at all.

const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const test = require('node:test');

const platforms = require('../rune/lib/platforms');
const { PACKED, assemblePlatform, packRelease, version } = require('../scripts/dist');

const WINDOWS = process.platform === 'win32';

function workspace(t) {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'rune-assemble-'));
  t.after(() => fs.rmSync(directory, { recursive: true, force: true }));
  return directory;
}

// A build artifact as it comes back down: readable, writable, and not executable.
function downloadedArtifact(directory, name) {
  const file = path.join(directory, name);
  fs.writeFileSync(file, '#!/bin/sh\nexit 0\n');
  fs.chmodSync(file, 0o644);
  return file;
}

test('6b.5 — the assembled binary is executable even though the artifact was not', { skip: WINDOWS && 'Windows files carry no executable bit' }, (t) => {
  const work = workspace(t);
  const entry = platforms.entryFor('linux', 'x64');
  const artifact = downloadedArtifact(work, entry.binary);

  assert.equal(fs.statSync(artifact).mode & 0o111, 0, 'the fixture starts out executable');

  const assembled = assemblePlatform(path.join(work, 'out'), entry, artifact, '1.2.3');

  const binary = path.join(assembled, platforms.BINARY_DIRECTORY, entry.binary);
  assert.notEqual(fs.statSync(binary).mode & 0o111, 0, 'the binary cannot be run');
});

test('6b.5 — every platform package gets the bit, not only this machine\'s', { skip: WINDOWS && 'Windows files carry no executable bit' }, (t) => {
  const work = workspace(t);

  for (const entry of platforms.PLATFORMS) {
    const assembled = assemblePlatform(
      path.join(work, entry.cpu, entry.os),
      entry,
      downloadedArtifact(fs.mkdtempSync(path.join(work, 'artifact-')), entry.binary),
      '1.2.3',
    );

    const binary = path.join(assembled, platforms.BINARY_DIRECTORY, entry.binary);
    assert.notEqual(fs.statSync(binary).mode & 0o111, 0, `${entry.package} ships a file nobody can run`);
  }
});

test('what was packed is written down beside the tarballs', (t) => {
  const work = workspace(t);
  const entry = platforms.entryFor(process.platform, process.arch);

  // node stands in for the native binary, as everywhere else in this suite.
  const release = packRelease({
    outDirectory: work,
    binaryFor: () => process.execPath,
    entries: [entry],
  });

  const record = JSON.parse(fs.readFileSync(path.join(release.directory, PACKED), 'utf8'));

  assert.equal(record.version, version());
  assert.deepEqual(
    record.packed.map((one) => one.name),
    [entry.package, '@gio-labs/rune'],
    'the meta package is recorded last',
  );
  for (const one of record.packed) {
    assert.ok(
      fs.existsSync(path.join(release.directory, one.tarball)),
      `${one.name} names a tarball that is not there`,
    );
  }
});
