use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use crate::permissions::PermissionMode;
use serde::{Deserialize, Serialize};

const SAFE_SHELL_COMMANDS: &[&str] = &["git", "ls", "cat", "grep"];

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum FilesystemIsolationMode {
    Off,
    #[default]
    WorkspaceOnly,
    AllowList,
}

impl FilesystemIsolationMode {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::WorkspaceOnly => "workspace-only",
            Self::AllowList => "allow-list",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SandboxConfig {
    pub enabled: Option<bool>,
    pub namespace_restrictions: Option<bool>,
    pub network_isolation: Option<bool>,
    pub filesystem_mode: Option<FilesystemIsolationMode>,
    pub allowed_mounts: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SandboxRequest {
    pub enabled: bool,
    pub namespace_restrictions: bool,
    pub network_isolation: bool,
    pub filesystem_mode: FilesystemIsolationMode,
    pub allowed_mounts: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ContainerEnvironment {
    pub in_container: bool,
    pub markers: Vec<String>,
}

#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SandboxStatus {
    pub enabled: bool,
    pub requested: SandboxRequest,
    pub supported: bool,
    pub active: bool,
    pub namespace_supported: bool,
    pub namespace_active: bool,
    pub network_supported: bool,
    pub network_active: bool,
    pub filesystem_mode: FilesystemIsolationMode,
    pub filesystem_active: bool,
    pub allowed_mounts: Vec<String>,
    pub in_container: bool,
    pub container_markers: Vec<String>,
    pub fallback_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxDetectionInputs<'a> {
    pub env_pairs: Vec<(String, String)>,
    pub dockerenv_exists: bool,
    pub containerenv_exists: bool,
    pub proc_1_cgroup: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinuxSandboxCommand {
    pub program: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolSandboxPolicy {
    mode: PermissionMode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SandboxDecision {
    Allow,
    RequireConfirmation { reason: String },
    Deny { reason: String },
}

impl ToolSandboxPolicy {
    #[must_use]
    pub fn from_permission_mode(mode: PermissionMode) -> Self {
        Self { mode }
    }

    #[must_use]
    pub fn evaluate_tool(&self, tool_name: &str) -> SandboxDecision {
        let normalized = tool_name.trim().to_ascii_lowercase();
        if self.is_shell_tool(&normalized) {
            return self.evaluate_shell_tool();
        }
        if self.is_write_tool(&normalized) {
            return self.evaluate_write_tool();
        }
        SandboxDecision::Allow
    }

    fn evaluate_shell_tool(&self) -> SandboxDecision {
        match self.mode {
            PermissionMode::ReadOnly => SandboxDecision::Deny {
                reason: format!(
                    "shell execution is blocked in {} mode",
                    self.mode.as_str()
                ),
            },
            PermissionMode::WorkspaceWrite => SandboxDecision::Allow,
            PermissionMode::Prompt => SandboxDecision::RequireConfirmation {
                reason: "shell execution requires approval in prompt mode".to_string(),
            },
            PermissionMode::DangerFullAccess | PermissionMode::Allow => SandboxDecision::Allow,
        }
    }

    fn evaluate_write_tool(&self) -> SandboxDecision {
        match self.mode {
            PermissionMode::ReadOnly | PermissionMode::Prompt => {
                SandboxDecision::RequireConfirmation {
                    reason: "file modification requires confirmation".to_string(),
                }
            }
            PermissionMode::WorkspaceWrite
            | PermissionMode::DangerFullAccess
            | PermissionMode::Allow => SandboxDecision::Allow,
        }
    }

    fn is_shell_tool(&self, tool_name: &str) -> bool {
        matches!(tool_name, "bash" | "powershell" | "run_command")
    }

    fn is_write_tool(&self, tool_name: &str) -> bool {
        matches!(
            tool_name,
            "write_file" | "edit_file" | "delete" | "delete_file"
        )
    }
}

#[must_use]
pub fn tool_requires_path_validation(tool_name: &str) -> bool {
    matches!(
        tool_name.trim().to_ascii_lowercase().as_str(),
        "read_file"
            | "write_file"
            | "edit_file"
            | "delete"
            | "delete_file"
            | "search"
            | "search_repo"
            | "glob_search"
            | "grep_search"
    )
}

pub fn validate_workspace_path(workspace_root: &Path, requested_path: &Path) -> Result<PathBuf, String> {
    let candidate = absolutize_path(workspace_root, requested_path);
    let canonical_requested = candidate
        .canonicalize()
        .map_err(|error| format!("unable to resolve path '{}': {error}", candidate.display()))?;
    let canonical_workspace = workspace_root
        .canonicalize()
        .map_err(|error| format!("invalid workspace root '{}': {error}", workspace_root.display()))?;

    if !canonical_requested.starts_with(&canonical_workspace) {
        return Err(format!(
            "path '{}' escapes workspace '{}'",
            canonical_requested.display(),
            canonical_workspace.display()
        ));
    }

    Ok(canonical_requested)
}

pub fn validate_write_target(workspace_root: &Path, requested_path: &Path) -> Result<(), String> {
    let candidate = absolutize_path(workspace_root, requested_path);
    let parent = candidate.parent().ok_or_else(|| {
        format!(
            "invalid write target '{}': missing parent directory",
            requested_path.display()
        )
    })?;
    let canonical_parent = parent.canonicalize().map_err(|error| {
        format!(
            "parent directory does not exist for '{}': {error}",
            candidate.display()
        )
    })?;
    let canonical_workspace = workspace_root
        .canonicalize()
        .map_err(|error| format!("invalid workspace root '{}': {error}", workspace_root.display()))?;

    if !canonical_parent.starts_with(&canonical_workspace) {
        return Err(format!(
            "write target '{}' is outside workspace '{}'",
            candidate.display(),
            canonical_workspace.display()
        ));
    }

    Ok(())
}

pub fn validate_tool_paths_for_input(
    workspace_root: &Path,
    tool_name: &str,
    input: &str,
) -> Result<(), String> {
    if !tool_requires_path_validation(tool_name) {
        return Ok(());
    }

    let normalized = tool_name.trim().to_ascii_lowercase();
    let value: serde_json::Value = serde_json::from_str(input)
        .map_err(|error| format!("invalid structured input for {normalized}: {error}"))?;
    let object = value
        .as_object()
        .ok_or_else(|| format!("tool input for {normalized} must be a JSON object"))?;

    let path_field = object
        .get("path")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from);
    let target_directory_field = object
        .get("target_directory")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from);

    match normalized.as_str() {
        "write_file" | "edit_file" => {
            let target = path_field.ok_or_else(|| format!("{normalized} requires 'path'"))?;
            validate_write_target(workspace_root, &target)
        }
        "read_file" | "delete" | "delete_file" => {
            let target = path_field.ok_or_else(|| format!("{normalized} requires 'path'"))?;
            validate_workspace_path(workspace_root, &target).map(|_| ())
        }
        "search" | "search_repo" | "glob_search" | "grep_search" => {
            if let Some(target) = path_field.or(target_directory_field) {
                validate_workspace_path(workspace_root, &target).map(|_| ())?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn absolutize_path(workspace_root: &Path, requested_path: &Path) -> PathBuf {
    if requested_path.is_absolute() {
        requested_path.to_path_buf()
    } else {
        workspace_root.join(requested_path)
    }
}

pub fn validate_shell_command(
    mode: PermissionMode,
    tool_name: &str,
    input: &str,
) -> Result<(), String> {
    if !matches!(
        tool_name.trim().to_ascii_lowercase().as_str(),
        "bash" | "powershell" | "run_command"
    ) {
        return Ok(());
    }

    if mode == PermissionMode::ReadOnly {
        return Err("shell execution disabled in read-only mode".to_string());
    }
    if mode == PermissionMode::DangerFullAccess || mode == PermissionMode::Allow {
        return Ok(());
    }

    let value: serde_json::Value = serde_json::from_str(input)
        .map_err(|error| format!("invalid shell input payload: {error}"))?;
    let command = value
        .get("command")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "shell input requires non-empty 'command'".to_string())?;

    let base = command.split_whitespace().next().unwrap_or_default();
    if SAFE_SHELL_COMMANDS.contains(&base) {
        return Ok(());
    }

    Err(format!("command '{base}' not allowed by sandbox policy"))
}

impl SandboxConfig {
    #[must_use]
    pub fn resolve_request(
        &self,
        enabled_override: Option<bool>,
        namespace_override: Option<bool>,
        network_override: Option<bool>,
        filesystem_mode_override: Option<FilesystemIsolationMode>,
        allowed_mounts_override: Option<Vec<String>>,
    ) -> SandboxRequest {
        SandboxRequest {
            enabled: enabled_override.unwrap_or(self.enabled.unwrap_or(true)),
            namespace_restrictions: namespace_override
                .unwrap_or(self.namespace_restrictions.unwrap_or(true)),
            network_isolation: network_override.unwrap_or(self.network_isolation.unwrap_or(false)),
            filesystem_mode: filesystem_mode_override
                .or(self.filesystem_mode)
                .unwrap_or_default(),
            allowed_mounts: allowed_mounts_override.unwrap_or_else(|| self.allowed_mounts.clone()),
        }
    }
}

#[must_use]
pub fn detect_container_environment() -> ContainerEnvironment {
    let proc_1_cgroup = fs::read_to_string("/proc/1/cgroup").ok();
    detect_container_environment_from(SandboxDetectionInputs {
        env_pairs: env::vars().collect(),
        dockerenv_exists: Path::new("/.dockerenv").exists(),
        containerenv_exists: Path::new("/run/.containerenv").exists(),
        proc_1_cgroup: proc_1_cgroup.as_deref(),
    })
}

#[must_use]
pub fn detect_container_environment_from(
    inputs: SandboxDetectionInputs<'_>,
) -> ContainerEnvironment {
    let mut markers = Vec::new();
    if inputs.dockerenv_exists {
        markers.push("/.dockerenv".to_string());
    }
    if inputs.containerenv_exists {
        markers.push("/run/.containerenv".to_string());
    }
    for (key, value) in inputs.env_pairs {
        let normalized = key.to_ascii_lowercase();
        if matches!(
            normalized.as_str(),
            "container" | "docker" | "podman" | "kubernetes_service_host"
        ) && !value.is_empty()
        {
            markers.push(format!("env:{key}={value}"));
        }
    }
    if let Some(cgroup) = inputs.proc_1_cgroup {
        for needle in ["docker", "containerd", "kubepods", "podman", "libpod"] {
            if cgroup.contains(needle) {
                markers.push(format!("/proc/1/cgroup:{needle}"));
            }
        }
    }
    markers.sort();
    markers.dedup();
    ContainerEnvironment {
        in_container: !markers.is_empty(),
        markers,
    }
}

#[must_use]
pub fn resolve_sandbox_status(config: &SandboxConfig, cwd: &Path) -> SandboxStatus {
    let request = config.resolve_request(None, None, None, None, None);
    resolve_sandbox_status_for_request(&request, cwd)
}

#[must_use]
pub fn resolve_sandbox_status_for_request(request: &SandboxRequest, cwd: &Path) -> SandboxStatus {
    let container = detect_container_environment();
    let namespace_supported = cfg!(target_os = "linux") && command_exists("unshare");
    let network_supported = namespace_supported;
    let filesystem_active =
        request.enabled && request.filesystem_mode != FilesystemIsolationMode::Off;
    let mut fallback_reasons = Vec::new();

    if request.enabled && request.namespace_restrictions && !namespace_supported {
        fallback_reasons
            .push("namespace isolation unavailable (requires Linux with `unshare`)".to_string());
    }
    if request.enabled && request.network_isolation && !network_supported {
        fallback_reasons
            .push("network isolation unavailable (requires Linux with `unshare`)".to_string());
    }
    if request.enabled
        && request.filesystem_mode == FilesystemIsolationMode::AllowList
        && request.allowed_mounts.is_empty()
    {
        fallback_reasons
            .push("filesystem allow-list requested without configured mounts".to_string());
    }

    let active = request.enabled
        && (!request.namespace_restrictions || namespace_supported)
        && (!request.network_isolation || network_supported);

    let allowed_mounts = normalize_mounts(&request.allowed_mounts, cwd);

    SandboxStatus {
        enabled: request.enabled,
        requested: request.clone(),
        supported: namespace_supported,
        active,
        namespace_supported,
        namespace_active: request.enabled && request.namespace_restrictions && namespace_supported,
        network_supported,
        network_active: request.enabled && request.network_isolation && network_supported,
        filesystem_mode: request.filesystem_mode,
        filesystem_active,
        allowed_mounts,
        in_container: container.in_container,
        container_markers: container.markers,
        fallback_reason: (!fallback_reasons.is_empty()).then(|| fallback_reasons.join("; ")),
    }
}

#[must_use]
pub fn build_linux_sandbox_command(
    command: &str,
    cwd: &Path,
    status: &SandboxStatus,
) -> Option<LinuxSandboxCommand> {
    if !cfg!(target_os = "linux")
        || !status.enabled
        || (!status.namespace_active && !status.network_active)
    {
        return None;
    }

    let mut args = vec![
        "--user".to_string(),
        "--map-root-user".to_string(),
        "--mount".to_string(),
        "--ipc".to_string(),
        "--pid".to_string(),
        "--uts".to_string(),
        "--fork".to_string(),
    ];
    if status.network_active {
        args.push("--net".to_string());
    }
    args.push("sh".to_string());
    args.push("-lc".to_string());
    args.push(command.to_string());

    let sandbox_home = cwd.join(".sandbox-home");
    let sandbox_tmp = cwd.join(".sandbox-tmp");
    let mut env = vec![
        ("HOME".to_string(), sandbox_home.display().to_string()),
        ("TMPDIR".to_string(), sandbox_tmp.display().to_string()),
        (
            "CLAW_SANDBOX_FILESYSTEM_MODE".to_string(),
            status.filesystem_mode.as_str().to_string(),
        ),
        (
            "CLAW_SANDBOX_ALLOWED_MOUNTS".to_string(),
            status.allowed_mounts.join(":"),
        ),
    ];
    if let Ok(path) = env::var("PATH") {
        env.push(("PATH".to_string(), path));
    }

    Some(LinuxSandboxCommand {
        program: "unshare".to_string(),
        args,
        env,
    })
}

fn normalize_mounts(mounts: &[String], cwd: &Path) -> Vec<String> {
    let cwd = cwd.to_path_buf();
    mounts
        .iter()
        .map(|mount| {
            let path = PathBuf::from(mount);
            if path.is_absolute() {
                path
            } else {
                cwd.join(path)
            }
        })
        .map(|path| path.display().to_string())
        .collect()
}

fn command_exists(command: &str) -> bool {
    env::var_os("PATH")
        .is_some_and(|paths| env::split_paths(&paths).any(|path| path.join(command).exists()))
}

#[cfg(test)]
mod tests {
    use super::{
        build_linux_sandbox_command, detect_container_environment_from, FilesystemIsolationMode,
        SandboxConfig, SandboxDecision, SandboxDetectionInputs, ToolSandboxPolicy,
        tool_requires_path_validation, validate_shell_command, validate_tool_paths_for_input,
        validate_workspace_path, validate_write_target,
    };
    use crate::permissions::PermissionMode;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn detects_container_markers_from_multiple_sources() {
        let detected = detect_container_environment_from(SandboxDetectionInputs {
            env_pairs: vec![("container".to_string(), "docker".to_string())],
            dockerenv_exists: true,
            containerenv_exists: false,
            proc_1_cgroup: Some("12:memory:/docker/abc"),
        });

        assert!(detected.in_container);
        assert!(detected
            .markers
            .iter()
            .any(|marker| marker == "/.dockerenv"));
        assert!(detected
            .markers
            .iter()
            .any(|marker| marker == "env:container=docker"));
        assert!(detected
            .markers
            .iter()
            .any(|marker| marker == "/proc/1/cgroup:docker"));
    }

    #[test]
    fn resolves_request_with_overrides() {
        let config = SandboxConfig {
            enabled: Some(true),
            namespace_restrictions: Some(true),
            network_isolation: Some(false),
            filesystem_mode: Some(FilesystemIsolationMode::WorkspaceOnly),
            allowed_mounts: vec!["logs".to_string()],
        };

        let request = config.resolve_request(
            Some(true),
            Some(false),
            Some(true),
            Some(FilesystemIsolationMode::AllowList),
            Some(vec!["tmp".to_string()]),
        );

        assert!(request.enabled);
        assert!(!request.namespace_restrictions);
        assert!(request.network_isolation);
        assert_eq!(request.filesystem_mode, FilesystemIsolationMode::AllowList);
        assert_eq!(request.allowed_mounts, vec!["tmp"]);
    }

    #[test]
    fn builds_linux_launcher_with_network_flag_when_requested() {
        let config = SandboxConfig::default();
        let status = super::resolve_sandbox_status_for_request(
            &config.resolve_request(
                Some(true),
                Some(true),
                Some(true),
                Some(FilesystemIsolationMode::WorkspaceOnly),
                None,
            ),
            Path::new("/workspace"),
        );

        if let Some(launcher) =
            build_linux_sandbox_command("printf hi", Path::new("/workspace"), &status)
        {
            assert_eq!(launcher.program, "unshare");
            assert!(launcher.args.iter().any(|arg| arg == "--mount"));
            assert!(launcher.args.iter().any(|arg| arg == "--net") == status.network_active);
        }
    }

    #[test]
    fn safe_mode_denies_shell_tools() {
        let policy = ToolSandboxPolicy::from_permission_mode(PermissionMode::ReadOnly);
        assert!(matches!(
            policy.evaluate_tool("bash"),
            SandboxDecision::Deny { reason } if reason.contains("read-only")
        ));
    }

    #[test]
    fn safe_mode_requires_confirmation_for_write_tools() {
        let policy = ToolSandboxPolicy::from_permission_mode(PermissionMode::ReadOnly);
        assert!(matches!(
            policy.evaluate_tool("write_file"),
            SandboxDecision::RequireConfirmation { reason }
                if reason.contains("file modification requires confirmation")
        ));
    }

    #[test]
    fn workspace_mode_allows_shell_tools_for_followup_allowlist_check() {
        let policy = ToolSandboxPolicy::from_permission_mode(PermissionMode::WorkspaceWrite);
        assert!(matches!(
            policy.evaluate_tool("bash"),
            SandboxDecision::Allow
        ));
    }

    #[test]
    fn validates_workspace_read_path_and_blocks_escape() {
        let root = unique_temp_path("sandbox-workspace-read");
        fs::create_dir_all(root.join("src")).expect("workspace");
        fs::write(root.join("src").join("main.rs"), "fn main(){}").expect("file");
        let outside = root
            .parent()
            .expect("parent")
            .join("outside.txt");
        fs::write(&outside, "x").expect("outside");

        assert!(validate_workspace_path(&root, Path::new("src/main.rs")).is_ok());
        assert!(validate_workspace_path(&root, &outside).is_err());
    }

    #[test]
    fn validates_write_target_parent_for_new_files() {
        let root = unique_temp_path("sandbox-workspace-write");
        fs::create_dir_all(root.join("newdir")).expect("workspace");
        let candidate = root.join("newdir").join("new.txt");
        assert!(validate_write_target(&root, &candidate).is_ok());
    }

    #[test]
    fn validates_tool_paths_for_file_tools() {
        let root = unique_temp_path("sandbox-tool-paths");
        fs::create_dir_all(root.join("src")).expect("workspace");
        fs::write(root.join("src").join("main.rs"), "fn main(){}").expect("file");

        let ok = validate_tool_paths_for_input(
            &root,
            "read_file",
            r#"{"path":"src/main.rs"}"#,
        );
        assert!(ok.is_ok());

        let denied = validate_tool_paths_for_input(
            &root,
            "read_file",
            r#"{"path":"../../etc/passwd"}"#,
        );
        assert!(denied.is_err());
    }

    #[test]
    fn marks_expected_tools_for_path_validation() {
        assert!(tool_requires_path_validation("read_file"));
        assert!(tool_requires_path_validation("write_file"));
        assert!(tool_requires_path_validation("glob_search"));
        assert!(!tool_requires_path_validation("bash"));
    }

    #[test]
    fn shell_command_allowlist_applies_in_workspace_mode() {
        assert!(validate_shell_command(
            PermissionMode::WorkspaceWrite,
            "bash",
            r#"{"command":"git status"}"#
        )
        .is_ok());
        assert!(validate_shell_command(
            PermissionMode::WorkspaceWrite,
            "bash",
            r#"{"command":"curl http://example.com"}"#
        )
        .is_err());
    }

    #[test]
    fn shell_command_allowlist_is_skipped_in_full_access() {
        assert!(validate_shell_command(
            PermissionMode::DangerFullAccess,
            "bash",
            r#"{"command":"curl http://example.com"}"#
        )
        .is_ok());
    }

    fn unique_temp_path(prefix: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{stamp}"))
    }
}
