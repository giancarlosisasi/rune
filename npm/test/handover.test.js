'use strict';

// Tests R5.1, R5.2 and R5.5 — a repository that pins rune runs the copy it pinned,
// whichever copy was reached.
//
// The oracle is always which binary ran, never what resolution answered: an assertion on
// the resolver's return value passes while the wrong process runs, and running the wrong
// process without a word is the whole defect. Two installs are built, one outside a
// repository boundary and one inside it, and the binary each presents is node itself — so
// the path it reports of itself names the install that was chosen.

const assert = require('node:assert/strict');
const { spawn } = require('node:child_process');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const test = require('node:test');

const platforms = require('../rune/lib/platforms');
const { currentPlatform, fakeInstall, remove, stub } = require('./helpers/install');

// A repository with its own install, and a second install that is nowhere above it — the
// shape of a developer who installed rune globally and then walked into their team's
// repository.
function twoInstalls({ repositoryHas = [currentPlatform()] } = {}) {
  const fixture = fs.mkdtempSync(path.join(os.tmpdir(), 'rune-handover-'));
  const repository = path.join(fixture, 'repo');

  const outside = fakeInstall({ at: path.join(fixture, 'elsewhere') });
  const inside = fakeInstall({ at: repository, present: repositoryHas });
  fs.mkdirSync(path.join(repository, '.git'), { recursive: true });

  return { fixture, repository, outside, inside };
}

function binaryIn(root) {
  const entry = platforms.entryFor(process.platform, process.arch);
  return path.join(root, 'node_modules', entry.package, platforms.BINARY_DIRECTORY, entry.binary);
}

function runShim(shim, args, { cwd, env } = {}) {
  return new Promise((resolve) => {
    const child = spawn(process.execPath, [shim, ...args], {
      cwd,
      env: { ...process.env, ...env },
    });

    let stdout = '';
    let stderr = '';
    child.stdout.setEncoding('utf8');
    child.stderr.setEncoding('utf8');
    child.stdout.on('data', (chunk) => {
      stdout += chunk;
    });
    child.stderr.on('data', (chunk) => {
      stderr += chunk;
    });
    child.stdin.end();

    child.on('close', (code) => resolve({ code, stdout, stderr, shimPid: child.pid }));
  });
}

// A temporary directory has more than one spelling, and Windows names one file whichever
// case it is asked for.
function samePath(left, right) {
  return path.resolve(fs.realpathSync(left)) === path.resolve(fs.realpathSync(right));
}

test('R5.1 — a copy reached from outside the repository runs the copy the repository pinned', async () => {
  const { fixture, repository, outside, inside } = twoInstalls();
  test.after(() => remove(fixture));

  const { code, stdout } = await runShim(outside.shim, [stub('identify.js')], { cwd: repository });
  const ran = JSON.parse(stdout);

  assert.equal(code, 0);
  assert.ok(
    samePath(ran.binary, binaryIn(inside.root)),
    `the copy outside the repository ran:\n  ${ran.binary}`,
  );
  assert.ok(!samePath(ran.binary, binaryIn(outside.root)), 'the outer install must not answer');
});

test('R5.2 — reaching the repository own install hands over to nothing', async () => {
  const { fixture, repository, inside } = twoInstalls();
  test.after(() => remove(fixture));

  const { code, stdout, shimPid } = await runShim(inside.shim, [stub('identify.js')], {
    cwd: repository,
  });
  const ran = JSON.parse(stdout);

  assert.equal(code, 0);
  assert.ok(samePath(ran.binary, binaryIn(inside.root)), 'the repository own binary must answer');
  assert.equal(ran.parent, shimPid, 'a wrapper stood between the two, so it handed over to itself');
});

test('R5.1 — the handover is what puts a second wrapper in the chain', async () => {
  const { fixture, repository, outside } = twoInstalls();
  test.after(() => remove(fixture));

  const { stdout, shimPid } = await runShim(outside.shim, [stub('identify.js')], {
    cwd: repository,
  });

  assert.notEqual(
    JSON.parse(stdout).parent,
    shimPid,
    'nothing was handed over, so the outer copy ran',
  );
});

test('R5.3 — the override outranks the repository own install', async () => {
  const { fixture, repository, outside } = twoInstalls();
  test.after(() => remove(fixture));

  const { code, stdout } = await runShim(outside.shim, [stub('identify.js')], {
    cwd: repository,
    env: { RUNE_BINARY_PATH: process.execPath },
  });

  assert.equal(code, 0);
  assert.ok(
    samePath(JSON.parse(stdout).binary, process.execPath),
    'an explicit instruction from a developer is never quietly overruled',
  );
});

test('R5.5 — a repository install that can produce no binary reports itself and nothing falls back', async () => {
  const { fixture, repository, outside } = twoInstalls({ repositoryHas: [] });
  test.after(() => remove(fixture));

  const { code, stdout, stderr } = await runShim(outside.shim, [stub('identify.js')], {
    cwd: repository,
  });

  assert.equal(code, 1);
  assert.equal(stdout, '', 'the copy outside the repository ran instead of reporting the broken one');
  assert.match(stderr, new RegExp(platforms.entryFor(process.platform, process.arch).package));
});

test('with no install in the repository, the copy that was reached answers as it always did', async () => {
  const fixture = fs.mkdtempSync(path.join(os.tmpdir(), 'rune-handover-'));
  test.after(() => remove(fixture));

  const repository = path.join(fixture, 'repo');
  fs.mkdirSync(path.join(repository, '.git'), { recursive: true });
  const outside = fakeInstall({ at: path.join(fixture, 'elsewhere') });

  const { code, stdout } = await runShim(outside.shim, [stub('identify.js')], { cwd: repository });

  assert.equal(code, 0);
  assert.ok(samePath(JSON.parse(stdout).binary, binaryIn(outside.root)));
});
