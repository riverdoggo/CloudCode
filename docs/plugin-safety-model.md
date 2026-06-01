# Plugin Safety Model

This document defines the plugin security model for `v0.1.0`.

## Goal

Enable plugin extensibility without allowing plugins to bypass runtime security controls.

## Core Safety Rule

Plugins may declare tools, but they cannot execute outside the runtime dispatch boundary.

All tool execution (built-in and plugin-provided) must pass through:

1. permission authorization
2. sandbox decision evaluation
3. workspace path boundary validation
4. shell allowlist enforcement
5. tool execution

## Trust Model

- **Built-in tools**: shipped with runtime, reviewed with project code.
- **External plugins**: third-party or local extensions loaded at runtime.

External plugins are untrusted by default and must always execute behind the same enforcement chain.

## Registration Model

Plugins register tools through runtime-managed metadata (name, description, schema, required permission).

Example declaration fields:

- `name`
- `description`
- `input_schema`
- `required_permission` (`ReadOnly`, `WorkspaceWrite`, `DangerFullAccess`)

The runtime is the source of truth for permission resolution and dispatch admission.

## Execution Model

Plugin handlers only implement tool behavior. They do not own security decisions.

Execution flow:

1. plugin registers tool metadata
2. runtime receives tool call
3. runtime runs enforcement chain
4. runtime invokes plugin handler only if checks pass
5. runtime captures result and emits tool output

## Filesystem Rule

Plugins must not receive raw unchecked filesystem authority.

- Runtime validates and constrains file paths first.
- Plugin handler receives only validated tool input.
- Any path outside workspace is denied before plugin execution.

## Shell Rule

Plugins must not spawn shell processes directly as a bypass path.

If plugin logic requires command execution, it must request runtime shell tooling so mode checks, allowlist checks, and audit logging are applied consistently.

## Permission Declaration and Enforcement

Each plugin tool must declare the minimum required permission mode.

Runtime behavior:

- declared permission is merged into permission policy
- tool call is denied when current mode is insufficient
- escalation prompts (if configured) are handled by runtime prompter path

## Audit Logging

All plugin tool executions should be logged with:

- plugin identifier
- tool name
- active mode
- allow/deny decision
- denial reason (when denied)

Example:

```text
Plugin tool executed
plugin=git-tools
tool=git_status
mode=workspace-write
decision=allow
```

## Non-Bypass Requirement

A plugin integration is invalid if it:

- invokes filesystem or shell actions outside runtime-dispatched tools
- skips permission policy resolution
- skips sandbox/path/shell checks
- suppresses audit logs for denied operations

## Release Gate for Plugin Safety

Before enabling external plugins for a release:

- plugin tools are visible in permission policy with explicit required mode
- plugin tool calls traverse the same runtime dispatch boundary as built-ins
- denied plugin actions produce audit logs
- path and shell policy apply identically to plugin and built-in tools

## Out of Scope for v0.1.0

- signed plugin bundles
- remote plugin execution sandboxes
- per-plugin OS-level isolation
- marketplace trust scoring

These can be layered on after baseline runtime-boundary enforcement remains stable.
