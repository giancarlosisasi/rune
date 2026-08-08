# rune-testkit

One fixture binary for the whole test suite. It is built by the workspace and spawned by the test
suites. It is never published and never shipped.

Part of the [rune](../../README.md) workspace.

Tests that need a real child process spawn this instead of `echo`, `sleep` or a shell one-liner.
Those differ between platforms and between shells, and the differences show up as flaky assertions
rather than as honest failures.

Two conventions matter to callers:

- Copies placed on `PATH` as fake tools are always named `*.exe`, on every operating system.
  Windows needs the extension to consider a file executable; Unix treats it as part of the name.
- Anything a test must synchronize on is announced with the `READY` token on stdout. No test in this
  project waits by sleeping.

Called as `-c "<command>"` it reports the invocation back, which is how a test proves which shell
actually ran.
