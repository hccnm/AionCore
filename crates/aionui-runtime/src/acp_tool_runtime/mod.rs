mod types;

#[cfg(test)]
use std::error::Error as StdError;
use std::fs::{self};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::Builder;
use crate::cache;
use crate::managed_resources;
use crate::node_runtime::DoctorRow;
use crate::node_runtime::ensure_node_runtime_with_reporter;

pub use types::{
    ManagedAcpToolError, ManagedAcpToolFailureKind, ManagedAcpToolId, ManagedAcpToolProgress,
    ManagedAcpToolProgressPhase, ManagedAcpToolProgressReporter, ManagedAcpToolSupport, ResolvedManagedAcpTool,
    SharedManagedAcpToolProgressReporter,
};

static INSTALL_LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
const MANAGED_ACP_SMOKE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);
const LOCAL_SOURCE_ENV_PREFIX: &str = "AIONUI_MANAGED_ACP_SOURCE_";
const LOCAL_TARBALL_ENV_PREFIX: &str = "AIONUI_MANAGED_ACP_TARBALL_";

#[derive(Debug, Clone, Copy)]
struct PlatformSpec {
    manifest_key: &'static str,
    npm_os: &'static str,
    npm_cpu: &'static str,
}

#[derive(Debug, Serialize)]
struct DevPackageJson<'a> {
    name: &'a str,
    private: bool,
}

#[derive(Debug, Deserialize)]
struct InstalledPackageJson {
    name: String,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    bin: serde_json::Value,
    #[serde(default)]
    main: Option<String>,
    #[serde(default)]
    exports: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PackageSmokeTarget {
    Import(PathBuf),
    SyntaxCheck(PathBuf),
}

#[derive(Debug, Serialize)]
struct LocalArtifactManifestWrite {
    entrypoint: String,
    path_entries: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum LocalManagedAcpOverride {
    SourceDir(PathBuf),
    Tarball(PathBuf),
}

pub async fn ensure_managed_acp_tool(tool: ManagedAcpToolId) -> Result<ResolvedManagedAcpTool, ManagedAcpToolError> {
    ensure_managed_acp_tool_with_reporter(tool, None).await
}

pub async fn ensure_managed_acp_tool_with_reporter(
    tool: ManagedAcpToolId,
    reporter: Option<&dyn ManagedAcpToolProgressReporter>,
) -> Result<ResolvedManagedAcpTool, ManagedAcpToolError> {
    let spec = platform_spec()?;
    let root = tool_root(tool, spec)?;
    let local_override = local_managed_acp_override(tool);

    if local_override.is_none()
        && let Ok(installed) = validate_tool_root(tool, &root, reporter)
    {
        return Ok(installed);
    }

    let lock = INSTALL_LOCK.get_or_init(|| tokio::sync::Mutex::new(()));
    let _guard = lock.lock().await;

    if local_override.is_none()
        && let Ok(installed) = validate_tool_root(tool, &root, reporter)
    {
        return Ok(installed);
    }

    if let Some(local_override) = local_override {
        return activate_local_override(tool, spec, &root, &local_override, reporter)
            .await
            .map_err(|error| report_failure(error, reporter));
    }

    if let Some(installed) =
        activate_local_tool_source(tool, spec, &root, reporter).map_err(|error| report_failure(error, reporter))?
    {
        return Ok(installed);
    }

    if maybe_prepare_local_runtime_tool_source(tool, spec, reporter)
        .await
        .map_err(|error| report_failure(error, reporter))?
    {
        return validate_tool_root(tool, &root, reporter).map_err(|error| report_failure(error, reporter));
    }

    Err(report_failure(unavailable_error(tool, &root), reporter))
}

pub async fn prepare_managed_acp_tool_to_root(
    tool: ManagedAcpToolId,
    root: &Path,
) -> Result<ResolvedManagedAcpTool, ManagedAcpToolError> {
    let spec = platform_spec()?;
    let target_root = bundle_tool_root(root, tool, spec);

    if let Some(local_override) = local_managed_acp_override(tool) {
        return prepare_local_override_to_bundle_root(tool, spec, root, &target_root, &local_override).await;
    }

    let node_runtime = ensure_node_runtime_with_reporter(None)
        .await
        .map_err(|error| ManagedAcpToolError::invalid(format!("prepare managed Node runtime: {error}")))?;
    prepare_package_spec_to_bundle_root(
        tool,
        spec,
        &node_runtime,
        root,
        &target_root,
        &format!("{}@{}", tool.package_name(), tool.version()),
    )
    .await
}

async fn prepare_package_spec_to_bundle_root(
    tool: ManagedAcpToolId,
    spec: PlatformSpec,
    node_runtime: &crate::ResolvedNodeRuntime,
    bundle_root: &Path,
    target_root: &Path,
    package_spec: &str,
) -> Result<ResolvedManagedAcpTool, ManagedAcpToolError> {
    let staging_root = bundle_prepare_staging_root(tool, spec, bundle_root);
    if staging_root.exists() {
        let _ = fs::remove_dir_all(&staging_root);
    }
    fs::create_dir_all(&staging_root).map_err(ManagedAcpToolError::io)?;

    let result =
        prepare_local_tool_source_to_root(tool, spec, node_runtime, &staging_root, target_root, package_spec).await;

    if let Err(error) = fs::remove_dir_all(&staging_root)
        && error.kind() != std::io::ErrorKind::NotFound
    {
        warn!(
            tool = tool.slug(),
            version = tool.version(),
            staging_root = %staging_root.display(),
            error = %error,
            "failed to clean up managed ACP bundle preparation staging directory"
        );
    }

    result
}

async fn prepare_local_override_to_bundle_root(
    tool: ManagedAcpToolId,
    spec: PlatformSpec,
    bundle_root: &Path,
    target_root: &Path,
    local_override: &LocalManagedAcpOverride,
) -> Result<ResolvedManagedAcpTool, ManagedAcpToolError> {
    match local_override {
        LocalManagedAcpOverride::SourceDir(source_root) if source_root.join("manifest.json").is_file() => {
            managed_resources::materialize_directory(source_root, target_root).map_err(ManagedAcpToolError::io)?;
            validate_tool_root(tool, target_root, None)
        }
        LocalManagedAcpOverride::SourceDir(source_root) => {
            if !source_root.join("package.json").is_file() {
                return Err(ManagedAcpToolError::invalid(format!(
                    "local {} source missing package.json under {}",
                    tool.display_name(),
                    source_root.display()
                )));
            }
            let node_runtime = ensure_node_runtime_with_reporter(None)
                .await
                .map_err(|error| ManagedAcpToolError::invalid(format!("prepare managed Node runtime: {error}")))?;
            prepare_local_project_source_to_bundle_root(tool, spec, &node_runtime, source_root, target_root).await
        }
        LocalManagedAcpOverride::Tarball(tarball_path) => {
            if !tarball_path.is_file() {
                return Err(ManagedAcpToolError::invalid(format!(
                    "local managed {} tarball missing: {}",
                    tool.display_name(),
                    tarball_path.display()
                )));
            }
            let node_runtime = ensure_node_runtime_with_reporter(None)
                .await
                .map_err(|error| ManagedAcpToolError::invalid(format!("prepare managed Node runtime: {error}")))?;
            prepare_package_spec_to_bundle_root(
                tool,
                spec,
                &node_runtime,
                bundle_root,
                target_root,
                &tarball_path.to_string_lossy(),
            )
            .await
        }
    }
}

async fn prepare_local_project_source_to_bundle_root(
    tool: ManagedAcpToolId,
    spec: PlatformSpec,
    node_runtime: &crate::ResolvedNodeRuntime,
    source_root: &Path,
    target_root: &Path,
) -> Result<ResolvedManagedAcpTool, ManagedAcpToolError> {
    let source_package_json = read_installed_package_json(&source_root.join("package.json"))?;
    if source_package_json.name != tool.package_name() {
        return Err(ManagedAcpToolError::invalid(format!(
            "local {} source expected package {}, found {}",
            tool.display_name(),
            tool.package_name(),
            source_package_json.name
        )));
    }
    let source_entrypoint = source_root.join(resolve_package_bin_entry(
        &source_package_json.name,
        &source_package_json.bin,
    )?);
    if !source_entrypoint.is_file() {
        return Err(ManagedAcpToolError::invalid(format!(
            "local {} entrypoint missing: {}",
            tool.display_name(),
            source_entrypoint.display()
        )));
    }
    validate_platform_binary(tool, source_root, spec)?;
    let source_smoke_target = resolve_package_smoke_target(source_root, &source_package_json)?;
    validate_package_smoke_target(node_runtime, source_root, tool, &source_smoke_target).await?;

    if target_root.exists() {
        fs::remove_dir_all(target_root).map_err(ManagedAcpToolError::io)?;
    }
    fs::create_dir_all(target_root).map_err(ManagedAcpToolError::io)?;
    copy_required_file(source_root, target_root, "package.json")?;
    copy_optional_file(source_root, target_root, "package-lock.json")?;
    copy_required_directory(source_root, target_root, "dist")?;
    copy_required_directory(source_root, target_root, "node_modules")?;

    let package_json = read_installed_package_json(&target_root.join("package.json"))?;
    let manifest = build_project_root_artifact_manifest(&package_json)?;
    fs::write(
        target_root.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest)
            .map_err(|error| ManagedAcpToolError::invalid(format!("serialize local ACP manifest: {error}")))?,
    )
    .map_err(ManagedAcpToolError::io)?;

    validate_platform_binary(tool, target_root, spec)?;
    let smoke_target = resolve_package_smoke_target(target_root, &package_json)?;
    validate_package_smoke_target(node_runtime, target_root, tool, &smoke_target).await?;
    validate_tool_root(tool, target_root, None)
}

fn report_failure(
    error: ManagedAcpToolError,
    reporter: Option<&dyn ManagedAcpToolProgressReporter>,
) -> ManagedAcpToolError {
    let (kind, status_code) = classify_error(&error);
    emit_progress(
        reporter,
        match status_code {
            Some(status) => ManagedAcpToolProgress::failed_with_status(kind, status, error.to_string()),
            None => ManagedAcpToolProgress::failed(kind, error.to_string()),
        },
    );
    error
}

fn classify_error(error: &ManagedAcpToolError) -> (ManagedAcpToolFailureKind, Option<u16>) {
    let message = error.to_string().to_ascii_lowercase();
    if message.contains("timed out") {
        return (ManagedAcpToolFailureKind::Timeout, None);
    }
    if let Some(status) = parse_http_status(&message) {
        return (ManagedAcpToolFailureKind::HttpStatus, Some(status));
    }
    if message.contains("unsupported") {
        return (ManagedAcpToolFailureKind::UnsupportedPlatform, None);
    }
    if message.contains("bundled managed") && message.contains("artifact missing") {
        return (ManagedAcpToolFailureKind::BundledResourceMissing, None);
    }
    if message.contains("bundled managed") && message.contains("artifact failed validation") {
        return (ManagedAcpToolFailureKind::BundledResourceInvalid, None);
    }
    if message.contains("bundled managed") && message.contains("artifact is invalid") {
        return (ManagedAcpToolFailureKind::BundledResourceInvalid, None);
    }
    if message.contains("checksum mismatch") {
        return (ManagedAcpToolFailureKind::ChecksumMismatch, None);
    }
    if message.contains("validate") || message.contains("entrypoint missing") {
        return (ManagedAcpToolFailureKind::ValidationFailed, None);
    }
    if message.contains("download") || message.contains("extract") || message.contains("connect failed") {
        return (ManagedAcpToolFailureKind::DownloadFailed, None);
    }
    (ManagedAcpToolFailureKind::Unknown, None)
}

fn parse_http_status(message: &str) -> Option<u16> {
    let marker = "http ";
    let start = message.find(marker)? + marker.len();
    let digits: String = message[start..].chars().take_while(|ch| ch.is_ascii_digit()).collect();
    digits.parse::<u16>().ok()
}

fn unavailable_error(tool: ManagedAcpToolId, root: &Path) -> ManagedAcpToolError {
    ManagedAcpToolError::invalid(format!(
        "managed {} artifact unavailable under {} and could not be prepared locally",
        tool.display_name(),
        root.display()
    ))
}

pub fn probe_managed_acp_tool_supported(tool: ManagedAcpToolId) -> ManagedAcpToolSupport {
    match platform_spec() {
        Ok(spec) => match tool_root(tool, spec) {
            Ok(root) => ManagedAcpToolSupport {
                supported: true,
                detail: format!(
                    "managed {} artifact supported under {}",
                    tool.display_name(),
                    root.display()
                ),
            },
            Err(error) => ManagedAcpToolSupport {
                supported: false,
                detail: error.to_string(),
            },
        },
        Err(error) => ManagedAcpToolSupport {
            supported: false,
            detail: error.to_string(),
        },
    }
}

pub fn doctor_snapshot() -> Vec<DoctorRow> {
    [ManagedAcpToolId::CodexAcp, ManagedAcpToolId::ClaudeAgentAcp]
        .into_iter()
        .map(doctor_row)
        .collect()
}

fn doctor_row(tool: ManagedAcpToolId) -> DoctorRow {
    match platform_spec() {
        Ok(spec) => match tool_root(tool, spec) {
            Ok(root) if !root.exists() => {
                if let Some(source_root) =
                    managed_resources::acp_tool_sources(tool.slug(), tool.version(), spec.manifest_key)
                        .into_iter()
                        .next()
                        .map(|source| source.root)
                {
                    return DoctorRow {
                        tool: tool.slug().into(),
                        source: "local".into(),
                        detail: source_root.display().to_string(),
                    };
                }
                DoctorRow {
                    tool: tool.slug().into(),
                    source: "managed".into(),
                    detail: format!("not installed (expected under {})", root.display()),
                }
            }
            Ok(root) => match validate_tool_root(tool, &root, None) {
                Ok(resolved) => DoctorRow {
                    tool: tool.slug().into(),
                    source: "managed".into(),
                    detail: resolved.entrypoint.display().to_string(),
                },
                Err(error) if root.exists() => DoctorRow {
                    tool: tool.slug().into(),
                    source: "managed".into(),
                    detail: format!("{} (root: {})", error, root.display()),
                },
                Err(_) => DoctorRow {
                    tool: tool.slug().into(),
                    source: "managed".into(),
                    detail: format!("not installed (expected under {})", root.display()),
                },
            },
            Err(error) => DoctorRow {
                tool: tool.slug().into(),
                source: "unavailable".into(),
                detail: error.to_string(),
            },
        },
        Err(error) => DoctorRow {
            tool: tool.slug().into(),
            source: "unavailable".into(),
            detail: error.to_string(),
        },
    }
}

fn activate_local_tool_source(
    tool: ManagedAcpToolId,
    spec: PlatformSpec,
    root: &Path,
    reporter: Option<&dyn ManagedAcpToolProgressReporter>,
) -> Result<Option<ResolvedManagedAcpTool>, ManagedAcpToolError> {
    if managed_resources::requires_bundled_resources() {
        let bundled_root = managed_resources::bundled_root_candidate()
            .ok_or_else(|| ManagedAcpToolError::invalid("bundled managed resources root unavailable"))?;
        let bundled_tool_root = bundled_root
            .join("acp")
            .join(tool.slug())
            .join(tool.version())
            .join(spec.manifest_key);
        if !bundled_tool_root.is_dir() {
            return Err(ManagedAcpToolError::invalid(format!(
                "bundled managed {} artifact missing under {}",
                tool.display_name(),
                bundled_tool_root.display()
            )));
        }
    }

    for source in managed_resources::acp_tool_sources(tool.slug(), tool.version(), spec.manifest_key) {
        emit_progress(
            reporter,
            ManagedAcpToolProgress::extracting(format!(
                "activating managed {} artifact from {}",
                tool.display_name(),
                source.root.display()
            )),
        );

        if let Err(error) = managed_resources::materialize_directory(&source.root, root) {
            warn!(
                tool = tool.slug(),
                version = tool.version(),
                source_root = %source.root.display(),
                target_root = %root.display(),
                error = %error,
                "failed to activate local managed ACP tool source"
            );
            if matches!(source.kind, managed_resources::ManagedResourceSourceKind::Bundled) {
                return Err(ManagedAcpToolError::invalid(format!(
                    "bundled managed {} artifact is invalid under {}: {}",
                    tool.display_name(),
                    source.root.display(),
                    error
                )));
            }
            continue;
        }

        match validate_tool_root(tool, root, reporter) {
            Ok(resolved) => {
                info!(
                    tool = tool.slug(),
                    version = tool.version(),
                    source_root = %source.root.display(),
                    target_root = %root.display(),
                    "managed ACP tool activated from local resources"
                );
                return Ok(Some(resolved));
            }
            Err(error) => {
                warn!(
                    tool = tool.slug(),
                    version = tool.version(),
                    source_root = %source.root.display(),
                    target_root = %root.display(),
                    error = %error,
                    "local managed ACP tool source failed validation"
                );
                let _ = fs::remove_dir_all(root);
                if matches!(source.kind, managed_resources::ManagedResourceSourceKind::Bundled) {
                    return Err(ManagedAcpToolError::invalid(format!(
                        "bundled managed {} artifact failed validation under {}: {}",
                        tool.display_name(),
                        source.root.display(),
                        error
                    )));
                }
            }
        }
    }

    Ok(None)
}

fn validate_tool_root(
    tool: ManagedAcpToolId,
    root: &Path,
    reporter: Option<&dyn ManagedAcpToolProgressReporter>,
) -> Result<ResolvedManagedAcpTool, ManagedAcpToolError> {
    emit_progress(
        reporter,
        ManagedAcpToolProgress::validating(format!(
            "validating managed {} artifact under {}",
            tool.display_name(),
            root.display()
        )),
    );
    let manifest = read_local_manifest(root)?;
    let entrypoint = root.join(&manifest.entrypoint);
    if !entrypoint.is_file() {
        return Err(ManagedAcpToolError::invalid(format!(
            "managed ACP entrypoint missing: {}",
            entrypoint.display()
        )));
    }

    let env_path_entries = manifest
        .path_entries
        .into_iter()
        .map(|entry| root.join(entry))
        .filter(|path| path.exists())
        .collect::<Vec<_>>();

    let resolved = ResolvedManagedAcpTool {
        id: tool,
        version: tool.version().to_owned(),
        root: root.to_path_buf(),
        entrypoint,
        env_path_entries,
    };
    emit_progress(
        reporter,
        ManagedAcpToolProgress::ready(format!("managed {} artifact is ready", tool.display_name())),
    );
    Ok(resolved)
}

async fn maybe_prepare_local_runtime_tool_source(
    tool: ManagedAcpToolId,
    spec: PlatformSpec,
    reporter: Option<&dyn ManagedAcpToolProgressReporter>,
) -> Result<bool, ManagedAcpToolError> {
    if managed_resources::requires_bundled_resources() {
        return Ok(false);
    }

    let target_root = tool_root(tool, spec)?;

    if target_root.exists() {
        fs::remove_dir_all(&target_root).map_err(ManagedAcpToolError::io)?;
    }

    emit_progress(
        reporter,
        ManagedAcpToolProgress::extracting(format!(
            "preparing managed {} artifact under local runtime resources",
            tool.display_name()
        )),
    );
    info!(
        tool = tool.slug(),
        version = tool.version(),
        platform = spec.manifest_key,
        target_root = %target_root.display(),
        "preparing managed ACP tool into local runtime resources"
    );

    let node_runtime = ensure_node_runtime_with_reporter(None)
        .await
        .map_err(|error| ManagedAcpToolError::invalid(format!("prepare managed Node runtime: {error}")))?;

    let staging_root = prepare_staging_root(tool, spec)?;
    if staging_root.exists() {
        let _ = fs::remove_dir_all(&staging_root);
    }
    fs::create_dir_all(&staging_root).map_err(ManagedAcpToolError::io)?;

    let result = prepare_local_tool_source(tool, spec, &node_runtime, &staging_root, &target_root).await;
    if let Err(error) = fs::remove_dir_all(&staging_root)
        && error.kind() != std::io::ErrorKind::NotFound
    {
        warn!(
            tool = tool.slug(),
            version = tool.version(),
            staging_root = %staging_root.display(),
            error = %error,
            "failed to clean up managed ACP local preparation staging directory"
        );
    }

    result?;
    Ok(true)
}

async fn prepare_local_tool_source(
    tool: ManagedAcpToolId,
    spec: PlatformSpec,
    node_runtime: &crate::ResolvedNodeRuntime,
    staging_root: &Path,
    target_root: &Path,
) -> Result<(), ManagedAcpToolError> {
    prepare_local_tool_source_to_root(
        tool,
        spec,
        node_runtime,
        staging_root,
        target_root,
        &format!("{}@{}", tool.package_name(), tool.version()),
    )
    .await
    .map(|_| ())
}

async fn prepare_local_tool_source_to_root(
    tool: ManagedAcpToolId,
    spec: PlatformSpec,
    node_runtime: &crate::ResolvedNodeRuntime,
    staging_root: &Path,
    target_root: &Path,
    package_spec: &str,
) -> Result<ResolvedManagedAcpTool, ManagedAcpToolError> {
    let project_dir = staging_root.join("project");
    let npm_cache_dir = staging_root.join("npm-cache");
    fs::create_dir_all(&project_dir).map_err(ManagedAcpToolError::io)?;
    fs::create_dir_all(&npm_cache_dir).map_err(ManagedAcpToolError::io)?;

    write_dev_package_json(&project_dir)?;
    run_npm_prepare_step(
        node_runtime,
        &project_dir,
        &npm_cache_dir,
        [
            "install",
            "--package-lock-only",
            "--ignore-scripts",
            "--include=optional",
            "--fund=false",
            "--audit=false",
            "--save-exact",
            "--os",
            spec.npm_os,
            "--cpu",
            spec.npm_cpu,
            package_spec,
        ],
        "generate managed ACP local lockfile",
    )
    .await?;
    run_npm_prepare_step(
        node_runtime,
        &project_dir,
        &npm_cache_dir,
        [
            "ci",
            "--omit=dev",
            "--ignore-scripts",
            "--include=optional",
            "--fund=false",
            "--audit=false",
            "--os",
            spec.npm_os,
            "--cpu",
            spec.npm_cpu,
        ],
        "install managed ACP local artifact",
    )
    .await?;

    let manifest = build_local_artifact_manifest(tool, &project_dir)?;
    validate_bridge_entrypoint(&project_dir, &manifest)?;
    validate_platform_binary(tool, &project_dir, spec)?;
    validate_dependency_tree(node_runtime, &project_dir, &npm_cache_dir, tool).await?;
    validate_package_smoke(node_runtime, &project_dir, tool).await?;

    let manifest_path = project_dir.join("manifest.json");
    fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest)
            .map_err(|error| ManagedAcpToolError::invalid(format!("serialize local ACP manifest: {error}")))?,
    )
    .map_err(ManagedAcpToolError::io)?;

    managed_resources::materialize_directory(&project_dir, target_root).map_err(ManagedAcpToolError::io)?;
    let resolved = validate_tool_root(tool, target_root, None)?;
    validate_dependency_tree(node_runtime, target_root, &npm_cache_dir, tool).await?;
    validate_package_smoke(node_runtime, target_root, tool).await?;
    info!(
        tool = tool.slug(),
        version = tool.version(),
        platform = spec.manifest_key,
        target_root = %target_root.display(),
        "prepared managed ACP tool under local runtime resources"
    );
    Ok(resolved)
}

async fn run_npm_prepare_step<const N: usize>(
    node_runtime: &crate::ResolvedNodeRuntime,
    project_dir: &Path,
    npm_cache_dir: &Path,
    args: [&str; N],
    label: &str,
) -> Result<(), ManagedAcpToolError> {
    let mut builder = Builder::from_resolved(&node_runtime.npm_command());
    builder
        .current_dir(project_dir)
        .env("npm_config_cache", npm_cache_dir)
        .args(args);
    let output = builder.output().await.map_err(ManagedAcpToolError::io)?;
    if output.status.success() {
        return Ok(());
    }

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    let detail = if stderr.is_empty() {
        stdout
    } else if stdout.is_empty() {
        stderr
    } else {
        format!("{stderr}; stdout: {stdout}")
    };
    Err(ManagedAcpToolError::invalid(format!(
        "{label} failed with exit code {:?}: {detail}",
        output.status.code()
    )))
}

fn write_dev_package_json(project_dir: &Path) -> Result<(), ManagedAcpToolError> {
    let package_json = DevPackageJson {
        name: "aionui-managed-acp-dev",
        private: true,
    };
    fs::write(
        project_dir.join("package.json"),
        serde_json::to_vec_pretty(&package_json)
            .map_err(|error| ManagedAcpToolError::invalid(format!("serialize local package.json: {error}")))?,
    )
    .map_err(ManagedAcpToolError::io)
}

fn build_local_artifact_manifest(
    tool: ManagedAcpToolId,
    project_dir: &Path,
) -> Result<LocalArtifactManifestWrite, ManagedAcpToolError> {
    let package_segments = package_path_segments(tool.package_name());
    let package_json_path = package_json_path(project_dir, tool.package_name());
    let package_json = read_installed_package_json(&package_json_path)?;
    let entrypoint_rel = resolve_package_bin_entry(&package_json.name, &package_json.bin)?;

    let mut entrypoint = PathBuf::from("node_modules");
    for segment in &package_segments {
        entrypoint.push(segment);
    }
    entrypoint.push(entrypoint_rel);

    Ok(LocalArtifactManifestWrite {
        entrypoint: normalize_slashes(&entrypoint),
        path_entries: vec!["node_modules/.bin".into()],
    })
}

fn build_project_root_artifact_manifest(
    package_json: &InstalledPackageJson,
) -> Result<LocalArtifactManifestWrite, ManagedAcpToolError> {
    let entrypoint = resolve_package_bin_entry(&package_json.name, &package_json.bin)?;
    Ok(LocalArtifactManifestWrite {
        entrypoint,
        path_entries: vec!["node_modules/.bin".into()],
    })
}

fn validate_bridge_entrypoint(
    project_dir: &Path,
    manifest: &LocalArtifactManifestWrite,
) -> Result<(), ManagedAcpToolError> {
    let entrypoint = project_dir.join(&manifest.entrypoint);
    if !entrypoint.is_file() {
        return Err(ManagedAcpToolError::invalid(format!(
            "resolved managed ACP entrypoint missing: {}",
            entrypoint.display()
        )));
    }
    Ok(())
}

fn copy_required_file(source_root: &Path, target_root: &Path, file_name: &str) -> Result<(), ManagedAcpToolError> {
    let source = source_root.join(file_name);
    if !source.is_file() {
        return Err(ManagedAcpToolError::invalid(format!(
            "local ACP source missing required file: {}",
            source.display()
        )));
    }
    fs::copy(&source, target_root.join(file_name)).map_err(ManagedAcpToolError::io)?;
    Ok(())
}

fn copy_optional_file(source_root: &Path, target_root: &Path, file_name: &str) -> Result<(), ManagedAcpToolError> {
    let source = source_root.join(file_name);
    if source.is_file() {
        fs::copy(&source, target_root.join(file_name)).map_err(ManagedAcpToolError::io)?;
    }
    Ok(())
}

fn copy_required_directory(
    source_root: &Path,
    target_root: &Path,
    directory_name: &str,
) -> Result<(), ManagedAcpToolError> {
    let source = source_root.join(directory_name);
    if !source.is_dir() {
        return Err(ManagedAcpToolError::invalid(format!(
            "local ACP source missing required directory: {}",
            source.display()
        )));
    }
    managed_resources::materialize_directory(&source, &target_root.join(directory_name))
        .map_err(ManagedAcpToolError::io)?;
    Ok(())
}

fn validate_platform_binary(
    tool: ManagedAcpToolId,
    project_dir: &Path,
    spec: PlatformSpec,
) -> Result<(), ManagedAcpToolError> {
    let expected = match tool {
        ManagedAcpToolId::CodexAcp => {
            let mut path = project_dir
                .join("node_modules")
                .join(format!("@zed-industries/codex-acp-{}", spec.manifest_key))
                .join("bin")
                .join("codex-acp");
            if spec.manifest_key.starts_with("win32-") {
                path.set_extension("exe");
            }
            path
        }
        ManagedAcpToolId::ClaudeAgentAcp => {
            let mut path = project_dir
                .join("node_modules")
                .join(format!("@anthropic-ai/claude-agent-sdk-{}", spec.manifest_key))
                .join("claude");
            if spec.manifest_key.starts_with("win32-") {
                path.set_extension("exe");
            }
            path
        }
    };

    if expected.is_file() {
        Ok(())
    } else {
        Err(ManagedAcpToolError::invalid(format!(
            "expected managed {} platform binary missing: {}",
            tool.display_name(),
            expected.display()
        )))
    }
}

async fn validate_dependency_tree(
    node_runtime: &crate::ResolvedNodeRuntime,
    project_dir: &Path,
    npm_cache_dir: &Path,
    tool: ManagedAcpToolId,
) -> Result<(), ManagedAcpToolError> {
    run_npm_prepare_step(
        node_runtime,
        project_dir,
        npm_cache_dir,
        ["ls", "--omit=dev", "--all"],
        &format!("validate managed {} dependency tree", tool.display_name()),
    )
    .await
}

async fn validate_package_smoke(
    node_runtime: &crate::ResolvedNodeRuntime,
    project_dir: &Path,
    tool: ManagedAcpToolId,
) -> Result<(), ManagedAcpToolError> {
    let package_json_path = package_json_path(project_dir, tool.package_name());
    let package_json = read_installed_package_json(&package_json_path)?;
    let smoke_target = resolve_package_smoke_target(&package_root(project_dir, &package_json.name), &package_json)?;
    validate_package_smoke_target(node_runtime, project_dir, tool, &smoke_target).await
}

fn package_json_path(project_dir: &Path, package_name: &str) -> PathBuf {
    package_root(project_dir, package_name).join("package.json")
}

fn package_root(project_dir: &Path, package_name: &str) -> PathBuf {
    let mut path = project_dir.join("node_modules");
    for segment in package_path_segments(package_name) {
        path.push(segment);
    }
    path
}

fn package_path_segments(package_name: &str) -> Vec<&str> {
    package_name.split('/').collect()
}

fn resolve_package_bin_entry(package_name: &str, bin_field: &serde_json::Value) -> Result<String, ManagedAcpToolError> {
    match bin_field {
        serde_json::Value::String(value) if !value.is_empty() => Ok(value.clone()),
        serde_json::Value::Object(entries) => {
            let short_name = package_name
                .rsplit('/')
                .next()
                .ok_or_else(|| ManagedAcpToolError::invalid("package name missing short name"))?;
            for key in [package_name, short_name] {
                if let Some(serde_json::Value::String(value)) = entries.get(key)
                    && !value.is_empty()
                {
                    return Ok(value.clone());
                }
            }
            entries
                .values()
                .find_map(|value| match value {
                    serde_json::Value::String(value) if !value.is_empty() => Some(value.clone()),
                    _ => None,
                })
                .ok_or_else(|| {
                    ManagedAcpToolError::invalid(format!("package {package_name} does not expose a usable bin entry"))
                })
        }
        _ => Err(ManagedAcpToolError::invalid(format!(
            "package {package_name} does not expose a usable bin entry"
        ))),
    }
}

fn resolve_package_smoke_target(
    package_root: &Path,
    package_json: &InstalledPackageJson,
) -> Result<PackageSmokeTarget, ManagedAcpToolError> {
    if let Some(entry) = resolve_package_import_entry(&package_json.exports, package_json.main.as_deref()) {
        return Ok(PackageSmokeTarget::Import(package_root.join(entry)));
    }

    let bin_entry = resolve_package_bin_entry(package_json.name.as_str(), &package_json.bin)?;
    Ok(PackageSmokeTarget::SyntaxCheck(package_root.join(bin_entry)))
}

fn resolve_package_import_entry(exports_field: &serde_json::Value, main_field: Option<&str>) -> Option<String> {
    let exports_entry = match exports_field {
        serde_json::Value::String(value) if !value.is_empty() => Some(value.clone()),
        serde_json::Value::Object(entries) => entries.get(".").and_then(|root| match root {
            serde_json::Value::String(value) if !value.is_empty() => Some(value.clone()),
            serde_json::Value::Object(root_entries) => root_entries
                .get("import")
                .and_then(|value| match value {
                    serde_json::Value::String(value) if !value.is_empty() => Some(value.clone()),
                    _ => None,
                })
                .or_else(|| {
                    root_entries.get("default").and_then(|value| match value {
                        serde_json::Value::String(value) if !value.is_empty() => Some(value.clone()),
                        _ => None,
                    })
                }),
            _ => None,
        }),
        _ => None,
    };

    exports_entry.or_else(|| main_field.and_then(|value| if value.is_empty() { None } else { Some(value.to_owned()) }))
}

fn normalize_slashes(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn read_installed_package_json(package_json_path: &Path) -> Result<InstalledPackageJson, ManagedAcpToolError> {
    let contents = fs::read_to_string(package_json_path).map_err(ManagedAcpToolError::io)?;
    serde_json::from_str(&contents).map_err(|error| {
        ManagedAcpToolError::invalid(format!(
            "parse installed package manifest failed for {}: {error}",
            package_json_path.display()
        ))
    })
}

fn local_managed_acp_override(tool: ManagedAcpToolId) -> Option<LocalManagedAcpOverride> {
    if let Some(path) = configured_override_path(&local_source_env_key(tool)) {
        return Some(LocalManagedAcpOverride::SourceDir(path));
    }

    configured_override_path(&local_tarball_env_key(tool)).map(LocalManagedAcpOverride::Tarball)
}

fn local_source_env_key(tool: ManagedAcpToolId) -> String {
    format!("{LOCAL_SOURCE_ENV_PREFIX}{}", local_override_env_suffix(tool))
}

fn local_tarball_env_key(tool: ManagedAcpToolId) -> String {
    format!("{LOCAL_TARBALL_ENV_PREFIX}{}", local_override_env_suffix(tool))
}

fn local_override_env_suffix(tool: ManagedAcpToolId) -> String {
    tool.slug()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect()
}

fn configured_override_path(env_key: &str) -> Option<PathBuf> {
    std::env::var_os(env_key)
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
}

async fn activate_local_override(
    tool: ManagedAcpToolId,
    spec: PlatformSpec,
    root: &Path,
    local_override: &LocalManagedAcpOverride,
    reporter: Option<&dyn ManagedAcpToolProgressReporter>,
) -> Result<ResolvedManagedAcpTool, ManagedAcpToolError> {
    match local_override {
        LocalManagedAcpOverride::SourceDir(source_root) => {
            emit_progress(
                reporter,
                ManagedAcpToolProgress::extracting(format!(
                    "activating local {} source from {}",
                    tool.display_name(),
                    source_root.display()
                )),
            );

            if source_root.join("manifest.json").is_file() {
                return validate_tool_root(tool, source_root, reporter);
            }

            validate_local_project_source(tool, spec, source_root, reporter).await
        }
        LocalManagedAcpOverride::Tarball(tarball_path) => {
            if !tarball_path.is_file() {
                return Err(ManagedAcpToolError::invalid(format!(
                    "local managed {} tarball missing: {}",
                    tool.display_name(),
                    tarball_path.display()
                )));
            }

            if root.exists() {
                fs::remove_dir_all(root).map_err(ManagedAcpToolError::io)?;
            }

            let node_runtime = ensure_node_runtime_with_reporter(None)
                .await
                .map_err(|error| ManagedAcpToolError::invalid(format!("prepare managed Node runtime: {error}")))?;
            let staging_root = prepare_staging_root(tool, spec)?;
            if staging_root.exists() {
                let _ = fs::remove_dir_all(&staging_root);
            }
            fs::create_dir_all(&staging_root).map_err(ManagedAcpToolError::io)?;

            let tarball_spec = tarball_path.to_string_lossy().into_owned();
            let result =
                prepare_local_tool_source_to_root(tool, spec, &node_runtime, &staging_root, root, &tarball_spec).await;

            if let Err(error) = fs::remove_dir_all(&staging_root)
                && error.kind() != std::io::ErrorKind::NotFound
            {
                warn!(
                    tool = tool.slug(),
                    version = tool.version(),
                    staging_root = %staging_root.display(),
                    error = %error,
                    "failed to clean up managed ACP local tarball staging directory"
                );
            }

            result
        }
    }
}

async fn validate_local_project_source(
    tool: ManagedAcpToolId,
    spec: PlatformSpec,
    source_root: &Path,
    reporter: Option<&dyn ManagedAcpToolProgressReporter>,
) -> Result<ResolvedManagedAcpTool, ManagedAcpToolError> {
    emit_progress(
        reporter,
        ManagedAcpToolProgress::validating(format!(
            "validating local {} source under {}",
            tool.display_name(),
            source_root.display()
        )),
    );

    let package_json_path = source_root.join("package.json");
    if !package_json_path.is_file() {
        return Err(ManagedAcpToolError::invalid(format!(
            "local {} source missing package.json under {}",
            tool.display_name(),
            source_root.display()
        )));
    }

    let package_json = read_installed_package_json(&package_json_path)?;
    if package_json.name != tool.package_name() {
        return Err(ManagedAcpToolError::invalid(format!(
            "local {} source expected package {}, found {}",
            tool.display_name(),
            tool.package_name(),
            package_json.name
        )));
    }

    let entrypoint_rel = resolve_package_bin_entry(&package_json.name, &package_json.bin)?;
    let entrypoint = source_root.join(&entrypoint_rel);
    if !entrypoint.is_file() {
        return Err(ManagedAcpToolError::invalid(format!(
            "local {} entrypoint missing: {}",
            tool.display_name(),
            entrypoint.display()
        )));
    }

    validate_platform_binary(tool, source_root, spec)?;

    let node_runtime = ensure_node_runtime_with_reporter(None)
        .await
        .map_err(|error| ManagedAcpToolError::invalid(format!("prepare managed Node runtime: {error}")))?;
    let smoke_target = resolve_package_smoke_target(source_root, &package_json)?;
    validate_package_smoke_target(&node_runtime, source_root, tool, &smoke_target).await?;

    let env_path_entries = {
        let candidate = source_root.join("node_modules").join(".bin");
        if candidate.exists() {
            vec![candidate]
        } else {
            Vec::new()
        }
    };

    let resolved = ResolvedManagedAcpTool {
        id: tool,
        version: package_json.version.unwrap_or_else(|| tool.version().to_owned()),
        root: source_root.to_path_buf(),
        entrypoint,
        env_path_entries,
    };
    emit_progress(
        reporter,
        ManagedAcpToolProgress::ready(format!("local {} source is ready", tool.display_name())),
    );
    Ok(resolved)
}

async fn validate_package_smoke_target(
    node_runtime: &crate::ResolvedNodeRuntime,
    current_dir: &Path,
    tool: ManagedAcpToolId,
    smoke_target: &PackageSmokeTarget,
) -> Result<(), ManagedAcpToolError> {
    let mut builder = Builder::clean_cli(node_runtime.node_path.clone());
    builder.current_dir(current_dir);
    match smoke_target {
        PackageSmokeTarget::Import(path) => {
            builder
                .arg("--input-type=module")
                .arg("-e")
                .arg("import { pathToFileURL } from 'node:url'; await import(pathToFileURL(process.argv[1]).href);")
                .arg(path);
        }
        PackageSmokeTarget::SyntaxCheck(path) => {
            builder.arg("--check").arg(path);
        }
    }
    let output = tokio::time::timeout(MANAGED_ACP_SMOKE_TIMEOUT, builder.output())
        .await
        .map_err(|_| {
            ManagedAcpToolError::invalid(format!(
                "smoke test for managed {} package timed out after {}s",
                tool.display_name(),
                MANAGED_ACP_SMOKE_TIMEOUT.as_secs()
            ))
        })?
        .map_err(ManagedAcpToolError::io)?;
    if output.status.success() {
        return Ok(());
    }

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    let detail = if stderr.is_empty() {
        stdout
    } else if stdout.is_empty() {
        stderr
    } else {
        format!("{stderr}; stdout: {stdout}")
    };
    Err(ManagedAcpToolError::invalid(format!(
        "smoke test for managed {} package failed with exit code {:?}: {detail}",
        tool.display_name(),
        output.status.code()
    )))
}

fn prepare_staging_root(tool: ManagedAcpToolId, spec: PlatformSpec) -> Result<PathBuf, ManagedAcpToolError> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let base =
        cache::managed_acp_tool_root().ok_or_else(|| ManagedAcpToolError::invalid("runtime cache dir unavailable"))?;
    Ok(base.join(".staging").join(format!(
        "{}-{}-{}-{}",
        tool.slug(),
        tool.version(),
        spec.manifest_key,
        nonce
    )))
}

fn bundle_prepare_staging_root(tool: ManagedAcpToolId, spec: PlatformSpec, bundle_root: &Path) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    bundle_root.join(".staging").join(format!(
        "{}-{}-{}-{}",
        tool.slug(),
        tool.version(),
        spec.manifest_key,
        nonce
    ))
}

fn bundle_tool_root(bundle_root: &Path, tool: ManagedAcpToolId, spec: PlatformSpec) -> PathBuf {
    bundle_root
        .join("acp")
        .join(tool.slug())
        .join(tool.version())
        .join(spec.manifest_key)
}

fn platform_spec() -> Result<PlatformSpec, ManagedAcpToolError> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Ok(PlatformSpec {
            manifest_key: "darwin-arm64",
            npm_os: "darwin",
            npm_cpu: "arm64",
        }),
        ("macos", "x86_64") => Ok(PlatformSpec {
            manifest_key: "darwin-x64",
            npm_os: "darwin",
            npm_cpu: "x64",
        }),
        ("linux", "aarch64") => Ok(PlatformSpec {
            manifest_key: "linux-arm64",
            npm_os: "linux",
            npm_cpu: "arm64",
        }),
        ("linux", "x86_64") => Ok(PlatformSpec {
            manifest_key: "linux-x64",
            npm_os: "linux",
            npm_cpu: "x64",
        }),
        ("windows", "x86_64") => Ok(PlatformSpec {
            manifest_key: "win32-x64",
            npm_os: "win32",
            npm_cpu: "x64",
        }),
        ("windows", "aarch64") => Ok(PlatformSpec {
            manifest_key: "win32-arm64",
            npm_os: "win32",
            npm_cpu: "arm64",
        }),
        (os, arch) => Err(ManagedAcpToolError::unsupported_platform(format!(
            "managed ACP tool unsupported on {os}/{arch}"
        ))),
    }
}

fn tool_root(tool: ManagedAcpToolId, spec: PlatformSpec) -> Result<PathBuf, ManagedAcpToolError> {
    cache::managed_acp_tool_root()
        .map(|root| root.join(tool.slug()).join(tool.version()).join(spec.manifest_key))
        .ok_or_else(|| ManagedAcpToolError::invalid("runtime cache dir unavailable"))
}

#[derive(Debug, Deserialize)]
struct LocalArtifactManifest {
    entrypoint: String,
    #[serde(default)]
    path_entries: Vec<String>,
}

fn read_local_manifest(root: &Path) -> Result<LocalArtifactManifest, ManagedAcpToolError> {
    let path = root.join("manifest.json");
    let contents = fs::read_to_string(&path).map_err(ManagedAcpToolError::io)?;
    serde_json::from_str(&contents).map_err(|error| {
        ManagedAcpToolError::invalid(format!(
            "parse local ACP manifest failed for {}: {error}",
            path.display()
        ))
    })
}

fn emit_progress(reporter: Option<&dyn ManagedAcpToolProgressReporter>, update: ManagedAcpToolProgress) {
    if let Some(reporter) = reporter {
        reporter.report(update);
    }
}

#[cfg(test)]
fn format_error_with_causes(error: &(dyn StdError + 'static)) -> String {
    let mut segments = vec![error.to_string()];
    let mut current = error.source();
    while let Some(source) = current {
        let message = source.to_string();
        if !message.is_empty() && segments.last() != Some(&message) {
            segments.push(message);
        }
        current = source.source();
    }
    segments.join(" | caused by: ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fmt;

    fn write_fake_ready_artifact(root: &Path) {
        std::fs::create_dir_all(root.join("dist")).unwrap();
        std::fs::create_dir_all(root.join("node_modules").join(".bin")).unwrap();
        std::fs::write(root.join("dist").join("index.js"), "console.log('ready');\n").unwrap();
        std::fs::write(
            root.join("manifest.json"),
            serde_json::to_vec_pretty(&json!({
                "entrypoint": "dist/index.js",
                "path_entries": ["node_modules/.bin"],
            }))
            .unwrap(),
        )
        .unwrap();
    }

    fn write_fake_local_project_source(root: &Path, spec: PlatformSpec) {
        std::fs::create_dir_all(root.join("dist")).unwrap();
        std::fs::create_dir_all(root.join("node_modules").join(".bin")).unwrap();
        std::fs::create_dir_all(
            root.join("node_modules")
                .join(format!("@anthropic-ai/claude-agent-sdk-{}", spec.manifest_key)),
        )
        .unwrap();
        std::fs::write(root.join("dist").join("index.js"), "console.log('ready');\n").unwrap();
        std::fs::write(root.join("dist").join("lib.js"), "export {};\n").unwrap();
        std::fs::write(
            root.join("node_modules")
                .join(format!("@anthropic-ai/claude-agent-sdk-{}", spec.manifest_key))
                .join("claude"),
            "",
        )
        .unwrap();
        std::fs::write(
            root.join("package.json"),
            serde_json::to_vec_pretty(&json!({
                "name": "@agentclientprotocol/claude-agent-acp",
                "version": "0.44.0",
                "bin": {
                    "claude-agent-acp": "dist/index.js"
                },
                "exports": {
                    ".": {
                        "import": "./dist/lib.js"
                    }
                }
            }))
            .unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn managed_acp_tool_command_uses_node_runtime() {
        let runtime = crate::ResolvedNodeRuntime {
            source: crate::ResolvedNodeSource::Managed,
            root: PathBuf::from("/tmp/node"),
            version: semver::Version::new(24, 11, 0),
            node_path: PathBuf::from("/tmp/node/bin/node"),
            npm_path: PathBuf::from("/tmp/node/bin/npm"),
            npm_args_prefix: vec![],
            npx_path: PathBuf::from("/tmp/node/bin/npx"),
            npx_args_prefix: vec![],
            env: vec![(
                std::ffi::OsString::from("PATH"),
                std::ffi::OsString::from("/tmp/node/bin"),
            )],
        };
        let tool = ResolvedManagedAcpTool {
            id: ManagedAcpToolId::CodexAcp,
            version: "0.15.0".into(),
            root: PathBuf::from("/tmp/tool"),
            entrypoint: PathBuf::from("/tmp/tool/dist/index.js"),
            env_path_entries: vec![PathBuf::from("/tmp/tool/bin")],
        };
        let command = tool.command(&runtime);
        assert_eq!(command.program, PathBuf::from("/tmp/node/bin/node"));
        assert_eq!(
            command.args_prefix,
            vec![std::ffi::OsString::from("/tmp/tool/dist/index.js")]
        );
        let path = command
            .env
            .iter()
            .find(|(key, _)| key == "PATH")
            .map(|(_, value)| value.clone())
            .unwrap();
        assert!(path.to_string_lossy().contains("/tmp/tool/bin"));
    }

    #[test]
    fn checksum_mismatch_classifies_separately() {
        let error = ManagedAcpToolError::invalid("managed ACP archive checksum mismatch");
        let (kind, status_code) = classify_error(&error);
        assert_eq!(kind, ManagedAcpToolFailureKind::ChecksumMismatch);
        assert_eq!(status_code, None);
    }

    #[test]
    fn classify_error_detects_bundled_acp_validation_failure() {
        let error = ManagedAcpToolError::invalid(
            "bundled managed Codex ACP artifact failed validation under /app/resources/managed-resources/acp/codex-acp/0.15.0/linux-x64: managed ACP entrypoint missing",
        );
        let (kind, status_code) = classify_error(&error);

        assert_eq!(kind, ManagedAcpToolFailureKind::BundledResourceInvalid);
        assert_eq!(status_code, None);
    }

    #[tokio::test]
    async fn bundled_acp_tool_missing_reports_bundled_resource_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let bundled_root = tmp.path().join("bundled");
        if !crate::test_support::run_in_env_child(
            "acp_tool_runtime::tests::bundled_acp_tool_missing_reports_bundled_resource_missing",
            |command| {
                command.env("AIONUI_BUNDLED_MANAGED_RESOURCES", &bundled_root);
            },
        ) {
            return;
        }

        crate::cache::init(tmp.path().join("data"));
        managed_resources::set_managed_resources_mode(managed_resources::ManagedResourcesMode::Bundled);

        let updates = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let reporter_updates = updates.clone();
        let reporter = move |update: ManagedAcpToolProgress| {
            reporter_updates.lock().unwrap().push(update);
        };

        let result = ensure_managed_acp_tool_with_reporter(ManagedAcpToolId::CodexAcp, Some(&reporter)).await;
        managed_resources::set_managed_resources_mode(managed_resources::ManagedResourcesMode::Download);

        let error = result.expect_err("missing bundled ACP tool should fail");
        assert!(error.to_string().contains("bundled managed Codex ACP artifact missing"));
        let updates = updates.lock().unwrap();
        assert!(updates.iter().any(|update| {
            update.phase == ManagedAcpToolProgressPhase::Failed
                && update.failure_kind == Some(ManagedAcpToolFailureKind::BundledResourceMissing)
        }));
    }

    #[test]
    fn format_error_with_causes_collects_nested_sources() {
        #[derive(Debug)]
        struct TestError {
            message: &'static str,
            source: Option<Box<dyn StdError + Send + Sync>>,
        }

        impl fmt::Display for TestError {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.message)
            }
        }

        impl StdError for TestError {
            fn source(&self) -> Option<&(dyn StdError + 'static)> {
                self.source.as_deref().map(|error| error as &(dyn StdError + 'static))
            }
        }

        let error = TestError {
            message: "top level",
            source: Some(Box::new(TestError {
                message: "middle",
                source: Some(Box::new(TestError {
                    message: "root cause",
                    source: None,
                })),
            })),
        };

        assert_eq!(
            format_error_with_causes(&error),
            "top level | caused by: middle | caused by: root cause"
        );
    }

    #[test]
    fn resolve_package_bin_entry_prefers_short_name_for_scoped_package() {
        let bin_field = serde_json::json!({
            "claude-agent-acp": "dist/index.js",
            "other": "dist/other.js"
        });
        let entry = resolve_package_bin_entry("@agentclientprotocol/claude-agent-acp", &bin_field).unwrap();
        assert_eq!(entry, "dist/index.js");
    }

    #[test]
    fn resolve_package_smoke_target_prefers_importable_entry_for_exported_package() {
        let tmp = tempfile::tempdir().unwrap();
        let project_dir = tmp
            .path()
            .join("node_modules")
            .join("@agentclientprotocol")
            .join("claude-agent-acp");
        let package_json = InstalledPackageJson {
            name: "@agentclientprotocol/claude-agent-acp".into(),
            version: None,
            bin: json!({
                "claude-agent-acp": "dist/index.js",
            }),
            main: Some("dist/lib.js".into()),
            exports: json!({
                ".": {
                    "types": "./dist/lib.d.ts",
                    "import": "./dist/lib.js"
                }
            }),
        };

        let target = resolve_package_smoke_target(&project_dir, &package_json).expect("smoke target");

        assert_eq!(
            target,
            PackageSmokeTarget::Import(project_dir.join("dist").join("lib.js"))
        );
    }

    #[test]
    fn resolve_package_smoke_target_falls_back_to_bin_check_for_cli_only_package() {
        let tmp = tempfile::tempdir().unwrap();
        let project_dir = tmp
            .path()
            .join("node_modules")
            .join("@zed-industries")
            .join("codex-acp");
        let package_json = InstalledPackageJson {
            name: "@zed-industries/codex-acp".into(),
            version: None,
            bin: json!({
                "codex-acp": "bin/codex-acp.js",
            }),
            main: None,
            exports: serde_json::Value::Null,
        };

        let target = resolve_package_smoke_target(&project_dir, &package_json).expect("smoke target");

        assert_eq!(
            target,
            PackageSmokeTarget::SyntaxCheck(project_dir.join("bin").join("codex-acp.js"))
        );
    }

    #[test]
    fn package_path_segments_preserve_scoped_package_structure() {
        assert_eq!(
            package_path_segments("@zed-industries/codex-acp"),
            vec!["@zed-industries", "codex-acp"]
        );
    }

    #[test]
    fn local_override_env_keys_use_slug_suffix() {
        assert_eq!(
            local_source_env_key(ManagedAcpToolId::ClaudeAgentAcp),
            "AIONUI_MANAGED_ACP_SOURCE_CLAUDE_AGENT_ACP"
        );
        assert_eq!(
            local_tarball_env_key(ManagedAcpToolId::ClaudeAgentAcp),
            "AIONUI_MANAGED_ACP_TARBALL_CLAUDE_AGENT_ACP"
        );
    }

    #[test]
    fn local_source_override_takes_precedence_over_tarball() {
        let tmp = tempfile::tempdir().unwrap();
        let source_dir = tmp.path().join("source");
        let tarball = tmp.path().join("tool.tgz");
        if !crate::test_support::run_in_env_child(
            "acp_tool_runtime::tests::local_source_override_takes_precedence_over_tarball",
            |command| {
                command
                    .env(local_source_env_key(ManagedAcpToolId::ClaudeAgentAcp), &source_dir)
                    .env(local_tarball_env_key(ManagedAcpToolId::ClaudeAgentAcp), &tarball);
            },
        ) {
            return;
        }

        let source_dir = PathBuf::from(
            std::env::var_os(local_source_env_key(ManagedAcpToolId::ClaudeAgentAcp)).expect("source override env"),
        );
        let override_source = local_managed_acp_override(ManagedAcpToolId::ClaudeAgentAcp);
        assert_eq!(override_source, Some(LocalManagedAcpOverride::SourceDir(source_dir)));
    }

    #[test]
    fn doctor_snapshot_includes_builtin_acp_tools() {
        let rows = doctor_snapshot();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].tool, "codex-acp");
        assert_eq!(rows[1].tool, "claude-agent-acp");
    }

    #[tokio::test]
    async fn local_source_override_activates_ready_artifact() {
        let tmp = tempfile::tempdir().unwrap();
        let source_root = tmp.path().join("local-acp");
        if !crate::test_support::run_in_env_child(
            "acp_tool_runtime::tests::local_source_override_activates_ready_artifact",
            |command| {
                command
                    .env(local_source_env_key(ManagedAcpToolId::ClaudeAgentAcp), &source_root)
                    .env_remove(local_tarball_env_key(ManagedAcpToolId::ClaudeAgentAcp));
            },
        ) {
            return;
        }

        let source_root = PathBuf::from(
            std::env::var_os(local_source_env_key(ManagedAcpToolId::ClaudeAgentAcp)).expect("source override env"),
        );
        crate::cache::init(tmp.path().join("data"));
        write_fake_ready_artifact(&source_root);

        let resolved = ensure_managed_acp_tool(ManagedAcpToolId::ClaudeAgentAcp)
            .await
            .expect("local source override should resolve");

        assert_eq!(resolved.root, source_root);
        assert_eq!(resolved.entrypoint, source_root.join("dist").join("index.js"));
        assert_eq!(
            resolved.env_path_entries,
            vec![source_root.join("node_modules").join(".bin")]
        );
    }

    #[tokio::test]
    async fn prepare_bundle_uses_ready_local_source_override() {
        let tmp = tempfile::tempdir().unwrap();
        let source_root = tmp.path().join("local-acp");
        if !crate::test_support::run_in_env_child(
            "acp_tool_runtime::tests::prepare_bundle_uses_ready_local_source_override",
            |command| {
                command
                    .env(local_source_env_key(ManagedAcpToolId::ClaudeAgentAcp), &source_root)
                    .env_remove(local_tarball_env_key(ManagedAcpToolId::ClaudeAgentAcp));
            },
        ) {
            return;
        }

        let source_root = PathBuf::from(
            std::env::var_os(local_source_env_key(ManagedAcpToolId::ClaudeAgentAcp)).expect("source override env"),
        );
        let bundle_root = tmp.path().join("bundle");
        let spec = platform_spec().unwrap();
        let expected_root = bundle_root
            .join("acp")
            .join("claude-agent-acp")
            .join("0.39.0")
            .join(spec.manifest_key);
        write_fake_ready_artifact(&source_root);

        let resolved = prepare_managed_acp_tool_to_root(ManagedAcpToolId::ClaudeAgentAcp, &bundle_root)
            .await
            .expect("ready local source override should be bundled");

        assert_eq!(resolved.root, expected_root);
        assert_eq!(resolved.entrypoint, expected_root.join("dist").join("index.js"));
        assert!(expected_root.join("manifest.json").is_file());
        assert!(expected_root.join("dist").join("index.js").is_file());
    }

    #[tokio::test]
    async fn prepare_bundle_copies_local_project_source_runtime_files() {
        let tmp = tempfile::tempdir().unwrap();
        let source_root = tmp.path().join("local-acp");
        if !crate::test_support::run_in_env_child(
            "acp_tool_runtime::tests::prepare_bundle_copies_local_project_source_runtime_files",
            |command| {
                command
                    .env(local_source_env_key(ManagedAcpToolId::ClaudeAgentAcp), &source_root)
                    .env_remove(local_tarball_env_key(ManagedAcpToolId::ClaudeAgentAcp));
            },
        ) {
            return;
        }

        let source_root = PathBuf::from(
            std::env::var_os(local_source_env_key(ManagedAcpToolId::ClaudeAgentAcp)).expect("source override env"),
        );
        let spec = platform_spec().unwrap();
        let bundle_root = tmp.path().join("bundle");
        let expected_root = bundle_root
            .join("acp")
            .join("claude-agent-acp")
            .join("0.39.0")
            .join(spec.manifest_key);
        write_fake_local_project_source(&source_root, spec);

        let node_runtime = crate::ResolvedNodeRuntime {
            source: crate::ResolvedNodeSource::Managed,
            root: PathBuf::from("/tmp/node"),
            version: semver::Version::new(24, 11, 0),
            node_path: PathBuf::from("/usr/bin/true"),
            npm_path: PathBuf::from("/usr/bin/false"),
            npm_args_prefix: vec![],
            npx_path: PathBuf::from("/usr/bin/false"),
            npx_args_prefix: vec![],
            env: vec![],
        };
        let resolved = prepare_local_project_source_to_bundle_root(
            ManagedAcpToolId::ClaudeAgentAcp,
            spec,
            &node_runtime,
            &source_root,
            &expected_root,
        )
        .await
        .expect("local project source should be copied into bundle");

        assert_eq!(resolved.root, expected_root);
        assert_eq!(resolved.version, "0.39.0");
        assert!(expected_root.join("manifest.json").is_file());
        assert!(expected_root.join("dist").join("index.js").is_file());
        assert!(
            expected_root
                .join("node_modules")
                .join(format!("@anthropic-ai/claude-agent-sdk-{}", spec.manifest_key))
                .join("claude")
                .is_file()
        );
    }

    #[tokio::test]
    async fn missing_local_tarball_override_reports_clear_error() {
        let tmp = tempfile::tempdir().unwrap();
        let tarball = tmp.path().join("missing.tgz");
        if !crate::test_support::run_in_env_child(
            "acp_tool_runtime::tests::missing_local_tarball_override_reports_clear_error",
            |command| {
                command
                    .env(local_tarball_env_key(ManagedAcpToolId::ClaudeAgentAcp), &tarball)
                    .env_remove(local_source_env_key(ManagedAcpToolId::ClaudeAgentAcp));
            },
        ) {
            return;
        }

        crate::cache::init(tmp.path().join("data"));

        let error = ensure_managed_acp_tool(ManagedAcpToolId::ClaudeAgentAcp)
            .await
            .expect_err("missing local tarball should fail");

        assert!(error.to_string().contains("local managed Claude ACP tarball missing"));
    }

    #[test]
    fn bundled_validation_failure_does_not_fallback_to_remote_download() {
        let tmp = tempfile::tempdir().unwrap();
        let bundled_root = tmp.path().join("bundled");
        if !crate::test_support::run_in_env_child(
            "acp_tool_runtime::tests::bundled_validation_failure_does_not_fallback_to_remote_download",
            |command| {
                command.env("AIONUI_BUNDLED_MANAGED_RESOURCES", &bundled_root);
            },
        ) {
            return;
        }
        let bundled_root = std::path::PathBuf::from(std::env::var_os("AIONUI_BUNDLED_MANAGED_RESOURCES").unwrap());
        let spec = platform_spec().unwrap();
        let source_root = bundled_root
            .join("acp")
            .join("codex-acp")
            .join("0.15.0")
            .join(spec.manifest_key);
        std::fs::create_dir_all(&source_root).unwrap();
        std::fs::write(
            source_root.join("manifest.json"),
            br#"{"entrypoint":"dist/index.js","path_entries":[]}"#,
        )
        .unwrap();

        let runtime_root = tmp.path().join("runtime");
        let tool_root = runtime_root.join("codex-acp").join("0.15.0").join(spec.manifest_key);

        managed_resources::set_managed_resources_mode(managed_resources::ManagedResourcesMode::Bundled);
        let result = activate_local_tool_source(ManagedAcpToolId::CodexAcp, spec, &tool_root, None);
        managed_resources::set_managed_resources_mode(managed_resources::ManagedResourcesMode::Download);

        let error = result.expect_err("bundled validation failure should abort");
        assert!(
            error
                .to_string()
                .contains("bundled managed Codex ACP artifact failed validation")
        );
    }

    #[test]
    fn bundle_tool_root_scopes_acp_output_under_tool_directory() {
        let bundle_root = std::path::Path::new("/tmp/bundle");
        let spec = PlatformSpec {
            manifest_key: "win32-x64",
            npm_os: "win32",
            npm_cpu: "x64",
        };

        let path = bundle_tool_root(bundle_root, ManagedAcpToolId::CodexAcp, spec);

        assert_eq!(
            path,
            bundle_root
                .join("acp")
                .join("codex-acp")
                .join("0.15.0")
                .join("win32-x64")
        );
    }
}
