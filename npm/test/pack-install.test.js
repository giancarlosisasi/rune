'use strict';

// Test 6a.3 — the happy path, through a real package manager.
//
// Everything else in this suite reaches the wrapper directly. This one packs tarballs,
// installs them into a throwaway project the way a user would, and then asks the
// installed `rune` to run something. It is the only test that can catch a mistake in the
// manifest, in `files`, or in the mode bits.

const assert = require('node:assert/strict');
const { spawnSync } = require('node:child_process');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const test = require('node:test');

const platforms = require('../rune/lib/platforms');
const { assembleMeta, assemblePlatform, pack, version } = require('../scripts/dist');
const { runNpm } = require('../scripts/npm-cli');
const { stub } = require('./helpers/install');

test('6a.3 — packed tarballs install and the installed rune runs', (t) => {
  const work = fs.mkdtempSync(path.join(os.tmpdir(), 'rune-pack-'));
  t.after(() => fs.rmSync(work, { recursive: true, force: true }));

  const entry = platforms.entryFor(process.platform, process.arch);
  const staging = path.join(work, 'staging');
  const tarballs = path.join(work, 'tarballs');

  // Node stands in for the native binary, exactly as in the other wrapper tests.
  pack(assemblePlatform(staging, entry, process.execPath, version()), tarballs);
  pack(assembleMeta(staging), tarballs);

  const project = path.join(work, 'project');
  fs.mkdirSync(project);
  fs.writeFileSync(
    path.join(project, 'package.json'),
    `${JSON.stringify({ name: 'throwaway', version: '0.0.0', private: true }, null, 2)}\n`,
  );

  // The pins name versions no registry has, so the platform package is installed from its
  // own tarball instead of being fetched.
  runNpm(
    [
      'install',
      '--no-audit',
      '--no-fund',
      '--omit=optional',
      ...fs.readdirSync(tarballs).map((file) => path.join(tarballs, file)),
    ],
    { cwd: project },
  );

  const payload = path.join(
    project,
    'node_modules',
    entry.package,
    platforms.BINARY_DIRECTORY,
    entry.binary,
  );

  assert.ok(fs.statSync(payload).isFile(), 'the binary is on disk');
  fs.accessSync(payload, fs.constants.X_OK);

  const linked = path.join(project, 'node_modules', '.bin', 'rune');
  const ran = spawnSync(process.platform === 'win32' ? `${linked}.cmd` : linked, [stub('echo-args.js'), 'installed'], {
    cwd: project,
    encoding: 'utf8',
    shell: process.platform === 'win32',
  });

  assert.equal(ran.status, 0, ran.stderr);
  assert.deepEqual(JSON.parse(ran.stdout), ['installed']);
});
