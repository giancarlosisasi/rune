# rune-cli

The `rune` binary. Parses the command line, asks `rune-config` what a script name stands for, and
hands the answer to `rune-exec`.

Part of the [rune](../../README.md) workspace. User documentation is at
[rune.gio-labs.com/cli](https://rune.gio-labs.com/cli/).

| Command | Module | Docs |
| --- | --- | --- |
| `rune run <name>` | `run.rs` | [rune run](https://rune.gio-labs.com/cli/run) |
| `rune list` | `list.rs` | [rune list](https://rune.gio-labs.com/cli/list) |
| `rune inspect <name>` | `inspect.rs` | [rune inspect](https://rune.gio-labs.com/cli/inspect) |
| `rune init` | `init.rs` | [rune init](https://rune.gio-labs.com/cli/init) |
| `rune cache clear` | `list.rs` | [rune cache](https://rune.gio-labs.com/cli/cache) |

`script.rs` holds what every subcommand does first: find the config, and say something useful when
the name the user typed is not in it.

The integration suites in `tests/` drive the built binary. `parity.rs` compares rune against `npm
run` byte for byte, and `tty.rs` runs it under a real pseudo-terminal. Both spawn `rune-testkit`
rather than shell one-liners.

```bash
just test
cargo nextest run -p rune-cli
```
