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

## Brand assets

`brand/mark.svg` is the only drawing in the repository. It paints with `currentColor`, so one file
serves every surface. Everything in `docs/public/` is derived from it and committed: the brand tile,
the light and dark nav logos, the icons at every size, `favicon.ico`, the 1200x630 social card,
`site.webmanifest` and `robots.txt`. `web-assets.json` holds the colours, the card copy and the
output paths. `@resvg/resvg-js` and `png-to-ico` are the renderer.

Edit `brand/mark.svg` or `web-assets.json`, then regenerate. Editing `docs/public/` directly is
overwritten on the next run.
