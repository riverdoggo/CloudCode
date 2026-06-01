# Rust Sandbox Implementation Pattern

This document defines a minimal, enforceable sandbox implementation pattern for `v0.1.0` using the existing Rust runtime boundaries.

It is designed to fit the current crates:

- `rust/crates/runtime`
- `rust/crates/tools`
- `rust/crates/claw-cli`

## Existing Enforcement Points

The repository already has the right primary boundary:

- `ConversationRuntime` authorizes each tool call before execution in `runtime/src/conversation.rs`.
- `PermissionPolicy` resolves allow/deny in `runtime/src/permissions.rs`.
- `GlobalToolRegistry` declares per-tool required permissions in `tools/src/lib.rs`.

This should remain the single gate:

```text
Agent loop -> permission policy -> tool executor -> tool implementation
```

No tool should bypass this path.

## Mode Mapping for v0.1.0

Current enum values:

- `ReadOnly`
- `WorkspaceWrite`
- `DangerFullAccess`

Product labels:

- `safe` -> `ReadOnly`
- `workspace` -> `WorkspaceWrite`
- `full-access` -> `DangerFullAccess`

The CLI and config can expose product labels while runtime uses the existing enum.

## Minimal Policy Model

Add a centralized policy object in `runtime/src/sandbox.rs`:

```rust
use std::path::{Path, PathBuf};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxMode {
    Safe,
    Workspace,
    FullAccess,
}

#[derive(Debug, Clone)]
pub struct SandboxPolicy {
    pub mode: SandboxMode,
    pub workspace_root: PathBuf,
    pub allowed_commands: BTreeSet<String>,
    pub require_write_confirmation: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SandboxDecision {
    Allow,
    Deny { reason: String },
    Confirm { reason: String },
}

impl SandboxPolicy {
    pub fn decide_tool(&self, tool_name: &str, raw_input: &str) -> SandboxDecision {
        match self.mode {
            SandboxMode::Safe => self.decide_safe(tool_name),
            SandboxMode::Workspace => self.decide_workspace(tool_name, raw_input),
            SandboxMode::FullAccess => SandboxDecision::Allow,
        }
    }

    fn decide_safe(&self, tool_name: &str) -> SandboxDecision {
        if tool_name.eq_ignore_ascii_case("bash") || tool_name.eq_ignore_ascii_case("powershell") {
            return SandboxDecision::Deny {
                reason: "shell execution is blocked in safe mode".to_string(),
            };
        }
        if matches!(tool_name, "write_file" | "edit_file") {
            return SandboxDecision::Confirm {
                reason: "file modifications require confirmation in safe mode".to_string(),
            };
        }
        SandboxDecision::Allow
    }

    fn decide_workspace(&self, tool_name: &str, raw_input: &str) -> SandboxDecision {
        if tool_name.eq_ignore_ascii_case("bash") || tool_name.eq_ignore_ascii_case("powershell") {
            if !command_is_allowlisted(raw_input, &self.allowed_commands) {
                return SandboxDecision::Deny {
                    reason: "command is not on allowlist for workspace mode".to_string(),
                };
            }
        }
        if self.require_write_confirmation && matches!(tool_name, "write_file" | "edit_file") {
            return SandboxDecision::Confirm {
                reason: "write confirmation required by workspace policy".to_string(),
            };
        }
        SandboxDecision::Allow
    }
}

fn command_is_allowlisted(raw_input: &str, allowed: &BTreeSet<String>) -> bool {
    // Minimal parser for v0.1.0: treat first token as command.
    let cmd = raw_input
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    allowed.contains(&cmd)
}

pub fn path_within_workspace(root: &Path, candidate: &Path) -> bool {
    let Ok(root_canon) = root.canonicalize() else { return false };
    let Ok(path_canon) = candidate.canonicalize() else { return false };
    path_canon.starts_with(root_canon)
}
```

This policy does not replace `PermissionPolicy`; it complements it with mode-specific sandbox semantics (command allowlist, confirmation intents, workspace-path checks).

## Dispatch Integration (Tool Boundary)

Integrate at the same place where permissions are already enforced in `runtime/src/conversation.rs`:

```rust
let permission_outcome = self.permission_policy.authorize(&tool_name, &input, maybe_prompter);
if let PermissionOutcome::Deny { reason } = permission_outcome {
    return deny_tool_result(reason);
}

let sandbox_decision = self.sandbox_policy.decide_tool(&tool_name, &input);
match sandbox_decision {
    SandboxDecision::Allow => { /* execute tool */ }
    SandboxDecision::Deny { reason } => return deny_tool_result(reason),
    SandboxDecision::Confirm { reason } => {
        // Reuse PermissionPrompter UI path for confirmation.
        // If no prompter exists, deny with reason.
    }
}
```

Order matters:

1. permission-mode gate
2. sandbox-policy gate
3. execute tool

## Filesystem Boundary Enforcement

For file tools (`read_file`, `write_file`, `edit_file`) apply workspace-path checks close to implementation entry points:

- reject unresolved/escaped paths
- require normalized absolute paths
- deny if target is outside workspace in `safe`/`workspace`

Pattern:

```rust
let target = workspace_root.join(user_path);
if !path_within_workspace(&workspace_root, &target) {
    return Err("path escapes workspace boundary".to_string());
}
```

This should run even if permission mode is permissive.

## Prompting and Confirmation

Use the existing `PermissionPrompter` path to keep UX unified:

- `Confirm` decision -> prompt user
- allow: continue tool execution
- deny: return `tool_result` with explicit reason

Keep prompt messages deterministic and auditable.

## Audit Logging

For every tool call, log:

- session id
- tool name
- mode
- decision (`allow`, `deny`, `confirm`)
- reason

Start with structured JSON lines. Persist per session.

## Required Tests (Minimum)

Add tests in Rust for:

1. safe mode denies shell
2. safe mode requests confirmation for writes
3. workspace mode denies non-allowlisted shell command
4. workspace mode allows allowlisted command
5. workspace mode blocks path traversal on file writes
6. full-access mode allows permitted tools
7. no-propter confirm path denies with clear reason
8. denied tool call produces deterministic `tool_result`

## Recommended Rollout Sequence

1. Add `SandboxPolicy` and decisions in `runtime/src/sandbox.rs`.
2. Inject policy into `ConversationRuntime`.
3. Enforce at tool-dispatch boundary in `conversation.rs`.
4. Add path checks for file tools.
5. Add command allowlist checks for shell tools.
6. Add/enable tests and wire CI gate.

## v0.1.0 Scope Guard

For first release, keep it minimal:

- static command allowlist
- workspace root restriction
- explicit deny/confirm messages
- no dynamic policy engine

This is enough to make safety behavior real, testable, and release-blocking.
