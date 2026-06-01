# agent

`agent` is a terminal coding agent that helps developers run AI coding tasks directly inside a repository.

## Core Product Definition

The first-class experience is a terminal workflow:

```text
agent
> fix authentication bug
> write tests for user service
> refactor auth module
```

or direct commands:

```text
agent fix bug in auth.rs
agent explain src/server.rs
agent test
```

The agent is designed to:

- analyze repositories
- read and edit files
- propose diffs
- execute shell commands safely
- maintain conversational context

## Model Provider Support

The project supports multiple model providers:

- OpenAI
- Anthropic
- Ollama
- Groq
- OpenRouter

Example model configuration:

```toml
model = "openai:gpt-4.1"
```

Local model support is a primary capability, not an optional extension. Supporting local models early avoids vendor lock-in and differentiates the tool from hosted-only coding assistants.

## Architecture Overview

Core components:

- CLI Interface
- Agent Runtime
- Tool System
- Plugin System
- Model Providers
- Context Engine
- Security Sandbox
- Optional HTTP API

High-level architecture:

```text
CLI
 │
Agent Runtime
 │
Tool Execution Layer
 │
Plugin Layer
 │
Model Adapter Layer
 │
Filesystem / Git / Shell
```

Data flow:

```text
User command
 ↓
CLI parses command
 ↓
Agent builds context
 ↓
Prompt sent to model
 ↓
Model returns action
 ↓
Tool executed
 ↓
Result returned to agent
 ↓
Loop continues until task complete
```

The architecture is plugin-first so tools, models, and behaviors can evolve without changing core runtime components.

## Project Structure

Target modular Rust layout:

```text
src/
 ├ main.rs
 ├ cli/
 │   ├ commands.rs
 │   └ repl.rs
 │
 ├ agent/
 │   ├ controller.rs
 │   ├ planner.rs
 │   └ context.rs
 │
 ├ tools/
 │   ├ read_file.rs
 │   ├ write_file.rs
 │   ├ search.rs
 │   ├ shell.rs
 │   └ git.rs
 │
 ├ providers/
 │   ├ openai.rs
 │   ├ anthropic.rs
 │   ├ ollama.rs
 │   └ router.rs
 │
 ├ plugins/
 │   ├ loader.rs
 │   └ registry.rs
 │
 ├ sandbox/
 │   └ permissions.rs
 │
 ├ diff/
 │   └ patch.rs
 │
 └ config/
     └ settings.rs
```

Optional directories:

- `server/`
- `python-sdk/`
- `docs/`
- `examples/`

## CLI Design

The CLI uses `clap` and supports:

- `agent`
- `agent fix <task>`
- `agent explain <file>`
- `agent test`
- `agent commit`
- `agent config`
- `agent demo`

Interactive REPL example:

```text
> analyze repository
> fix failing tests
> generate documentation
```

CLI UX priorities:

- fast startup
- clear feedback
- readable diff previews
- predictable command syntax
- minimal cognitive overhead

## Agent Loop

```text
while task_not_finished:
    build_context()
    send_prompt()
    receive_response()

    if response.contains_tool_call:
        execute_tool()

    update_context()
```

The first version stays intentionally minimal while supporting:

- multi-step reasoning
- tool invocation
- incremental context updates

## Tool System

Minimum tools:

- `read_file`
- `write_file`
- `search_repo`
- `run_command`
- `git_diff`

Example tool schema:

```json
{
  "name": "read_file",
  "description": "Read a file from the workspace",
  "parameters": {
    "path": "string"
  }
}
```

All file modifications must flow through diff generation; files must never be overwritten directly.

Patch-apply behavior is runtime-mediated:

```text
Proposed change: src/auth.rs

--- src/auth.rs
+++ src/auth.rs
- let token = ""
+ let token = generate_token()

Apply this patch? (y/N)
```

## Tool Sandboxing

Permission modes:

- `safe`
- `workspace`
- `full-access`

Sandbox enforces:

- command allowlists
- directory access restrictions
- permission prompts
- execution limits

`safe` mode should block shell execution, block external network calls, and require confirmation for file modifications.

## Context Engine

Context retrieval should combine:

- keyword search
- git awareness
- syntax parsing (for example via Tree-sitter)

Context windows must remain within model token limits.

## Configuration

Default configuration path:

- `~/.agent/config.toml`

Example:

```toml
model = "anthropic:claude-sonnet"
temperature = 0.2
permission_mode = "workspace"

[providers.openai]
api_key = "..."

[providers.ollama]
endpoint = "http://localhost:11434"
```

Configuration also supports plugin enablement and permission mode defaults.

## CI / CD Requirements

CI enforces:

- `cargo fmt --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- coverage checks

GitHub Actions should run format, lint, unit tests, and integration tests on pull requests and pushes.

## Developer Onboarding

Quick start commands:

```bash
make dev
```

or:

```bash
./scripts/dev.sh
```

or on Windows:

```powershell
./scripts/dev.ps1
```

Each command should install dependencies, run the agent, and start a demo workspace.

## Testing Strategy

Test coverage should include:

- unit tests
- API contract tests
- agent loop tests
- tool execution tests

Python tests should be modular, for example:

```text
tests/
  agent/
  tools/
  api/
  integration/
```

## Release Strategy

Tagged releases should include:

- changelog
- version tag
- binary artifacts for Linux, macOS, and Windows

Install methods:

- `curl install.sh | bash`
- `cargo install agent-cli`

## Public Demo Mode

`agent demo` should:

- start a sample workspace
- run a scripted session
- demonstrate file editing and agent reasoning

## Documentation

Minimum docs:

- architecture overview: `docs/architecture.md`
- security model: `docs/security.md`
- tool API docs: `docs/tool-api.md`
- configuration guide: `docs/configuration.md`
- contribution guide: `docs/contributing.md`
- golden path E2E spec: `docs/golden-path-e2e.md`
- sandbox MVP spec: `docs/sandbox-mvp-spec.md`
- sandbox implementation pattern: `docs/sandbox-implementation-pattern.md`
- plugin safety model: `docs/plugin-safety-model.md`
- skills system (v0.2): `docs/skills-system.md`
- v0.1.0 acceptance checklist: `docs/v0.1.0-acceptance-checklist.md`
- v0.1.0 test matrix: `docs/test-matrix.md`

## 30-Day Execution Plan

Week 1:

- define runtime direction and finalize architecture

Week 2:

- implement CLI, model adapters, and basic tool system

Week 3:

- add context retrieval, diff editing, and test coverage

Week 4:

- prepare first release: `v0.1.0`
- include binary builds and demo documentation

## Additional Priorities

- Add plugin system early for long-term adaptability.
- Prioritize local model support from the beginning.
- Design tool sandboxing carefully to avoid unsafe execution.
- Keep the initial version minimal and stable.
- Invest heavily in terminal UX for adoption.
