# rune-out

Where rune's own output goes.

Part of the [rune](../../README.md) workspace. User documentation is at
[Prefixed output](https://rune.gio-labs.com/reference/output).

Two rules this crate exists to enforce. Stdout belongs to the child process, so everything rune says
about itself goes to stderr; only a command whose product is text — `rune list` — writes to stdout.
And output goes through here rather than through `println!`, which is why `print_stdout` and
`print_stderr` are denied workspace-wide.

| Module | Responsibility |
| --- | --- |
| `multiplex.rs` | A pure function from a sequence of chunks to the bytes a terminal receives |
| `channel.rs` | The queue feeding it: many producers, one writer |
| `color.rs` | How much colour rune may use, and what it tells a child about colour |

Colour is detected once, at the edge, and passed downstream as a value. That is what lets a test set
the level instead of faking a terminal.

```bash
cargo nextest run -p rune-out
```
