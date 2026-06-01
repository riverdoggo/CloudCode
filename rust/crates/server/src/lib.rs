use std::collections::HashMap;
use std::convert::Infallible;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_stream::stream;
use axum::extract::{Path as AxumPath, Query, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{Html, IntoResponse};
use axum::routing::{get, post};
use axum::{Json, Router};
use runtime::{ContentBlock, ConversationMessage, Session as RuntimeSession};
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, RwLock};

pub type SessionId = String;
pub type SessionStore = Arc<RwLock<HashMap<SessionId, Session>>>;

const BROADCAST_CAPACITY: usize = 64;
const MAX_FILE_PREVIEW_BYTES: usize = 64 * 1024;
const SKIPPED_ENTRY_NAMES: &[&str] = &[".git", ".port_sessions", "target", "venv", "__pycache__"];

#[derive(Clone)]
pub struct AppState {
    sessions: SessionStore,
    next_session_id: Arc<AtomicU64>,
    workspace_root: Arc<PathBuf>,
    api_settings: Arc<RwLock<ApiSettings>>,
}

impl AppState {
    #[must_use]
    pub fn new() -> Self {
        let workspace_root = detect_workspace_root();
        let api_settings = load_api_settings(&workspace_root);
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            next_session_id: Arc::new(AtomicU64::new(1)),
            workspace_root: Arc::new(workspace_root),
            api_settings: Arc::new(RwLock::new(api_settings)),
        }
    }

    fn allocate_session_id(&self) -> SessionId {
        let id = self.next_session_id.fetch_add(1, Ordering::Relaxed);
        format!("session-{id}")
    }

    fn workspace_root(&self) -> &Path {
        self.workspace_root.as_ref().as_path()
    }

    fn settings_path(&self) -> PathBuf {
        self.workspace_root().join(".cloud-code.local.json")
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone)]
pub struct Session {
    pub id: SessionId,
    pub created_at: u64,
    pub conversation: RuntimeSession,
    events: broadcast::Sender<SessionEvent>,
}

impl Session {
    fn new(id: SessionId) -> Self {
        let (events, _) = broadcast::channel(BROADCAST_CAPACITY);
        Self {
            id,
            created_at: unix_timestamp_millis(),
            conversation: RuntimeSession::new(),
            events,
        }
    }

    fn subscribe(&self) -> broadcast::Receiver<SessionEvent> {
        self.events.subscribe()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
enum SessionEvent {
    Snapshot {
        session_id: SessionId,
        session: RuntimeSession,
    },
    Message {
        session_id: SessionId,
        message: ConversationMessage,
    },
}

impl SessionEvent {
    fn event_name(&self) -> &'static str {
        match self {
            Self::Snapshot { .. } => "snapshot",
            Self::Message { .. } => "message",
        }
    }

    fn to_sse_event(&self) -> Result<Event, serde_json::Error> {
        Ok(Event::default()
            .event(self.event_name())
            .data(serde_json::to_string(self)?))
    }
}

#[derive(Debug, Serialize)]
struct ErrorResponse {
    error: String,
}

type ApiError = (StatusCode, Json<ErrorResponse>);
type ApiResult<T> = Result<T, ApiError>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CreateSessionResponse {
    pub session_id: SessionId,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionSummary {
    pub id: SessionId,
    pub created_at: u64,
    pub message_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ListSessionsResponse {
    pub sessions: Vec<SessionSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionDetailsResponse {
    pub id: SessionId,
    pub created_at: u64,
    pub session: RuntimeSession,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SendMessageRequest {
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HealthResponse {
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectOverviewResponse {
    pub app_name: String,
    pub workspace_name: String,
    pub workspace_path: String,
    pub repo_summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GitInfoResponse {
    pub branch: String,
    pub remote: Option<String>,
    pub clean: bool,
    pub changed_files: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceEntry {
    pub name: String,
    pub path: String,
    pub kind: String,
    pub meta: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceListResponse {
    pub workspace_root: String,
    pub current_path: String,
    pub parent_path: Option<String>,
    pub entries: Vec<WorkspaceEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceFileResponse {
    pub path: String,
    pub directory_path: String,
    pub content: String,
    pub truncated: bool,
    pub line_count: usize,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WorkspaceQuery {
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ApiSettings {
    pub provider_name: String,
    pub base_url: String,
    pub model_name: String,
    pub api_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ApiSettingsResponse {
    pub provider_name: String,
    pub base_url: String,
    pub model_name: String,
    pub has_api_key: bool,
    pub api_key_masked: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct UpdateApiSettingsRequest {
    pub provider_name: String,
    pub base_url: String,
    pub model_name: String,
    pub api_key: Option<String>,
    pub clear_api_key: bool,
}

#[must_use]
pub fn app(state: AppState) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/assets/app.css", get(app_css))
        .route("/assets/app.js", get(app_js))
        .route("/api/health", get(health))
        .route("/api/project", get(project_overview))
        .route("/api/git", get(git_info))
        .route("/api/settings", get(api_settings).post(save_api_settings))
        .route("/api/workspace/list", get(workspace_list))
        .route("/api/workspace/file", get(workspace_file))
        .route("/api/pick-files", get(pick_files))
        .route("/api/pick-folder", get(pick_folder))
        .route("/api/sessions", post(create_session).get(list_sessions))
        .route("/api/sessions/{id}", get(get_session))
        .route("/api/sessions/{id}/events", get(stream_session_events))
        .route("/api/sessions/{id}/message", post(send_message))
        .route("/sessions", post(create_session).get(list_sessions))
        .route("/sessions/{id}", get(get_session))
        .route("/sessions/{id}/events", get(stream_session_events))
        .route("/sessions/{id}/message", post(send_message))
        .with_state(state)
}

async fn index() -> Html<&'static str> {
    Html(include_str!("../../../../gui/index.html"))
}

async fn app_css() -> impl IntoResponse {
    (
        [(axum::http::header::CONTENT_TYPE, "text/css; charset=utf-8")],
        include_str!("../../../../gui/app.css"),
    )
}

async fn app_js() -> impl IntoResponse {
    (
        [(
            axum::http::header::CONTENT_TYPE,
            "application/javascript; charset=utf-8",
        )],
        include_str!("../../../../gui/app.js"),
    )
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".to_string(),
    })
}

async fn project_overview(State(state): State<AppState>) -> Json<ProjectOverviewResponse> {
    let workspace_root = state.workspace_root();
    let workspace_name = workspace_root
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("workspace")
        .to_string();

    Json(ProjectOverviewResponse {
        app_name: "cloud-code".to_string(),
        workspace_name,
        workspace_path: workspace_root.display().to_string(),
        repo_summary:
            "Minimal local coding shell with chat, repo file browsing, git context, and API settings."
                .to_string(),
    })
}

async fn git_info(State(state): State<AppState>) -> Json<GitInfoResponse> {
    let workspace_root = state.workspace_root();
    let branch = git_output(workspace_root, &["branch", "--show-current"]).unwrap_or_default();
    let remote = git_output(workspace_root, &["remote", "get-url", "origin"]);
    let changed_files = git_output(workspace_root, &["status", "--short"])
        .map(|status| {
            status
                .lines()
                .filter(|line| !line.trim().is_empty())
                .count()
        })
        .unwrap_or(0);

    Json(GitInfoResponse {
        branch,
        remote,
        clean: changed_files == 0,
        changed_files,
    })
}

async fn api_settings(State(state): State<AppState>) -> Json<ApiSettingsResponse> {
    let settings = state.api_settings.read().await.clone();
    Json(settings.to_response())
}

async fn save_api_settings(
    State(state): State<AppState>,
    Json(payload): Json<UpdateApiSettingsRequest>,
) -> ApiResult<Json<ApiSettingsResponse>> {
    let response = {
        let mut settings = state.api_settings.write().await;
        settings.provider_name = payload.provider_name.trim().to_string();
        settings.base_url = payload.base_url.trim().to_string();
        settings.model_name = payload.model_name.trim().to_string();

        if payload.clear_api_key {
            settings.api_key.clear();
        } else if let Some(api_key) = payload.api_key {
            let trimmed = api_key.trim();
            if !trimmed.is_empty() {
                settings.api_key = trimmed.to_string();
            }
        }

        save_api_settings_to_disk(&state.settings_path(), &settings)
            .map_err(|error| internal_error(error.to_string()))?;
        settings.to_response()
    };

    Ok(Json(response))
}

async fn workspace_list(
    State(state): State<AppState>,
    Query(query): Query<WorkspaceQuery>,
) -> ApiResult<Json<WorkspaceListResponse>> {
    let requested_path =
        normalize_relative_path(query.path.as_deref().unwrap_or_default()).map_err(bad_request)?;
    let target = resolve_existing_workspace_path(state.workspace_root(), &requested_path)?;

    if !target.is_dir() {
        return Err(not_found(format!(
            "`{}` is not a directory",
            display_relative_path(&requested_path)
        )));
    }

    let mut entries = fs::read_dir(&target)
        .map_err(|error| internal_error(error.to_string()))?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let file_type = entry.file_type().ok()?;
            let name = entry.file_name().to_string_lossy().to_string();
            if should_skip_entry(&name) {
                return None;
            }

            let relative_path =
                relative_path_from_root(state.workspace_root(), &entry.path()).ok()?;
            let kind = if file_type.is_dir() {
                "directory".to_string()
            } else {
                "file".to_string()
            };
            let meta = if file_type.is_dir() {
                "Folder".to_string()
            } else {
                let size = entry
                    .metadata()
                    .ok()
                    .map(|metadata| metadata.len())
                    .unwrap_or(0);
                format!("{} bytes", size)
            };

            Some(WorkspaceEntry {
                name,
                path: relative_path,
                kind,
                meta,
            })
        })
        .collect::<Vec<_>>();

    entries.sort_by(|left, right| {
        left.kind
            .cmp(&right.kind)
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
    });

    Ok(Json(WorkspaceListResponse {
        workspace_root: state.workspace_root().display().to_string(),
        current_path: display_relative_path(&requested_path),
        parent_path: parent_display_path(&requested_path),
        entries,
    }))
}

async fn workspace_file(
    State(state): State<AppState>,
    Query(query): Query<WorkspaceQuery>,
) -> ApiResult<Json<WorkspaceFileResponse>> {
    let requested_path =
        normalize_relative_path(query.path.as_deref().unwrap_or_default()).map_err(bad_request)?;
    let target = resolve_existing_workspace_path(state.workspace_root(), &requested_path)?;

    if !target.is_file() {
        return Err(not_found(format!(
            "`{}` is not a file",
            display_relative_path(&requested_path)
        )));
    }

    let bytes = fs::read(&target).map_err(|error| internal_error(error.to_string()))?;
    let truncated = bytes.len() > MAX_FILE_PREVIEW_BYTES;
    let preview = if truncated {
        &bytes[..MAX_FILE_PREVIEW_BYTES]
    } else {
        bytes.as_slice()
    };
    let content = String::from_utf8_lossy(preview).into_owned();
    let line_count = content.lines().count().max(1);
    let directory_path = requested_path
        .parent()
        .map(display_relative_path)
        .unwrap_or_default();

    Ok(Json(WorkspaceFileResponse {
        path: display_relative_path(&requested_path),
        directory_path,
        content,
        truncated,
        line_count,
    }))
}

async fn create_session(
    State(state): State<AppState>,
) -> (StatusCode, Json<CreateSessionResponse>) {
    let session_id = state.allocate_session_id();
    let session = Session::new(session_id.clone());

    state
        .sessions
        .write()
        .await
        .insert(session_id.clone(), session);

    (
        StatusCode::CREATED,
        Json(CreateSessionResponse { session_id }),
    )
}

async fn list_sessions(State(state): State<AppState>) -> Json<ListSessionsResponse> {
    let sessions = state.sessions.read().await;
    let mut summaries = sessions
        .values()
        .map(|session| SessionSummary {
            id: session.id.clone(),
            created_at: session.created_at,
            message_count: session.conversation.messages.len(),
        })
        .collect::<Vec<_>>();
    summaries.sort_by(|left, right| left.id.cmp(&right.id));

    Json(ListSessionsResponse {
        sessions: summaries,
    })
}

async fn get_session(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<SessionId>,
) -> ApiResult<Json<SessionDetailsResponse>> {
    let sessions = state.sessions.read().await;
    let session = sessions
        .get(&id)
        .ok_or_else(|| not_found(format!("session `{id}` not found")))?;

    Ok(Json(SessionDetailsResponse {
        id: session.id.clone(),
        created_at: session.created_at,
        session: session.conversation.clone(),
    }))
}

async fn send_message(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<SessionId>,
    Json(payload): Json<SendMessageRequest>,
) -> ApiResult<StatusCode> {
    let mut expanded_prompt = payload.message.clone();

    if expanded_prompt.starts_with("Context:\n") {
        if let Some((context_block, rest_of_prompt)) = expanded_prompt.clone().split_once("\n\n") {
            let mut enhanced_context = String::new();
            for line in context_block.lines() {
                if let Some(file_path) = line.strip_prefix("- file: ") {
                    let path = std::path::PathBuf::from(file_path.trim());
                    if path.is_file() {
                        if let Ok(content) = std::fs::read_to_string(&path) {
                            enhanced_context.push_str(&format!("- file: {}\n```\n{}\n```\n", file_path, content));
                        } else {
                            enhanced_context.push_str(&format!("- file: {} (Error: Could not read file)\n", file_path));
                        }
                    } else {
                        enhanced_context.push_str(&format!("- file: {} (Error: File not found)\n", file_path));
                    }
                } else if let Some(path_dir) = line.strip_prefix("- path: ") {
                    let path = std::path::PathBuf::from(path_dir.trim());
                    if path.is_dir() {
                         if let Ok(entries) = std::fs::read_dir(&path) {
                             let mut tree = Vec::new();
                             for entry in entries.flatten() {
                                 tree.push(entry.file_name().to_string_lossy().to_string());
                             }
                             enhanced_context.push_str(&format!("- directory contents parameter: {}\nFiles inside:\n{}\n", path_dir, tree.join("\n")));
                         }
                    } else if path.is_file() {
                        if let Ok(content) = std::fs::read_to_string(&path) {
                            enhanced_context.push_str(&format!("- path (file fallback): {}\n```\n{}\n```\n", path_dir, content));
                        }
                    } else {
                         enhanced_context.push_str(&format!("- path: {} (Not found)\n", path_dir));
                    }
                } else {
                    enhanced_context.push_str(line);
                    enhanced_context.push('\n');
                }
            }
            expanded_prompt = format!("{}\n\n{}", enhanced_context, rest_of_prompt);
        }
    }

    // Retain original simplified message in local history
    let message = ConversationMessage::user_text(payload.message.clone());
    
    // We will supply expanded_prompt to the network, but display generic payload.message to User.
    // However, if we do that here, the history logic iterates over `session.messages`. 
    // We should patch the very last message just before transmission.
    
    let settings = load_api_settings(state.workspace_root());
    
    let (broadcaster, history) = {
        let mut sessions = state.sessions.write().await;
        let session = sessions
            .get_mut(&id)
            .ok_or_else(|| not_found(format!("session `{id}` not found")))?;
        session.conversation.messages.push(message.clone());
        (session.events.clone(), session.conversation.messages.clone())
    };

    let _ = broadcaster.send(SessionEvent::Message {
        session_id: id.clone(),
        message,
    });

    if settings.api_key.is_empty() {
        let assistant = ConversationMessage::assistant(vec![ContentBlock::Text { 
            text: "No API key configured. Please configure your API key in the settings panel.\n\nYou can use OpenRouter or any OpenAI-compatible endpoint.".to_string() 
        }]);
        let mut sessions = state.sessions.write().await;
        if let Some(session) = sessions.get_mut(&id) {
            session.conversation.messages.push(assistant.clone());
        }
        let _ = broadcaster.send(SessionEvent::Message { session_id: id, message: assistant });
        return Ok(StatusCode::NO_CONTENT);
    }
    
    let base_url = settings.base_url.trim_end_matches('/');
    let api_url = if base_url.ends_with("/keys") || base_url.is_empty() {
        "https://openrouter.ai/api/v1/chat/completions".to_string()
    } else {
        format!("{base_url}/chat/completions")
    };

    let client = reqwest::Client::new();
    let mut messages = Vec::new();
    for msg in history {
        let role = match msg.role {
            runtime::MessageRole::System | runtime::MessageRole::User | runtime::MessageRole::Tool => "user",
            runtime::MessageRole::Assistant => "assistant",
        };
        let mut content = String::new();
        for block in msg.blocks {
            if let ContentBlock::Text { text } = block {
                content.push_str(&text);
            }
        }
        if !content.is_empty() {
                // Inject our fully expanded prompt into the final payload transmission ONLY for the user's latest prompt
                let expanded_content = if role == "user" && content == payload.message {
                    expanded_prompt.clone()
                } else {
                    content
                };
                
            messages.push(serde_json::json!({
                "role": role,
                "content": expanded_content
            }));
        }
    }

    let request_body = serde_json::json!({
        "model": if settings.model_name.is_empty() { "openrouter/auto" } else { &settings.model_name },
        "messages": messages
    });

    let res = client.post(&api_url)
        .header("Authorization", format!("Bearer {}", settings.api_key))
        .json(&request_body)
        .send()
        .await;

    let assistant = match res {
        Ok(response) => {
            if response.status().is_success() {
                if let Ok(json) = response.json::<serde_json::Value>().await {
                    if let Some(content) = json["choices"][0]["message"]["content"].as_str() {
                        ConversationMessage::assistant(vec![ContentBlock::Text { text: content.to_string() }])
                    } else {
                        ConversationMessage::assistant(vec![ContentBlock::Text { text: "No content returned by LLM.".to_string() }])
                    }
                } else {
                    ConversationMessage::assistant(vec![ContentBlock::Text { text: "Failed to parse JSON response from LLM.".to_string() }])
                }
            } else {
                let status = response.status();
                let txt = response.text().await.unwrap_or_default();
                ConversationMessage::assistant(vec![ContentBlock::Text { text: format!("API Error {}:\n{}", status, txt) }])
            }
        }
        Err(e) => {
            ConversationMessage::assistant(vec![ContentBlock::Text { text: format!("Network request failed: {}", e) }])
        }
    };

    let mut sessions = state.sessions.write().await;
    if let Some(session) = sessions.get_mut(&id) {
        session.conversation.messages.push(assistant.clone());
    }
    let _ = broadcaster.send(SessionEvent::Message { session_id: id, message: assistant });

    Ok(StatusCode::NO_CONTENT)
}

async fn stream_session_events(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<SessionId>,
) -> ApiResult<impl IntoResponse> {
    let (snapshot, mut receiver) = {
        let sessions = state.sessions.read().await;
        let session = sessions
            .get(&id)
            .ok_or_else(|| not_found(format!("session `{id}` not found")))?;
        (
            SessionEvent::Snapshot {
                session_id: session.id.clone(),
                session: session.conversation.clone(),
            },
            session.subscribe(),
        )
    };

    let stream = stream! {
        if let Ok(event) = snapshot.to_sse_event() {
            yield Ok::<Event, Infallible>(event);
        }

        loop {
            match receiver.recv().await {
                Ok(event) => {
                    if let Ok(sse_event) = event.to_sse_event() {
                        yield Ok::<Event, Infallible>(sse_event);
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    };

    Ok(Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15))))
}

fn unix_timestamp_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after epoch")
        .as_millis() as u64
}



fn detect_workspace_root() -> PathBuf {
    if let Some(path) = std::env::var("CLOUD_CODE_WORKSPACE_ROOT")
        .ok()
        .map(|value| value.trim().trim_matches('"').to_string())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
    {
        return canonical_or_original(path);
    }

    let current_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    if current_dir
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("rust"))
    {
        if let Some(parent) = current_dir.parent() {
            return canonical_or_original(parent.to_path_buf());
        }
    }

    canonical_or_original(current_dir)
}

fn canonical_or_original(path: PathBuf) -> PathBuf {
    fs::canonicalize(&path).unwrap_or(path)
}

fn load_api_settings(workspace_root: &Path) -> ApiSettings {
    let path = workspace_root.join(".cloud-code.local.json");
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(_) => return ApiSettings::default(),
    };

    serde_json::from_str(&contents).unwrap_or_default()
}

fn save_api_settings_to_disk(path: &Path, settings: &ApiSettings) -> Result<(), std::io::Error> {
    let json = serde_json::to_string_pretty(settings)
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    fs::write(path, json)
}

fn git_output(workspace_root: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(workspace_root)
        .args(args)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!stdout.is_empty()).then_some(stdout)
}

fn should_skip_entry(name: &str) -> bool {
    SKIPPED_ENTRY_NAMES.contains(&name) || name.ends_with(".pyc")
}

fn normalize_relative_path(raw: &str) -> Result<PathBuf, String> {
    let trimmed = raw.trim().replace('\\', "/");
    if trimmed.is_empty() || trimmed == "/" {
        return Ok(PathBuf::new());
    }

    let path = Path::new(&trimmed);
    let mut normalized = PathBuf::new();

    for component in path.components() {
        match component {
            Component::Normal(part) => normalized.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err("path must stay inside the workspace".to_string())
            }
        }
    }

    Ok(normalized)
}

fn resolve_existing_workspace_path(
    workspace_root: &Path,
    relative_path: &Path,
) -> ApiResult<PathBuf> {
    let target = workspace_root.join(relative_path);
    let canonical_target = fs::canonicalize(&target).map_err(|_| {
        not_found(format!(
            "`{}` was not found",
            display_relative_path(relative_path)
        ))
    })?;

    if !canonical_target.starts_with(workspace_root) {
        return Err(bad_request(
            "path must stay inside the workspace".to_string(),
        ));
    }

    Ok(canonical_target)
}

fn relative_path_from_root(workspace_root: &Path, full_path: &Path) -> Result<String, String> {
    let canonical_full_path = fs::canonicalize(full_path).map_err(|error| error.to_string())?;
    let relative = canonical_full_path
        .strip_prefix(workspace_root)
        .map_err(|error| error.to_string())?;
    Ok(display_relative_path(relative))
}

fn display_relative_path(path: &Path) -> String {
    if path.as_os_str().is_empty() {
        String::new()
    } else {
        path.to_string_lossy().replace('\\', "/")
    }
}

fn parent_display_path(path: &Path) -> Option<String> {
    path.parent().map(display_relative_path)
}

fn mask_api_key(api_key: &str) -> String {
    if api_key.is_empty() {
        return String::new();
    }

    let visible_prefix = api_key.chars().take(4).collect::<String>();
    let visible_suffix = api_key
        .chars()
        .rev()
        .take(4)
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();

    if api_key.chars().count() <= 8 {
        format!("{visible_prefix}...")
    } else {
        format!("{visible_prefix}...{visible_suffix}")
    }
}

impl ApiSettings {
    fn to_response(&self) -> ApiSettingsResponse {
        ApiSettingsResponse {
            provider_name: self.provider_name.clone(),
            base_url: self.base_url.clone(),
            model_name: self.model_name.clone(),
            has_api_key: !self.api_key.is_empty(),
            api_key_masked: mask_api_key(&self.api_key),
        }
    }
}

fn not_found(message: String) -> ApiError {
    (
        StatusCode::NOT_FOUND,
        Json(ErrorResponse { error: message }),
    )
}

fn bad_request(message: String) -> ApiError {
    (
        StatusCode::BAD_REQUEST,
        Json(ErrorResponse { error: message }),
    )
}

fn internal_error(message: String) -> ApiError {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorResponse { error: message }),
    )
}

async fn pick_files() -> Json<Vec<String>> {
    let result = tokio::task::spawn_blocking(|| rfd::FileDialog::new().pick_files())
        .await
        .unwrap_or(None);

    let paths = result
        .unwrap_or_default()
        .into_iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect();
    Json(paths)
}

async fn pick_folder() -> Json<Option<String>> {
    let result = tokio::task::spawn_blocking(|| rfd::FileDialog::new().pick_folder())
        .await
        .unwrap_or(None);

    let path = result.map(|p| p.to_string_lossy().to_string());
    Json(path)
}

#[cfg(test)]
mod tests {
    use super::{
        app, mask_api_key, normalize_relative_path, AppState, CreateSessionResponse,
        ListSessionsResponse, SessionDetailsResponse,
    };
    use reqwest::Client;
    use std::net::SocketAddr;
    use std::time::Duration;
    use tokio::net::TcpListener;
    use tokio::task::JoinHandle;
    use tokio::time::timeout;

    struct TestServer {
        address: SocketAddr,
        handle: JoinHandle<()>,
    }

    impl TestServer {
        async fn spawn() -> Self {
            let listener = TcpListener::bind("127.0.0.1:0")
                .await
                .expect("test listener should bind");
            let address = listener
                .local_addr()
                .expect("listener should report local address");
            let handle = tokio::spawn(async move {
                axum::serve(listener, app(AppState::default()))
                    .await
                    .expect("server should run");
            });

            Self { address, handle }
        }

        fn url(&self, path: &str) -> String {
            format!("http://{}{}", self.address, path)
        }
    }

    impl Drop for TestServer {
        fn drop(&mut self) {
            self.handle.abort();
        }
    }

    async fn create_session(client: &Client, server: &TestServer) -> CreateSessionResponse {
        client
            .post(server.url("/sessions"))
            .send()
            .await
            .expect("create request should succeed")
            .error_for_status()
            .expect("create request should return success")
            .json::<CreateSessionResponse>()
            .await
            .expect("create response should parse")
    }

    async fn next_sse_frame(response: &mut reqwest::Response, buffer: &mut String) -> String {
        loop {
            if let Some(index) = buffer.find("\n\n") {
                let frame = buffer[..index].to_string();
                let remainder = buffer[index + 2..].to_string();
                *buffer = remainder;
                return frame;
            }

            let next_chunk = timeout(Duration::from_secs(5), response.chunk())
                .await
                .expect("SSE stream should yield within timeout")
                .expect("SSE stream should remain readable")
                .expect("SSE stream should stay open");
            buffer.push_str(&String::from_utf8_lossy(&next_chunk));
        }
    }

    #[tokio::test]
    async fn creates_and_lists_sessions() {
        let server = TestServer::spawn().await;
        let client = Client::new();
        let created = create_session(&client, &server).await;
        let sessions = client
            .get(server.url("/sessions"))
            .send()
            .await
            .expect("list request should succeed")
            .error_for_status()
            .expect("list request should return success")
            .json::<ListSessionsResponse>()
            .await
            .expect("list response should parse");
        let details = client
            .get(server.url(&format!("/sessions/{}", created.session_id)))
            .send()
            .await
            .expect("details request should succeed")
            .error_for_status()
            .expect("details request should return success")
            .json::<SessionDetailsResponse>()
            .await
            .expect("details response should parse");
        assert_eq!(created.session_id, "session-1");
        assert_eq!(sessions.sessions.len(), 1);
        assert_eq!(sessions.sessions[0].id, created.session_id);
        assert_eq!(sessions.sessions[0].message_count, 0);
        assert_eq!(details.id, "session-1");
        assert!(details.session.messages.is_empty());
    }

    #[tokio::test]
    async fn streams_message_events_and_persists_message_flow() {
        let server = TestServer::spawn().await;
        let client = Client::new();
        let created = create_session(&client, &server).await;
        let mut response = client
            .get(server.url(&format!("/sessions/{}/events", created.session_id)))
            .send()
            .await
            .expect("events request should succeed")
            .error_for_status()
            .expect("events request should return success");
        let mut buffer = String::new();
        let snapshot_frame = next_sse_frame(&mut response, &mut buffer).await;
        let send_status = client
            .post(server.url(&format!("/sessions/{}/message", created.session_id)))
            .json(&super::SendMessageRequest {
                message: "hello from test".to_string(),
            })
            .send()
            .await
            .expect("message request should succeed")
            .status();
        let message_frame = next_sse_frame(&mut response, &mut buffer).await;
        let details = client
            .get(server.url(&format!("/sessions/{}", created.session_id)))
            .send()
            .await
            .expect("details request should succeed")
            .error_for_status()
            .expect("details request should return success")
            .json::<SessionDetailsResponse>()
            .await
            .expect("details response should parse");
        assert_eq!(send_status, reqwest::StatusCode::NO_CONTENT);
        assert!(snapshot_frame.contains("event: snapshot"));
        assert!(snapshot_frame.contains("\"session_id\":\"session-1\""));
        assert!(message_frame.contains("event: message"));
        assert!(message_frame.contains("hello from test"));
        assert_eq!(details.session.messages.len(), 2);
        assert_eq!(
            details.session.messages[0],
            runtime::ConversationMessage::user_text("hello from test")
        );
        assert!(matches!(
            &details.session.messages[1],
            runtime::ConversationMessage {
                role: runtime::MessageRole::Assistant,
                ..
            }
        ));
    }

    #[test]
    fn rejects_parent_segments_in_workspace_paths() {
        assert!(normalize_relative_path("../secrets.txt").is_err());
        assert!(normalize_relative_path("src/../../oops").is_err());
        assert!(normalize_relative_path("src/main.rs").is_ok());
    }

    #[test]
    fn masks_api_key_for_display() {
        assert_eq!(mask_api_key(""), "");
        assert_eq!(mask_api_key("1234567"), "1234...");
        assert_eq!(mask_api_key("1234567890"), "1234...7890");
    }
}
