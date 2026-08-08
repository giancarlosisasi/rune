# rune-config

Finding `rune.config.ts`, evaluating it, and turning a script name into everything spawning it
needs.

Part of the [rune](../../README.md) workspace. User documentation is at
[rune.gio-labs.com/config](https://rune.gio-labs.com/config/).

## The path a config takes

| Step | Module | What happens |
| --- | --- | --- |
| Discover | `discover.rs` | Walk up from the working directory, stop at the repository boundary |
| Strip | `strip.rs` | Remove TypeScript types with oxc, producing runnable JavaScript |
| Evaluate | `eval.rs`, `globals.rs`, `resolve.rs` | Run the module graph in an embedded QuickJS engine |
| Validate | `schema.rs` | Reject a bad shape with a message naming the script it came from |
| Cache | `cache.rs` | Content in, hash out, hit or miss. No TTL and no mtime comparison |
| Resolve | `inherit.rs`, `compose.rs` | A name becomes a command, or an ordered list of runs |

`env.rs` and `envfile.rs` decide what a config may know about the machine and how dotenv files are
read. `trace.rs` maps an engine stack trace back to the TypeScript the user wrote, and `suggest.rs`
turns a name nobody defined into the name they meant.

A config runs with one ambient object — `rune.env`, `rune.platform`, `rune.isCI` — and nothing else.
npm packages and Node built-ins do not exist at evaluation time, so an import must be a relative
`.ts` file in the repository. Every refused name says what does work instead.

```bash
cargo nextest run -p rune-config
```
