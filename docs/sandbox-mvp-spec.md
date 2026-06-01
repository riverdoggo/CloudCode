# Sandbox MVP Spec (v0.1.0)

Status: Implemented (Phases 1-4 complete)  
Target Release: v0.1.0

This is the minimum enforceable sandbox behavior required for first release.

## Permission Modes

- `safe`
- `workspace`
- `full-access`

## Effective Runtime Mapping

- `safe` maps to runtime `ReadOnly`
- `workspace` maps to runtime `WorkspaceWrite`
- `full-access` maps to runtime `DangerFullAccess` / `Allow`

## Mode Behavior Summary

| Mode | File Access | Shell Access | Confirmation |
| --- | --- | --- | --- |
| ReadOnly (`safe`) | read-only within workspace | denied | write operations require confirmation |
| WorkspaceWrite (`workspace`) | read/write within workspace | allowlisted commands only | destructive operations may prompt |
| DangerFullAccess / Allow (`full-access`) | unrestricted | unrestricted | optional confirmation depending on tool |

Workspace shell allowlist in `WorkspaceWrite` mode:

- `git`
- `ls`
- `cat`
- `grep`

## Runtime Enforcement Chain

Tool execution chain is centralized in runtime dispatch:

1. Permission authorization
2. Sandbox decision evaluation
3. Workspace path boundary validation
4. Shell command allowlist enforcement
5. Tool execution
6. Patch proposal gate for mutation tools
7. Diff preview rendering
8. Confirmation prompt
9. Patch apply

## Filesystem Boundary Rules

- Filesystem tools are restricted to workspace root boundaries.
- Path traversal attempts (for example `../`) are rejected.
- Symlink escapes are prevented by canonical path resolution checks.
- New file writes are validated using the canonical parent directory.
- Any path outside workspace is denied before tool execution.

Workspace boundary enforcement ensures all filesystem operations remain inside workspace root. Attempts to access paths outside workspace produce sandbox denial.

## Shell Execution Policy

ReadOnly:

- Shell tools disabled.

WorkspaceWrite:

- Shell tools are enabled only for allowlisted base commands.

DangerFullAccess / Allow:

- Shell tools are unrestricted.

The validator checks the base executable from the command string:

- `git status` -> allowed in `WorkspaceWrite`
- `curl https://example.com` -> denied in `WorkspaceWrite`

## Runtime Patch Gate (Phase 4)

All filesystem mutations are mediated by the runtime patch gate.

Mutation tools (`write_file`, `edit_file`, `delete_file`) do not modify files directly. Instead they return a patch proposal which is reviewed and applied by the runtime.

Execution flow:

```text
tool execution
-> patch proposal returned
-> diff preview rendered
-> confirmation prompt
-> patch application
```

### Patch Proposal Shape

```json
{
  "type": "patch_proposal",
  "operation": "write_file",
  "filePath": "src/auth.rs",
  "original": "...",
  "modified": "...",
  "structuredPatch": []
}
```

Proposals describe intended changes only. The runtime validates and applies proposals; tools cannot bypass this mechanism.

### Conflict Protection

Before applying a patch, the runtime verifies that current file content still matches the proposal `original` content. If the file changed since proposal generation, patch apply is rejected with a conflict error.

## Security Rationale

The sandbox policy exists to prevent accidental or malicious destructive actions while preserving useful coding workflows. Restricting filesystem operations to workspace and restricting shell commands by mode reduces risk of unintended system changes.

## Policy Requirements

### safe

- Deny shell execution.
- Deny external network access.
- Require confirmation before any file write.
- Restrict writes to workspace only after explicit approval.

### workspace

- Allow shell execution only for allowlisted commands.
- Allow file reads/writes only within workspace root.
- Require confirmation for destructive commands.

### full-access

- Allow full tool execution for trusted local use.
- Keep tool invocation audit logs enabled.

## Enforcement Points

- Check permission mode before tool dispatch.
- Resolve and normalize target paths before policy checks.
- Deny on policy mismatch with explicit error messages.
- Require user approval before patch apply in constrained modes.

## Audit Requirements

- Log tool name, arguments, mode, allow/deny decision, and reason.
- Persist logs per session for troubleshooting.
