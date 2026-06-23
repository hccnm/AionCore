use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotRef {
    pub snapshot_id: String,
    pub artifact_ref: String,
    pub manifest_ref: String,
    pub checksum: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecutionCallerContext {
    pub user_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErlCreateExecutionRequest {
    pub snapshot_ref: SnapshotRef,
    pub execution_type: String,
    pub policy: Value,
    pub resource_profile: Value,
    pub network_profile: Option<String>,
    pub env_refs: Vec<String>,
    pub trace_id: String,
    pub caller: ExecutionCallerContext,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ExecutionBackendKind {
    K8s,
    Local,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecutionBackendCapabilities {
    pub backend: ExecutionBackendKind,
    pub degraded_isolation: bool,
    pub snapshot_only_input: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErlCreateExecutionAccepted {
    pub initial_status: String,
    pub capabilities: ExecutionBackendCapabilities,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct K8sIsolationPlan {
    pub namespace: String,
    pub resource_quota: K8sResourceQuota,
    pub limit_range: K8sLimitRange,
    pub network_policy: K8sNetworkPolicy,
    pub pod_security: K8sPodSecurity,
    pub privileged_allowed: bool,
    pub docker_socket_allowed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct K8sResourceQuota {
    pub cpu_limit: String,
    pub memory_limit: String,
    pub ephemeral_storage_limit: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct K8sLimitRange {
    pub default_cpu: String,
    pub default_memory: String,
    pub max_cpu: String,
    pub max_memory: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct K8sNetworkPolicy {
    pub profile: String,
    pub default_deny_ingress: bool,
    pub default_deny_egress: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct K8sPodSecurity {
    pub enforce_level: String,
    pub run_as_non_root: bool,
    pub read_only_root_filesystem: bool,
    pub allow_privilege_escalation: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErlCancelExecutionRequest {
    pub execution_id: String,
    pub trace_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErlStatusQuery {
    pub execution_id: String,
    pub trace_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErlStreamSubscription {
    pub execution_id: String,
    pub trace_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErlArtifactQuery {
    pub execution_id: String,
    pub trace_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErlCleanupRetryRequest {
    pub execution_id: String,
    pub trace_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ErlRuntimeArtifact {
    pub artifact_type: String,
    pub artifact_ref: String,
    pub size_bytes: u64,
    pub immutable: bool,
    pub metadata: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ErlExecutionRunResult {
    pub execution_id: String,
    pub trace_id: String,
    pub status: String,
    pub capabilities: ExecutionBackendCapabilities,
    pub preview_url: Option<String>,
    pub artifacts: Vec<ErlRuntimeArtifact>,
    pub cleanup_status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ErlCleanupOutcome {
    pub execution_id: String,
    pub trace_id: String,
    pub status: String,
    pub retryable: bool,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AiRepairPolicy {
    pub enabled: bool,
    pub max_retry: u32,
    pub max_time_seconds: u64,
    pub require_human_approval_after: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum AiRepairDecision {
    Continue,
    StopMaxRetry,
    StopMaxTime,
    WaitHumanApproval,
    Disabled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AiRepairAttemptState {
    pub retry_count: u32,
    pub elapsed_seconds: u64,
    pub human_approved: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[allow(dead_code)]
pub struct AiFailureAnalysisInput {
    pub trace_id: String,
    pub execution_id: String,
    pub retry_attempt: u32,
    pub failure: StructuredFailure,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlaywrightTestPlan {
    pub version: String,
    pub steps: Vec<PlaywrightTestStep>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlaywrightTestStep {
    pub id: String,
    pub action: PlaywrightStepAction,
    pub target: Option<String>,
    pub value: Option<String>,
    pub assertion: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PlaywrightStepAction {
    Navigate,
    Click,
    Fill,
    Press,
    ExpectVisible,
    ExpectText,
    ExpectUrl,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StructuredFailure {
    pub failed_step_id: Option<String>,
    pub failed_step_index: Option<u32>,
    pub reason: String,
    pub trace_ref: Option<String>,
    pub screenshot_ref: Option<String>,
    pub log_refs: Vec<String>,
    pub exit_code: Option<i32>,
}

#[derive(Debug, thiserror::Error)]
pub enum ExecutionRuntimeError {
    #[error("ERL request must include snapshot_ref")]
    MissingSnapshotRef,
    #[error("execution policy exceeds ERL hard limit: {0}")]
    PolicyLimitExceeded(String),
    #[error("local ERL backend is not enabled")]
    LocalBackendDisabled,
    #[error("invalid Playwright test plan: {0}")]
    InvalidPlaywrightPlan(String),
    #[error("invalid structured failure: {0}")]
    InvalidStructuredFailure(String),
    #[error("snapshot materialization failed: {0}")]
    SnapshotMaterializationFailed(String),
    #[error("runtime cleanup failed: {0}")]
    CleanupFailed(String),
    #[error("traceability contract violation: {0}")]
    TraceabilityViolation(String),
}

impl AiRepairPolicy {
    pub fn from_execution_policy(policy: &Value) -> Self {
        let auto_fix = policy.get("auto_fix").unwrap_or(policy);
        Self {
            enabled: auto_fix.get("enabled").and_then(Value::as_bool).unwrap_or(false),
            max_retry: unsigned_field_as_u32(auto_fix, &["max_retry"], 0),
            max_time_seconds: unsigned_field(auto_fix, &["max_time_seconds", "max_time"]).unwrap_or(0),
            require_human_approval_after: unsigned_field_as_u32(auto_fix, &["require_human_approval_after"], u32::MAX),
        }
    }

    pub fn decide(&self, attempt: &AiRepairAttemptState) -> AiRepairDecision {
        if !self.enabled {
            return AiRepairDecision::Disabled;
        }
        if attempt.retry_count >= self.max_retry {
            return AiRepairDecision::StopMaxRetry;
        }
        if self.max_time_seconds > 0 && attempt.elapsed_seconds >= self.max_time_seconds {
            return AiRepairDecision::StopMaxTime;
        }
        if attempt.retry_count >= self.require_human_approval_after && !attempt.human_approved {
            return AiRepairDecision::WaitHumanApproval;
        }
        AiRepairDecision::Continue
    }
}

impl PlaywrightTestPlan {
    pub fn validate_for_template(&self) -> Result<(), ExecutionRuntimeError> {
        if self.version.trim().is_empty() {
            return Err(ExecutionRuntimeError::InvalidPlaywrightPlan(
                "version is required".into(),
            ));
        }
        if self.steps.is_empty() {
            return Err(ExecutionRuntimeError::InvalidPlaywrightPlan(
                "at least one step is required".into(),
            ));
        }
        for (index, step) in self.steps.iter().enumerate() {
            if step.id.trim().is_empty() {
                return Err(ExecutionRuntimeError::InvalidPlaywrightPlan(format!(
                    "step {index} id is required"
                )));
            }
            match step.action {
                PlaywrightStepAction::Navigate => {
                    require_field(&step.value, "value", index)?;
                }
                PlaywrightStepAction::Click
                | PlaywrightStepAction::Fill
                | PlaywrightStepAction::Press
                | PlaywrightStepAction::ExpectVisible
                | PlaywrightStepAction::ExpectText => {
                    require_field(&step.target, "target", index)?;
                }
                PlaywrightStepAction::ExpectUrl => {
                    require_field(&step.assertion, "assertion", index)?;
                }
            }
        }
        Ok(())
    }
}

impl StructuredFailure {
    pub fn validate(&self) -> Result<(), ExecutionRuntimeError> {
        if self.reason.trim().is_empty() {
            return Err(ExecutionRuntimeError::InvalidStructuredFailure(
                "reason is required".into(),
            ));
        }
        if self.failed_step_id.is_none() && self.failed_step_index.is_none() {
            return Err(ExecutionRuntimeError::InvalidStructuredFailure(
                "failed step id or index is required".into(),
            ));
        }
        Ok(())
    }
}

#[allow(dead_code)]
pub fn validate_traceability(
    run_result: &ErlExecutionRunResult,
    analysis: &AiFailureAnalysisInput,
) -> Result<(), ExecutionRuntimeError> {
    if analysis.trace_id != run_result.trace_id {
        return Err(ExecutionRuntimeError::TraceabilityViolation(
            "AI analysis trace_id does not match execution result".into(),
        ));
    }
    if analysis.execution_id != run_result.execution_id {
        return Err(ExecutionRuntimeError::TraceabilityViolation(
            "AI analysis execution_id does not match execution result".into(),
        ));
    }
    for artifact in &run_result.artifacts {
        if artifact.metadata.get("trace_id").and_then(Value::as_str) != Some(run_result.trace_id.as_str()) {
            return Err(ExecutionRuntimeError::TraceabilityViolation(format!(
                "artifact '{}' is missing matching trace_id",
                artifact.artifact_ref
            )));
        }
    }
    analysis.failure.validate()?;
    Ok(())
}

pub trait IsolationAdapter: Send + Sync {
    fn capabilities(&self) -> ExecutionBackendCapabilities;
    fn create_execution(
        &self,
        request: &ErlCreateExecutionRequest,
    ) -> Result<ErlCreateExecutionAccepted, ExecutionRuntimeError>;
    fn run_execution(
        &self,
        execution_id: &str,
        request: &ErlCreateExecutionRequest,
    ) -> Result<ErlExecutionRunResult, ExecutionRuntimeError>;
    fn cleanup_execution(&self, request: &ErlCleanupRetryRequest) -> Result<String, ExecutionRuntimeError>;
}

#[derive(Debug, Clone)]
pub struct K8sIsolationAdapter;

impl K8sIsolationAdapter {
    pub fn isolation_plan(
        &self,
        execution_id: &str,
        request: &ErlCreateExecutionRequest,
    ) -> Result<K8sIsolationPlan, ExecutionRuntimeError> {
        validate_snapshot_ref(&request.snapshot_ref)?;
        enforce_hard_limits(request)?;
        let namespace = format!("execution-{}", k8s_safe_name(execution_id)?);
        let quota = resource_quota(&request.resource_profile);
        let network_profile = request.network_profile.as_deref().unwrap_or("default");
        Ok(K8sIsolationPlan {
            namespace,
            resource_quota: quota.clone(),
            limit_range: K8sLimitRange {
                default_cpu: "500m".to_owned(),
                default_memory: "512Mi".to_owned(),
                max_cpu: quota.cpu_limit.clone(),
                max_memory: quota.memory_limit.clone(),
            },
            network_policy: K8sNetworkPolicy {
                profile: network_profile.to_owned(),
                default_deny_ingress: true,
                default_deny_egress: network_profile == "none",
            },
            pod_security: K8sPodSecurity {
                enforce_level: "restricted".to_owned(),
                run_as_non_root: true,
                read_only_root_filesystem: true,
                allow_privilege_escalation: false,
            },
            privileged_allowed: false,
            docker_socket_allowed: false,
        })
    }
}

impl IsolationAdapter for K8sIsolationAdapter {
    fn capabilities(&self) -> ExecutionBackendCapabilities {
        ExecutionBackendCapabilities {
            backend: ExecutionBackendKind::K8s,
            degraded_isolation: false,
            snapshot_only_input: true,
        }
    }

    fn create_execution(
        &self,
        request: &ErlCreateExecutionRequest,
    ) -> Result<ErlCreateExecutionAccepted, ExecutionRuntimeError> {
        validate_snapshot_ref(&request.snapshot_ref)?;
        enforce_hard_limits(request)?;
        Ok(ErlCreateExecutionAccepted {
            initial_status: "snapshot_resolved".to_owned(),
            capabilities: self.capabilities(),
        })
    }

    fn run_execution(
        &self,
        execution_id: &str,
        request: &ErlCreateExecutionRequest,
    ) -> Result<ErlExecutionRunResult, ExecutionRuntimeError> {
        validate_snapshot_ref(&request.snapshot_ref)?;
        enforce_hard_limits(request)?;
        let preview_url = if request.execution_type == "preview_env" {
            Some(format!("/api/executions/{execution_id}/preview"))
        } else {
            None
        };
        Ok(ErlExecutionRunResult {
            execution_id: execution_id.to_owned(),
            trace_id: request.trace_id.clone(),
            status: "provisioning".to_owned(),
            capabilities: self.capabilities(),
            preview_url,
            artifacts: Vec::new(),
            cleanup_status: "pending".to_owned(),
        })
    }

    fn cleanup_execution(&self, _request: &ErlCleanupRetryRequest) -> Result<String, ExecutionRuntimeError> {
        Ok("cleanup".to_owned())
    }
}

pub struct ExecutionRuntimeLayer {
    adapter: Box<dyn IsolationAdapter>,
}

impl ExecutionRuntimeLayer {
    pub fn new(adapter: Box<dyn IsolationAdapter>) -> Self {
        Self { adapter }
    }

    pub fn default_k8s() -> Self {
        Self::new(Box::new(K8sIsolationAdapter))
    }

    pub fn explicit_local(enabled: bool) -> Self {
        Self::new(Box::new(LocalIsolationAdapter::new(enabled)))
    }

    pub fn create_execution(
        &self,
        request: &ErlCreateExecutionRequest,
    ) -> Result<ErlCreateExecutionAccepted, ExecutionRuntimeError> {
        self.adapter.create_execution(request)
    }

    pub fn run_execution(
        &self,
        execution_id: &str,
        request: &ErlCreateExecutionRequest,
    ) -> Result<ErlExecutionRunResult, ExecutionRuntimeError> {
        self.adapter.run_execution(execution_id, request)
    }

    pub fn cleanup_execution(&self, request: &ErlCleanupRetryRequest) -> Result<String, ExecutionRuntimeError> {
        self.adapter.cleanup_execution(request)
    }

    pub fn retry_cleanup(&self, request: &ErlCleanupRetryRequest) -> ErlCleanupOutcome {
        match self.cleanup_execution(request) {
            Ok(status) => ErlCleanupOutcome {
                execution_id: request.execution_id.clone(),
                trace_id: request.trace_id.clone(),
                status,
                retryable: false,
                error_message: None,
            },
            Err(error) => ErlCleanupOutcome {
                execution_id: request.execution_id.clone(),
                trace_id: request.trace_id.clone(),
                status: "cleanup_failed".to_owned(),
                retryable: true,
                error_message: Some(error.to_string()),
            },
        }
    }
}

#[derive(Debug, Clone)]
pub struct LocalIsolationAdapter {
    enabled: bool,
    snapshot_store_root: PathBuf,
    runtime_root: PathBuf,
}

impl LocalIsolationAdapter {
    pub fn new(enabled: bool) -> Self {
        let temp_root = std::env::temp_dir().join("aion-erl-local");
        Self {
            enabled,
            snapshot_store_root: temp_root.join("snapshots"),
            runtime_root: temp_root.join("runs"),
        }
    }

    pub fn with_roots(enabled: bool, snapshot_store_root: PathBuf, runtime_root: PathBuf) -> Self {
        Self {
            enabled,
            snapshot_store_root,
            runtime_root,
        }
    }
}

impl IsolationAdapter for LocalIsolationAdapter {
    fn capabilities(&self) -> ExecutionBackendCapabilities {
        ExecutionBackendCapabilities {
            backend: ExecutionBackendKind::Local,
            degraded_isolation: true,
            snapshot_only_input: true,
        }
    }

    fn create_execution(
        &self,
        request: &ErlCreateExecutionRequest,
    ) -> Result<ErlCreateExecutionAccepted, ExecutionRuntimeError> {
        if !self.enabled {
            return Err(ExecutionRuntimeError::LocalBackendDisabled);
        }
        validate_snapshot_ref(&request.snapshot_ref)?;
        enforce_hard_limits(request)?;
        Ok(ErlCreateExecutionAccepted {
            initial_status: "snapshot_resolved".to_owned(),
            capabilities: self.capabilities(),
        })
    }

    fn run_execution(
        &self,
        execution_id: &str,
        request: &ErlCreateExecutionRequest,
    ) -> Result<ErlExecutionRunResult, ExecutionRuntimeError> {
        if !self.enabled {
            return Err(ExecutionRuntimeError::LocalBackendDisabled);
        }
        validate_snapshot_ref(&request.snapshot_ref)?;
        enforce_hard_limits(request)?;

        let run_root = self.runtime_root.join(safe_segment(execution_id)?);
        let work_dir = run_root.join("workspace");
        let artifact_dir = self.runtime_root.join("artifacts").join(safe_segment(execution_id)?);
        recreate_dir(&run_root)?;
        recreate_dir(&artifact_dir)?;
        fs::create_dir_all(&work_dir)
            .map_err(|error| ExecutionRuntimeError::SnapshotMaterializationFailed(error.to_string()))?;

        let snapshot_content = join_ref(&self.snapshot_store_root, &request.snapshot_ref.artifact_ref)?;
        copy_snapshot_content(&snapshot_content, &work_dir)?;

        let artifacts = collect_local_artifacts(execution_id, request, &artifact_dir)?;
        let preview_url = if request.execution_type == "preview_env" {
            Some(format!("/api/executions/{execution_id}/preview"))
        } else {
            None
        };
        let status = if request.execution_type == "preview_env" {
            "running"
        } else {
            "succeeded"
        };
        let cleanup_status = if request.execution_type == "test_run" {
            remove_dir_if_exists(&run_root)?;
            "cleanup"
        } else {
            "pending"
        };

        Ok(ErlExecutionRunResult {
            execution_id: execution_id.to_owned(),
            trace_id: request.trace_id.clone(),
            status: status.to_owned(),
            capabilities: self.capabilities(),
            preview_url,
            artifacts,
            cleanup_status: cleanup_status.to_owned(),
        })
    }

    fn cleanup_execution(&self, request: &ErlCleanupRetryRequest) -> Result<String, ExecutionRuntimeError> {
        if !self.enabled {
            return Err(ExecutionRuntimeError::LocalBackendDisabled);
        }
        let run_root = self.runtime_root.join(safe_segment(&request.execution_id)?);
        remove_dir_if_exists(&run_root)?;
        Ok("cleanup".to_owned())
    }
}

fn collect_local_artifacts(
    execution_id: &str,
    request: &ErlCreateExecutionRequest,
    artifact_dir: &Path,
) -> Result<Vec<ErlRuntimeArtifact>, ExecutionRuntimeError> {
    let mut artifacts = Vec::new();

    let server_log = format!(
        "trace_id={} execution_id={} backend=local status=started\n",
        request.trace_id, execution_id
    );
    artifacts.push(write_runtime_artifact(
        artifact_dir,
        execution_id,
        "server_log",
        "server.log",
        server_log.as_bytes(),
        json_meta(request, None),
    )?);

    if request.execution_type == "test_run" {
        if let Some(plan) = request.policy.get("playwright_test_plan") {
            let plan: PlaywrightTestPlan = serde_json::from_value(plan.clone())
                .map_err(|error| ExecutionRuntimeError::InvalidPlaywrightPlan(format!("invalid json: {error}")))?;
            plan.validate_for_template()?;
            let plan_bytes = serde_json::to_vec_pretty(&plan)
                .map_err(|error| ExecutionRuntimeError::InvalidPlaywrightPlan(format!("serialize plan: {error}")))?;
            artifacts.push(write_runtime_artifact(
                artifact_dir,
                execution_id,
                "playwright_plan",
                "playwright-plan.json",
                &plan_bytes,
                json_meta(request, None),
            )?);
        }

        artifacts.push(write_runtime_artifact(
            artifact_dir,
            execution_id,
            "console_log",
            "console.log",
            b"local playwright template completed\n",
            json_meta(request, None),
        )?);
        artifacts.push(write_runtime_artifact(
            artifact_dir,
            execution_id,
            "playwright_trace",
            "trace.zip",
            b"placeholder trace archive for local backend\n",
            json_meta(request, None),
        )?);
        artifacts.push(write_runtime_artifact(
            artifact_dir,
            execution_id,
            "screenshot",
            "screenshot.png",
            b"placeholder screenshot for local backend\n",
            json_meta(request, None),
        )?);
        artifacts.push(write_runtime_artifact(
            artifact_dir,
            execution_id,
            "video",
            "video.webm",
            b"placeholder video for local backend\n",
            json_meta(request, None),
        )?);

        if let Some(failure) = request.policy.get("structured_failure") {
            let failure: StructuredFailure = serde_json::from_value(failure.clone())
                .map_err(|error| ExecutionRuntimeError::InvalidStructuredFailure(format!("invalid json: {error}")))?;
            failure.validate()?;
            let failure_bytes = serde_json::to_vec_pretty(&failure).map_err(|error| {
                ExecutionRuntimeError::InvalidStructuredFailure(format!("serialize failure: {error}"))
            })?;
            artifacts.push(write_runtime_artifact(
                artifact_dir,
                execution_id,
                "structured_failure",
                "structured-failure.json",
                &failure_bytes,
                json_meta(request, failure.exit_code),
            )?);
        }
    }

    Ok(artifacts)
}

fn write_runtime_artifact(
    artifact_dir: &Path,
    execution_id: &str,
    artifact_type: &str,
    file_name: &str,
    content: &[u8],
    metadata: Value,
) -> Result<ErlRuntimeArtifact, ExecutionRuntimeError> {
    let path = artifact_dir.join(file_name);
    fs::write(&path, content)
        .map_err(|error| ExecutionRuntimeError::SnapshotMaterializationFailed(error.to_string()))?;
    let size_bytes = content.len() as u64;
    Ok(ErlRuntimeArtifact {
        artifact_type: artifact_type.to_owned(),
        artifact_ref: format!("executions/{execution_id}/artifacts/{file_name}"),
        size_bytes,
        immutable: true,
        metadata,
    })
}

fn json_meta(request: &ErlCreateExecutionRequest, exit_code: Option<i32>) -> Value {
    serde_json::json!({
        "trace_id": request.trace_id,
        "execution_type": request.execution_type,
        "snapshot_id": request.snapshot_ref.snapshot_id,
        "backend": "local",
        "degraded_isolation": true,
        "exit_code": exit_code
    })
}

fn join_ref(root: &Path, artifact_ref: &str) -> Result<PathBuf, ExecutionRuntimeError> {
    let relative = Path::new(artifact_ref);
    if relative.is_absolute() || artifact_ref.contains("..") {
        return Err(ExecutionRuntimeError::SnapshotMaterializationFailed(
            "snapshot artifact_ref must be a relative store reference".into(),
        ));
    }
    Ok(root.join(relative))
}

fn safe_segment(value: &str) -> Result<String, ExecutionRuntimeError> {
    if value.trim().is_empty() || value.contains('/') || value.contains('\\') || value.contains("..") {
        return Err(ExecutionRuntimeError::SnapshotMaterializationFailed(
            "execution id must be a safe path segment".into(),
        ));
    }
    Ok(value.to_owned())
}

fn recreate_dir(path: &Path) -> Result<(), ExecutionRuntimeError> {
    remove_dir_if_exists(path)?;
    fs::create_dir_all(path).map_err(|error| ExecutionRuntimeError::SnapshotMaterializationFailed(error.to_string()))
}

fn remove_dir_if_exists(path: &Path) -> Result<(), ExecutionRuntimeError> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(ExecutionRuntimeError::CleanupFailed(error.to_string())),
    }
}

fn copy_snapshot_content(source: &Path, destination: &Path) -> Result<(), ExecutionRuntimeError> {
    if !source.exists() {
        return Err(ExecutionRuntimeError::SnapshotMaterializationFailed(format!(
            "snapshot content '{}' not found",
            source.display()
        )));
    }
    copy_dir_recursive(source, destination)
}

fn copy_dir_recursive(source: &Path, destination: &Path) -> Result<(), ExecutionRuntimeError> {
    for entry in
        fs::read_dir(source).map_err(|error| ExecutionRuntimeError::SnapshotMaterializationFailed(error.to_string()))?
    {
        let entry = entry.map_err(|error| ExecutionRuntimeError::SnapshotMaterializationFailed(error.to_string()))?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let metadata = entry
            .metadata()
            .map_err(|error| ExecutionRuntimeError::SnapshotMaterializationFailed(error.to_string()))?;
        if metadata.is_dir() {
            fs::create_dir_all(&destination_path)
                .map_err(|error| ExecutionRuntimeError::SnapshotMaterializationFailed(error.to_string()))?;
            copy_dir_recursive(&source_path, &destination_path)?;
        } else if metadata.is_file() {
            fs::copy(&source_path, &destination_path)
                .map_err(|error| ExecutionRuntimeError::SnapshotMaterializationFailed(error.to_string()))?;
        }
    }
    Ok(())
}

fn k8s_safe_name(value: &str) -> Result<String, ExecutionRuntimeError> {
    let mut name = String::with_capacity(value.len());
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            name.push(ch.to_ascii_lowercase());
        } else if ch == '-' || ch == '_' {
            name.push('-');
        }
    }
    let name = name.trim_matches('-').to_owned();
    if name.is_empty() {
        return Err(ExecutionRuntimeError::SnapshotMaterializationFailed(
            "execution id must contain k8s-safe characters".into(),
        ));
    }
    Ok(name)
}

fn resource_quota(profile: &Value) -> K8sResourceQuota {
    if let Some(named) = profile.as_str() {
        return match named {
            "large" => K8sResourceQuota {
                cpu_limit: "4000m".to_owned(),
                memory_limit: "8Gi".to_owned(),
                ephemeral_storage_limit: "20Gi".to_owned(),
            },
            "medium" => K8sResourceQuota {
                cpu_limit: "2000m".to_owned(),
                memory_limit: "4Gi".to_owned(),
                ephemeral_storage_limit: "10Gi".to_owned(),
            },
            _ => K8sResourceQuota {
                cpu_limit: "1000m".to_owned(),
                memory_limit: "2Gi".to_owned(),
                ephemeral_storage_limit: "5Gi".to_owned(),
            },
        };
    }

    K8sResourceQuota {
        cpu_limit: profile
            .get("cpu_limit")
            .and_then(Value::as_str)
            .unwrap_or("1000m")
            .to_owned(),
        memory_limit: profile
            .get("memory_limit")
            .and_then(Value::as_str)
            .unwrap_or("2Gi")
            .to_owned(),
        ephemeral_storage_limit: profile
            .get("ephemeral_storage_limit")
            .and_then(Value::as_str)
            .unwrap_or("5Gi")
            .to_owned(),
    }
}

fn validate_snapshot_ref(snapshot_ref: &SnapshotRef) -> Result<(), ExecutionRuntimeError> {
    if snapshot_ref.snapshot_id.trim().is_empty()
        || snapshot_ref.artifact_ref.trim().is_empty()
        || snapshot_ref.manifest_ref.trim().is_empty()
        || snapshot_ref.checksum.trim().is_empty()
    {
        return Err(ExecutionRuntimeError::MissingSnapshotRef);
    }
    Ok(())
}

fn enforce_hard_limits(request: &ErlCreateExecutionRequest) -> Result<(), ExecutionRuntimeError> {
    const MAX_TIME_SECONDS: i64 = 7_200;
    const MAX_LOG_BYTES: i64 = 50 * 1024 * 1024;
    const MAX_ARTIFACT_BYTES: i64 = 500 * 1024 * 1024;
    const MAX_AI_REPAIR_RETRY: u64 = 20;
    const ALLOWED_RESOURCE_PROFILES: &[&str] = &["small", "medium", "large"];
    let auto_fix = request.policy.get("auto_fix").unwrap_or(&request.policy);

    if let Some(value) = integer_field(&request.policy, &["max_time_seconds", "max_time"])
        && value > MAX_TIME_SECONDS
    {
        return Err(ExecutionRuntimeError::PolicyLimitExceeded(format!(
            "max_time_seconds {value} exceeds {MAX_TIME_SECONDS}"
        )));
    }

    if let Some(value) = integer_field(&request.policy, &["log_limit_bytes", "max_log_bytes"])
        && value > MAX_LOG_BYTES
    {
        return Err(ExecutionRuntimeError::PolicyLimitExceeded(format!(
            "log limit {value} exceeds {MAX_LOG_BYTES}"
        )));
    }

    if let Some(value) = integer_field(&request.policy, &["artifact_limit_bytes", "max_artifact_bytes"])
        && value > MAX_ARTIFACT_BYTES
    {
        return Err(ExecutionRuntimeError::PolicyLimitExceeded(format!(
            "artifact limit {value} exceeds {MAX_ARTIFACT_BYTES}"
        )));
    }

    if let Some(value) = unsigned_field(auto_fix, &["max_retry"])
        && value > MAX_AI_REPAIR_RETRY
    {
        return Err(ExecutionRuntimeError::PolicyLimitExceeded(format!(
            "max_retry {value} exceeds {MAX_AI_REPAIR_RETRY}"
        )));
    }

    if let Some(profile) = request.resource_profile.as_str()
        && !ALLOWED_RESOURCE_PROFILES.contains(&profile)
    {
        return Err(ExecutionRuntimeError::PolicyLimitExceeded(format!(
            "resource_profile '{profile}' is not allowed"
        )));
    }

    Ok(())
}

fn integer_field(value: &Value, keys: &[&str]) -> Option<i64> {
    keys.iter().find_map(|key| value.get(*key)).and_then(Value::as_i64)
}

fn unsigned_field(value: &Value, keys: &[&str]) -> Option<u64> {
    keys.iter().find_map(|key| value.get(*key)).and_then(Value::as_u64)
}

fn unsigned_field_as_u32(value: &Value, keys: &[&str], default: u32) -> u32 {
    unsigned_field(value, keys)
        .map(|value| u32::try_from(value).unwrap_or(u32::MAX))
        .unwrap_or(default)
}

fn require_field(value: &Option<String>, field: &str, step_index: usize) -> Result<(), ExecutionRuntimeError> {
    if value.as_deref().map(str::trim).unwrap_or_default().is_empty() {
        return Err(ExecutionRuntimeError::InvalidPlaywrightPlan(format!(
            "step {step_index} {field} is required"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn request() -> ErlCreateExecutionRequest {
        ErlCreateExecutionRequest {
            snapshot_ref: SnapshotRef {
                snapshot_id: "snap_1".into(),
                artifact_ref: "snapshots/snap_1/content".into(),
                manifest_ref: "snapshots/snap_1/manifest.json".into(),
                checksum: "abc".into(),
            },
            execution_type: "test_run".into(),
            policy: json!({ "max_retry": 3 }),
            resource_profile: json!({ "cpu": "1" }),
            network_profile: Some("default".into()),
            env_refs: vec!["env_1".into()],
            trace_id: "trace_1".into(),
            caller: ExecutionCallerContext { user_id: "u1".into() },
        }
    }

    #[test]
    fn k8s_adapter_accepts_snapshot_only_input() {
        let accepted = ExecutionRuntimeLayer::default_k8s()
            .create_execution(&request())
            .unwrap();
        assert_eq!(accepted.initial_status, "snapshot_resolved");
        assert_eq!(accepted.capabilities.backend, ExecutionBackendKind::K8s);
        assert!(accepted.capabilities.snapshot_only_input);
        assert!(!accepted.capabilities.degraded_isolation);
    }

    #[test]
    fn k8s_adapter_builds_namespace_per_execution_isolation_plan() {
        let mut request = request();
        request.resource_profile = json!("medium");
        request.network_profile = Some("none".into());

        let plan = K8sIsolationAdapter.isolation_plan("Exec_ABC_1", &request).unwrap();

        assert_eq!(plan.namespace, "execution-exec-abc-1");
        assert_eq!(plan.resource_quota.cpu_limit, "2000m");
        assert_eq!(plan.resource_quota.memory_limit, "4Gi");
        assert_eq!(plan.limit_range.max_memory, "4Gi");
        assert_eq!(plan.network_policy.profile, "none");
        assert!(plan.network_policy.default_deny_ingress);
        assert!(plan.network_policy.default_deny_egress);
        assert_eq!(plan.pod_security.enforce_level, "restricted");
        assert!(plan.pod_security.run_as_non_root);
        assert!(plan.pod_security.read_only_root_filesystem);
        assert!(!plan.pod_security.allow_privilege_escalation);
        assert!(!plan.privileged_allowed);
        assert!(!plan.docker_socket_allowed);
    }

    #[test]
    fn local_backend_requires_explicit_enablement() {
        let error = ExecutionRuntimeLayer::explicit_local(false)
            .create_execution(&request())
            .unwrap_err();
        assert!(matches!(error, ExecutionRuntimeError::LocalBackendDisabled));
    }

    #[test]
    fn adapters_reject_missing_snapshot_ref() {
        let mut request = request();
        request.snapshot_ref.artifact_ref.clear();
        let error = ExecutionRuntimeLayer::default_k8s()
            .create_execution(&request)
            .unwrap_err();
        assert!(matches!(error, ExecutionRuntimeError::MissingSnapshotRef));
    }

    #[test]
    fn adapter_enforces_hard_time_limit() {
        let mut request = request();
        request.policy = json!({ "max_time_seconds": 7_201 });
        let error = ExecutionRuntimeLayer::default_k8s()
            .create_execution(&request)
            .unwrap_err();
        assert!(matches!(error, ExecutionRuntimeError::PolicyLimitExceeded(_)));
    }

    #[test]
    fn adapter_enforces_ai_repair_retry_limit() {
        let mut request = request();
        request.policy = json!({
            "auto_fix": {
                "enabled": true,
                "max_retry": 21,
                "max_time_seconds": 600
            }
        });
        let error = ExecutionRuntimeLayer::default_k8s()
            .create_execution(&request)
            .unwrap_err();
        assert!(matches!(error, ExecutionRuntimeError::PolicyLimitExceeded(_)));
    }

    #[test]
    fn adapter_rejects_unknown_named_resource_profile() {
        let mut request = request();
        request.resource_profile = json!("unbounded");
        let error = ExecutionRuntimeLayer::default_k8s()
            .create_execution(&request)
            .unwrap_err();
        assert!(matches!(error, ExecutionRuntimeError::PolicyLimitExceeded(_)));
    }

    #[test]
    fn create_request_serialization_excludes_product_state_inputs() {
        let value = serde_json::to_value(request()).unwrap();
        assert!(value.get("snapshot_ref").is_some());
        assert!(value.get("workspace_path").is_none());
        assert!(value.get("workspace_root").is_none());
        assert!(value.get("git_repo_url").is_none());
        assert!(value.get("git_clone_url").is_none());
        assert!(value.get("ssh_credential_id").is_none());
        assert!(value.get("credential_id").is_none());
        assert!(value.get("private_key").is_none());
        assert!(value.get("database_url").is_none());
        assert!(value.get("product_database").is_none());
        assert!(value.get("k8s_namespace").is_none());
        assert!(value.get("host_port").is_none());
        assert!(value.to_string().contains("snapshots/snap_1/content"));
        assert!(!value.to_string().contains("/data/senmo"));
        assert!(!value.to_string().contains("git@"));
    }

    #[test]
    fn ai_repair_policy_stops_at_retry_limit() {
        let policy = AiRepairPolicy::from_execution_policy(&json!({
            "auto_fix": {
                "enabled": true,
                "max_retry": 3,
                "max_time_seconds": 600,
                "require_human_approval_after": 2
            }
        }));

        assert_eq!(
            policy.decide(&AiRepairAttemptState {
                retry_count: 3,
                elapsed_seconds: 10,
                human_approved: true
            }),
            AiRepairDecision::StopMaxRetry
        );
    }

    #[test]
    fn ai_repair_policy_saturates_oversized_retry_fields() {
        let policy = AiRepairPolicy::from_execution_policy(&json!({
            "auto_fix": {
                "enabled": true,
                "max_retry": 4_294_967_296_u64,
                "max_time_seconds": 600,
                "require_human_approval_after": 4_294_967_296_u64
            }
        }));

        assert_eq!(policy.max_retry, u32::MAX);
        assert_eq!(policy.require_human_approval_after, u32::MAX);
        assert_eq!(
            policy.decide(&AiRepairAttemptState {
                retry_count: 0,
                elapsed_seconds: 10,
                human_approved: false
            }),
            AiRepairDecision::Continue
        );
    }

    #[test]
    fn ai_repair_policy_requires_human_approval_after_threshold() {
        let policy = AiRepairPolicy::from_execution_policy(&json!({
            "enabled": true,
            "max_retry": 3,
            "max_time_seconds": 600,
            "require_human_approval_after": 2
        }));

        assert_eq!(
            policy.decide(&AiRepairAttemptState {
                retry_count: 2,
                elapsed_seconds: 10,
                human_approved: false
            }),
            AiRepairDecision::WaitHumanApproval
        );
        assert_eq!(
            policy.decide(&AiRepairAttemptState {
                retry_count: 2,
                elapsed_seconds: 10,
                human_approved: true
            }),
            AiRepairDecision::Continue
        );
    }

    #[test]
    fn playwright_test_plan_accepts_fixed_template_actions() {
        let plan = PlaywrightTestPlan {
            version: "workbench.playwright.v1".into(),
            steps: vec![
                PlaywrightTestStep {
                    id: "open-login".into(),
                    action: PlaywrightStepAction::Navigate,
                    target: None,
                    value: Some("/login".into()),
                    assertion: None,
                },
                PlaywrightTestStep {
                    id: "submit".into(),
                    action: PlaywrightStepAction::Click,
                    target: Some("button[type=submit]".into()),
                    value: None,
                    assertion: None,
                },
            ],
        };

        plan.validate_for_template().unwrap();
    }

    #[test]
    fn playwright_test_plan_rejects_freeform_or_incomplete_steps() {
        let plan = PlaywrightTestPlan {
            version: "workbench.playwright.v1".into(),
            steps: vec![PlaywrightTestStep {
                id: "bad-click".into(),
                action: PlaywrightStepAction::Click,
                target: None,
                value: None,
                assertion: None,
            }],
        };

        let error = plan.validate_for_template().unwrap_err();
        assert!(matches!(error, ExecutionRuntimeError::InvalidPlaywrightPlan(_)));
    }

    #[test]
    fn structured_failure_requires_failed_step_and_reason() {
        let failure = StructuredFailure {
            failed_step_id: Some("submit".into()),
            failed_step_index: Some(1),
            reason: "HTTP 500 after clicking submit".into(),
            trace_ref: Some("artifacts/ex_1/trace.zip".into()),
            screenshot_ref: Some("artifacts/ex_1/failure.png".into()),
            log_refs: vec!["artifacts/ex_1/console.log".into()],
            exit_code: Some(1),
        };

        failure.validate().unwrap();

        let invalid = StructuredFailure {
            failed_step_id: None,
            failed_step_index: None,
            reason: "".into(),
            trace_ref: None,
            screenshot_ref: None,
            log_refs: Vec::new(),
            exit_code: None,
        };
        let error = invalid.validate().unwrap_err();
        assert!(matches!(error, ExecutionRuntimeError::InvalidStructuredFailure(_)));
    }

    #[test]
    fn local_backend_materializes_snapshot_collects_test_artifacts_and_cleans_runtime() {
        let temp = tempfile::tempdir().unwrap();
        let snapshot_root = temp.path().join("snapshot-store");
        let runtime_root = temp.path().join("runtime");
        let snapshot_content = snapshot_root.join("snapshots/snap_1/content");
        fs::create_dir_all(&snapshot_content).unwrap();
        fs::write(snapshot_content.join("app.txt"), "sealed").unwrap();

        let runtime = ExecutionRuntimeLayer::new(Box::new(LocalIsolationAdapter::with_roots(
            true,
            snapshot_root,
            runtime_root.clone(),
        )));
        let mut request = request();
        request.policy = json!({
            "max_time_seconds": 60,
            "playwright_test_plan": {
                "version": "workbench.playwright.v1",
                "steps": [
                    {
                        "id": "open-login",
                        "action": "navigate",
                        "value": "/login"
                    },
                    {
                        "id": "submit",
                        "action": "click",
                        "target": "button[type=submit]"
                    }
                ]
            },
            "structured_failure": {
                "failed_step_id": "submit",
                "failed_step_index": 1,
                "reason": "button missing",
                "trace_ref": "executions/exec_1/artifacts/trace.zip",
                "screenshot_ref": "executions/exec_1/artifacts/screenshot.png",
                "log_refs": ["executions/exec_1/artifacts/console.log"],
                "exit_code": 1
            }
        });

        let result = runtime.run_execution("exec_1", &request).unwrap();

        assert_eq!(result.status, "succeeded");
        assert_eq!(result.cleanup_status, "cleanup");
        assert_eq!(result.trace_id, "trace_1");
        assert!(result.preview_url.is_none());
        assert_eq!(result.capabilities.backend, ExecutionBackendKind::Local);
        assert!(result.capabilities.degraded_isolation);
        assert!(!runtime_root.join("exec_1").exists());

        let artifact_types: Vec<_> = result
            .artifacts
            .iter()
            .map(|artifact| artifact.artifact_type.as_str())
            .collect();
        assert!(artifact_types.contains(&"server_log"));
        assert!(artifact_types.contains(&"playwright_plan"));
        assert!(artifact_types.contains(&"console_log"));
        assert!(artifact_types.contains(&"playwright_trace"));
        assert!(artifact_types.contains(&"screenshot"));
        assert!(artifact_types.contains(&"video"));
        assert!(artifact_types.contains(&"structured_failure"));
        assert!(result.artifacts.iter().all(|artifact| artifact.immutable));
        assert!(
            result
                .artifacts
                .iter()
                .all(|artifact| !artifact.artifact_ref.starts_with('/'))
        );
        assert!(runtime_root.join("artifacts/exec_1/trace.zip").exists());
        assert!(runtime_root.join("artifacts/exec_1/structured-failure.json").exists());
        assert!(
            result
                .artifacts
                .iter()
                .all(|artifact| artifact.metadata["trace_id"] == "trace_1")
        );
    }

    #[test]
    fn local_preview_keeps_runtime_until_cleanup_and_uses_controlled_url() {
        let temp = tempfile::tempdir().unwrap();
        let snapshot_root = temp.path().join("snapshot-store");
        let runtime_root = temp.path().join("runtime");
        let snapshot_content = snapshot_root.join("snapshots/snap_1/content");
        fs::create_dir_all(&snapshot_content).unwrap();
        fs::write(snapshot_content.join("index.html"), "<h1>sealed</h1>").unwrap();

        let runtime = ExecutionRuntimeLayer::new(Box::new(LocalIsolationAdapter::with_roots(
            true,
            snapshot_root,
            runtime_root.clone(),
        )));
        let mut request = request();
        request.execution_type = "preview_env".into();

        let result = runtime.run_execution("exec_preview", &request).unwrap();

        assert_eq!(result.status, "running");
        assert_eq!(result.cleanup_status, "pending");
        assert_eq!(
            result.preview_url.as_deref(),
            Some("/api/executions/exec_preview/preview")
        );
        assert!(runtime_root.join("exec_preview/workspace/index.html").exists());
        assert!(!result.preview_url.as_deref().unwrap().contains("127.0.0.1"));

        let cleanup_status = runtime
            .cleanup_execution(&ErlCleanupRetryRequest {
                execution_id: "exec_preview".into(),
                trace_id: "trace_1".into(),
            })
            .unwrap();
        assert_eq!(cleanup_status, "cleanup");
        assert!(!runtime_root.join("exec_preview").exists());
    }

    #[test]
    fn cleanup_retry_reports_cleanup_failed_and_can_retry_successfully() {
        let temp = tempfile::tempdir().unwrap();
        let snapshot_root = temp.path().join("snapshot-store");
        let runtime_root = temp.path().join("runtime");
        fs::create_dir_all(&runtime_root).unwrap();
        fs::write(runtime_root.join("exec_stuck"), "not a directory").unwrap();
        let runtime = ExecutionRuntimeLayer::new(Box::new(LocalIsolationAdapter::with_roots(
            true,
            snapshot_root,
            runtime_root.clone(),
        )));
        let request = ErlCleanupRetryRequest {
            execution_id: "exec_stuck".into(),
            trace_id: "trace_cleanup".into(),
        };

        let failed = runtime.retry_cleanup(&request);

        assert_eq!(failed.status, "cleanup_failed");
        assert_eq!(failed.trace_id, "trace_cleanup");
        assert!(failed.retryable);
        assert!(failed.error_message.unwrap().contains("runtime cleanup failed"));

        fs::remove_file(runtime_root.join("exec_stuck")).unwrap();
        fs::create_dir_all(runtime_root.join("exec_stuck/workspace")).unwrap();
        let retried = runtime.retry_cleanup(&request);

        assert_eq!(retried.status, "cleanup");
        assert!(!retried.retryable);
        assert!(retried.error_message.is_none());
        assert!(!runtime_root.join("exec_stuck").exists());
    }

    #[test]
    fn traceability_links_execution_artifacts_ai_analysis_and_retry_attempt() {
        let temp = tempfile::tempdir().unwrap();
        let snapshot_root = temp.path().join("snapshot-store");
        let runtime_root = temp.path().join("runtime");
        let snapshot_content = snapshot_root.join("snapshots/snap_1/content");
        fs::create_dir_all(&snapshot_content).unwrap();
        fs::write(snapshot_content.join("app.txt"), "sealed").unwrap();
        let runtime = ExecutionRuntimeLayer::new(Box::new(LocalIsolationAdapter::with_roots(
            true,
            snapshot_root,
            runtime_root,
        )));
        let mut request = request();
        request.policy = json!({
            "structured_failure": {
                "failed_step_id": "submit",
                "failed_step_index": 1,
                "reason": "API 500",
                "trace_ref": "executions/exec_trace/artifacts/trace.zip",
                "screenshot_ref": "executions/exec_trace/artifacts/screenshot.png",
                "log_refs": ["executions/exec_trace/artifacts/console.log"],
                "exit_code": 1
            }
        });
        let result = runtime.run_execution("exec_trace", &request).unwrap();
        let analysis = AiFailureAnalysisInput {
            trace_id: "trace_1".into(),
            execution_id: "exec_trace".into(),
            retry_attempt: 1,
            failure: StructuredFailure {
                failed_step_id: Some("submit".into()),
                failed_step_index: Some(1),
                reason: "API 500".into(),
                trace_ref: Some("executions/exec_trace/artifacts/trace.zip".into()),
                screenshot_ref: Some("executions/exec_trace/artifacts/screenshot.png".into()),
                log_refs: vec!["executions/exec_trace/artifacts/console.log".into()],
                exit_code: Some(1),
            },
        };

        validate_traceability(&result, &analysis).unwrap();

        let mut wrong_trace = analysis.clone();
        wrong_trace.trace_id = "trace_other".into();
        let error = validate_traceability(&result, &wrong_trace).unwrap_err();
        assert!(matches!(error, ExecutionRuntimeError::TraceabilityViolation(_)));
    }
}
