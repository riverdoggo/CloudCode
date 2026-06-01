# v0.1.0 Test Matrix

This matrix tracks minimum validation for release readiness.

## Execution Modes

| Area | Scenario | Expected Result | Status |
| --- | --- | --- | --- |
| CLI | `agent` interactive startup | REPL starts with no crash | TODO |
| CLI | `agent fix <task>` | Executes full loop with diff preview | TODO |
| CLI | `agent explain <file>` | Returns contextual explanation | TODO |
| CLI | `agent test` | Runs configured test command | TODO |

## Permission and Sandbox

| Mode | Scenario | Expected Result | Status |
| --- | --- | --- | --- |
| safe | run shell tool | Blocked with policy message | TODO |
| safe | external network tool | Blocked with policy message | TODO |
| safe | file write | Requires explicit confirmation | TODO |
| workspace | write inside workspace | Allowed after policy checks | TODO |
| workspace | write outside workspace | Denied | TODO |
| full-access | shell command | Allowed with audit logging | TODO |

## Shell Allowlist

| Mode | Command | Expected Result | Status |
| --- | --- | --- | --- |
| workspace | `git status` | Allowed | TODO |
| workspace | `curl https://example.com` | Sandbox denial | TODO |
| safe | `git status` | Denied before allowlist execution | TODO |
| full-access | `bash build.sh` | Allowed (allowlist bypass) | TODO |

## Patch Gate Validation

| Scenario | Mode | Expected Result | Status |
| --- | --- | --- | --- |
| Patch proposal preview rendered | WorkspaceWrite | Diff shown | TODO |
| Patch rejected without confirmation | WorkspaceWrite | No file change | TODO |
| Patch applied after confirmation | WorkspaceWrite | File modified | TODO |
| Patch apply denied | ReadOnly | Rejected | TODO |
| Patch conflict detected | WorkspaceWrite | Apply rejected | TODO |

## Provider Compatibility

| Provider | Scenario | Expected Result | Status |
| --- | --- | --- | --- |
| OpenAI | complete golden path | Success with patch output | TODO |
| Anthropic | complete golden path | Success with patch output | TODO |
| Ollama (local) | complete golden path | Success with patch output | TODO |
| Groq | prompt + tool call roundtrip | Success | TODO |
| OpenRouter | prompt + tool call roundtrip | Success | TODO |

## Failure Handling

| Failure Type | Scenario | Expected Result | Status |
| --- | --- | --- | --- |
| Provider auth | invalid key | Clear auth error and recovery hint | TODO |
| Provider timeout | delayed response | Timeout surfaced with retry guidance | TODO |
| Tool denial | disallowed command | Policy denial shown, task continues/fails safely | TODO |
| Patch conflict | apply against changed file | Conflict reported with retry path | TODO |
| Test failure | post-edit test run fails | Failure summarized with command output | TODO |

## Context Quality

| Scenario | Expected Result | Status |
| --- | --- | --- |
| Large repo query | Context remains within token budget | TODO |
| Irrelevant files nearby | Retrieval prioritizes likely files/symbols | TODO |
| Repeated turns | Context updates incrementally without duplication blowup | TODO |
