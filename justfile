set windows-shell := ["powershell.exe", "-NoLogo", "-Command"]

_default:
  just --list --unsorted

# Compile every crate in the workspace
build:
  cargo build --workspace

# Run all tests: nextest for unit + integration, cargo for doctests.
# The nextest profile is `default` here and `ci` in the workflow.
test profile="default":
  # `rune-testkit` is a plain binary with no tests of its own, so a test build never
  # selects it. The suites spawn it, so it has to be built on purpose.
  cargo build --workspace --bins
  cargo nextest run --workspace --profile {{profile}}
  cargo test --workspace --doc

# Run the packaging and release tests: the wrapper, the platform table, the published
# types, and the decisions the release makes before it publishes
test-npm:
  node --test "npm/test/**/*.test.js"

# What CI will run: format check + lints, warnings are errors
lint:
  cargo fmt --all --check
  cargo clippy --workspace --all-targets -- --deny warnings

# Auto-fix formatting and machine-fixable lints
fix:
  cargo fmt --all
  cargo clippy --workspace --all-targets --fix --allow-dirty --allow-staged

# Assemble and pack this machine's platform package and the meta package, exactly as the
# release pipeline does it, so a local dry run predicts what the pipeline will publish
dist:
  cargo build --release
  node npm/scripts/dist.js

# Install the packed tarballs into the fixture project and run scripts through npm,
# the way the pipeline does on every operating system before it publishes anything
smoke:
  node npm/scripts/smoke-install.js

# Gate G8: measure a warm run against the budget, as the release does
bench:
  cargo build --release --bin rune
  node npm/scripts/benchmark.js

# Rehearse a release without publishing: the gate, the pin validation, the packaging and
# the retry wrapper all run; nothing reaches the registry
release-rehearsal: dist smoke
  node npm/scripts/rehearse.js

# Serve the documentation site with hot reload
docs:
  pnpm --filter rune-website dev

# Type-check and build the documentation site, as CI will
docs-build:
  pnpm --filter rune-website typecheck
  pnpm --filter rune-website build