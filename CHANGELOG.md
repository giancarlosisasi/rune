# Changelog

Every notable change to rune.
## 0.1.4 — 2026-08-10


### Added

- Write the config where the project is, and seed scripts that run
- Run the rune a repository installs, whichever copy was reached
- Name a package narrowing for what it is


### Fixed

- Ship the licence with every package and publish no scripts
- Report the reason a config search actually ended with

## 0.1.3 — 2026-08-09


### Added

- Improve cwd and argument handling

## 0.1.2 — 2026-08-09


### Added

- Export rune from the supplied module instead of a global

## 0.1.1 — 2026-08-08


### Added

- Implement config parsing and loading
- Run scripts through the platform shell
- Forward arguments after `--` and select commands by system
- Extend scripts and let a package narrow a shared one
- Read a script env file without letting it override the environment
- Scaffold a starter config with rune init
- Run scripts in order with serial groups and dependsOn
- Make the output of several scripts attributable on one terminal
- Run scripts at the same time with parallel groups
- Give scripts a timeout, retries and a way to be ended
- Configure and setup npm publishing
- Publish rune to npm from a merged version bump
- Create a package name on npm before its first release


### Fixed

- Look up parent environment variables the way Windows names them
- Make the suite pass on macOS and on a CRLF checkout
- Compare paths by what they point at, not by how they are spelled
- Redact the longest spelling of a path first

