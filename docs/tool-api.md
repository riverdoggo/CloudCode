# Tool API

Tools are exposed to the model as structured capabilities with typed parameters.

## Minimum Tool Set

- `read_file`
- `write_file`
- `search_repo`
- `run_command`
- `git_diff`

## Example Schema

```json
{
  "name": "read_file",
  "description": "Read a file from the workspace",
  "parameters": {
    "path": "string"
  }
}
```

## Tool Contracts

- Inputs must be validated before execution.
- Outputs must be deterministic and structured.
- Errors must include actionable messages.
- `write_file` must produce and apply patch-style diffs, not direct overwrite.

## Execution Policy

- Tool calls run through sandbox permission checks.
- Disallowed operations return explicit policy errors.
- Long-running commands must enforce timeout limits.
