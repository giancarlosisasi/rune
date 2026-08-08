# rune

A script runner for JavaScript and TypeScript monorepos, written in Rust. Commands live in one
typed config at the repository root, and every package calls them by name.

**[rune.gio-labs.com](https://rune.gio-labs.com)** — full documentation.

## Install

Install once, at the workspace root. One native binary per platform resolves through optional
dependencies, so nothing is compiled and no postinstall script runs.

```bash
pnpm add -D @giancarlosio/rune    # npm install --save-dev / yarn add -D / bun add -d
```

Check the binary:

```bash
pnpm rune --version
```

## The first config

`rune init` writes a starter config next to the nearest `package.json`. Put it at the repository
root — rune finds it by walking up from wherever it is invoked.

```bash
pnpm rune init
```

```ts
// rune.config.ts
import { defineConfig } from '@giancarlosio/rune';

export default defineConfig({
  scripts: {
    build: {
      command: 'tsc --build',
      description: 'Compile every package',
    },
    test: {
      command: 'vitest run',
      env: { NODE_ENV: 'test' },
      dependsOn: ['build'],
    },
    dev: {
      parallel: ['dev:api', 'dev:web'],
      description: 'Serve the API and the web app',
    },
    'dev:api': { command: 'tsx watch src/server.ts', cwd: 'packages/api' },
    'dev:web': { command: 'vite', cwd: 'packages/web' },
  },
});
```

`defineConfig` is an identity function that supplies the types. A script declares exactly one of
`command`, `extends`, `serial` or `parallel`.

## Run

```bash
pnpm rune run build
pnpm rune run test -- --watch    # everything after -- is appended to the command
```

The script gets the real terminal and rune exits with the child's code. Rune's own diagnostics go
to stderr, so stdout belongs to the script.

## Commands

| Command | What it does |
| --- | --- |
| `rune run <name>` | Run a script and propagate its exit code |
| `rune list` | Every script visible from here, with descriptions |
| `rune inspect <name>` | The resolved command, environment and chain, without spawning |
| `rune init` | Write a starter config, optionally seeded from `package.json` |
| `rune cache clear` | Remove every cached config result |

## Working on rune

A Cargo workspace and the documentation site. [`just`](https://github.com/casey/just) holds the
commands; `just --list` prints them all.

```bash
just build   # compile every crate
just test    # nextest for unit and integration tests, then doctests
just lint    # what CI runs: fmt check and clippy, warnings are errors
just fix     # apply formatting and machine-fixable lints
just docs    # documentation site with hot reload
```

| Crate | Responsibility |
| --- | --- |
| [`rune-cli`](crates/rune-cli) | The `rune` binary: argument parsing and the five commands |
| [`rune-config`](crates/rune-config) | Finding, evaluating and caching `rune.config.ts` |
| [`rune-exec`](crates/rune-exec) | Spawning scripts, groups, timeouts, retries and teardown |
| [`rune-out`](crates/rune-out) | Where rune's own output goes, and prefixed group output |
| [`rune-testkit`](crates/rune-testkit) | A fixture binary the test suites spawn. Never shipped |
| [`website`](website) | The documentation site |

## License

[MIT](LICENSE)
