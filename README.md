# rune

A script runner for JavaScript and TypeScript monorepos, written in Rust. Commands live in one
typed config at the repository root, and every package calls them by name.

**[rune.gio-labs.com](https://rune.gio-labs.com)** — full documentation.

## Install

Install once, at the workspace root. One native binary per platform resolves through optional
dependencies, so nothing is compiled and no postinstall script runs.

```bash
pnpm add -D @gio-labs/rune    # npm install --save-dev / yarn add -D / bun add -d
```

Check the binary:

```bash
pnpm rune --version
```

### Supported platforms

<!-- platforms:start -->

| System | Architecture | Package | Binary |
| --- | --- | --- | --- |
| Windows | x64 | `@gio-labs/rune-win32-x64` | native |
| Windows | arm64 | `@gio-labs/rune-win32-arm64` | ships the x64 binary, run under emulation |
| macOS | x64 | `@gio-labs/rune-darwin-x64` | native |
| macOS | arm64 | `@gio-labs/rune-darwin-arm64` | native |
| Linux | x64 | `@gio-labs/rune-linux-x64` | native |
| Linux | arm64 | `@gio-labs/rune-linux-arm64` | native |

<!-- platforms:end -->

The Linux binaries are statically linked, so one build per architecture runs on Alpine, Debian and
everything between. Every release is checked in both a musl container and a GNU C library
container before it is published.

## The first config

`rune init` writes a starter config next to the nearest `package.json`. Put it at the repository
root — rune finds it by walking up from wherever it is invoked.

```bash
pnpm rune init
```

```ts
// rune.config.ts
import { defineConfig } from '@gio-labs/rune';

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

## What config code can do

The config is real TypeScript, evaluated before any script runs. Types are stripped and the file
is run in an embedded JavaScript engine, so a command can be computed instead of written out.

```ts
import { rune } from '@gio-labs/rune';

const port = process.env.CI ? 4000 : 3000;   // ✗ there is no `process`
const port = rune.isCI ? 4000 : 3000;        // ✓
```

| Available | What it is |
| --- | --- |
| `rune.platform` | `'win32'`, `'darwin'` or `'linux'` |
| `rune.env` | The environment rune was invoked with, read-only |
| `rune.isCI` | Whether `CI` is set |
| Relative imports | `./scripts/helpers.ts` and anything it imports, TypeScript included |

`rune` is an export of `@gio-labs/rune`, so the published types cover it and the import works in
any file the config pulls in.

Bare imports of npm packages do not resolve: the engine is not Node, and a config that needed
`node_modules` would make loading a config as slow as the scripts it describes. Everything a
config needs goes in a file beside it.

Where a command genuinely differs per system, say so instead of branching:

```ts
'open:coverage': {
  command: {
    default: 'xdg-open coverage/index.html',
    win32: 'start coverage/index.html',
    darwin: 'open coverage/index.html',
  },
},
```

## Run

```bash
pnpm rune run build
pnpm rune run test --watch       # everything after the name is appended to the command
pnpm rune run --root test        # rune's own options come before the name
```

The script gets the real terminal and rune exits with the child's code. Rune's own diagnostics go
to stderr, so stdout belongs to the script.

Everything after the script name belongs to the command, so a `package.json` script needs no
separator:

```json
{ "scripts": { "test": "rune run test" } }
```

`npm test -- --watch` and `pnpm test -- --watch` both work. The package manager appends `--watch` to
the command string, which is why the `--` never arrives and why nothing needs it to. Typing `--`
yourself still works and is still the way to pass a value that would otherwise read as one of rune's
own options: `rune run build -- --root`.

### Calling rune on Windows

`pnpm exec rune`, a `package.json` script, and Git Bash all hand your arguments to rune unchanged.

Running `node_modules\.bin\rune.CMD` by hand does not. That file is a batch shim the package manager
generated, and `cmd.exe` re-parses everything through it, so `&`, `^`, `|`, `<`, `>`, `(`, `)`, `%`
and `!` in an argument change meaning before rune starts. This is true of every tool installed from
npm, not of rune alone.

Arguments rune passes **to** a tool are safe in the same situation. Nearly every tool in
`node_modules/.bin` is a batch file that re-reads its own arguments, so rune escapes them for both
readers when the command resolves to a `.cmd` or `.bat` file. The limit: a batch file that itself
calls another batch file adds a third reader, and nothing rune does covers that. No runner in this
ecosystem does.

## Commands

| Command | What it does |
| --- | --- |
| `rune run <name>` | Run a script and propagate its exit code |
| `rune list` | Every script visible from here, with descriptions |
| `rune inspect <name>` | The resolved command, environment and chain, without spawning |
| `rune init` | Write a starter config, optionally seeded from `package.json` |
| `rune cache clear` | Remove every cached config result |

## Using rune with Turbo or Nx

Rune is a script registry and a runner. It is not a task graph: no caching, no topological
ordering, no remote execution. Turbo and Nx sit on top of it and keep doing that part.

**Add `rune.config.ts` to your task runner's declared inputs.** Rune's whole point is that the
script strings in `package.json` stop changing, and those strings are part of what Turbo and Nx
hash to decide whether a task can be replayed from cache. A change that only exists in the rune
config is invisible to them: the cache key does not move, and the task is served from cache with
the old command's result. Nothing reports an error.

```json
// turbo.json
{
  "tasks": {
    "build": {
      "inputs": ["$TURBO_DEFAULT$", "../../rune.config.ts", "../../scripts/**"]
    }
  }
}
```

Include every file the config imports, not only the config itself. For Nx, the equivalent is
`namedInputs` / `inputs` on the target.

## Working on rune

A Cargo workspace and the documentation site. [`just`](https://github.com/casey/just) holds the
commands; `just --list` prints them all.

```bash
just build   # compile every crate
just test    # nextest for unit and integration tests, then doctests
just lint    # what CI runs: fmt check and clippy, warnings are errors
just fix     # apply formatting and machine-fixable lints
just dist    # assemble and pack this machine's packages, as the release does
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

### Releasing

A release is a merged pull request. Run the **version bump** workflow with the version to release;
it opens a pull request carrying the new number, the regenerated pins and a generated changelog.
Merging it builds every platform, runs the binaries under both Linux C libraries, installs the
packed tarballs on each operating system, checks the warm-run benchmark, and only then publishes —
with provenance, through OIDC, with no registry token anywhere in this repository.

## License

[MIT](LICENSE)
