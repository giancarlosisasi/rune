# rune

One place for the scripts of a whole monorepo. Every package runs them with
`rune run <name>`, so a shared command changes in one file instead of in every
`package.json`.

```bash
pnpm add -D @giancarlosio/rune
```

```ts title="rune.config.ts"
import { defineConfig } from '@giancarlosio/rune';

export default defineConfig({
  scripts: {
    build: { command: 'tsc --build', description: 'Compile every package' },
    'build:ci': { extends: 'build', appendArgs: ['--force'] },
    ci: { serial: ['lint', 'build:ci', 'test'] },
  },
});
```

Documentation: <https://github.com/giancarlosisasi/rune>

## What gets installed

This package carries a small Node wrapper and the TypeScript types. The binary itself
arrives through one of six platform packages, listed here as exact-pinned optional
dependencies, of which a package manager installs exactly one. Nothing is compiled and
no install script runs.

| Platform | Architecture |
| --- | --- |
| Windows | x64, and arm64 through emulation |
| macOS | x64, arm64 |
| Linux | x64, arm64, statically linked against musl — one binary for every distribution |

## Node

The package declares no `engines`, so it installs under any Node and can report what
went wrong on a platform it does not support. The wrapper's test suite runs on Node 20,
22 and 24; 20 is the lowest of those, and so the tested floor.

## When the binary is not there

Run the command once and the wrapper says which of the known causes it can prove — a
lockfile that dropped the optional dependency, a `node_modules` shared between two
systems, an unsupported platform — and prints the exact repair for the package manager
in use.

`RUNE_BINARY_PATH` runs a binary of your own, for working on rune itself. If it names a
file that is not there, that is an error rather than a quiet fall back to the installed
release.

## License

MIT
