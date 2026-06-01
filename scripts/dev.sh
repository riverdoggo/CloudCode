#!/usr/bin/env bash
set -euo pipefail

echo "Starting agent development bootstrap..."

if ! command -v cargo >/dev/null 2>&1; then
  echo "Rust toolchain is required. Install rustup first."
  exit 1
fi

if [ ! -f ".agent/config.toml" ] && [ -f ".agent/config.toml.example" ]; then
  cp ".agent/config.toml.example" ".agent/config.toml"
  echo "Created .agent/config.toml from example."
fi

echo "Building Rust workspace..."
(cd rust && cargo build --workspace)

echo "Launching CLI..."
exec cargo run -p claw-cli --bin cloud-code -- "$@"
