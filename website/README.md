# rune-website

The documentation site at [rune.gio-labs.com](https://rune.gio-labs.com). Rspress v2, static output.

## Local

```bash
just docs         # dev server with hot reload
just docs-build   # typecheck, then production build
```

Or directly, from this directory:

| Script | What it does |
| --- | --- |
| `pnpm dev` | Dev server |
| `pnpm build` | Static build into `doc_build/` |
| `pnpm preview` | Serve the built output |
| `pnpm typecheck` | `tsc --noEmit` |

Run `typecheck` before every commit. `rspress build` accepts an unknown config option silently, so
an option that moved between versions looks like it is working until a page renders wrong.
