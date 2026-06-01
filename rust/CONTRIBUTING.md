# Contributing

Thanks for contributing to Cloud Code.

## Local development

From the repo root:

- `make dev`
- `./scripts/dev.sh`
- `./scripts/dev.ps1`

Or from `rust/`:

```bash
cargo fmt
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## Pull requests

- Keep changes focused and reviewable.
- Run format, clippy, and tests before opening a PR.
- Update `CLOUD.md` or docs when workflows change.
