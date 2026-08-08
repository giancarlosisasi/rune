'use strict';

// Tests 6a.4 to 6a.8 — what the wrapper does to the process it runs.

const assert = require('node:assert/strict');
const { spawn } = require('node:child_process');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');

const platforms = require('../rune/lib/platforms');
const { fakeInstall, remove, stub } = require('./helpers/install');

// Every case runs against one installed tree; none of them writes to it.
const install = fakeInstall();
test.after(() => remove(install.root));

function runShim(args, { input, env } = {}) {
  return new Promise((resolve) => {
    const child = spawn(process.execPath, [install.shim, ...args], {
      cwd: install.root,
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

    if (input !== undefined) {
      child.stdin.end(input);
    } else {
      child.stdin.end();
    }

    child.on('close', (code, signal) => resolve({ code, signal, stdout, stderr }));
  });
}

test('6a.4 — arguments reach the binary byte for byte', async () => {
  const forwarded = ['an argument with spaces', '"quoted"', '--', "it's ñön-ASCII 😀", '-x=1'];

  const { stdout, code } = await runShim([stub('echo-args.js'), ...forwarded]);

  assert.equal(code, 0);
  assert.deepEqual(JSON.parse(stdout), forwarded);
});

test('6a.5 — the binary is handed the wrapper own streams', async () => {
  const { stdout, stderr, code } = await runShim([stub('streams.js')], { input: 'from the parent' });

  assert.equal(code, 0);
  assert.equal(stdout, 'heard: from the parent\n');
  assert.equal(stderr, 'this went to standard error\n');
});

test('6a.6 — the exit code is the binary own code', async () => {
  const { code } = await runShim([stub('exit-code.js'), '2']);

  assert.equal(code, 2);
});

test('6a.7 — a fatal signal is re-raised, not turned into a number', { skip: process.platform === 'win32' && 'POSIX signals' }, async () => {
  const { code, signal } = await runShim([stub('self-signal.js'), 'SIGTERM']);

  assert.equal(signal, 'SIGTERM', 'the caller must see a signal death, not an exit of 143');
  assert.equal(code, null);
});

test('6a.8 — the override runs the binary it names', async () => {
  const { stdout, code } = await runShim([stub('echo-args.js'), 'through the override'], {
    env: { RUNE_BINARY_PATH: process.execPath },
  });

  assert.equal(code, 0);
  assert.deepEqual(JSON.parse(stdout), ['through the override']);
});

test('6a.8 — an override that names nothing is a hard error', async () => {
  const missing = path.join(install.root, 'build', 'rune-that-is-not-there');

  const { code, stderr, stdout } = await runShim([stub('echo-args.js')], {
    env: { RUNE_BINARY_PATH: missing },
  });

  assert.equal(code, 1);
  assert.equal(stdout, '', 'the installed binary must not run instead');
  assert.match(stderr, /RUNE_BINARY_PATH names a binary that is not there/);
  assert.ok(stderr.includes(missing), 'the message has to name the path');
});

test('the wrapper resolves the real payload as an executable file', () => {
  const entry = platforms.entryFor(process.platform, process.arch);
  const binary = path.join(
    install.root,
    'node_modules',
    entry.package,
    platforms.BINARY_DIRECTORY,
    entry.binary,
  );

  assert.ok(fs.statSync(binary).isFile());
  fs.accessSync(binary, fs.constants.X_OK);
});
