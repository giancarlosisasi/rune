// Every script variant that exists in the published version, in one config that has to
// do two things at once: compile clean against the published types, and load in rune.
//
// This is the only place in the project where the TypeScript definition and the Rust
// deserializer are compared. A field either side learns about on its own shows up here as
// a failure rather than as a user's bug report.
//
// The tsconfig beside this file sets `"types": []`, so nothing ambient reaches it. That
// makes the clean compile a second assertion: the published types are enough on their
// own, and no user has to hand-write a declaration to read `rune`.

import { defineConfig, rune } from '@gio-labs/rune';

const reporter = rune.isCI ? 'junit' : 'dot';
const threads = rune.env.CPU_COUNT ?? '4';
const remove = rune.platform === 'win32' ? 'rmdir /s /q dist' : 'rm -rf dist';

export default defineConfig({
  scripts: {
    environment: {
      command: `vitest run --reporter=${reporter} --maxWorkers=${threads} && echo ${remove}`,
      description: 'Reads all three properties of the imported environment object',
    },
    clean: {
      command: { default: 'rm -rf dist', win32: 'rmdir /s /q dist' },
      description: 'Remove the build output',
    },
    build: {
      command: 'tsc -b',
      description: 'Compile every package',
      dependsOn: ['clean'],
      cwd: '.',
      env: { NODE_ENV: 'production' },
      envFile: 'fixture.env',
    },
    'build:ci': {
      extends: 'build',
      appendArgs: ['--force'],
      description: 'The build with nothing served from a previous run',
    },
    lint: {
      command: 'eslint .',
      timeout: 120_000,
      killSignal: 'SIGTERM',
      killTimeout: 5_000,
    },
    test: {
      command: 'vitest run',
      retries: 2,
      retryDelay: 'exponential',
    },
    dev: {
      command: 'vite',
      interactive: true,
    },
    verify: {
      serial: ['lint', 'test'],
      continueOnError: true,
      description: 'Everything that has to pass before a merge',
    },
    watch: {
      parallel: ['dev', 'test'],
      successPolicy: 'first',
    },
  },
});
