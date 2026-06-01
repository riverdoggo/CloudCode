use std::collections::BTreeMap;
use std::fmt::{Display, Formatter};
use std::path::PathBuf;

use crate::compact::{
    compact_session, estimate_session_tokens, CompactionConfig, CompactionResult,
};
use crate::config::RuntimeFeatureConfig;
use crate::hooks::{HookRunResult, HookRunner};
use crate::permissions::{
    PermissionMode, PermissionOutcome, PermissionPolicy, PermissionPromptDecision,
    PermissionPrompter, PermissionRequest,
};
use crate::file_ops::{apply_patch_proposal, PatchProposal};
use crate::sandbox::{
    validate_shell_command, validate_tool_paths_for_input, SandboxDecision, ToolSandboxPolicy,
};
use crate::session::{ContentBlock, ConversationMessage, Session};
use crate::usage::{TokenUsage, UsageTracker};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiRequest {
    pub system_prompt: Vec<String>,
    pub messages: Vec<ConversationMessage>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssistantEvent {
    TextDelta(String),
    ToolUse {
        id: String,
        name: String,
        input: String,
    },
    Usage(TokenUsage),
    MessageStop,
}

pub trait ApiClient {
    fn stream(&mut self, request: ApiRequest) -> Result<Vec<AssistantEvent>, RuntimeError>;
}

pub trait ToolExecutor {
    fn execute(&mut self, tool_name: &str, input: &str) -> Result<String, ToolError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolError {
    message: String,
}

impl ToolError {
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl Display for ToolError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for ToolError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeError {
    message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeErrorCode {
    PermissionDenied,
    PathDenied,
    AllowlistDenied,
    PatchConflict,
    MutationBypassDetected,
}

impl RuntimeErrorCode {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PermissionDenied => "permission_denied",
            Self::PathDenied => "path_denied",
            Self::AllowlistDenied => "allowlist_denied",
            Self::PatchConflict => "patch_conflict",
            Self::MutationBypassDetected => "mutation_bypass_detected",
        }
    }
}

impl RuntimeError {
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl Display for RuntimeError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for RuntimeError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnSummary {
    pub assistant_messages: Vec<ConversationMessage>,
    pub tool_results: Vec<ConversationMessage>,
    pub iterations: usize,
    pub usage: TokenUsage,
}

pub struct ConversationRuntime<C, T> {
    session: Session,
    api_client: C,
    tool_executor: T,
    permission_policy: PermissionPolicy,
    sandbox_policy: ToolSandboxPolicy,
    workspace_root: PathBuf,
    system_prompt: Vec<String>,
    max_iterations: usize,
    usage_tracker: UsageTracker,
    hook_runner: HookRunner,
}

impl<C, T> ConversationRuntime<C, T>
where
    C: ApiClient,
    T: ToolExecutor,
{
    #[must_use]
    pub fn new(
        session: Session,
        api_client: C,
        tool_executor: T,
        permission_policy: PermissionPolicy,
        system_prompt: Vec<String>,
    ) -> Self {
        Self::new_with_features(
            session,
            api_client,
            tool_executor,
            permission_policy,
            system_prompt,
            RuntimeFeatureConfig::default(),
        )
    }

    #[must_use]
    pub fn new_with_features(
        session: Session,
        api_client: C,
        tool_executor: T,
        permission_policy: PermissionPolicy,
        system_prompt: Vec<String>,
        feature_config: RuntimeFeatureConfig,
    ) -> Self {
        let usage_tracker = UsageTracker::from_session(&session);
        let sandbox_policy = ToolSandboxPolicy::from_permission_mode(permission_policy.active_mode());
        let workspace_root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        Self {
            session,
            api_client,
            tool_executor,
            permission_policy,
            sandbox_policy,
            workspace_root,
            system_prompt,
            max_iterations: usize::MAX,
            usage_tracker,
            hook_runner: HookRunner::from_feature_config(&feature_config),
        }
    }

    #[must_use]
    pub fn with_max_iterations(mut self, max_iterations: usize) -> Self {
        self.max_iterations = max_iterations;
        self
    }

    #[must_use]
    pub fn with_workspace_root(mut self, workspace_root: impl Into<PathBuf>) -> Self {
        self.workspace_root = workspace_root.into();
        self
    }

    pub fn run_turn(
        &mut self,
        user_input: impl Into<String>,
        mut prompter: Option<&mut dyn PermissionPrompter>,
    ) -> Result<TurnSummary, RuntimeError> {
        self.session
            .messages
            .push(ConversationMessage::user_text(user_input.into()));

        let mut assistant_messages = Vec::new();
        let mut tool_results = Vec::new();
        let mut iterations = 0;

        loop {
            iterations += 1;
            if iterations > self.max_iterations {
                return Err(RuntimeError::new(
                    "conversation loop exceeded the maximum number of iterations",
                ));
            }

            let request = ApiRequest {
                system_prompt: self.system_prompt.clone(),
                messages: self.session.messages.clone(),
            };
            let events = self.api_client.stream(request)?;
            let (assistant_message, usage) = build_assistant_message(events)?;
            if let Some(usage) = usage {
                self.usage_tracker.record(usage);
            }
            let pending_tool_uses = assistant_message
                .blocks
                .iter()
                .filter_map(|block| match block {
                    ContentBlock::ToolUse { id, name, input } => {
                        Some((id.clone(), name.clone(), input.clone()))
                    }
                    _ => None,
                })
                .collect::<Vec<_>>();

            self.session.messages.push(assistant_message.clone());
            assistant_messages.push(assistant_message);

            if pending_tool_uses.is_empty() {
                break;
            }

            for (tool_use_id, tool_name, input) in pending_tool_uses {
                let permission_outcome =
                    self.authorize_tool_call(&tool_name, &input, &mut prompter);

                let result_message = match permission_outcome {
                    PermissionOutcome::Allow => {
                        if let Err(reason) = self.resolve_sandbox_decision(
                            &tool_name,
                            &input,
                            &mut prompter,
                        ) {
                            ConversationMessage::tool_result(
                                tool_use_id,
                                tool_name,
                                runtime_error_payload(
                                    RuntimeErrorCode::PermissionDenied,
                                    &reason,
                                    None,
                                ),
                                true,
                            )
                        } else if let Err(reason) =
                            self.validate_workspace_paths_for_tool(&tool_name, &input)
                        {
                            eprintln!(
                                "Sandbox: denied path outside workspace; tool={tool_name} reason={reason}"
                            );
                            ConversationMessage::tool_result(
                                tool_use_id,
                                tool_name,
                                runtime_error_payload(
                                    RuntimeErrorCode::PathDenied,
                                    &reason,
                                    None,
                                ),
                                true,
                            )
                        } else if let Err(reason) =
                            self.validate_shell_command_for_tool(&tool_name, &input)
                        {
                            let command = extract_command_for_audit(&input);
                            eprintln!(
                                "Sandbox: denied shell command; tool={tool_name} command={command} mode={} reason={reason}",
                                self.permission_policy.active_mode().as_str()
                            );
                            ConversationMessage::tool_result(
                                tool_use_id,
                                tool_name,
                                runtime_error_payload(
                                    RuntimeErrorCode::AllowlistDenied,
                                    &reason,
                                    None,
                                ),
                                true,
                            )
                        } else {
                            let pre_hook_result =
                                self.hook_runner.run_pre_tool_use(&tool_name, &input);
                            if pre_hook_result.is_denied() {
                                let deny_message =
                                    format!("PreToolUse hook denied tool `{tool_name}`");
                                ConversationMessage::tool_result(
                                    tool_use_id,
                                    tool_name,
                                    format_hook_message(&pre_hook_result, &deny_message),
                                    true,
                                )
                            } else {
                                let (mut output, mut is_error) =
                                    match self.tool_executor.execute(&tool_name, &input) {
                                        Ok(output) => (output, false),
                                        Err(error) => (error.to_string(), true),
                                    };
                                if let Some((gated_output, gated_error)) = self
                                    .handle_patch_proposal_gate(&tool_name, &output, &mut prompter)
                                {
                                    output = gated_output;
                                    is_error = gated_error;
                                }
                                output =
                                    merge_hook_feedback(pre_hook_result.messages(), output, false);

                                let post_hook_result = self
                                    .hook_runner
                                    .run_post_tool_use(&tool_name, &input, &output, is_error);
                                if post_hook_result.is_denied() {
                                    is_error = true;
                                }
                                output = merge_hook_feedback(
                                    post_hook_result.messages(),
                                    output,
                                    post_hook_result.is_denied(),
                                );

                                ConversationMessage::tool_result(
                                    tool_use_id,
                                    tool_name,
                                    output,
                                    is_error,
                                )
                            }
                        }
                    }
                    PermissionOutcome::Deny { reason } => {
                        ConversationMessage::tool_result(
                            tool_use_id,
                            tool_name,
                            runtime_error_payload(RuntimeErrorCode::PermissionDenied, &reason, None),
                            true,
                        )
                    }
                };
                self.session.messages.push(result_message.clone());
                tool_results.push(result_message);
            }
        }

        Ok(TurnSummary {
            assistant_messages,
            tool_results,
            iterations,
            usage: self.usage_tracker.cumulative_usage(),
        })
    }

    #[must_use]
    pub fn compact(&self, config: CompactionConfig) -> CompactionResult {
        compact_session(&self.session, config)
    }

    #[must_use]
    pub fn estimated_tokens(&self) -> usize {
        estimate_session_tokens(&self.session)
    }

    #[must_use]
    pub fn usage(&self) -> &UsageTracker {
        &self.usage_tracker
    }

    #[must_use]
    pub fn session(&self) -> &Session {
        &self.session
    }

    #[must_use]
    pub fn into_session(self) -> Session {
        self.session
    }

    fn resolve_sandbox_decision(
        &mut self,
        tool_name: &str,
        input: &str,
        prompter: &mut Option<&mut dyn PermissionPrompter>,
    ) -> Result<(), String> {
        match self.sandbox_policy.evaluate_tool(tool_name) {
            SandboxDecision::Allow => Ok(()),
            SandboxDecision::Deny { reason } => Err(reason),
            SandboxDecision::RequireConfirmation { reason } => {
                let request = PermissionRequest {
                    tool_name: tool_name.to_string(),
                    input: input.to_string(),
                    current_mode: self.permission_policy.active_mode(),
                    required_mode: self.required_confirmation_mode(tool_name),
                };
                match prompter.as_mut() {
                    Some(prompt) => match prompt.decide(&request) {
                        PermissionPromptDecision::Allow => Ok(()),
                        PermissionPromptDecision::Deny { reason } => Err(reason),
                    },
                    None => Err(reason),
                }
            }
        }
    }

    fn authorize_tool_call(
        &self,
        tool_name: &str,
        input: &str,
        prompter: &mut Option<&mut dyn PermissionPrompter>,
    ) -> PermissionOutcome {
        if let Some(prompt) = prompter.as_mut() {
            self.permission_policy
                .authorize(tool_name, input, Some(*prompt))
        } else {
            self.permission_policy.authorize(tool_name, input, None)
        }
    }

    fn required_confirmation_mode(&self, tool_name: &str) -> PermissionMode {
        if matches!(
            tool_name.trim().to_ascii_lowercase().as_str(),
            "bash" | "powershell" | "run_command"
        ) {
            PermissionMode::DangerFullAccess
        } else {
            PermissionMode::WorkspaceWrite
        }
    }

    fn validate_workspace_paths_for_tool(
        &self,
        tool_name: &str,
        input: &str,
    ) -> Result<(), String> {
        validate_tool_paths_for_input(&self.workspace_root, tool_name, input)
    }

    fn validate_shell_command_for_tool(&self, tool_name: &str, input: &str) -> Result<(), String> {
        validate_shell_command(self.permission_policy.active_mode(), tool_name, input)
    }

    fn handle_patch_proposal_gate(
        &self,
        tool_name: &str,
        output: &str,
        prompter: &mut Option<&mut dyn PermissionPrompter>,
    ) -> Option<(String, bool)> {
        if !matches!(tool_name, "write_file" | "edit_file" | "delete" | "delete_file") {
            return None;
        }
        let proposal: PatchProposal = match serde_json::from_str(output) {
            Ok(parsed) => parsed,
            Err(_) => {
                eprintln!(
                    "Runtime: mutation bypass detected tool={} mode={}",
                    tool_name,
                    self.permission_policy.active_mode().as_str()
                );
                return Some((
                    runtime_error_payload(
                        RuntimeErrorCode::MutationBypassDetected,
                        "tool returned mutation result without patch_proposal",
                        Some(tool_name),
                    ),
                    true,
                ))
            }
        };
        if proposal.kind != "patch_proposal" {
            eprintln!(
                "Runtime: mutation bypass detected tool={} mode={}",
                tool_name,
                self.permission_policy.active_mode().as_str()
            );
            return Some((
                runtime_error_payload(
                    RuntimeErrorCode::MutationBypassDetected,
                    "tool returned mutation result without patch_proposal",
                    Some(tool_name),
                ),
                true,
            ));
        }

        if self.permission_policy.active_mode() == PermissionMode::ReadOnly {
            return Some((
                runtime_error_payload(
                    RuntimeErrorCode::PermissionDenied,
                    "patch rejected: read-only mode cannot apply edits",
                    Some(tool_name),
                ),
                true,
            ));
        }

        let preview = render_patch_preview(&proposal);
        match self.approve_patch(proposal.file_path.as_str(), &preview, prompter) {
            Ok(true) => {}
            Ok(false) => {
                return Some((
                    runtime_error_payload(
                        RuntimeErrorCode::PermissionDenied,
                        "patch rejected by confirmation gate",
                        Some(tool_name),
                    ),
                    true,
                ))
            }
            Err(error) => {
                return Some((
                    runtime_error_payload(
                        RuntimeErrorCode::PermissionDenied,
                        &error,
                        Some(tool_name),
                    ),
                    true,
                ))
            }
        }

        match apply_patch_proposal(&self.workspace_root, &proposal) {
            Ok(result) => {
                eprintln!(
                    "Patch applied file={} lines_changed={} mode={}",
                    result.file_path,
                    result.lines_changed,
                    self.permission_policy.active_mode().as_str()
                );
                Some((
                    format!(
                        "{}\n\nPatch applied: {} (lines_changed={})",
                        preview, result.file_path, result.lines_changed
                    ),
                    false,
                ))
            }
            Err(error) => Some((
                format!(
                    "{}\n\n{}",
                    preview,
                    runtime_error_payload(
                        RuntimeErrorCode::PatchConflict,
                        &format!("patch apply failed: {error}"),
                        Some(tool_name),
                    )
                ),
                true,
            )),
        }
    }

    fn approve_patch(
        &self,
        file_path: &str,
        preview: &str,
        prompter: &mut Option<&mut dyn PermissionPrompter>,
    ) -> Result<bool, String> {
        let request = PermissionRequest {
            tool_name: "apply_patch".to_string(),
            input: format!("file={file_path}\n{preview}"),
            current_mode: self.permission_policy.active_mode(),
            required_mode: PermissionMode::WorkspaceWrite,
        };
        match prompter.as_mut() {
            Some(prompt) => match prompt.decide(&request) {
                PermissionPromptDecision::Allow => Ok(true),
                PermissionPromptDecision::Deny { reason: _ } => Ok(false),
            },
            None => Err("patch apply requires interactive confirmation".to_string()),
        }
    }
}

fn build_assistant_message(
    events: Vec<AssistantEvent>,
) -> Result<(ConversationMessage, Option<TokenUsage>), RuntimeError> {
    let mut text = String::new();
    let mut blocks = Vec::new();
    let mut finished = false;
    let mut usage = None;

    for event in events {
        match event {
            AssistantEvent::TextDelta(delta) => text.push_str(&delta),
            AssistantEvent::ToolUse { id, name, input } => {
                flush_text_block(&mut text, &mut blocks);
                blocks.push(ContentBlock::ToolUse { id, name, input });
            }
            AssistantEvent::Usage(value) => usage = Some(value),
            AssistantEvent::MessageStop => {
                finished = true;
            }
        }
    }

    flush_text_block(&mut text, &mut blocks);

    if !finished {
        return Err(RuntimeError::new(
            "assistant stream ended without a message stop event",
        ));
    }
    if blocks.is_empty() {
        return Err(RuntimeError::new("assistant stream produced no content"));
    }

    Ok((
        ConversationMessage::assistant_with_usage(blocks, usage),
        usage,
    ))
}

fn flush_text_block(text: &mut String, blocks: &mut Vec<ContentBlock>) {
    if !text.is_empty() {
        blocks.push(ContentBlock::Text {
            text: std::mem::take(text),
        });
    }
}

fn format_hook_message(result: &HookRunResult, fallback: &str) -> String {
    if result.messages().is_empty() {
        fallback.to_string()
    } else {
        result.messages().join("\n")
    }
}

fn merge_hook_feedback(messages: &[String], output: String, denied: bool) -> String {
    if messages.is_empty() {
        return output;
    }

    let mut sections = Vec::new();
    if !output.trim().is_empty() {
        sections.push(output);
    }
    let label = if denied {
        "Hook feedback (denied)"
    } else {
        "Hook feedback"
    };
    sections.push(format!("{label}:\n{}", messages.join("\n")));
    sections.join("\n\n")
}

fn extract_command_for_audit(input: &str) -> String {
    serde_json::from_str::<serde_json::Value>(input)
        .ok()
        .and_then(|value| value.get("command").and_then(serde_json::Value::as_str).map(str::to_string))
        .unwrap_or_else(|| "<unavailable>".to_string())
}

fn render_patch_preview(proposal: &PatchProposal) -> String {
    let mut lines = vec![
        format!("--- {}", proposal.file_path),
        format!("+++ {}", proposal.file_path),
        String::new(),
    ];
    for hunk in &proposal.structured_patch {
        for line in &hunk.lines {
            lines.push(line.clone());
        }
    }
    lines.join("\n")
}

fn runtime_error_payload(
    code: RuntimeErrorCode,
    message: &str,
    tool_name: Option<&str>,
) -> String {
    let mut payload = serde_json::Map::new();
    payload.insert(
        "type".to_string(),
        serde_json::Value::String("error".to_string()),
    );
    payload.insert(
        "code".to_string(),
        serde_json::Value::String(code.as_str().to_string()),
    );
    payload.insert(
        "message".to_string(),
        serde_json::Value::String(message.to_string()),
    );
    if let Some(tool_name) = tool_name {
        payload.insert(
            "tool".to_string(),
            serde_json::Value::String(tool_name.to_string()),
        );
    }
    serde_json::Value::Object(payload).to_string()
}

type ToolHandler = Box<dyn FnMut(&str) -> Result<String, ToolError>>;

#[derive(Default)]
pub struct StaticToolExecutor {
    handlers: BTreeMap<String, ToolHandler>,
}

impl StaticToolExecutor {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn register(
        mut self,
        tool_name: impl Into<String>,
        handler: impl FnMut(&str) -> Result<String, ToolError> + 'static,
    ) -> Self {
        self.handlers.insert(tool_name.into(), Box::new(handler));
        self
    }
}

impl ToolExecutor for StaticToolExecutor {
    fn execute(&mut self, tool_name: &str, input: &str) -> Result<String, ToolError> {
        self.handlers
            .get_mut(tool_name)
            .ok_or_else(|| ToolError::new(format!("unknown tool: {tool_name}")))?(input)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ApiClient, ApiRequest, AssistantEvent, ConversationRuntime, RuntimeError,
        StaticToolExecutor,
    };
    use crate::compact::CompactionConfig;
    use crate::config::{RuntimeFeatureConfig, RuntimeHookConfig};
    use crate::permissions::{
        PermissionMode, PermissionPolicy, PermissionPromptDecision, PermissionPrompter,
        PermissionRequest,
    };
    use crate::prompt::{ProjectContext, SystemPromptBuilder};
    use crate::session::{ContentBlock, MessageRole, Session};
    use crate::usage::TokenUsage;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct ScriptedApiClient {
        call_count: usize,
    }

    impl ApiClient for ScriptedApiClient {
        fn stream(&mut self, request: ApiRequest) -> Result<Vec<AssistantEvent>, RuntimeError> {
            self.call_count += 1;
            match self.call_count {
                1 => {
                    assert!(request
                        .messages
                        .iter()
                        .any(|message| message.role == MessageRole::User));
                    Ok(vec![
                        AssistantEvent::TextDelta("Let me calculate that.".to_string()),
                        AssistantEvent::ToolUse {
                            id: "tool-1".to_string(),
                            name: "add".to_string(),
                            input: "2,2".to_string(),
                        },
                        AssistantEvent::Usage(TokenUsage {
                            input_tokens: 20,
                            output_tokens: 6,
                            cache_creation_input_tokens: 1,
                            cache_read_input_tokens: 2,
                        }),
                        AssistantEvent::MessageStop,
                    ])
                }
                2 => {
                    let last_message = request
                        .messages
                        .last()
                        .expect("tool result should be present");
                    assert_eq!(last_message.role, MessageRole::Tool);
                    Ok(vec![
                        AssistantEvent::TextDelta("The answer is 4.".to_string()),
                        AssistantEvent::Usage(TokenUsage {
                            input_tokens: 24,
                            output_tokens: 4,
                            cache_creation_input_tokens: 1,
                            cache_read_input_tokens: 3,
                        }),
                        AssistantEvent::MessageStop,
                    ])
                }
                _ => Err(RuntimeError::new("unexpected extra API call")),
            }
        }
    }

    struct PromptAllowOnce;

    impl PermissionPrompter for PromptAllowOnce {
        fn decide(&mut self, request: &PermissionRequest) -> PermissionPromptDecision {
            assert_eq!(request.tool_name, "add");
            PermissionPromptDecision::Allow
        }
    }

    #[test]
    fn runs_user_to_tool_to_result_loop_end_to_end_and_tracks_usage() {
        let api_client = ScriptedApiClient { call_count: 0 };
        let tool_executor = StaticToolExecutor::new().register("add", |input| {
            let total = input
                .split(',')
                .map(|part| part.parse::<i32>().expect("input must be valid integer"))
                .sum::<i32>();
            Ok(total.to_string())
        });
        let permission_policy = PermissionPolicy::new(PermissionMode::WorkspaceWrite);
        let system_prompt = SystemPromptBuilder::new()
            .with_project_context(ProjectContext {
                cwd: PathBuf::from("/tmp/project"),
                current_date: "2026-03-31".to_string(),
                git_status: None,
                git_diff: None,
                instruction_files: Vec::new(),
            })
            .with_os("linux", "6.8")
            .build();
        let mut runtime = ConversationRuntime::new(
            Session::new(),
            api_client,
            tool_executor,
            permission_policy,
            system_prompt,
        );

        let summary = runtime
            .run_turn("what is 2 + 2?", Some(&mut PromptAllowOnce))
            .expect("conversation loop should succeed");

        assert_eq!(summary.iterations, 2);
        assert_eq!(summary.assistant_messages.len(), 2);
        assert_eq!(summary.tool_results.len(), 1);
        assert_eq!(runtime.session().messages.len(), 4);
        assert_eq!(summary.usage.output_tokens, 10);
        assert!(matches!(
            runtime.session().messages[1].blocks[1],
            ContentBlock::ToolUse { .. }
        ));
        assert!(matches!(
            runtime.session().messages[2].blocks[0],
            ContentBlock::ToolResult {
                is_error: false,
                ..
            }
        ));
    }

    #[test]
    fn records_denied_tool_results_when_prompt_rejects() {
        struct RejectPrompter;
        impl PermissionPrompter for RejectPrompter {
            fn decide(&mut self, _request: &PermissionRequest) -> PermissionPromptDecision {
                PermissionPromptDecision::Deny {
                    reason: "not now".to_string(),
                }
            }
        }

        struct SingleCallApiClient;
        impl ApiClient for SingleCallApiClient {
            fn stream(&mut self, request: ApiRequest) -> Result<Vec<AssistantEvent>, RuntimeError> {
                if request
                    .messages
                    .iter()
                    .any(|message| message.role == MessageRole::Tool)
                {
                    return Ok(vec![
                        AssistantEvent::TextDelta("I could not use the tool.".to_string()),
                        AssistantEvent::MessageStop,
                    ]);
                }
                Ok(vec![
                    AssistantEvent::ToolUse {
                        id: "tool-1".to_string(),
                        name: "blocked".to_string(),
                        input: "secret".to_string(),
                    },
                    AssistantEvent::MessageStop,
                ])
            }
        }

        let mut runtime = ConversationRuntime::new(
            Session::new(),
            SingleCallApiClient,
            StaticToolExecutor::new(),
            PermissionPolicy::new(PermissionMode::WorkspaceWrite),
            vec!["system".to_string()],
        );

        let summary = runtime
            .run_turn("use the tool", Some(&mut RejectPrompter))
            .expect("conversation should continue after denied tool");

        assert_eq!(summary.tool_results.len(), 1);
        assert!(matches!(
            &summary.tool_results[0].blocks[0],
            ContentBlock::ToolResult { is_error: true, output, .. } if output == "not now"
        ));
    }

    #[test]
    fn denies_tool_use_when_pre_tool_hook_blocks() {
        struct SingleCallApiClient;
        impl ApiClient for SingleCallApiClient {
            fn stream(&mut self, request: ApiRequest) -> Result<Vec<AssistantEvent>, RuntimeError> {
                if request
                    .messages
                    .iter()
                    .any(|message| message.role == MessageRole::Tool)
                {
                    return Ok(vec![
                        AssistantEvent::TextDelta("blocked".to_string()),
                        AssistantEvent::MessageStop,
                    ]);
                }
                Ok(vec![
                    AssistantEvent::ToolUse {
                        id: "tool-1".to_string(),
                        name: "blocked".to_string(),
                        input: r#"{"path":"secret.txt"}"#.to_string(),
                    },
                    AssistantEvent::MessageStop,
                ])
            }
        }

        let mut runtime = ConversationRuntime::new_with_features(
            Session::new(),
            SingleCallApiClient,
            StaticToolExecutor::new().register("blocked", |_input| {
                panic!("tool should not execute when hook denies")
            }),
            PermissionPolicy::new(PermissionMode::DangerFullAccess),
            vec!["system".to_string()],
            RuntimeFeatureConfig::default().with_hooks(RuntimeHookConfig::new(
                vec![shell_snippet("printf 'blocked by hook'; exit 2")],
                Vec::new(),
            )),
        );

        let summary = runtime
            .run_turn("use the tool", None)
            .expect("conversation should continue after hook denial");

        assert_eq!(summary.tool_results.len(), 1);
        let ContentBlock::ToolResult {
            is_error, output, ..
        } = &summary.tool_results[0].blocks[0]
        else {
            panic!("expected tool result block");
        };
        assert!(
            *is_error,
            "hook denial should produce an error result: {output}"
        );
        assert!(
            output.contains("denied tool") || output.contains("blocked by hook"),
            "unexpected hook denial output: {output:?}"
        );
    }

    #[test]
    fn appends_post_tool_hook_feedback_to_tool_result() {
        struct TwoCallApiClient {
            calls: usize,
        }

        impl ApiClient for TwoCallApiClient {
            fn stream(&mut self, request: ApiRequest) -> Result<Vec<AssistantEvent>, RuntimeError> {
                self.calls += 1;
                match self.calls {
                    1 => Ok(vec![
                        AssistantEvent::ToolUse {
                            id: "tool-1".to_string(),
                            name: "add".to_string(),
                            input: r#"{"lhs":2,"rhs":2}"#.to_string(),
                        },
                        AssistantEvent::MessageStop,
                    ]),
                    2 => {
                        assert!(request
                            .messages
                            .iter()
                            .any(|message| message.role == MessageRole::Tool));
                        Ok(vec![
                            AssistantEvent::TextDelta("done".to_string()),
                            AssistantEvent::MessageStop,
                        ])
                    }
                    _ => Err(RuntimeError::new("unexpected extra API call")),
                }
            }
        }

        let mut runtime = ConversationRuntime::new_with_features(
            Session::new(),
            TwoCallApiClient { calls: 0 },
            StaticToolExecutor::new().register("add", |_input| Ok("4".to_string())),
            PermissionPolicy::new(PermissionMode::DangerFullAccess),
            vec!["system".to_string()],
            RuntimeFeatureConfig::default().with_hooks(RuntimeHookConfig::new(
                vec![shell_snippet("printf 'pre hook ran'")],
                vec![shell_snippet("printf 'post hook ran'")],
            )),
        );

        let summary = runtime
            .run_turn("use add", None)
            .expect("tool loop succeeds");

        assert_eq!(summary.tool_results.len(), 1);
        let ContentBlock::ToolResult {
            is_error, output, ..
        } = &summary.tool_results[0].blocks[0]
        else {
            panic!("expected tool result block");
        };
        assert!(
            !*is_error,
            "post hook should preserve non-error result: {output:?}"
        );
        assert!(
            output.contains('4'),
            "tool output missing value: {output:?}"
        );
        assert!(
            output.contains("pre hook ran"),
            "tool output missing pre hook feedback: {output:?}"
        );
        assert!(
            output.contains("post hook ran"),
            "tool output missing post hook feedback: {output:?}"
        );
    }

    #[test]
    fn safe_mode_blocks_shell_tool_execution() {
        struct TwoCallApiClient {
            calls: usize,
        }
        impl ApiClient for TwoCallApiClient {
            fn stream(&mut self, request: ApiRequest) -> Result<Vec<AssistantEvent>, RuntimeError> {
                self.calls += 1;
                match self.calls {
                    1 => Ok(vec![
                        AssistantEvent::ToolUse {
                            id: "tool-1".to_string(),
                            name: "bash".to_string(),
                            input: "echo hi".to_string(),
                        },
                        AssistantEvent::MessageStop,
                    ]),
                    2 => {
                        assert!(request
                            .messages
                            .iter()
                            .any(|message| message.role == MessageRole::Tool));
                        Ok(vec![
                            AssistantEvent::TextDelta("done".to_string()),
                            AssistantEvent::MessageStop,
                        ])
                    }
                    _ => Err(RuntimeError::new("unexpected extra API call")),
                }
            }
        }
        let mut runtime = ConversationRuntime::new(
            Session::new(),
            TwoCallApiClient { calls: 0 },
            StaticToolExecutor::new().register("bash", |_input| {
                panic!("shell tool should not execute in read-only mode")
            }),
            PermissionPolicy::new(PermissionMode::ReadOnly)
                .with_tool_requirement("bash", PermissionMode::ReadOnly),
            vec!["system".to_string()],
        );
        let summary = runtime.run_turn("run shell", None).expect("turn should succeed");
        let ContentBlock::ToolResult {
            is_error, output, ..
        } = &summary.tool_results[0].blocks[0]
        else {
            panic!("expected tool result block");
        };
        assert!(*is_error);
        assert!(output.contains("shell execution is blocked"));
    }

    #[test]
    fn safe_mode_patch_apply_is_rejected() {
        struct AllowPrompter;
        impl PermissionPrompter for AllowPrompter {
            fn decide(&mut self, request: &PermissionRequest) -> PermissionPromptDecision {
                assert!(matches!(request.tool_name.as_str(), "write_file" | "apply_patch"));
                PermissionPromptDecision::Allow
            }
        }
        struct TwoCallApiClient {
            calls: usize,
        }
        impl ApiClient for TwoCallApiClient {
            fn stream(&mut self, _request: ApiRequest) -> Result<Vec<AssistantEvent>, RuntimeError> {
                self.calls += 1;
                match self.calls {
                    1 => Ok(vec![
                        AssistantEvent::ToolUse {
                            id: "tool-1".to_string(),
                            name: "write_file".to_string(),
                            input: r#"{"path":"demo.txt","content":"ok"}"#.to_string(),
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
        let mut runtime = ConversationRuntime::new(
            Session::new(),
            TwoCallApiClient { calls: 0 },
            StaticToolExecutor::new().register("write_file", |_input| {
                Ok(
                    r#"{"type":"patch_proposal","operation":"write_file","filePath":"demo.txt","original":"","modified":"ok","structuredPatch":[{"oldStart":1,"oldLines":0,"newStart":1,"newLines":1,"lines":["+ok"]}]}"#
                        .to_string(),
                )
            }),
            PermissionPolicy::new(PermissionMode::ReadOnly)
                .with_tool_requirement("write_file", PermissionMode::ReadOnly),
            vec!["system".to_string()],
        );
        let summary = runtime
            .run_turn("write file", Some(&mut AllowPrompter))
            .expect("turn should succeed");
        let ContentBlock::ToolResult {
            is_error, output, ..
        } = &summary.tool_results[0].blocks[0]
        else {
            panic!("expected tool result block");
        };
        assert!(*is_error);
        assert!(output.contains("read-only mode"));
    }

    #[test]
    fn workspace_allowlist_command_allowed() {
        struct TwoCallApiClient {
            calls: usize,
        }
        impl ApiClient for TwoCallApiClient {
            fn stream(&mut self, _request: ApiRequest) -> Result<Vec<AssistantEvent>, RuntimeError> {
                self.calls += 1;
                match self.calls {
                    1 => Ok(vec![
                        AssistantEvent::ToolUse {
                            id: "tool-1".to_string(),
                            name: "bash".to_string(),
                            input: r#"{"command":"git status"}"#.to_string(),
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
        let mut runtime = ConversationRuntime::new(
            Session::new(),
            TwoCallApiClient { calls: 0 },
            StaticToolExecutor::new().register("bash", |_input| Ok("shell ok".to_string())),
            PermissionPolicy::new(PermissionMode::WorkspaceWrite)
                .with_tool_requirement("bash", PermissionMode::WorkspaceWrite),
            vec!["system".to_string()],
        );
        let summary = runtime.run_turn("run shell", None).expect("turn should succeed");
        let ContentBlock::ToolResult {
            is_error, output, ..
        } = &summary.tool_results[0].blocks[0]
        else {
            panic!("expected tool result block");
        };
        assert!(!*is_error);
        assert!(output.contains("shell ok"));
    }

    #[test]
    fn workspace_disallowed_command_blocked() {
        struct TwoCallApiClient {
            calls: usize,
        }
        impl ApiClient for TwoCallApiClient {
            fn stream(&mut self, _request: ApiRequest) -> Result<Vec<AssistantEvent>, RuntimeError> {
                self.calls += 1;
                match self.calls {
                    1 => Ok(vec![
                        AssistantEvent::ToolUse {
                            id: "tool-1".to_string(),
                            name: "bash".to_string(),
                            input: r#"{"command":"curl http://example.com"}"#.to_string(),
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
        let mut runtime = ConversationRuntime::new(
            Session::new(),
            TwoCallApiClient { calls: 0 },
            StaticToolExecutor::new().register("bash", |_input| {
                panic!("disallowed shell command should not execute")
            }),
            PermissionPolicy::new(PermissionMode::WorkspaceWrite)
                .with_tool_requirement("bash", PermissionMode::WorkspaceWrite),
            vec!["system".to_string()],
        );
        let summary = runtime.run_turn("run shell", None).expect("turn should succeed");
        let ContentBlock::ToolResult {
            is_error, output, ..
        } = &summary.tool_results[0].blocks[0]
        else {
            panic!("expected tool result block");
        };
        assert!(*is_error);
        assert!(output.contains("not allowed by sandbox policy"));
    }

    #[test]
    fn full_access_shell_allowed() {
        struct TwoCallApiClient {
            calls: usize,
        }
        impl ApiClient for TwoCallApiClient {
            fn stream(&mut self, _request: ApiRequest) -> Result<Vec<AssistantEvent>, RuntimeError> {
                self.calls += 1;
                match self.calls {
                    1 => Ok(vec![
                        AssistantEvent::ToolUse {
                            id: "tool-1".to_string(),
                            name: "bash".to_string(),
                            input: r#"{"command":"bash build.sh"}"#.to_string(),
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
        let mut runtime = ConversationRuntime::new(
            Session::new(),
            TwoCallApiClient { calls: 0 },
            StaticToolExecutor::new().register("bash", |_input| Ok("full access ok".to_string())),
            PermissionPolicy::new(PermissionMode::DangerFullAccess)
                .with_tool_requirement("bash", PermissionMode::DangerFullAccess),
            vec!["system".to_string()],
        );
        let summary = runtime.run_turn("run shell", None).expect("turn should succeed");
        let ContentBlock::ToolResult {
            is_error, output, ..
        } = &summary.tool_results[0].blocks[0]
        else {
            panic!("expected tool result block");
        };
        assert!(!*is_error);
        assert!(output.contains("full access ok"));
    }

    #[test]
    fn workspace_traversal_denied() {
        struct TwoCallApiClient {
            calls: usize,
        }
        impl ApiClient for TwoCallApiClient {
            fn stream(&mut self, _request: ApiRequest) -> Result<Vec<AssistantEvent>, RuntimeError> {
                self.calls += 1;
                match self.calls {
                    1 => Ok(vec![
                        AssistantEvent::ToolUse {
                            id: "tool-1".to_string(),
                            name: "read_file".to_string(),
                            input: r#"{"path":"../../etc/passwd"}"#.to_string(),
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
        let workspace = unique_temp_path("runtime-workspace-traversal");
        fs::create_dir_all(workspace.join("src")).expect("workspace dir");
        let mut runtime = ConversationRuntime::new(
            Session::new(),
            TwoCallApiClient { calls: 0 },
            StaticToolExecutor::new().register("read_file", |_input| {
                panic!("read tool should not execute for traversal input")
            }),
            PermissionPolicy::new(PermissionMode::WorkspaceWrite)
                .with_tool_requirement("read_file", PermissionMode::ReadOnly),
            vec!["system".to_string()],
        )
        .with_workspace_root(&workspace);

        let summary = runtime.run_turn("read file", None).expect("turn should succeed");
        let ContentBlock::ToolResult {
            is_error, output, ..
        } = &summary.tool_results[0].blocks[0]
        else {
            panic!("expected tool result block");
        };
        assert!(*is_error);
        assert!(output.contains("escapes workspace") || output.contains("outside workspace"));
    }

    #[test]
    fn workspace_file_read_allowed() {
        struct TwoCallApiClient {
            calls: usize,
        }
        impl ApiClient for TwoCallApiClient {
            fn stream(&mut self, _request: ApiRequest) -> Result<Vec<AssistantEvent>, RuntimeError> {
                self.calls += 1;
                match self.calls {
                    1 => Ok(vec![
                        AssistantEvent::ToolUse {
                            id: "tool-1".to_string(),
                            name: "read_file".to_string(),
                            input: r#"{"path":"src/main.rs"}"#.to_string(),
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
        let workspace = unique_temp_path("runtime-workspace-read-allowed");
        fs::create_dir_all(workspace.join("src")).expect("workspace dir");
        fs::write(workspace.join("src").join("main.rs"), "fn main() {}").expect("file");
        let mut runtime = ConversationRuntime::new(
            Session::new(),
            TwoCallApiClient { calls: 0 },
            StaticToolExecutor::new()
                .register("read_file", |_input| Ok("read complete".to_string())),
            PermissionPolicy::new(PermissionMode::WorkspaceWrite)
                .with_tool_requirement("read_file", PermissionMode::ReadOnly),
            vec!["system".to_string()],
        )
        .with_workspace_root(&workspace);

        let summary = runtime.run_turn("read file", None).expect("turn should succeed");
        let ContentBlock::ToolResult {
            is_error, output, ..
        } = &summary.tool_results[0].blocks[0]
        else {
            panic!("expected tool result block");
        };
        assert!(!*is_error);
        assert!(output.contains("read complete"));
    }

    #[test]
    fn workspace_write_new_file_allowed() {
        struct AllowPrompter;
        impl PermissionPrompter for AllowPrompter {
            fn decide(&mut self, request: &PermissionRequest) -> PermissionPromptDecision {
                assert_eq!(request.tool_name, "apply_patch");
                PermissionPromptDecision::Allow
            }
        }
        struct TwoCallApiClient {
            calls: usize,
        }
        impl ApiClient for TwoCallApiClient {
            fn stream(&mut self, _request: ApiRequest) -> Result<Vec<AssistantEvent>, RuntimeError> {
                self.calls += 1;
                match self.calls {
                    1 => Ok(vec![
                        AssistantEvent::ToolUse {
                            id: "tool-1".to_string(),
                            name: "write_file".to_string(),
                            input: r#"{"path":"nested/new.txt","content":"ok"}"#.to_string(),
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
        let workspace = unique_temp_path("runtime-workspace-write-allowed");
        fs::create_dir_all(workspace.join("nested")).expect("workspace dir");
        let mut runtime = ConversationRuntime::new(
            Session::new(),
            TwoCallApiClient { calls: 0 },
            StaticToolExecutor::new()
                .register("write_file", |_input| Ok("write completed".to_string())),
            PermissionPolicy::new(PermissionMode::WorkspaceWrite)
                .with_tool_requirement("write_file", PermissionMode::WorkspaceWrite),
            vec!["system".to_string()],
        )
        .with_workspace_root(&workspace);

        let summary = runtime
            .run_turn("write file", Some(&mut AllowPrompter))
            .expect("turn should succeed");
        let ContentBlock::ToolResult {
            is_error, output, ..
        } = &summary.tool_results[0].blocks[0]
        else {
            panic!("expected tool result block");
        };
        assert!(!*is_error);
        assert!(output.contains("Patch applied"));
        assert!(workspace.join("nested/new.txt").exists());
    }

    #[test]
    fn patch_rejected_without_confirmation() {
        struct TwoCallApiClient {
            calls: usize,
        }
        impl ApiClient for TwoCallApiClient {
            fn stream(&mut self, _request: ApiRequest) -> Result<Vec<AssistantEvent>, RuntimeError> {
                self.calls += 1;
                match self.calls {
                    1 => Ok(vec![
                        AssistantEvent::ToolUse {
                            id: "tool-1".to_string(),
                            name: "write_file".to_string(),
                            input: r#"{"path":"nested/new.txt","content":"ok"}"#.to_string(),
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
        let workspace = unique_temp_path("runtime-patch-reject");
        fs::create_dir_all(workspace.join("nested")).expect("workspace dir");
        let mut runtime = ConversationRuntime::new(
            Session::new(),
            TwoCallApiClient { calls: 0 },
            StaticToolExecutor::new().register("write_file", |_input| {
                Ok(
                    r#"{"type":"patch_proposal","operation":"write_file","filePath":"nested/new.txt","original":"","modified":"ok","structuredPatch":[{"oldStart":1,"oldLines":0,"newStart":1,"newLines":1,"lines":["+ok"]}]}"#
                        .to_string(),
                )
            }),
            PermissionPolicy::new(PermissionMode::WorkspaceWrite)
                .with_tool_requirement("write_file", PermissionMode::WorkspaceWrite),
            vec!["system".to_string()],
        )
        .with_workspace_root(&workspace);
        let summary = runtime.run_turn("write file", None).expect("turn should succeed");
        let ContentBlock::ToolResult {
            is_error, output, ..
        } = &summary.tool_results[0].blocks[0]
        else {
            panic!("expected tool result block");
        };
        assert!(*is_error);
        assert!(output.contains("requires interactive confirmation"));
    }

    #[test]
    fn patch_conflict_detected() {
        struct AllowPrompter;
        impl PermissionPrompter for AllowPrompter {
            fn decide(&mut self, _request: &PermissionRequest) -> PermissionPromptDecision {
                PermissionPromptDecision::Allow
            }
        }
        struct TwoCallApiClient {
            calls: usize,
        }
        impl ApiClient for TwoCallApiClient {
            fn stream(&mut self, _request: ApiRequest) -> Result<Vec<AssistantEvent>, RuntimeError> {
                self.calls += 1;
                match self.calls {
                    1 => Ok(vec![
                        AssistantEvent::ToolUse {
                            id: "tool-1".to_string(),
                            name: "write_file".to_string(),
                            input: r#"{"path":"nested/conflict.txt","content":"new"}"#.to_string(),
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
        let workspace = unique_temp_path("runtime-patch-conflict");
        fs::create_dir_all(workspace.join("nested")).expect("workspace dir");
        fs::write(workspace.join("nested/conflict.txt"), "changed").expect("seed file");
        let mut runtime = ConversationRuntime::new(
            Session::new(),
            TwoCallApiClient { calls: 0 },
            StaticToolExecutor::new().register("write_file", |_input| {
                Ok(
                    r#"{"type":"patch_proposal","operation":"write_file","filePath":"nested/conflict.txt","original":"old","modified":"new","structuredPatch":[{"oldStart":1,"oldLines":1,"newStart":1,"newLines":1,"lines":["-old","+new"]}]}"#
                        .to_string(),
                )
            }),
            PermissionPolicy::new(PermissionMode::WorkspaceWrite)
                .with_tool_requirement("write_file", PermissionMode::WorkspaceWrite),
            vec!["system".to_string()],
        )
        .with_workspace_root(&workspace);
        let summary = runtime
            .run_turn("write file", Some(&mut AllowPrompter))
            .expect("turn should succeed");
        let ContentBlock::ToolResult {
            is_error, output, ..
        } = &summary.tool_results[0].blocks[0]
        else {
            panic!("expected tool result block");
        };
        assert!(*is_error);
        assert!(output.contains("patch conflict"));
        assert!(output.contains("---"));
        assert!(output.contains("+++"));
        let payload = extract_error_payload(output);
        assert_eq!(payload["code"], "patch_conflict");
    }

    #[test]
    fn mutation_tool_without_patch_proposal_rejected() {
        struct TwoCallApiClient {
            calls: usize,
        }
        impl ApiClient for TwoCallApiClient {
            fn stream(&mut self, _request: ApiRequest) -> Result<Vec<AssistantEvent>, RuntimeError> {
                self.calls += 1;
                match self.calls {
                    1 => Ok(vec![
                        AssistantEvent::ToolUse {
                            id: "tool-1".to_string(),
                            name: "write_file".to_string(),
                            input: r#"{"path":"nested/new.txt","content":"ok"}"#.to_string(),
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
        let workspace = unique_temp_path("runtime-mutation-bypass");
        fs::create_dir_all(workspace.join("nested")).expect("workspace dir");
        let mut runtime = ConversationRuntime::new(
            Session::new(),
            TwoCallApiClient { calls: 0 },
            StaticToolExecutor::new().register("write_file", |_input| {
                Ok("{\"type\":\"update\",\"filePath\":\"nested/new.txt\"}".to_string())
            }),
            PermissionPolicy::new(PermissionMode::WorkspaceWrite)
                .with_tool_requirement("write_file", PermissionMode::WorkspaceWrite),
            vec!["system".to_string()],
        )
        .with_workspace_root(&workspace);
        let summary = runtime.run_turn("write file", None).expect("turn should succeed");
        let ContentBlock::ToolResult {
            is_error, output, ..
        } = &summary.tool_results[0].blocks[0]
        else {
            panic!("expected tool result block");
        };
        assert!(*is_error);
        let payload = extract_error_payload(output);
        assert_eq!(payload["code"], "mutation_bypass_detected");
        assert!(payload["message"]
            .as_str()
            .expect("message")
            .contains("without patch_proposal"));
    }

    #[test]
    fn external_write_blocked() {
        struct TwoCallApiClient {
            calls: usize,
        }
        impl ApiClient for TwoCallApiClient {
            fn stream(&mut self, _request: ApiRequest) -> Result<Vec<AssistantEvent>, RuntimeError> {
                self.calls += 1;
                match self.calls {
                    1 => Ok(vec![
                        AssistantEvent::ToolUse {
                            id: "tool-1".to_string(),
                            name: "write_file".to_string(),
                            input: external_write_input(),
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
        let workspace = unique_temp_path("runtime-external-write-blocked");
        fs::create_dir_all(workspace.join("nested")).expect("workspace dir");
        let mut runtime = ConversationRuntime::new(
            Session::new(),
            TwoCallApiClient { calls: 0 },
            StaticToolExecutor::new().register("write_file", |_input| {
                panic!("write tool should not execute for external path")
            }),
            PermissionPolicy::new(PermissionMode::WorkspaceWrite)
                .with_tool_requirement("write_file", PermissionMode::WorkspaceWrite),
            vec!["system".to_string()],
        )
        .with_workspace_root(&workspace);

        let summary = runtime.run_turn("write file", None).expect("turn should succeed");
        let ContentBlock::ToolResult {
            is_error, output, ..
        } = &summary.tool_results[0].blocks[0]
        else {
            panic!("expected tool result block");
        };
        assert!(*is_error);
        assert!(output.contains("outside workspace"));
    }

    #[test]
    fn reconstructs_usage_tracker_from_restored_session() {
        struct SimpleApi;
        impl ApiClient for SimpleApi {
            fn stream(
                &mut self,
                _request: ApiRequest,
            ) -> Result<Vec<AssistantEvent>, RuntimeError> {
                Ok(vec![
                    AssistantEvent::TextDelta("done".to_string()),
                    AssistantEvent::MessageStop,
                ])
            }
        }

        let mut session = Session::new();
        session
            .messages
            .push(crate::session::ConversationMessage::assistant_with_usage(
                vec![ContentBlock::Text {
                    text: "earlier".to_string(),
                }],
                Some(TokenUsage {
                    input_tokens: 11,
                    output_tokens: 7,
                    cache_creation_input_tokens: 2,
                    cache_read_input_tokens: 1,
                }),
            ));

        let runtime = ConversationRuntime::new(
            session,
            SimpleApi,
            StaticToolExecutor::new(),
            PermissionPolicy::new(PermissionMode::DangerFullAccess),
            vec!["system".to_string()],
        );

        assert_eq!(runtime.usage().turns(), 1);
        assert_eq!(runtime.usage().cumulative_usage().total_tokens(), 21);
    }

    #[test]
    fn compacts_session_after_turns() {
        struct SimpleApi;
        impl ApiClient for SimpleApi {
            fn stream(
                &mut self,
                _request: ApiRequest,
            ) -> Result<Vec<AssistantEvent>, RuntimeError> {
                Ok(vec![
                    AssistantEvent::TextDelta("done".to_string()),
                    AssistantEvent::MessageStop,
                ])
            }
        }

        let mut runtime = ConversationRuntime::new(
            Session::new(),
            SimpleApi,
            StaticToolExecutor::new(),
            PermissionPolicy::new(PermissionMode::DangerFullAccess),
            vec!["system".to_string()],
        );
        runtime.run_turn("a", None).expect("turn a");
        runtime.run_turn("b", None).expect("turn b");
        runtime.run_turn("c", None).expect("turn c");

        let result = runtime.compact(CompactionConfig {
            preserve_recent_messages: 2,
            max_estimated_tokens: 1,
        });
        assert!(result.summary.contains("Conversation summary"));
        assert_eq!(
            result.compacted_session.messages[0].role,
            MessageRole::System
        );
    }

    #[cfg(windows)]
    fn shell_snippet(script: &str) -> String {
        script.replace('\'', "\"")
    }

    #[cfg(not(windows))]
    fn shell_snippet(script: &str) -> String {
        script.to_string()
    }

    fn unique_temp_path(prefix: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{stamp}"))
    }

    #[cfg(windows)]
    fn external_write_input() -> String {
        r#"{"path":"C:\\Windows\\System32\\config\\SAM","content":"blocked"}"#.to_string()
    }

    #[cfg(not(windows))]
    fn external_write_input() -> String {
        r#"{"path":"/etc/hosts","content":"blocked"}"#.to_string()
    }

    fn extract_error_payload(output: &str) -> serde_json::Value {
        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(output) {
            return parsed;
        }
        let start = output.find('{').expect("error payload should be present");
        serde_json::from_str(&output[start..]).expect("valid error payload")
    }
}
