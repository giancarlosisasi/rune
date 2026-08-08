'use strict';

// Test 6a.10 — the messages are the feature, so the exact text is what is asserted.

const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const test = require('node:test');

const { diagnose } = require('../rune/lib/diagnose');
const inspect = require('../rune/lib/inspect');
const { assertSnapshot } = require('./helpers/snapshot');

test('an unsupported platform is told what is supported', () => {
  assertSnapshot(
    'diagnostic-unsupported',
    diagnose({ kind: 'unsupported', platform: 'freebsd', arch: 'x64', hostPlatform: 'freebsd' }),
  );
});

test('a lockfile that omits the package is named, cited and repaired', () => {
  assertSnapshot(
    'diagnostic-lockfile',
    diagnose({
      kind: 'missing',
      platform: 'linux',
      arch: 'x64',
      package: '@gio-labs/rune-linux-x64',
      hostPlatform: 'linux',
      tried: ['@gio-labs/rune-linux-x64/bin/rune'],
      lockfile: { path: '/repo/pnpm-lock.yaml', manager: 'pnpm', mentionsPackage: false },
      packageManager: { name: 'pnpm', major: 11 },
    }),
  );
});

test('binaries for another platform explain the boundary they came across', () => {
  assertSnapshot(
    'diagnostic-foreign',
    diagnose({
      kind: 'missing',
      platform: 'win32',
      arch: 'x64',
      package: '@gio-labs/rune-win32-x64',
      hostPlatform: 'win32',
      tried: ['@gio-labs/rune-win32-x64/bin/rune.exe'],
      lockfile: { path: 'C:\\repo\\package-lock.json', manager: 'npm', mentionsPackage: true },
      foreign: ['@gio-labs/rune-linux-x64'],
      packageManager: { name: 'npm', major: 11 },
    }),
  );
});

test('a plain missing package still gets a repair command', () => {
  assertSnapshot(
    'diagnostic-missing',
    diagnose({
      kind: 'missing',
      platform: 'darwin',
      arch: 'arm64',
      package: '@gio-labs/rune-darwin-arm64',
      hostPlatform: 'darwin',
      tried: [
        '@gio-labs/rune-darwin-arm64/bin/rune',
        '@gio-labs/rune-darwin-x64/bin/rune',
      ],
      foreign: [],
      packageManager: undefined,
    }),
  );
});

test('an override that names nothing says why nothing else was tried', () => {
  assertSnapshot(
    'diagnostic-override',
    diagnose({
      kind: 'override-missing',
      variable: 'RUNE_BINARY_PATH',
      path: '/home/dev/rune/target/debug/rune',
      hostPlatform: 'linux',
    }),
  );
});

test('the repair command is the one for the manager in use', () => {
  const managers = ['npm', 'pnpm', 'bun', 'yarn'].flatMap((name) =>
    name === 'yarn' ? [{ name, major: 1 }, { name, major: 4 }] : [{ name, major: 11 }],
  );

  const commands = [...managers, undefined].flatMap((packageManager) =>
    ['linux', 'win32'].map((hostPlatform) => {
      const message = diagnose({
        kind: 'missing',
        platform: hostPlatform,
        arch: 'x64',
        package: '@gio-labs/rune-linux-x64',
        hostPlatform,
        packageManager,
      });
      const repair = message.split('repair it with:')[1].trim();
      return `${packageManager ? `${packageManager.name} ${packageManager.major}` : 'unknown'} on ${hostPlatform}: ${repair}`;
    }),
  );

  assertSnapshot('repair-commands', `${commands.join('\n')}\n`);
});

test('the package manager is read from the environment it sets', () => {
  const agent = 'pnpm/11.20.0 npm/? node/v24.14.1 win32 x64';

  assert.deepEqual(inspect.detectPackageManager({ npm_config_user_agent: agent }), {
    name: 'pnpm',
    major: 11,
  });
  assert.equal(inspect.detectPackageManager({}), undefined);
});

test('the lockfile is found by walking up out of node_modules', () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'rune-lock-'));
  const nested = path.join(root, 'node_modules', '@gio-labs', 'rune');
  fs.mkdirSync(nested, { recursive: true });
  fs.writeFileSync(path.join(root, 'yarn.lock'), '"@gio-labs/rune@^0.1.0":\n');

  const found = inspect.findLockfile(nested);

  assert.equal(found.manager, 'yarn');
  assert.equal(found.path, path.join(root, 'yarn.lock'));
  assert.equal(inspect.lockfileMentions(found.path, '@gio-labs/rune-linux-x64'), false);
  assert.equal(inspect.lockfileMentions(found.path, '@gio-labs/rune'), true);

  fs.rmSync(root, { recursive: true, force: true });
});

test('platform packages for another system are found where they are', () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'rune-foreign-'));
  const scope = path.join(root, 'node_modules', '@gio-labs');
  fs.mkdirSync(path.join(scope, 'rune-linux-arm64'), { recursive: true });
  fs.mkdirSync(path.join(scope, 'rune'), { recursive: true });

  assert.deepEqual(inspect.foreignPackages(path.join(scope, 'rune')), [
    '@gio-labs/rune-linux-arm64',
  ]);

  fs.rmSync(root, { recursive: true, force: true });
});
