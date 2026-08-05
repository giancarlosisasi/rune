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

# What CI will run: format check + lints, warnings are errors
lint:
  cargo fmt --all --check
  cargo clippy --workspace --all-targets -- --deny warnings

# Auto-fix formatting and machine-fixable lints
fix:
  cargo fmt --all
  cargo clippy --workspace --all-targets --fix --allow-dirty --allow-staged