# Golden Path E2E Spec

This document defines the primary end-to-end workflow that must be stable before `v0.1.0`.

## User Scenario

```text
agent
> fix bug in src/auth.rs
```

or:

```text
agent fix bug in src/auth.rs
```

## Required System Behavior

1. Parse the task and identify candidate files.
2. Build bounded context from repository content.
3. Send prompt to configured model provider.
4. Receive tool calls from model response.
5. Run centralized sandbox enforcement before any tool execution:
   - permission authorization
   - sandbox decision evaluation
   - workspace path boundary validation
   - shell allowlist validation (for shell tools)
6. Execute allowed tools.
7. For mutation tools, require patch proposal output (`type=patch_proposal`) rather than direct file mutation.
8. Run runtime patch gate:
   - validate patch proposal payload
   - render diff preview
   - request confirmation
   - apply patch through runtime path
9. Report changed files and patch status.
10. Optionally run tests and summarize results.

## CLI Example

```text
Proposed change: src/auth.rs

--- src/auth.rs
+++ src/auth.rs
- let token = ""
+ let token = generate_token()

Apply this patch? (y/N)
```

## Acceptance Criteria

- The flow completes without manual file editing.
- Every write operation is represented as a diff.
- Unsafe tool actions are blocked by permission policy.
- Errors are actionable (provider, tool, patch, or test failure).
- Final output includes summary of edits and verification steps.

## Non-Goals for v0.1.0

- Advanced autonomous planning loops.
- Multi-repo orchestration.
- Long-lived background job schedulers.
