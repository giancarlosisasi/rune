set windows-shell := ["powershell.exe", "-NoLogo", "-Command"]

_default:
  just --list --unsorted

# Compile every crate in the workspace
build:
  cargo build --workspace

# Run all tests
# Run all tests: nextest for unit + integration, cargo for doctests
test:
  cargo nextest run --workspace
  cargo test --workspace --doc

# Run the packaging tests: the wrapper, the platform table, and the published types
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

# Serve the documentation site with hot reload
docs:
  pnpm --filter rune-website dev

# Type-check and build the documentation site, as CI will
docs-build:
  pnpm --filter rune-website typecheck
  pnpm --filter rune-website build