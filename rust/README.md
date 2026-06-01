# Cloud Code — Rust implementation

A terminal coding agent: fast Rust CLI, tool execution, plugins, and multi-provider model support.

## Quick start

```bash
cd rust
cargo build --release
```

Install globally:

```powershell
# from repo root
./install-cloud-code.ps1
```

Then in a new terminal:

```bash
cloud-code --version
cloud-code
```

## Development

```bash
cd rust
cargo fmt
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

From the repo root you can also use `make dev`, `./scripts/dev.sh`, or `./scripts/dev.ps1`.

## Layout

```text
rust/crates/
  cloud-code binary   claw-cli/   (package name; binary: cloud-code)
  api/                Model providers and HTTP client
  runtime/            Agent loop, prompts, sessions
  tools/              Built-in tools
  plugins/            Plugin loader
  commands/           Slash commands
  compat-harness/     Optional upstream manifest extraction (dev)
```

## Project guidance

Repos can include a root `CLOUD.md` with instructions for the agent. Run `/init` in a project to generate starter config (`.claw.json`, `CLOUD.md`, `.gitignore` entries).
