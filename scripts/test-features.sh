#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

# Exercise both optional library features independently, the default
# configuration, and the combined configuration.
cargo test --locked --workspace
cargo test --locked --workspace --no-default-features
cargo test --locked -p tinyskills --no-default-features --features archive
cargo test --locked -p tinyskills --no-default-features --features registry
cargo test --locked --workspace --all-features
