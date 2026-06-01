# Skills System (v0.2 Design)

Status: Planned (post-`v0.1.0`)  
Goal: Add retrievable project capabilities without increasing prompt bloat.

## Purpose

The skills system provides concise, reusable project guidance (patterns, conventions, workflows) that can be injected into context when relevant.

Skills are not tools and must not change runtime security boundaries.

## Non-Goals for v0.1.0

- No mandatory skills support for release.
- No broad automatic loading of all skills.
- No skill-based bypass of sandbox/permission checks.

## Core Design Rules

1. Skills are **retrieved**, not globally preloaded.
2. Default path loads **zero skills** unless selected.
3. Keep skill payloads short and operational.
4. Inject skills only during context assembly (before model call).
5. Preserve existing runtime dispatch enforcement unchanged.

## Proposed Structure

```text
skills/
  index.md
  frontend-react.md
  backend-rust-api.md
  database-postgres.md
  testing.md
```

`skills/index.md` is a discovery catalog, not a large context block.

## Skill File Format (Minimal)

Each skill file should include:

- skill name
- when to use
- concise guidelines
- common tasks

Example:

```text
Skill: Frontend React Development

Use when changing React UI code.

Guidelines:
- prefer functional components
- keep components focused
- separate UI and state logic
```

## Selection Model

### Manual selection (first milestone)

- user explicitly references a skill (for example: `using frontend-react skill`)
- CLI flag option may be added later (`--skill frontend-react`)

### Optional automatic selection (later)

- lightweight keyword matching against skill metadata
- deterministic ordering
- strict cap on selected skills

## Hard Limits (Required)

- max skills loaded per run: `3`
- max size per skill file loaded into prompt: `~2 KB`
- truncate or reject oversized skills with explicit warning

These limits prevent context explosion and degraded model quality.

## Runtime Integration Point

Skills should be loaded in the prompt/context assembly pipeline:

```text
user request
-> planner/context assembly
-> skill retrieval (optional)
-> prompt build
-> model call
```

Skills are not runtime execution tools and do not participate in tool dispatch.

## Security and Boundary Guarantees

Skills must never:

- invoke tools directly
- modify filesystem/shell policy
- bypass permission or sandbox checks

All execution continues through runtime enforcement:

1. permission authorization
2. sandbox decision
3. workspace path validation
4. shell allowlist enforcement
5. tool execution

## Error Handling Requirements

- unknown skill name: return clear warning and continue without skill
- oversized skill file: skip or truncate with explicit message
- malformed `skills/index.md`: non-fatal, fall back to manual path only

## Testing Requirements (v0.2)

- no-skill baseline path unchanged
- manual skill selection loads expected skill
- skill limit cap (`3`) enforced
- per-skill size limit enforced
- deterministic selection order
- unknown skill handling is non-fatal

## Rollout Plan

1. Add file format and index parser.
2. Add manual skill selection only.
3. Add limits and warnings.
4. Add tests.
5. Optionally add auto-selection heuristics.

## Success Criteria

- improved consistency on repeated domain tasks
- no measurable prompt bloat in default path
- no regressions in runtime safety model
- no degradation in golden-path reliability
