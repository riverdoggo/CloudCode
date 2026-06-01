use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use runtime::{
    ApiClient, ApiRequest, AssistantEvent, ContentBlock, ConversationRuntime, PermissionMode,
    PermissionPolicy, PermissionPromptDecision, PermissionPrompter, PermissionRequest, RuntimeError,
    Session, StaticToolExecutor,
};
use serde_json::json;

struct SingleToolApiClient {
    calls: usize,
    tool_name: String,
    input: String,
}

impl SingleToolApiClient {
    fn new(tool_name: impl Into<String>, input: impl Into<String>) -> Self {
        Self {
            calls: 0,
            tool_name: tool_name.into(),
            input: input.into(),
        }
    }
}

impl ApiClient for SingleToolApiClient {
    fn stream(&mut self, _request: ApiRequest) -> Result<Vec<AssistantEvent>, RuntimeError> {
        self.calls += 1;
        match self.calls {
            1 => Ok(vec![
                AssistantEvent::ToolUse {
                    id: "tool-1".to_string(),
                    name: self.tool_name.clone(),
                    input: self.input.clone(),
                },
                AssistantEvent::MessageStop,
            ]),
            2 => Ok(vec![
                AssistantEvent::TextDelta("done".to_string()),
                AssistantEvent::MessageStop,
            ]),
            _ => Err(RuntimeError::new("unexpected extra API call")),
        }
    }
}

struct AllowPrompter;

impl PermissionPrompter for AllowPrompter {
    fn decide(&mut self, _request: &PermissionRequest) -> PermissionPromptDecision {
        PermissionPromptDecision::Allow
    }
}

#[derive(Clone, Copy)]
enum ToolExecutorMode {
    Proposal,
    Conflict,
    Bypass,
    PanicOnExecute,
}

struct GoldenScenario {
    name: &'static str,
    tool_name: &'static str,
    input: String,
    prompt: &'static str,
    executor_mode: ToolExecutorMode,
    require_confirmation: bool,
    expected_error: bool,
    expected_status: &'static str,
    expected_error_code: Option<&'static str>,
    expected_contains: &'static str,
    initial_file: Option<(&'static str, &'static str)>,
    expected_file: Option<(&'static str, &'static str)>,
}

const STATUS_OK: &str = "ok";
const STATUS_PATCH_APPLIED: &str = "patch_applied";
const STATUS_PATCH_CONFLICT: &str = "patch_conflict";
const STATUS_PATH_DENIED: &str = "path_denied";
const STATUS_MUTATION_BYPASS: &str = "mutation_bypass_detected";

#[test]
fn golden_path_scenarios() {
    let mut scenarios = Vec::new();
    scenarios.push(GoldenScenario {
        name: "patch_apply_flow",
        tool_name: "write_file",
        input: r#"{"path":"src/auth.rs","content":"fn auth() { return true; }\n"}"#.to_string(),
        prompt: "fix bug in src/auth.rs",
        executor_mode: ToolExecutorMode::Proposal,
        require_confirmation: true,
        expected_error: false,
        expected_status: STATUS_PATCH_APPLIED,
        expected_error_code: None,
        expected_contains: "Patch applied",
        initial_file: Some(("src/auth.rs", "fn auth() { return false; }\n")),
        expected_file: Some(("src/auth.rs", "fn auth() { return true; }\n")),
    });
    scenarios.push(GoldenScenario {
        name: "patch_conflict",
        tool_name: "write_file",
        input: r#"{"path":"src/auth.rs","content":"fn auth() { return true; }\n"}"#.to_string(),
        prompt: "fix bug in src/auth.rs",
        executor_mode: ToolExecutorMode::Conflict,
        require_confirmation: true,
        expected_error: true,
        expected_status: STATUS_PATCH_CONFLICT,
        expected_error_code: Some("patch_conflict"),
        expected_contains: "\"code\":\"patch_conflict\"",
        initial_file: Some(("src/auth.rs", "fn auth() { return false; }\n")),
        expected_file: None,
    });
    scenarios.push(GoldenScenario {
        name: "sandbox_path_denial",
        tool_name: "write_file",
        input: if cfg!(windows) {
            r#"{"path":"C:\\Windows\\System32\\drivers\\etc\\hosts","content":"blocked"}"#
                .to_string()
        } else {
            r#"{"path":"/etc/hosts","content":"blocked"}"#.to_string()
        },
        prompt: "attempt external write",
        executor_mode: ToolExecutorMode::PanicOnExecute,
        require_confirmation: false,
        expected_error: true,
        expected_status: STATUS_PATH_DENIED,
        expected_error_code: Some("path_denied"),
        expected_contains: "\"code\":\"path_denied\"",
        initial_file: None,
        expected_file: None,
    });
    scenarios.push(GoldenScenario {
        name: "mutation_bypass",
        tool_name: "write_file",
        input: r#"{"path":"src/auth.rs","content":"new"}"#.to_string(),
        prompt: "write file",
        executor_mode: ToolExecutorMode::Bypass,
        require_confirmation: true,
        expected_error: true,
        expected_status: STATUS_MUTATION_BYPASS,
        expected_error_code: Some("mutation_bypass_detected"),
        expected_contains: "\"code\":\"mutation_bypass_detected\"",
        initial_file: Some(("src/auth.rs", "old")),
        expected_file: None,
    });

    for scenario in scenarios {
        run_scenario(scenario);
    }
}

fn patch_proposal_executor(
    workspace_root: &std::path::Path,
    mode: ToolExecutorMode,
) -> StaticToolExecutor {
    let workspace_root = workspace_root.to_path_buf();
    StaticToolExecutor::new().register("write_file", move |input| {
        if matches!(mode, ToolExecutorMode::PanicOnExecute) {
            panic!("executor should not run when sandbox denies");
        }
        if matches!(mode, ToolExecutorMode::Bypass) {
            return Ok(r#"{"type":"update"}"#.to_string());
        }
        let value: serde_json::Value =
            serde_json::from_str(input).map_err(|error| runtime::ToolError::new(error.to_string()))?;
        let path = value
            .get("path")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| runtime::ToolError::new("missing path"))?;
        let content = value
            .get("content")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| runtime::ToolError::new("missing content"))?;
        let absolute = workspace_root.join(path);
        let original = fs::read_to_string(&absolute).unwrap_or_default();
        let proposal_original = if matches!(mode, ToolExecutorMode::Conflict) {
            "stale".to_string()
        } else {
            original.clone()
        };
        Ok(json!({
            "type": "patch_proposal",
            "operation": "write_file",
            "filePath": path,
            "original": proposal_original,
            "modified": content,
            "structuredPatch": [
                {
                    "oldStart": 1,
                    "oldLines": 1,
                    "newStart": 1,
                    "newLines": 1,
                    "lines": [
                        format!("-{}", original.lines().next().unwrap_or_default()),
                        format!("+{}", content.lines().next().unwrap_or_default())
                    ]
                }
            ]
        })
        .to_string())
    })
}

fn run_scenario(scenario: GoldenScenario) {
    let workspace = unique_temp_path(scenario.name);
    fs::create_dir_all(workspace.join("src")).expect("create workspace");
    if let Some((path, content)) = scenario.initial_file {
        let absolute = workspace.join(path);
        fs::create_dir_all(absolute.parent().expect("parent")).expect("mkdirs");
        fs::write(absolute, content).expect("seed file");
    }

    let mut runtime = ConversationRuntime::new(
        Session::new(),
        SingleToolApiClient::new(scenario.tool_name, scenario.input),
        patch_proposal_executor(&workspace, scenario.executor_mode),
        PermissionPolicy::new(PermissionMode::WorkspaceWrite)
            .with_tool_requirement(scenario.tool_name, PermissionMode::WorkspaceWrite),
        vec!["system".to_string()],
    )
    .with_workspace_root(&workspace);

    let summary = if scenario.require_confirmation {
        runtime
            .run_turn(scenario.prompt, Some(&mut AllowPrompter))
            .expect("turn succeeds")
    } else {
        runtime.run_turn(scenario.prompt, None).expect("turn succeeds")
    };
    let (output, is_error) = first_tool_result(&summary);
    assert_eq!(
        is_error, scenario.expected_error,
        "scenario '{}' error mismatch",
        scenario.name
    );
    assert!(
        output.contains(scenario.expected_contains),
        "scenario '{}' missing output fragment '{}': {output}",
        scenario.name,
        scenario.expected_contains
    );
    let status = extract_status(&output);
    assert_eq!(
        status, scenario.expected_status,
        "scenario '{}' status mismatch",
        scenario.name
    );
    if let Some(code) = scenario.expected_error_code {
        assert!(
            output.contains(&format!("\"code\":\"{code}\"")),
            "scenario '{}' missing error code '{}': {output}",
            scenario.name,
            code
        );
    }

    if let Some((path, expected)) = scenario.expected_file {
        let content = fs::read_to_string(workspace.join(path)).expect("read expected file");
        assert_eq!(content, expected, "scenario '{}': file mismatch", scenario.name);
    }
}

fn extract_status(output: &str) -> &'static str {
    if output.contains("Patch applied:") {
        STATUS_PATCH_APPLIED
    } else if output.contains("\"code\":\"patch_conflict\"") {
        STATUS_PATCH_CONFLICT
    } else if output.contains("\"code\":\"path_denied\"") {
        STATUS_PATH_DENIED
    } else if output.contains("\"code\":\"mutation_bypass_detected\"") {
        STATUS_MUTATION_BYPASS
    } else {
        STATUS_OK
    }
}

fn first_tool_result(summary: &runtime::TurnSummary) -> (String, bool) {
    let block = summary
        .tool_results
        .first()
        .expect("tool result should exist")
        .blocks
        .first()
        .expect("content block");
    let ContentBlock::ToolResult {
        output, is_error, ..
    } = block
    else {
        panic!("expected tool result block");
    };
    (output.clone(), *is_error)
}

fn unique_temp_path(prefix: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    std::env::temp_dir().join(format!("{prefix}-{stamp}"))
}
