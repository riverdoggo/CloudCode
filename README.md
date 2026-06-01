# Cloud Code

**Cloud Code** is a terminal coding agent: analyze repositories, edit files with diffs, run shell commands safely, and work with multiple AI providers (OpenAI, Anthropic, Ollama, Groq, OpenRouter, and more).

Repository: [github.com/riverdoggo/CloudCode](https://github.com/riverdoggo/CloudCode)

## Install

**Windows (PowerShell):**

```powershell
./install-cloud-code.ps1
```

**From source (Rust):**

```bash
cd rust
cargo install --path crates/claw-cli --force
```

Open a new terminal, then:

```bash
cloud-code --version
cloud-code
```

## Usage

Interactive REPL:

```bash
cloud-code
```

One-shot prompt:

```bash
cloud-code prompt "explain this codebase"
```

With a model:

```bash
cloud-code --model sonnet prompt "fix the failing test"
```

## Configuration

- Project: `.claw.json`, optional `CLOUD.md` (agent instructions)
- User settings: under your home directory (see `docs/configuration.md`)
- Secrets: copy `.env.example` to `.env` locally (never commit `.env`)

## Development

```bash
cd rust
cargo fmt
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Or from the repo root: `make dev`, `./scripts/dev.sh`, or `./scripts/dev.ps1`.

## Documentation

- [Architecture](docs/architecture.md)
- [Configuration](docs/configuration.md)
- [Security](docs/security.md)
- [Contributing](docs/contributing.md)
- [Tool API](docs/tool-api.md)

## License

MIT — see [rust/Cargo.toml](rust/Cargo.toml) workspace metadata.
