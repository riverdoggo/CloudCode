# Architecture Overview

The `agent` project is a terminal coding agent with a modular runtime.

## Components

- CLI interface
- Agent runtime
- Tool execution layer
- Plugin system
- Model adapter layer
- Context engine
- Security sandbox
- Optional HTTP API

## Data Flow

```text
User command
 -> CLI parse
 -> Context assembly
 -> Model request
 -> Tool call decision
 -> Tool execution
 -> Context update
 -> Continue until complete
```

## Runtime Boundaries

- CLI handles user I/O and command parsing.
- Runtime owns loop state, planning, and context evolution.
- Tools perform scoped operations (files, git, shell).
- Providers abstract model APIs and local endpoints.
- Plugins extend tools/providers/behaviors without changing runtime core.

## Design Goals

- Keep core loop minimal in v0.1.
- Enforce diff-first file editing.
- Keep provider interfaces consistent across hosted and local models.
- Prioritize predictable CLI UX and fast startup.
