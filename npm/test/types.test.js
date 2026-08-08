'use strict';

// Tests 6a.11 and 6a.12 — the published types, and whether rune agrees with them.

const assert = require('node:assert/strict');
const { spawnSync } = require('node:child_process');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');

const { fakeInstall, remove } = require('./helpers/install');

const FIXTURES = path.join(__dirname, 'fixtures', 'types');
const WORKSPACE = path.join(__dirname, '..', '..');
const TSC = require.resolve('typescript/bin/tsc');

// A directory with the meta package installed under node_modules, holding the fixture
// files a case needs. tsc has to resolve `@giancarlosio/rune` through that package's own
// manifest: pointing it straight at the file in this repository would prove the file
// compiles and nothing about whether the published package leads anyone to it.
function project(files) {
  const install = fakeInstall();
  for (const name of ['tsconfig.json', ...files]) {
    fs.copyFileSync(path.join(FIXTURES, name), path.join(install.root, name));
  }
  return install.root;
}

function typecheck(root) {
  const result = spawnSync(process.execPath, [TSC, '--project', root], { encoding: 'utf8' });
  return `${result.stdout}${result.stderr}`.trim();
}

test('6a.11 — every combination the format forbids is a type error, one by one', () => {
  const root = project(['illegal.ts']);

  // An unused `@ts-expect-error` is itself an error, so this passes only when each marked
  // line really is rejected — never because the fixture had a typo in it.
  assert.equal(typecheck(root), '');
  remove(root);
});

test('an illegal combination that stops being illegal fails the suite', () => {
  const root = project([]);
  fs.writeFileSync(
    path.join(root, 'illegal.ts'),
    [
      "import type { Script } from '@giancarlosio/rune';",
      '// @ts-expect-error this line is perfectly legal',
      "export const legal: Script = { command: 'tsc -b' };",
      '',
    ].join('\n'),
  );

  assert.match(typecheck(root), /Unused '@ts-expect-error' directive/);
  remove(root);
});

test('6a.12 — a config the published types accept is a config rune loads', () => {
  const root = project(['rune.config.ts', 'fixture.env']);
  // rune walks up looking for the repository it is in. Without a boundary the walk leaves
  // the throwaway directory and finds whatever config sits above it on this machine.
  fs.mkdirSync(path.join(root, '.git'), { recursive: true });
  fs.writeFileSync(path.join(root, '.git', 'HEAD'), 'ref: refs/heads/main\n');

  assert.equal(typecheck(root), '', 'the fixture must typecheck before rune is asked');

  const listed = spawnSync(runeBinary(), ['list'], { cwd: root, encoding: 'utf8' });

  assert.equal(listed.status, 0, `rune rejected a config tsc accepted:\n${listed.stderr}`);
  for (const script of ['clean', 'build', 'build:ci', 'lint', 'test', 'dev', 'verify', 'watch']) {
    assert.match(listed.stdout, new RegExp(`\\b${script}\\b`));
  }
  remove(root);
});

// Whichever build is already there, so the suite does not force a profile on the machine.
function runeBinary() {
  const name = `rune${process.platform === 'win32' ? '.exe' : ''}`;
  for (const profile of ['debug', 'release']) {
    const candidate = path.join(WORKSPACE, 'target', profile, name);
    if (fs.existsSync(candidate)) {
      return candidate;
    }
  }

  const build = spawnSync('cargo', ['build', '--bin', 'rune'], {
    cwd: WORKSPACE,
    encoding: 'utf8',
    shell: process.platform === 'win32',
  });
  assert.equal(build.status, 0, `cargo build --bin rune failed:\n${build.stderr}`);

  return path.join(WORKSPACE, 'target', 'debug', name);
}
