# Configuration Guide

Default configuration file:

- `~/.agent/config.toml`

## Basic Example

```toml
model = "openai:gpt-4.1"
temperature = 0.2
permission_mode = "workspace"

[providers.openai]
api_key = "..."

[providers.anthropic]
api_key = "..."

[providers.ollama]
endpoint = "http://localhost:11434"

[providers.groq]
api_key = "..."

[providers.openrouter]
api_key = "..."
```

## Provider Selection

Use a provider-qualified model string:

- `openai:gpt-4.1`
- `anthropic:claude-sonnet`
- `ollama:qwen2.5-coder:7b`
- `groq:llama-3.3-70b-versatile`
- `openrouter:anthropic/claude-sonnet-4`

## Plugins

Configure plugin enablement in config and load them during runtime startup.

## Permissions

Set `permission_mode` to one of:

- `safe`
- `workspace`
- `full-access`
