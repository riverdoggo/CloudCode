# Contributing

Thanks for contributing to `agent`.

## Local Development

Use one of:

- `make dev`
- `./scripts/dev.sh`
- `./scripts/dev.ps1`

## Quality Gates

Before submitting a PR, run:

```bash
cd rust
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## Project Priorities

- Keep the first version minimal and stable.
- Preserve diff-first edit behavior.
- Keep sandbox restrictions explicit and testable.
- Maintain local model support as a first-class path.

## Runtime Ownership Policy

- Rust is the only production runtime implementation path.
- Python is reserved for tests, parity checks, fixtures, and developer tooling.
- New agent runtime behavior must be implemented in Rust crates under `rust/crates/*`.
- Python changes that alter runtime semantics must be paired with equivalent Rust behavior in the same milestone.
- If a feature cannot be delivered in Rust yet, capture it as a roadmap item instead of shipping Python-only runtime logic.

## Testing Expectations

- Add unit tests for new logic.
- Add integration tests for tool/model wiring changes.
- Add API contract tests for server endpoint changes.
