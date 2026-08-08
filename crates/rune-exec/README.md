# rune-exec

Running what a script name stands for: one command, an ordered sequence, or several scripts at
once.

Part of the [rune](../../README.md) workspace. User documentation is at
[Groups](https://rune.gio-labs.com/config/groups) and
[Timeouts and retries](https://rune.gio-labs.com/config/lifecycle).

| Module | Responsibility |
| --- | --- |
| `spawn.rs` | A resolved script becomes a running child. Wiring is decided here and nowhere else |
| `shell.rs` | Which shell reads the command, and how the command reaches it |
| `environment.rs` | The four layers a child environment is built from |
| `bin_paths.rs` | Why a bare tool name works: `node_modules/.bin` on `PATH` |
| `quote.rs` | Making one pass-through argument survive one shell |
| `group.rs` | Several scripts at once, and ending them together |
| `lifecycle.rs` | Timeouts, retries, retry delay, kill signal and kill timeout |
| `signals.rs` | Ctrl+C reaches the child. Rune records it and keeps waiting |
| `teardown.rs` | Killing a process tree, and knowing when nothing is left to kill |

A single script inherits rune's own standard input, output and error, so colours, progress bars and
interactive prompts behave as they do without rune. Members of a parallel group are piped instead,
so their output can be labelled.

Inside, this is a tokio runtime waiting on many children and many pipes at once. None of that
reaches a caller: every entry point is an ordinary blocking function returning a `Completion`.

```bash
cargo nextest run -p rune-exec
```
