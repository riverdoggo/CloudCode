# Security Model

The agent enforces permission modes and sandbox controls for tool execution.

## Permission Modes

- `safe`: read-focused mode with strict execution limits.
- `workspace`: allows operations in workspace boundaries.
- `full-access`: broader capabilities for trusted environments.

## Safe Mode Requirements

- Block shell command execution.
- Block external network calls.
- Require confirmation for file modifications.
- Restrict writes unless explicitly approved.

## Sandbox Controls

- Command allowlists for executable tools.
- Directory restrictions rooted at workspace.
- Per-action prompts for sensitive operations.
- Execution timeouts and output limits.

## Diff-First Writes

All file updates must flow through generated diffs. Direct overwrite behavior is disallowed.

## Operational Guidance

- Default to `safe` for unknown repositories.
- Elevate permissions only when the user approves.
- Log tool calls to aid review and auditing.
