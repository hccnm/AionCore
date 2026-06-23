#![allow(clippy::disallowed_types)]

use std::path::{Path as FsPath, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Form, Multipart, Path, Request, State};
use axum::http::{HeaderMap, header};
use axum::middleware::{Next, from_fn_with_state};
use axum::response::Response;
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use aionui_api_types::ApiResponse;
use aionui_auth::{CurrentUser, GatewayAuthConfig, hash_password};
use aionui_common::{ApiError, encrypt_string};
use aionui_db::{
    CreateAuditLogParams, CreateExecutionArtifactParams, CreateExecutionRunParams, CreateGitProjectParams,
    CreateGitSshCredentialParams, CreatePlatformUserParams, CreateSnapshotParams, CreateWorkspaceParams,
    ExecutionArtifactRow, ExecutionRunRow, GitProjectRow, GitSshCredentialRow, IAuditLogRepository,
    IExecutionArtifactRepository, IExecutionRunRepository, IExternalIdentityRepository, IGitProjectRepository,
    IGitSshCredentialRepository, IPlatformUserRepository, IRoleRepository, ISnapshotRepository, IWorkspaceRepository,
    PlatformUserRow, RoleRow, SnapshotRow, UpsertExternalIdentityParams, UpsertRoleParams, WorkspaceRow,
};
use aionui_file::{ContentUpdateEvent, ContentUpdateOperation, FileWatchEvent};
use aionui_runtime::Builder as CmdBuilder;

use crate::execution_runtime::{
    ErlCreateExecutionRequest, ErlRuntimeArtifact, ExecutionCallerContext, ExecutionRuntimeLayer, SnapshotRef,
};
use crate::workspace_resolver::{WorkspaceResolveError, WorkspaceResolveMode, WorkspaceResolver, safe_join_under_root};

const SUPER_ADMIN_ROLE_KEY: &str = "super_admin";
const DEFAULT_USER_ROLE_KEY: &str = "ordinary_user";
const ADMIN_PERMISSION: &str = "*";

#[derive(Clone)]
pub struct WorkbenchRouterState {
    pub users: Arc<dyn IPlatformUserRepository>,
    pub external_identities: Arc<dyn IExternalIdentityRepository>,
    pub roles: Arc<dyn IRoleRepository>,
    pub git_ssh_credentials: Arc<dyn IGitSshCredentialRepository>,
    pub git_projects: Arc<dyn IGitProjectRepository>,
    pub workspaces: Arc<dyn IWorkspaceRepository>,
    pub snapshots: Arc<dyn ISnapshotRepository>,
    pub execution_runs: Arc<dyn IExecutionRunRepository>,
    pub execution_artifacts: Arc<dyn IExecutionArtifactRepository>,
    pub audit_logs: Arc<dyn IAuditLogRepository>,
    pub senmo_root: PathBuf,
    pub gateway_auth: Option<GatewayAuthConfig>,
    pub encryption_key: [u8; 32],
    pub git_ops: Arc<dyn GitProjectOps>,
}

#[async_trait::async_trait]
pub trait GitProjectOps: Send + Sync {
    async fn verify_access(
        &self,
        state: &WorkbenchRouterState,
        credential: &GitSshCredentialRow,
        repo_ssh_url: &str,
    ) -> Result<i64, ApiError>;

    async fn clone_workspace(
        &self,
        state: &WorkbenchRouterState,
        user_id: &str,
        project: &GitProjectRow,
        branch_ref: Option<&str>,
        destination: &FsPath,
    ) -> Result<(), ApiError>;
}

#[derive(Debug, Default)]
pub struct DefaultGitProjectOps;

#[async_trait::async_trait]
impl GitProjectOps for DefaultGitProjectOps {
    async fn verify_access(
        &self,
        state: &WorkbenchRouterState,
        credential: &GitSshCredentialRow,
        repo_ssh_url: &str,
    ) -> Result<i64, ApiError> {
        verify_git_project_access(state, credential, repo_ssh_url).await
    }

    async fn clone_workspace(
        &self,
        state: &WorkbenchRouterState,
        user_id: &str,
        project: &GitProjectRow,
        branch_ref: Option<&str>,
        destination: &FsPath,
    ) -> Result<(), ApiError> {
        clone_git_project_workspace(state, user_id, project, branch_ref, destination).await
    }
}

#[derive(Debug, Deserialize)]
struct UpsertRoleRequest {
    role_key: String,
    role_name: String,
    #[serde(default = "enabled_status")]
    status: String,
    #[serde(default)]
    permissions: Vec<String>,
    #[serde(default = "default_sort_order")]
    sort_order: i32,
    #[serde(default)]
    is_system: bool,
}

#[derive(Debug, Deserialize)]
struct UpdateRoleRequest {
    role_name: Option<String>,
    status: Option<String>,
    permissions: Option<Vec<String>>,
    sort_order: Option<i32>,
}

#[derive(Debug, Clone, Serialize)]
struct PermissionDescriptor {
    key: &'static str,
    label: &'static str,
    description: &'static str,
}

#[derive(Debug, Deserialize)]
struct UpdateStatusRequest {
    status: String,
}

#[derive(Debug, Deserialize)]
struct ResetPasswordRequest {
    password: String,
}

#[derive(Debug, Serialize)]
struct AdminUserResponse {
    id: String,
    phone: Option<String>,
    username: Option<String>,
    display_name: Option<String>,
    email: Option<String>,
    avatar_path: Option<String>,
    status: String,
    roles: Vec<AdminUserRoleSummary>,
    external_identities: Vec<AdminUserExternalIdentitySummary>,
    created_at: i64,
    updated_at: i64,
    last_login: Option<i64>,
}

#[derive(Debug, Serialize)]
struct AdminUserRoleSummary {
    id: String,
    role_key: String,
    role_name: String,
}

#[derive(Debug, Serialize)]
struct AdminUserExternalIdentitySummary {
    provider: String,
    app_id: String,
    external_user_id: String,
    phone_snapshot: Option<String>,
    is_admin: bool,
    last_synced_at: i64,
}

#[derive(Debug, Deserialize)]
struct UpdateWorkspaceRequest {
    name: Option<String>,
    status: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CreateWorkspaceRequest {
    name: String,
    #[serde(default = "blank_source_type")]
    source_type: String,
    git_project_id: Option<String>,
    branch_ref: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UserSyncForm {
    #[serde(rename = "type")]
    sync_type: i32,
    #[serde(default)]
    app_id: Option<String>,
    #[serde(default, alias = "userId", alias = "user_id")]
    external_user_id: String,
    #[serde(default, alias = "mobile")]
    phone: Option<String>,
    #[serde(default, alias = "name", alias = "realName")]
    display_name: Option<String>,
    #[serde(default)]
    email: Option<String>,
    #[serde(default, alias = "isAdmin")]
    is_admin: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct CreateSshCredentialRequest {
    name: String,
    private_key: String,
    passphrase: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GenerateSshCredentialRequest {
    name: String,
    comment: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CreateGitProjectRequest {
    credential_id: String,
    repo_ssh_url: String,
    display_name: String,
    default_branch: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UpdateGitProjectRequest {
    credential_id: Option<String>,
    repo_ssh_url: Option<String>,
    display_name: Option<String>,
    default_branch: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CreateSnapshotRequest {
    #[serde(default)]
    environment_metadata: Value,
    #[serde(default)]
    retention_policy: Value,
}

#[derive(Debug, Deserialize)]
struct CreateExecutionRequest {
    snapshot_id: Option<String>,
    execution_type: String,
    #[serde(default)]
    policy: Value,
    #[serde(default)]
    resource_profile: Value,
    network_profile: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UpdateExecutionStatusRequest {
    status: String,
}

#[derive(Debug, Deserialize)]
struct WorkspaceFilePathRequest {
    #[serde(default)]
    relative_path: String,
}

#[derive(Debug, Deserialize)]
struct WorkspaceFileWriteRequest {
    relative_path: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct WorkspaceFileRenameRequest {
    relative_path: String,
    new_name: String,
}

#[derive(Debug, Serialize)]
struct WorkspaceResponse {
    id: String,
    name: String,
    source_type: String,
    git_project_id: Option<String>,
    branch_ref: Option<String>,
    status: String,
    root_rel_path: String,
    created_at: i64,
    updated_at: i64,
}

#[derive(Debug, Serialize)]
struct SshCredentialResponse {
    id: String,
    name: String,
    fingerprint: String,
    public_key: Option<String>,
    status: String,
    last_verified_at: Option<i64>,
    created_at: i64,
    updated_at: i64,
}

#[derive(Debug, Serialize)]
struct GeneratedSshCredentialResponse {
    credential: SshCredentialResponse,
    public_key: String,
}

#[derive(Debug, Serialize)]
struct GitProjectResponse {
    id: String,
    credential_id: String,
    repo_ssh_url: String,
    display_name: String,
    default_branch: Option<String>,
    status: String,
    last_verified_at: Option<i64>,
    last_synced_at: Option<i64>,
    created_at: i64,
    updated_at: i64,
}

#[derive(Debug, Serialize)]
struct SnapshotResponse {
    id: String,
    workspace_id: String,
    source_kind: String,
    artifact_ref: String,
    manifest_ref: String,
    checksum: String,
    size_bytes: i64,
    sealed_at: i64,
    expires_at: Option<i64>,
    gc_tombstone_at: Option<i64>,
    created_at: i64,
}

#[derive(Debug, Serialize)]
struct SnapshotGcResponse {
    scanned: usize,
    deleted_content: usize,
    skipped_active: usize,
    tombstoned: usize,
}

#[derive(Debug, Serialize)]
struct ExecutionRunResponse {
    id: String,
    workspace_id: String,
    snapshot_id: String,
    owner_user_id: String,
    execution_type: String,
    status: String,
    policy: Value,
    resource_profile: Value,
    network_profile: Option<String>,
    trace_id: String,
    preview_url: Option<String>,
    created_at: i64,
    started_at: Option<i64>,
    finished_at: Option<i64>,
    updated_at: i64,
}

#[derive(Debug, Serialize)]
struct ExecutionArtifactResponse {
    id: String,
    execution_id: String,
    artifact_type: String,
    ref_: String,
    metadata: Value,
    immutable: bool,
    created_at: i64,
}

#[derive(Debug, Serialize)]
struct WorkspaceFileEntryResponse {
    name: String,
    relative_path: String,
    is_dir: bool,
    size_bytes: Option<u64>,
}

#[derive(Debug, Serialize)]
struct WorkspaceFileMetadataResponse {
    relative_path: String,
    is_dir: bool,
    size_bytes: u64,
    modified_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
struct WorkspaceRuntimeContextResponse {
    workspace_id: String,
    relative_path: String,
    display_path: String,
}

#[derive(Debug, Clone)]
struct WorkspaceRuntimeContext {
    response: WorkspaceRuntimeContextResponse,
    cwd: PathBuf,
}

pub fn workbench_routes(state: WorkbenchRouterState) -> Router {
    Router::new()
        .route("/api/admin/users", get(list_users_handler))
        .route("/api/admin/users/{user_id}", get(get_user_handler))
        .route("/api/admin/users/{user_id}/status", post(update_user_status_handler))
        .route(
            "/api/admin/users/{user_id}/reset-password",
            post(reset_user_password_handler),
        )
        .route(
            "/api/admin/users/{user_id}/sync-status",
            get(user_sync_status_handler).post(user_sync_status_handler),
        )
        .route("/api/admin/roles", get(list_roles_handler).post(upsert_role_handler))
        .route("/api/admin/permissions", get(list_permissions_handler))
        .route(
            "/api/admin/roles/{role_id}",
            get(get_role_handler).patch(update_role_handler),
        )
        .route("/api/admin/roles/{role_id}/status", post(update_role_status_handler))
        .route(
            "/api/admin/roles/{role_id}/users/{user_id}",
            post(assign_role_handler).delete(revoke_role_handler),
        )
        .route(
            "/api/workspaces",
            get(list_workspaces_handler).post(create_workspace_handler),
        )
        .route(
            "/api/git/ssh-credentials",
            get(list_ssh_credentials_handler).post(create_ssh_credential_handler),
        )
        .route(
            "/api/git/ssh-credentials/generate",
            post(generate_ssh_credential_handler),
        )
        .route(
            "/api/git/projects",
            get(list_git_projects_handler).post(create_git_project_handler),
        )
        .route(
            "/api/git/projects/{project_id}",
            get(get_git_project_handler)
                .patch(update_git_project_handler)
                .delete(delete_git_project_handler),
        )
        .route("/api/git/projects/{project_id}/sync", post(sync_git_project_handler))
        .route(
            "/api/git/projects/{project_id}/archive",
            post(archive_git_project_handler),
        )
        .route(
            "/api/workspaces/{workspace_id}",
            get(get_workspace_handler)
                .patch(update_workspace_handler)
                .delete(delete_workspace_handler),
        )
        .route(
            "/api/workspaces/{workspace_id}/archive",
            post(archive_workspace_handler),
        )
        .route(
            "/api/workspaces/{workspace_id}/restore",
            post(restore_workspace_handler),
        )
        .route(
            "/api/workspaces/{workspace_id}/snapshots",
            get(list_snapshots_handler).post(create_snapshot_handler),
        )
        .route(
            "/api/workspaces/{workspace_id}/executions",
            get(list_executions_handler).post(create_execution_handler),
        )
        .route(
            "/api/executions/{execution_id}/status",
            post(update_execution_status_handler),
        )
        .route("/api/executions/{execution_id}/cancel", post(cancel_execution_handler))
        .route(
            "/api/executions/{execution_id}/artifacts",
            get(list_execution_artifacts_handler),
        )
        .route("/api/admin/snapshots/gc", post(snapshot_gc_handler))
        .route(
            "/api/workspaces/{workspace_id}/files/list",
            post(list_workspace_directory_handler),
        )
        .route(
            "/api/workspaces/{workspace_id}/files/metadata",
            post(workspace_file_metadata_handler),
        )
        .route(
            "/api/workspaces/{workspace_id}/files/read",
            post(read_workspace_file_handler),
        )
        .route(
            "/api/workspaces/{workspace_id}/files/read-buffer",
            post(read_workspace_file_buffer_handler),
        )
        .route(
            "/api/workspaces/{workspace_id}/files/write",
            post(write_workspace_file_handler),
        )
        .route(
            "/api/workspaces/{workspace_id}/files/upload",
            post(upload_workspace_file_handler),
        )
        .route(
            "/api/workspaces/{workspace_id}/files/mkdir",
            post(mkdir_workspace_handler),
        )
        .route(
            "/api/workspaces/{workspace_id}/files/delete",
            post(delete_workspace_path_handler),
        )
        .route(
            "/api/workspaces/{workspace_id}/files/rename",
            post(rename_workspace_path_handler),
        )
        .route(
            "/api/workspaces/{workspace_id}/files/watch-payload",
            post(workspace_file_watch_payload_handler),
        )
        .route(
            "/api/workspaces/{workspace_id}/terminals",
            post(workspace_terminal_context_handler),
        )
        .route(
            "/api/workspaces/{workspace_id}/git/context",
            post(workspace_git_context_handler),
        )
        .with_state(state)
}

pub fn workbench_public_routes(state: WorkbenchRouterState) -> Router {
    Router::new()
        .route("/api/open-platform/user-sync", post(user_sync_handler))
        .route_layer(from_fn_with_state(state.clone(), user_sync_auth_middleware))
        .with_state(state)
}

async fn user_sync_auth_middleware(
    State(state): State<WorkbenchRouterState>,
    request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    verify_sync_secret(&state, request.headers())?;
    Ok(next.run(request).await)
}

async fn list_users_handler(
    State(state): State<WorkbenchRouterState>,
    Extension(user): Extension<CurrentUser>,
) -> Result<Json<ApiResponse<Vec<AdminUserResponse>>>, ApiError> {
    ensure_permission(&state, &user, "admin:user:list").await?;
    let users = state.users.list_users(200, 0).await.map_err(db_error)?;
    let mut response = Vec::with_capacity(users.len());
    for user in users {
        response.push(admin_user_response(&state, user).await?);
    }
    Ok(Json(ApiResponse::ok(response)))
}

async fn get_user_handler(
    State(state): State<WorkbenchRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(user_id): Path<String>,
) -> Result<Json<ApiResponse<AdminUserResponse>>, ApiError> {
    ensure_permission(&state, &user, "admin:user:read").await?;
    let target = state
        .users
        .find_by_id(&user_id)
        .await
        .map_err(db_error)?
        .ok_or_else(|| ApiError::NotFound(format!("User '{user_id}' not found")))?;
    Ok(Json(ApiResponse::ok(admin_user_response(&state, target).await?)))
}

async fn update_user_status_handler(
    State(state): State<WorkbenchRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(user_id): Path<String>,
    Json(req): Json<UpdateStatusRequest>,
) -> Result<Json<ApiResponse<()>>, ApiError> {
    ensure_permission(&state, &user, "admin:user:update").await?;
    validate_status(&req.status, &["enabled", "disabled"])?;
    state
        .users
        .update_status(&user_id, &req.status)
        .await
        .map_err(db_error)?;
    audit(
        &state,
        Some(&user.id),
        "user",
        Some(&user_id),
        "update_status",
        json!({ "status": req.status }),
    )
    .await;
    Ok(Json(ApiResponse::ok(())))
}

async fn reset_user_password_handler(
    State(state): State<WorkbenchRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(user_id): Path<String>,
    Json(req): Json<ResetPasswordRequest>,
) -> Result<Json<ApiResponse<()>>, ApiError> {
    ensure_permission(&state, &user, "admin:user:reset-password").await?;
    if req.password.len() < 8 || req.password.len() > 128 {
        return Err(ApiError::BadRequest(
            "Password length must be between 8 and 128 characters".into(),
        ));
    }
    let password_hash =
        hash_password(&req.password).map_err(|error| ApiError::Internal(format!("Password hash error: {error}")))?;
    state
        .users
        .update_password_hash(&user_id, Some(&password_hash))
        .await
        .map_err(db_error)?;
    audit(
        &state,
        Some(&user.id),
        "user",
        Some(&user_id),
        "reset_password",
        json!({}),
    )
    .await;
    Ok(Json(ApiResponse::ok(())))
}

async fn user_sync_status_handler(
    State(state): State<WorkbenchRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(user_id): Path<String>,
) -> Result<Json<ApiResponse<Value>>, ApiError> {
    ensure_permission(&state, &user, "admin:user:sync-status").await?;
    let target = state
        .users
        .find_by_id(&user_id)
        .await
        .map_err(db_error)?
        .ok_or_else(|| ApiError::NotFound(format!("User '{user_id}' not found")))?;
    let identities = state
        .external_identities
        .list_by_user(&user_id)
        .await
        .map_err(db_error)?;
    Ok(Json(ApiResponse::ok(json!({
        "user_id": target.id,
        "status": target.status,
        "external_identities": identities
    }))))
}

async fn list_roles_handler(
    State(state): State<WorkbenchRouterState>,
    Extension(user): Extension<CurrentUser>,
) -> Result<Json<ApiResponse<Vec<RoleRow>>>, ApiError> {
    ensure_permission(&state, &user, "admin:role:list").await?;
    let roles = state.roles.list_roles().await.map_err(db_error)?;
    Ok(Json(ApiResponse::ok(roles)))
}

async fn list_permissions_handler(
    State(state): State<WorkbenchRouterState>,
    Extension(user): Extension<CurrentUser>,
) -> Result<Json<ApiResponse<Vec<PermissionDescriptor>>>, ApiError> {
    ensure_permission(&state, &user, "admin:role:list").await?;
    Ok(Json(ApiResponse::ok(permission_registry().to_vec())))
}

async fn upsert_role_handler(
    State(state): State<WorkbenchRouterState>,
    Extension(user): Extension<CurrentUser>,
    Json(req): Json<UpsertRoleRequest>,
) -> Result<Json<ApiResponse<RoleRow>>, ApiError> {
    ensure_permission(&state, &user, "admin:role:create").await?;
    validate_status(&req.status, &["enabled", "disabled"])?;
    let role = state
        .roles
        .upsert_role(UpsertRoleParams {
            role_key: req.role_key,
            role_name: req.role_name,
            status: req.status,
            permissions: Value::Array(req.permissions.into_iter().map(Value::String).collect()),
            sort_order: req.sort_order,
            is_system: req.is_system,
        })
        .await
        .map_err(db_error)?;
    audit(
        &state,
        Some(&user.id),
        "role",
        Some(&role.id),
        "upsert",
        json!({ "role_key": role.role_key }),
    )
    .await;
    Ok(Json(ApiResponse::ok(role)))
}

async fn get_role_handler(
    State(state): State<WorkbenchRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(role_id): Path<String>,
) -> Result<Json<ApiResponse<RoleRow>>, ApiError> {
    ensure_permission(&state, &user, "admin:role:read").await?;
    let role = find_role_by_id(&state, &role_id).await?;
    Ok(Json(ApiResponse::ok(role)))
}

async fn update_role_handler(
    State(state): State<WorkbenchRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(role_id): Path<String>,
    Json(req): Json<UpdateRoleRequest>,
) -> Result<Json<ApiResponse<RoleRow>>, ApiError> {
    ensure_permission(&state, &user, "admin:role:update").await?;
    let existing = find_role_by_id(&state, &role_id).await?;
    let status = req.status.unwrap_or_else(|| existing.status.clone());
    validate_status(&status, &["enabled", "disabled"])?;
    let permissions = req
        .permissions
        .map(|items| Value::Array(items.into_iter().map(Value::String).collect()))
        .unwrap_or(existing.permissions);
    let role = state
        .roles
        .upsert_role(UpsertRoleParams {
            role_key: existing.role_key,
            role_name: req.role_name.unwrap_or(existing.role_name),
            status,
            permissions,
            sort_order: req.sort_order.unwrap_or(existing.sort_order),
            is_system: existing.is_system,
        })
        .await
        .map_err(db_error)?;
    audit(
        &state,
        Some(&user.id),
        "role",
        Some(&role.id),
        "update",
        json!({ "role_key": role.role_key }),
    )
    .await;
    Ok(Json(ApiResponse::ok(role)))
}

async fn update_role_status_handler(
    State(state): State<WorkbenchRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(role_id): Path<String>,
    Json(req): Json<UpdateStatusRequest>,
) -> Result<Json<ApiResponse<()>>, ApiError> {
    ensure_permission(&state, &user, "admin:role:update").await?;
    validate_status(&req.status, &["enabled", "disabled"])?;
    state
        .roles
        .set_role_status(&role_id, &req.status)
        .await
        .map_err(db_error)?;
    audit(
        &state,
        Some(&user.id),
        "role",
        Some(&role_id),
        "update_status",
        json!({ "status": req.status }),
    )
    .await;
    Ok(Json(ApiResponse::ok(())))
}

async fn assign_role_handler(
    State(state): State<WorkbenchRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path((role_id, user_id)): Path<(String, String)>,
) -> Result<Json<ApiResponse<()>>, ApiError> {
    ensure_permission(&state, &user, "admin:role:assign").await?;
    state.roles.assign_role(&user_id, &role_id).await.map_err(db_error)?;
    audit(
        &state,
        Some(&user.id),
        "user_role",
        Some(&user_id),
        "assign",
        json!({ "role_id": role_id }),
    )
    .await;
    Ok(Json(ApiResponse::ok(())))
}

async fn revoke_role_handler(
    State(state): State<WorkbenchRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path((role_id, user_id)): Path<(String, String)>,
) -> Result<Json<ApiResponse<()>>, ApiError> {
    ensure_permission(&state, &user, "admin:role:assign").await?;
    state.roles.revoke_role(&user_id, &role_id).await.map_err(db_error)?;
    audit(
        &state,
        Some(&user.id),
        "user_role",
        Some(&user_id),
        "revoke",
        json!({ "role_id": role_id }),
    )
    .await;
    Ok(Json(ApiResponse::ok(())))
}

async fn list_workspaces_handler(
    State(state): State<WorkbenchRouterState>,
    Extension(user): Extension<CurrentUser>,
) -> Result<Json<ApiResponse<Vec<WorkspaceResponse>>>, ApiError> {
    let workspaces = state.workspaces.list_by_owner(&user.id).await.map_err(db_error)?;
    Ok(Json(ApiResponse::ok(
        workspaces.into_iter().map(workspace_response).collect(),
    )))
}

async fn create_workspace_handler(
    State(state): State<WorkbenchRouterState>,
    Extension(user): Extension<CurrentUser>,
    Json(req): Json<CreateWorkspaceRequest>,
) -> Result<Json<ApiResponse<WorkspaceResponse>>, ApiError> {
    let root_rel_path = format!("{}/{}", user.id, aionui_common::generate_prefixed_id("wsdir"));
    let absolute_root = safe_join_under_root(&state.senmo_root, &root_rel_path).map_err(workspace_resolve_error)?;
    let (source_type, git_project_id, audit_action) = match req.source_type.as_str() {
        "blank" => {
            if req.git_project_id.is_some() {
                return Err(ApiError::BadRequest(
                    "Blank workspace must not include git_project_id".into(),
                ));
            }
            tokio::fs::create_dir_all(&absolute_root)
                .await
                .map_err(|error| ApiError::Internal(format!("Failed to create workspace directory: {error}")))?;
            ("blank".to_owned(), None, "create_blank")
        }
        "git_project" => {
            let project_id = req
                .git_project_id
                .as_deref()
                .ok_or_else(|| ApiError::BadRequest("git_project_id is required".into()))?;
            let project = owned_git_project(&state, &user.id, project_id).await?;
            state
                .git_ops
                .clone_workspace(&state, &user.id, &project, req.branch_ref.as_deref(), &absolute_root)
                .await?;
            ("git_project".to_owned(), Some(project.id), "create_git_project")
        }
        other => {
            return Err(ApiError::BadRequest(format!(
                "Unsupported workspace source_type '{other}'"
            )));
        }
    };

    let workspace = state
        .workspaces
        .create_workspace(CreateWorkspaceParams {
            owner_user_id: user.id.clone(),
            name: req.name,
            source_type,
            git_project_id,
            branch_ref: req.branch_ref,
            status: "active".into(),
            root_rel_path,
        })
        .await
        .map_err(db_error)?;
    audit(
        &state,
        Some(&user.id),
        "workspace",
        Some(&workspace.id),
        audit_action,
        json!({ "root_rel_path": workspace.root_rel_path, "git_project_id": workspace.git_project_id }),
    )
    .await;
    Ok(Json(ApiResponse::ok(workspace_response(workspace))))
}

async fn list_ssh_credentials_handler(
    State(state): State<WorkbenchRouterState>,
    Extension(user): Extension<CurrentUser>,
) -> Result<Json<ApiResponse<Vec<SshCredentialResponse>>>, ApiError> {
    let credentials = state
        .git_ssh_credentials
        .list_by_owner(&user.id)
        .await
        .map_err(db_error)?;
    Ok(Json(ApiResponse::ok(
        credentials.into_iter().map(ssh_credential_response).collect(),
    )))
}

async fn create_ssh_credential_handler(
    State(state): State<WorkbenchRouterState>,
    Extension(user): Extension<CurrentUser>,
    Json(req): Json<CreateSshCredentialRequest>,
) -> Result<Json<ApiResponse<SshCredentialResponse>>, ApiError> {
    let private_key = req.private_key.trim();
    if private_key.is_empty() {
        return Err(ApiError::BadRequest("private_key is required".into()));
    }
    validate_ssh_private_key_format(private_key)?;
    let encrypted_private_key = encrypt_string(private_key, &state.encryption_key)
        .map_err(|error| ApiError::Internal(format!("Credential encryption failed: {error}")))?;
    let encrypted_passphrase = match req.passphrase.as_deref().filter(|value| !value.is_empty()) {
        Some(passphrase) => Some(
            encrypt_string(passphrase, &state.encryption_key)
                .map_err(|error| ApiError::Internal(format!("Credential encryption failed: {error}")))?,
        ),
        None => None,
    };
    let credential = state
        .git_ssh_credentials
        .create_credential(CreateGitSshCredentialParams {
            owner_user_id: user.id.clone(),
            name: req.name,
            fingerprint: fingerprint_private_key(private_key),
            encrypted_private_key,
            encrypted_passphrase,
            public_key: None,
            status: "pending".into(),
        })
        .await
        .map_err(db_error)?;
    audit(
        &state,
        Some(&user.id),
        "git_ssh_credential",
        Some(&credential.id),
        "create",
        json!({ "fingerprint": credential.fingerprint }),
    )
    .await;
    Ok(Json(ApiResponse::ok(ssh_credential_response(credential))))
}

async fn generate_ssh_credential_handler(
    State(state): State<WorkbenchRouterState>,
    Extension(user): Extension<CurrentUser>,
    Json(req): Json<GenerateSshCredentialRequest>,
) -> Result<Json<ApiResponse<GeneratedSshCredentialResponse>>, ApiError> {
    let key_dir = state
        .senmo_root
        .join("tmp")
        .join(aionui_common::generate_prefixed_id("sshgen"));
    tokio::fs::create_dir_all(&key_dir)
        .await
        .map_err(|error| ApiError::Internal(format!("Failed to create temporary key directory: {error}")))?;
    let private_key_path = key_dir.join("id_ed25519");
    let comment = req.comment.unwrap_or_else(|| format!("senmo-{}", user.id));
    let mut builder = CmdBuilder::clean_cli("ssh-keygen");
    builder.args([
        "-q",
        "-t",
        "ed25519",
        "-N",
        "",
        "-C",
        &comment,
        "-f",
        &private_key_path.to_string_lossy(),
    ]);
    let output = tokio::time::timeout(Duration::from_secs(10), builder.output())
        .await
        .map_err(|_| {
            tracing::warn!(key_dir = %key_dir.display(), "ssh-keygen timed out while generating credential");
            ApiError::Internal("ssh-keygen timed out".into())
        })?
        .map_err(|error| {
            tracing::warn!(key_dir = %key_dir.display(), error = %error, "ssh-keygen failed to start");
            ApiError::Internal(format!("ssh-keygen failed to start: {error}"))
        })?;
    if !output.status.success() {
        let message = sanitize_process_error_with_default(&output.stderr, "ssh-keygen failed");
        tracing::warn!(
            key_dir = %key_dir.display(),
            status = ?output.status.code(),
            error = %message,
            "ssh-keygen failed while generating credential"
        );
        let _ = tokio::fs::remove_dir_all(&key_dir).await;
        return Err(ApiError::Internal(message));
    }

    let private_key = tokio::fs::read_to_string(&private_key_path)
        .await
        .map_err(|error| ApiError::Internal(format!("Generated private key missing: {error}")))?;
    let public_key = tokio::fs::read_to_string(private_key_path.with_extension("pub"))
        .await
        .map_err(|error| ApiError::Internal(format!("Generated public key missing: {error}")))?
        .trim()
        .to_owned();
    let _ = tokio::fs::remove_dir_all(&key_dir).await;

    let encrypted_private_key = encrypt_string(&private_key, &state.encryption_key).map_err(|error| {
        tracing::warn!(error = %error, "failed to encrypt generated SSH private key");
        ApiError::Internal(format!("Credential encryption failed: {error}"))
    })?;

    let credential = state
        .git_ssh_credentials
        .create_credential(CreateGitSshCredentialParams {
            owner_user_id: user.id.clone(),
            name: req.name,
            fingerprint: fingerprint_private_key(&private_key),
            encrypted_private_key,
            encrypted_passphrase: None,
            public_key: Some(public_key.clone()),
            status: "pending".into(),
        })
        .await
        .map_err(|error| {
            tracing::warn!(error = %error, "failed to persist generated SSH credential");
            db_error(error)
        })?;
    audit(
        &state,
        Some(&user.id),
        "git_ssh_credential",
        Some(&credential.id),
        "generate",
        json!({ "fingerprint": credential.fingerprint }),
    )
    .await;
    Ok(Json(ApiResponse::ok(GeneratedSshCredentialResponse {
        credential: ssh_credential_response(credential),
        public_key,
    })))
}

async fn list_git_projects_handler(
    State(state): State<WorkbenchRouterState>,
    Extension(user): Extension<CurrentUser>,
) -> Result<Json<ApiResponse<Vec<GitProjectResponse>>>, ApiError> {
    let projects = state.git_projects.list_by_owner(&user.id).await.map_err(db_error)?;
    Ok(Json(ApiResponse::ok(
        projects.into_iter().map(git_project_response).collect(),
    )))
}

async fn create_git_project_handler(
    State(state): State<WorkbenchRouterState>,
    Extension(user): Extension<CurrentUser>,
    Json(req): Json<CreateGitProjectRequest>,
) -> Result<Json<ApiResponse<GitProjectResponse>>, ApiError> {
    let credential = owned_credential(&state, &user.id, &req.credential_id).await?;
    let verified_at = state
        .git_ops
        .verify_access(&state, &credential, &req.repo_ssh_url)
        .await?;
    let project = state
        .git_projects
        .create_project(CreateGitProjectParams {
            owner_user_id: user.id.clone(),
            credential_id: credential.id,
            repo_ssh_url: req.repo_ssh_url,
            display_name: req.display_name,
            default_branch: req.default_branch,
            status: "active".into(),
        })
        .await
        .map_err(db_error)?;
    state
        .git_projects
        .update_status(&project.id, "active", Some(verified_at), Some(verified_at))
        .await
        .map_err(db_error)?;
    state
        .git_ssh_credentials
        .update_status(&project.credential_id, "verified", Some(verified_at))
        .await
        .map_err(db_error)?;
    audit(
        &state,
        Some(&user.id),
        "git_project",
        Some(&project.id),
        "create",
        json!({ "repo_ssh_url": project.repo_ssh_url, "credential_id": project.credential_id }),
    )
    .await;
    let project = owned_git_project(&state, &user.id, &project.id).await?;
    Ok(Json(ApiResponse::ok(git_project_response(project))))
}

async fn get_git_project_handler(
    State(state): State<WorkbenchRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(project_id): Path<String>,
) -> Result<Json<ApiResponse<GitProjectResponse>>, ApiError> {
    let project = owned_git_project(&state, &user.id, &project_id).await?;
    Ok(Json(ApiResponse::ok(git_project_response(project))))
}

async fn update_git_project_handler(
    State(state): State<WorkbenchRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(project_id): Path<String>,
    Json(req): Json<UpdateGitProjectRequest>,
) -> Result<Json<ApiResponse<GitProjectResponse>>, ApiError> {
    let project = owned_git_project(&state, &user.id, &project_id).await?;
    let credential_id = req.credential_id.as_deref().unwrap_or(&project.credential_id);
    let repo_ssh_url = req.repo_ssh_url.as_deref().unwrap_or(&project.repo_ssh_url);
    let display_name = req.display_name.as_deref().unwrap_or(&project.display_name);
    let default_branch = req.default_branch.as_deref().or(project.default_branch.as_deref());
    let credential = owned_credential(&state, &user.id, credential_id).await?;
    if credential_id != project.credential_id || repo_ssh_url != project.repo_ssh_url {
        let verified_at = state.git_ops.verify_access(&state, &credential, repo_ssh_url).await?;
        state
            .git_ssh_credentials
            .update_status(&credential.id, "verified", Some(verified_at))
            .await
            .map_err(db_error)?;
    }
    let project = state
        .git_projects
        .update_project(&project_id, credential_id, repo_ssh_url, display_name, default_branch)
        .await
        .map_err(db_error)?;
    audit(
        &state,
        Some(&user.id),
        "git_project",
        Some(&project.id),
        "update",
        json!({ "repo_ssh_url": project.repo_ssh_url, "credential_id": project.credential_id }),
    )
    .await;
    Ok(Json(ApiResponse::ok(git_project_response(project))))
}

async fn delete_git_project_handler(
    State(state): State<WorkbenchRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(project_id): Path<String>,
) -> Result<Json<ApiResponse<()>>, ApiError> {
    let project = owned_git_project(&state, &user.id, &project_id).await?;
    if state
        .workspaces
        .has_active_by_git_project(&project.id)
        .await
        .map_err(db_error)?
    {
        return Err(ApiError::Conflict(
            "Git project has active workspaces and cannot be deleted".into(),
        ));
    }
    state
        .git_projects
        .update_status(&project.id, "deleted", project.last_verified_at, project.last_synced_at)
        .await
        .map_err(db_error)?;
    audit(
        &state,
        Some(&user.id),
        "git_project",
        Some(&project.id),
        "delete",
        json!({}),
    )
    .await;
    Ok(Json(ApiResponse::ok(())))
}

async fn sync_git_project_handler(
    State(state): State<WorkbenchRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(project_id): Path<String>,
) -> Result<Json<ApiResponse<GitProjectResponse>>, ApiError> {
    let project = owned_git_project(&state, &user.id, &project_id).await?;
    let credential = owned_credential(&state, &user.id, &project.credential_id).await?;
    let verified_at = state
        .git_ops
        .verify_access(&state, &credential, &project.repo_ssh_url)
        .await?;
    state
        .git_projects
        .update_status(&project.id, "active", Some(verified_at), Some(verified_at))
        .await
        .map_err(db_error)?;
    state
        .git_ssh_credentials
        .update_status(&credential.id, "verified", Some(verified_at))
        .await
        .map_err(db_error)?;
    audit(
        &state,
        Some(&user.id),
        "git_project",
        Some(&project.id),
        "sync",
        json!({ "status": "active" }),
    )
    .await;
    let project = owned_git_project(&state, &user.id, &project_id).await?;
    Ok(Json(ApiResponse::ok(git_project_response(project))))
}

async fn archive_git_project_handler(
    State(state): State<WorkbenchRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(project_id): Path<String>,
) -> Result<Json<ApiResponse<()>>, ApiError> {
    let project = owned_git_project(&state, &user.id, &project_id).await?;
    state
        .git_projects
        .update_status(
            &project.id,
            "archived",
            project.last_verified_at,
            project.last_synced_at,
        )
        .await
        .map_err(db_error)?;
    audit(
        &state,
        Some(&user.id),
        "git_project",
        Some(&project.id),
        "archive",
        json!({}),
    )
    .await;
    Ok(Json(ApiResponse::ok(())))
}

async fn get_workspace_handler(
    State(state): State<WorkbenchRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(workspace_id): Path<String>,
) -> Result<Json<ApiResponse<WorkspaceResponse>>, ApiError> {
    let workspace = owned_workspace(&state, &user.id, &workspace_id).await?;
    Ok(Json(ApiResponse::ok(workspace_response(workspace))))
}

async fn update_workspace_handler(
    State(state): State<WorkbenchRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(workspace_id): Path<String>,
    Json(req): Json<UpdateWorkspaceRequest>,
) -> Result<Json<ApiResponse<WorkspaceResponse>>, ApiError> {
    let workspace = owned_workspace(&state, &user.id, &workspace_id).await?;
    if workspace.status == "deleted" {
        return Err(ApiError::BadRequest("Deleted workspace cannot be updated".into()));
    }
    if let Some(status) = req.status.as_deref() {
        validate_status(status, &["active", "archived"])?;
        state
            .workspaces
            .update_status(&workspace_id, status)
            .await
            .map_err(db_error)?;
    }
    if let Some(name) = req.name.as_deref().filter(|value| !value.trim().is_empty()) {
        state
            .workspaces
            .update_name(&workspace_id, name.trim())
            .await
            .map_err(db_error)?;
    }
    let updated = owned_workspace(&state, &user.id, &workspace_id).await?;
    audit(
        &state,
        Some(&user.id),
        "workspace",
        Some(&workspace_id),
        "update",
        json!({ "name_changed": req.name.is_some(), "status": req.status }),
    )
    .await;
    Ok(Json(ApiResponse::ok(workspace_response(updated))))
}

async fn archive_workspace_handler(
    State(state): State<WorkbenchRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(workspace_id): Path<String>,
) -> Result<Json<ApiResponse<()>>, ApiError> {
    let workspace = owned_workspace(&state, &user.id, &workspace_id).await?;
    if workspace.status == "deleted" {
        return Err(ApiError::BadRequest("Deleted workspace cannot be archived".into()));
    }
    state
        .workspaces
        .update_status(&workspace_id, "archived")
        .await
        .map_err(db_error)?;
    audit(
        &state,
        Some(&user.id),
        "workspace",
        Some(&workspace_id),
        "archive",
        json!({}),
    )
    .await;
    Ok(Json(ApiResponse::ok(())))
}

async fn restore_workspace_handler(
    State(state): State<WorkbenchRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(workspace_id): Path<String>,
) -> Result<Json<ApiResponse<()>>, ApiError> {
    let workspace = owned_workspace(&state, &user.id, &workspace_id).await?;
    if workspace.status == "deleted" {
        return Err(ApiError::BadRequest("Deleted workspace cannot be restored".into()));
    }
    state
        .workspaces
        .update_status(&workspace_id, "active")
        .await
        .map_err(db_error)?;
    audit(
        &state,
        Some(&user.id),
        "workspace",
        Some(&workspace_id),
        "restore",
        json!({}),
    )
    .await;
    Ok(Json(ApiResponse::ok(())))
}

async fn delete_workspace_handler(
    State(state): State<WorkbenchRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(workspace_id): Path<String>,
) -> Result<Json<ApiResponse<()>>, ApiError> {
    let workspace = owned_workspace(&state, &user.id, &workspace_id).await?;
    state.workspaces.mark_deleted(&workspace_id).await.map_err(db_error)?;
    let absolute_root =
        safe_join_under_root(&state.senmo_root, &workspace.root_rel_path).map_err(workspace_resolve_error)?;
    if let Err(error) = tokio::fs::remove_dir_all(&absolute_root).await {
        tracing::warn!(workspace_id = %workspace_id, error = %error, "failed to remove workspace directory");
    }
    audit(
        &state,
        Some(&user.id),
        "workspace",
        Some(&workspace_id),
        "delete",
        json!({}),
    )
    .await;
    Ok(Json(ApiResponse::ok(())))
}

async fn list_snapshots_handler(
    State(state): State<WorkbenchRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(workspace_id): Path<String>,
) -> Result<Json<ApiResponse<Vec<SnapshotResponse>>>, ApiError> {
    let _workspace = owned_workspace(&state, &user.id, &workspace_id).await?;
    let snapshots = state
        .snapshots
        .list_by_workspace(&workspace_id)
        .await
        .map_err(db_error)?;
    Ok(Json(ApiResponse::ok(
        snapshots.into_iter().map(snapshot_response).collect(),
    )))
}

async fn create_snapshot_handler(
    State(state): State<WorkbenchRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(workspace_id): Path<String>,
    Json(req): Json<CreateSnapshotRequest>,
) -> Result<Json<ApiResponse<SnapshotResponse>>, ApiError> {
    let workspace = owned_workspace(&state, &user.id, &workspace_id).await?;
    let snapshot = create_snapshot_for_workspace(
        &state,
        &user.id,
        workspace,
        req.environment_metadata,
        req.retention_policy,
    )
    .await?;
    audit(
        &state,
        Some(&user.id),
        "snapshot",
        Some(&snapshot.id),
        "create",
        json!({ "workspace_id": workspace_id, "checksum": snapshot.checksum }),
    )
    .await;
    Ok(Json(ApiResponse::ok(snapshot_response(snapshot))))
}

async fn list_executions_handler(
    State(state): State<WorkbenchRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(workspace_id): Path<String>,
) -> Result<Json<ApiResponse<Vec<ExecutionRunResponse>>>, ApiError> {
    let _workspace = owned_workspace(&state, &user.id, &workspace_id).await?;
    let executions = state
        .execution_runs
        .list_by_workspace(&workspace_id)
        .await
        .map_err(db_error)?
        .into_iter()
        .filter(|execution| execution.owner_user_id == user.id)
        .map(execution_run_response)
        .collect();
    Ok(Json(ApiResponse::ok(executions)))
}

async fn create_execution_handler(
    State(state): State<WorkbenchRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(workspace_id): Path<String>,
    Json(req): Json<CreateExecutionRequest>,
) -> Result<Json<ApiResponse<ExecutionRunResponse>>, ApiError> {
    let workspace = owned_workspace(&state, &user.id, &workspace_id).await?;
    validate_execution_type(&req.execution_type)?;
    let snapshot = if let Some(snapshot_id) = req.snapshot_id.as_deref() {
        let snapshot = state
            .snapshots
            .find_by_id(snapshot_id)
            .await
            .map_err(db_error)?
            .ok_or_else(|| ApiError::NotFound(format!("Snapshot '{snapshot_id}' not found")))?;
        if snapshot.workspace_id != workspace_id {
            return Err(ApiError::Forbidden("Snapshot does not belong to workspace".into()));
        }
        snapshot
    } else {
        create_snapshot_for_workspace(&state, &user.id, workspace, json!({}), json!({})).await?
    };
    let trace_id = aionui_common::generate_prefixed_id("trace");
    let policy = normalized_execution_policy(&req);
    let erl_request = ErlCreateExecutionRequest {
        snapshot_ref: SnapshotRef {
            snapshot_id: snapshot.id.clone(),
            artifact_ref: snapshot.artifact_ref.clone(),
            manifest_ref: snapshot.manifest_ref.clone(),
            checksum: snapshot.checksum.clone(),
        },
        execution_type: req.execution_type.clone(),
        policy: policy.clone(),
        resource_profile: req.resource_profile.clone(),
        network_profile: req.network_profile.clone(),
        env_refs: Vec::new(),
        trace_id: trace_id.clone(),
        caller: ExecutionCallerContext {
            user_id: user.id.clone(),
        },
    };
    let runtime = ExecutionRuntimeLayer::default_k8s();
    let erl_acceptance = runtime
        .create_execution(&erl_request)
        .map_err(|error| ApiError::BadRequest(error.to_string()))?;
    let mut execution = state
        .execution_runs
        .create_execution_run(CreateExecutionRunParams {
            workspace_id: workspace_id.clone(),
            snapshot_id: snapshot.id,
            owner_user_id: user.id.clone(),
            execution_type: req.execution_type,
            status: erl_acceptance.initial_status,
            policy,
            resource_profile: req.resource_profile,
            network_profile: req.network_profile,
            trace_id,
        })
        .await
        .map_err(db_error)?;
    let execution_id_for_runtime = execution.id.clone();
    let erl_request_for_runtime = erl_request.clone();
    let runtime_result =
        tokio::task::spawn_blocking(move || runtime.run_execution(&execution_id_for_runtime, &erl_request_for_runtime))
            .await
            .map_err(|error| {
                tracing::error!(error = %error, "execution runtime task failed");
                ApiError::Internal("Execution runtime task failed".into())
            })?
            .map_err(|error| ApiError::BadRequest(error.to_string()))?;
    state
        .execution_runs
        .update_status(
            &execution.id,
            &runtime_result.status,
            Some(aionui_common::now_ms()),
            if runtime_result.status == "succeeded" || runtime_result.status == "failed" {
                Some(aionui_common::now_ms())
            } else {
                None
            },
        )
        .await
        .map_err(db_error)?;
    if runtime_result.preview_url.is_some() {
        state
            .execution_runs
            .set_preview_url(&execution.id, runtime_result.preview_url.as_deref())
            .await
            .map_err(db_error)?;
    }
    persist_runtime_artifacts(&state, &execution.id, runtime_result.artifacts).await?;
    execution.status = runtime_result.status;
    execution.preview_url = runtime_result.preview_url;
    execution.started_at = Some(aionui_common::now_ms());
    audit(
        &state,
        Some(&user.id),
        "execution_run",
        Some(&execution.id),
        "create",
        json!({ "workspace_id": workspace_id, "snapshot_id": execution.snapshot_id, "execution_type": execution.execution_type }),
    )
    .await;
    audit(
        &state,
        Some(&user.id),
        "workspace",
        Some(&workspace_id),
        "runtime_start",
        json!({ "execution_id": execution.id, "snapshot_id": execution.snapshot_id, "execution_type": execution.execution_type }),
    )
    .await;
    Ok(Json(ApiResponse::ok(execution_run_response(execution))))
}

async fn persist_runtime_artifacts(
    state: &WorkbenchRouterState,
    execution_id: &str,
    artifacts: Vec<ErlRuntimeArtifact>,
) -> Result<(), ApiError> {
    for artifact in artifacts {
        state
            .execution_artifacts
            .create_artifact(CreateExecutionArtifactParams {
                execution_id: execution_id.to_owned(),
                artifact_type: artifact.artifact_type,
                ref_: artifact.artifact_ref,
                metadata: artifact.metadata,
                immutable: artifact.immutable,
            })
            .await
            .map_err(db_error)?;
    }
    Ok(())
}

async fn update_execution_status_handler(
    State(state): State<WorkbenchRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(execution_id): Path<String>,
    Json(req): Json<UpdateExecutionStatusRequest>,
) -> Result<Json<ApiResponse<ExecutionRunResponse>>, ApiError> {
    validate_execution_status(&req.status)?;
    let execution = owned_execution(&state, &user.id, &execution_id).await?;
    let now = aionui_common::now_ms();
    let started_at = if req.status == "running" && execution.started_at.is_none() {
        Some(now)
    } else {
        execution.started_at
    };
    let finished_at = if is_terminal_execution_status(&req.status) {
        Some(now)
    } else {
        execution.finished_at
    };
    state
        .execution_runs
        .update_status(&execution_id, &req.status, started_at, finished_at)
        .await
        .map_err(db_error)?;
    audit(
        &state,
        Some(&user.id),
        "execution_run",
        Some(&execution_id),
        "update_status",
        json!({ "status": req.status }),
    )
    .await;
    let execution = owned_execution(&state, &user.id, &execution_id).await?;
    Ok(Json(ApiResponse::ok(execution_run_response(execution))))
}

async fn cancel_execution_handler(
    State(state): State<WorkbenchRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(execution_id): Path<String>,
) -> Result<Json<ApiResponse<ExecutionRunResponse>>, ApiError> {
    let execution = owned_execution(&state, &user.id, &execution_id).await?;
    if is_terminal_execution_status(&execution.status) {
        return Err(ApiError::Conflict("Execution is already terminal".into()));
    }
    let now = aionui_common::now_ms();
    state
        .execution_runs
        .update_status(
            &execution_id,
            "cancelled",
            execution.started_at.or(Some(now)),
            Some(now),
        )
        .await
        .map_err(db_error)?;
    state
        .execution_runs
        .set_preview_url(&execution_id, None)
        .await
        .map_err(db_error)?;
    audit(
        &state,
        Some(&user.id),
        "execution_run",
        Some(&execution_id),
        "cancel",
        json!({ "previous_status": execution.status, "execution_type": execution.execution_type }),
    )
    .await;
    let execution = owned_execution(&state, &user.id, &execution_id).await?;
    Ok(Json(ApiResponse::ok(execution_run_response(execution))))
}

async fn list_execution_artifacts_handler(
    State(state): State<WorkbenchRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(execution_id): Path<String>,
) -> Result<Json<ApiResponse<Vec<ExecutionArtifactResponse>>>, ApiError> {
    let _execution = owned_execution(&state, &user.id, &execution_id).await?;
    let artifacts = state
        .execution_artifacts
        .list_by_execution(&execution_id)
        .await
        .map_err(db_error)?
        .into_iter()
        .map(execution_artifact_response)
        .collect();
    Ok(Json(ApiResponse::ok(artifacts)))
}

async fn snapshot_gc_handler(
    State(state): State<WorkbenchRouterState>,
    Extension(user): Extension<CurrentUser>,
) -> Result<Json<ApiResponse<SnapshotGcResponse>>, ApiError> {
    ensure_permission(&state, &user, "admin:snapshot:gc").await?;
    let response = run_snapshot_gc(&state, 100).await?;
    audit(
        &state,
        Some(&user.id),
        "snapshot",
        None,
        "gc",
        json!({
            "scanned": response.scanned,
            "deleted_content": response.deleted_content,
            "skipped_active": response.skipped_active,
            "tombstoned": response.tombstoned
        }),
    )
    .await;
    Ok(Json(ApiResponse::ok(response)))
}

async fn list_workspace_directory_handler(
    State(state): State<WorkbenchRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(workspace_id): Path<String>,
    Json(req): Json<WorkspaceFilePathRequest>,
) -> Result<Json<ApiResponse<Vec<WorkspaceFileEntryResponse>>>, ApiError> {
    let resolved = resolve_workspace_path(
        &state,
        &user.id,
        &workspace_id,
        &req.relative_path,
        WorkspaceResolveMode::Existing,
    )
    .await?;
    let mut entries = tokio::fs::read_dir(&resolved.absolute_path)
        .await
        .map_err(|error| ApiError::BadRequest(format!("Failed to list workspace directory: {error}")))?;
    let mut response = Vec::new();
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|error| ApiError::Internal(format!("Failed to read workspace directory: {error}")))?
    {
        let metadata = entry
            .metadata()
            .await
            .map_err(|error| ApiError::Internal(format!("Failed to inspect workspace entry: {error}")))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let relative_path = join_relative_display_path(&resolved.relative_path, &name);
        response.push(WorkspaceFileEntryResponse {
            name,
            relative_path,
            is_dir: metadata.is_dir(),
            size_bytes: if metadata.is_file() { Some(metadata.len()) } else { None },
        });
    }
    response.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then(a.name.cmp(&b.name)));
    Ok(Json(ApiResponse::ok(response)))
}

async fn workspace_file_metadata_handler(
    State(state): State<WorkbenchRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(workspace_id): Path<String>,
    Json(req): Json<WorkspaceFilePathRequest>,
) -> Result<Json<ApiResponse<WorkspaceFileMetadataResponse>>, ApiError> {
    let resolved = resolve_workspace_path(
        &state,
        &user.id,
        &workspace_id,
        &req.relative_path,
        WorkspaceResolveMode::Existing,
    )
    .await?;
    let metadata = tokio::fs::metadata(&resolved.absolute_path)
        .await
        .map_err(|error| ApiError::BadRequest(format!("Failed to inspect workspace path: {error}")))?;
    let modified_at = metadata.modified().ok().and_then(|time| {
        time.duration_since(std::time::UNIX_EPOCH)
            .ok()
            .and_then(|duration| i64::try_from(duration.as_millis()).ok())
    });
    Ok(Json(ApiResponse::ok(WorkspaceFileMetadataResponse {
        relative_path: resolved.relative_path,
        is_dir: metadata.is_dir(),
        size_bytes: metadata.len(),
        modified_at,
    })))
}

async fn read_workspace_file_handler(
    State(state): State<WorkbenchRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(workspace_id): Path<String>,
    Json(req): Json<WorkspaceFilePathRequest>,
) -> Result<Json<ApiResponse<Option<String>>>, ApiError> {
    let resolved = resolve_workspace_path(
        &state,
        &user.id,
        &workspace_id,
        &req.relative_path,
        WorkspaceResolveMode::Existing,
    )
    .await?;
    let metadata = tokio::fs::metadata(&resolved.absolute_path)
        .await
        .map_err(|error| ApiError::BadRequest(format!("Failed to inspect workspace file: {error}")))?;
    if metadata.is_dir() {
        return Err(ApiError::BadRequest("Path is a directory".into()));
    }
    let content = tokio::fs::read_to_string(&resolved.absolute_path)
        .await
        .map_err(|error| ApiError::BadRequest(format!("Failed to read workspace file as UTF-8: {error}")))?;
    Ok(Json(ApiResponse::ok(Some(content))))
}

async fn read_workspace_file_buffer_handler(
    State(state): State<WorkbenchRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(workspace_id): Path<String>,
    Json(req): Json<WorkspaceFilePathRequest>,
) -> Result<Json<ApiResponse<Option<String>>>, ApiError> {
    let resolved = resolve_workspace_path(
        &state,
        &user.id,
        &workspace_id,
        &req.relative_path,
        WorkspaceResolveMode::Existing,
    )
    .await?;
    let metadata = tokio::fs::metadata(&resolved.absolute_path)
        .await
        .map_err(|error| ApiError::BadRequest(format!("Failed to inspect workspace file: {error}")))?;
    if metadata.is_dir() {
        return Err(ApiError::BadRequest("Path is a directory".into()));
    }
    let bytes = tokio::fs::read(&resolved.absolute_path)
        .await
        .map_err(|error| ApiError::BadRequest(format!("Failed to read workspace file: {error}")))?;
    use base64::Engine;
    Ok(Json(ApiResponse::ok(Some(
        base64::engine::general_purpose::STANDARD.encode(bytes),
    ))))
}

async fn write_workspace_file_handler(
    State(state): State<WorkbenchRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(workspace_id): Path<String>,
    Json(req): Json<WorkspaceFileWriteRequest>,
) -> Result<Json<ApiResponse<bool>>, ApiError> {
    let resolved = resolve_workspace_path(
        &state,
        &user.id,
        &workspace_id,
        &req.relative_path,
        WorkspaceResolveMode::ParentForCreate,
    )
    .await?;
    if let Some(parent) = resolved.absolute_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|error| ApiError::Internal(format!("Failed to create workspace parent directory: {error}")))?;
    }
    tokio::fs::write(&resolved.absolute_path, req.content)
        .await
        .map_err(|error| ApiError::Internal(format!("Failed to write workspace file: {error}")))?;
    Ok(Json(ApiResponse::ok(true)))
}

async fn upload_workspace_file_handler(
    State(state): State<WorkbenchRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(workspace_id): Path<String>,
    mut multipart: Multipart,
) -> Result<Json<ApiResponse<WorkspaceFileMetadataResponse>>, ApiError> {
    let mut relative_path = None;
    let mut file_bytes = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|error| ApiError::BadRequest(format!("Invalid multipart upload: {error}")))?
    {
        match field.name() {
            Some("relative_path") => {
                relative_path = Some(
                    field
                        .text()
                        .await
                        .map_err(|error| ApiError::BadRequest(format!("Invalid relative_path field: {error}")))?,
                );
            }
            Some("file") => {
                file_bytes = Some(
                    field
                        .bytes()
                        .await
                        .map_err(|error| ApiError::BadRequest(format!("Invalid file field: {error}")))?,
                );
            }
            _ => {}
        }
    }
    let relative_path = relative_path.ok_or_else(|| ApiError::BadRequest("relative_path is required".into()))?;
    let file_bytes = file_bytes.ok_or_else(|| ApiError::BadRequest("file field is required".into()))?;
    let resolved = resolve_workspace_path(
        &state,
        &user.id,
        &workspace_id,
        &relative_path,
        WorkspaceResolveMode::ParentForCreate,
    )
    .await?;
    if let Some(parent) = resolved.absolute_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|error| ApiError::Internal(format!("Failed to create workspace parent directory: {error}")))?;
    }
    tokio::fs::write(&resolved.absolute_path, &file_bytes)
        .await
        .map_err(|error| ApiError::Internal(format!("Failed to upload workspace file: {error}")))?;
    Ok(Json(ApiResponse::ok(WorkspaceFileMetadataResponse {
        relative_path: resolved.relative_path,
        is_dir: false,
        size_bytes: u64::try_from(file_bytes.len()).unwrap_or(u64::MAX),
        modified_at: None,
    })))
}

async fn mkdir_workspace_handler(
    State(state): State<WorkbenchRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(workspace_id): Path<String>,
    Json(req): Json<WorkspaceFilePathRequest>,
) -> Result<Json<ApiResponse<()>>, ApiError> {
    let resolved = resolve_workspace_path(
        &state,
        &user.id,
        &workspace_id,
        &req.relative_path,
        WorkspaceResolveMode::ParentForCreate,
    )
    .await?;
    tokio::fs::create_dir_all(&resolved.absolute_path)
        .await
        .map_err(|error| ApiError::Internal(format!("Failed to create workspace directory: {error}")))?;
    Ok(Json(ApiResponse::ok(())))
}

async fn delete_workspace_path_handler(
    State(state): State<WorkbenchRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(workspace_id): Path<String>,
    Json(req): Json<WorkspaceFilePathRequest>,
) -> Result<Json<ApiResponse<()>>, ApiError> {
    let resolved = resolve_workspace_path(
        &state,
        &user.id,
        &workspace_id,
        &req.relative_path,
        WorkspaceResolveMode::Existing,
    )
    .await?;
    if resolved.absolute_path == resolved.workspace_root {
        return Err(ApiError::BadRequest(
            "Workspace root cannot be deleted through file API".into(),
        ));
    }
    let metadata = tokio::fs::metadata(&resolved.absolute_path)
        .await
        .map_err(|error| ApiError::BadRequest(format!("Failed to inspect workspace path: {error}")))?;
    if metadata.is_dir() {
        tokio::fs::remove_dir_all(&resolved.absolute_path)
            .await
            .map_err(|error| ApiError::Internal(format!("Failed to delete workspace directory: {error}")))?;
    } else {
        tokio::fs::remove_file(&resolved.absolute_path)
            .await
            .map_err(|error| ApiError::Internal(format!("Failed to delete workspace file: {error}")))?;
    }
    Ok(Json(ApiResponse::ok(())))
}

async fn rename_workspace_path_handler(
    State(state): State<WorkbenchRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(workspace_id): Path<String>,
    Json(req): Json<WorkspaceFileRenameRequest>,
) -> Result<Json<ApiResponse<WorkspaceFileMetadataResponse>>, ApiError> {
    if req.new_name.contains('/') || req.new_name.contains('\\') || req.new_name == "." || req.new_name == ".." {
        return Err(ApiError::BadRequest("new_name must be a single path segment".into()));
    }
    let resolved = resolve_workspace_path(
        &state,
        &user.id,
        &workspace_id,
        &req.relative_path,
        WorkspaceResolveMode::Existing,
    )
    .await?;
    if resolved.absolute_path == resolved.workspace_root {
        return Err(ApiError::BadRequest(
            "Workspace root cannot be renamed through file API".into(),
        ));
    }
    let parent_relative = std::path::Path::new(&resolved.relative_path)
        .parent()
        .map(|value| value.to_string_lossy().replace('\\', "/"))
        .unwrap_or_default();
    let new_relative = join_relative_display_path(&parent_relative, &req.new_name);
    let new_resolved = resolve_workspace_path(
        &state,
        &user.id,
        &workspace_id,
        &new_relative,
        WorkspaceResolveMode::ParentForCreate,
    )
    .await?;
    tokio::fs::rename(&resolved.absolute_path, &new_resolved.absolute_path)
        .await
        .map_err(|error| ApiError::Internal(format!("Failed to rename workspace path: {error}")))?;
    let metadata = tokio::fs::metadata(&new_resolved.absolute_path)
        .await
        .map_err(|error| ApiError::Internal(format!("Failed to inspect renamed workspace path: {error}")))?;
    Ok(Json(ApiResponse::ok(WorkspaceFileMetadataResponse {
        relative_path: new_resolved.relative_path,
        is_dir: metadata.is_dir(),
        size_bytes: metadata.len(),
        modified_at: None,
    })))
}

async fn workspace_file_watch_payload_handler(
    State(state): State<WorkbenchRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(workspace_id): Path<String>,
    Json(req): Json<WorkspaceFilePathRequest>,
) -> Result<Json<ApiResponse<Value>>, ApiError> {
    let resolved = resolve_workspace_path(
        &state,
        &user.id,
        &workspace_id,
        &req.relative_path,
        WorkspaceResolveMode::Existing,
    )
    .await?;
    let content_update = build_saas_content_update_event(
        &workspace_id,
        &resolved.relative_path,
        ContentUpdateOperation::Write,
        None,
    );
    let file_watch = build_saas_file_watch_event(&workspace_id, &resolved.relative_path, "change");
    Ok(Json(ApiResponse::ok(json!({
        "content_update": content_update,
        "file_watch": file_watch
    }))))
}

async fn workspace_terminal_context_handler(
    State(state): State<WorkbenchRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(workspace_id): Path<String>,
    Json(req): Json<WorkspaceFilePathRequest>,
) -> Result<Json<ApiResponse<WorkspaceRuntimeContextResponse>>, ApiError> {
    let context = resolve_workspace_runtime_context(&state, &user.id, &workspace_id, &req.relative_path).await?;
    let _internal_cwd = &context.cwd;
    audit(
        &state,
        Some(&user.id),
        "workspace",
        Some(&workspace_id),
        "terminal_context",
        json!({ "relative_path": context.response.relative_path }),
    )
    .await;
    Ok(Json(ApiResponse::ok(context.response)))
}

async fn workspace_git_context_handler(
    State(state): State<WorkbenchRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(workspace_id): Path<String>,
    Json(req): Json<WorkspaceFilePathRequest>,
) -> Result<Json<ApiResponse<WorkspaceRuntimeContextResponse>>, ApiError> {
    let context = resolve_workspace_runtime_context(&state, &user.id, &workspace_id, &req.relative_path).await?;
    let _internal_cwd = &context.cwd;
    audit(
        &state,
        Some(&user.id),
        "workspace",
        Some(&workspace_id),
        "git_context",
        json!({ "relative_path": context.response.relative_path }),
    )
    .await;
    Ok(Json(ApiResponse::ok(context.response)))
}

async fn user_sync_handler(
    State(state): State<WorkbenchRouterState>,
    headers: HeaderMap,
    Form(form): Form<UserSyncForm>,
) -> Result<Json<Value>, ApiError> {
    verify_sync_secret(&state, &headers)?;
    if form.external_user_id.trim().is_empty() {
        return Err(ApiError::BadRequest("external user id is required".into()));
    }

    let app_id = form
        .app_id
        .clone()
        .or_else(|| state.gateway_auth.as_ref().map(|config| config.app_id.clone()))
        .ok_or_else(|| ApiError::BadRequest("app_id is required".into()))?;
    let status = match form.sync_type {
        0 | 4 => "disabled",
        1..=3 => "enabled",
        other => return Err(ApiError::BadRequest(format!("Unsupported sync type '{other}'"))),
    };

    let provider = state
        .gateway_auth
        .as_ref()
        .map(|config| config.provider.clone())
        .unwrap_or_else(|| "modo_open_platform".to_string());
    let existing_identity = state
        .external_identities
        .find_identity(&provider, &app_id, &form.external_user_id)
        .await
        .map_err(db_error)?;

    let user = if let Some(identity) = existing_identity {
        state
            .users
            .update_from_sync(
                &identity.user_id,
                form.phone.as_deref(),
                form.display_name.as_deref(),
                form.email.as_deref(),
                status,
            )
            .await
            .map_err(db_error)?
    } else if let Some(phone) = form.phone.as_deref().filter(|value| !value.trim().is_empty()) {
        if let Some(existing_user) = state.users.find_by_phone(phone).await.map_err(db_error)? {
            state
                .users
                .update_from_sync(
                    &existing_user.id,
                    Some(phone),
                    form.display_name.as_deref(),
                    form.email.as_deref(),
                    status,
                )
                .await
                .map_err(db_error)?
        } else {
            state
                .users
                .create_user(CreatePlatformUserParams {
                    phone: Some(phone.to_string()),
                    username: None,
                    display_name: form.display_name.clone(),
                    email: form.email.clone(),
                    password_hash: None,
                    status: status.to_string(),
                })
                .await
                .map_err(db_error)?
        }
    } else {
        return Err(ApiError::BadRequest("phone is required for first sync".into()));
    };

    state
        .external_identities
        .upsert_identity(UpsertExternalIdentityParams {
            provider: provider.clone(),
            app_id: app_id.clone(),
            external_user_id: form.external_user_id.clone(),
            user_id: user.id.clone(),
            phone_snapshot: user.phone.clone(),
            external_role_ids: Value::Array(Vec::new()),
            is_admin: form.is_admin.unwrap_or(false),
            raw_payload: json!({
                "type": form.sync_type,
                "external_user_id": form.external_user_id,
                "phone": form.phone,
                "display_name": form.display_name,
                "email": form.email,
                "is_admin": form.is_admin
            }),
            last_synced_at: aionui_common::now_ms(),
        })
        .await
        .map_err(db_error)?;

    sync_system_roles(&state, &user, form.sync_type, form.is_admin.unwrap_or(false)).await?;

    audit(
        &state,
        Some(&user.id),
        "user",
        Some(&user.id),
        "external_sync",
        json!({ "type": form.sync_type, "provider": provider, "app_id": app_id, "status": status }),
    )
    .await;

    Ok(Json(json!({ "status": 0, "message": "ok" })))
}

async fn ensure_permission(
    state: &WorkbenchRouterState,
    user: &CurrentUser,
    required_permission: &str,
) -> Result<(), ApiError> {
    let roles = state.roles.list_user_roles(&user.id).await.map_err(db_error)?;
    let allowed = roles.iter().any(|role| {
        role.status == "enabled"
            && (role.role_key == SUPER_ADMIN_ROLE_KEY
                || role.permissions.as_array().is_some_and(|permissions| {
                    permissions.iter().any(|value| {
                        value.as_str().is_some_and(|permission| {
                            permission == ADMIN_PERMISSION
                                || permission == "admin"
                                || permission == required_permission
                                || permission_matches_prefix(permission, required_permission)
                        })
                    })
                }))
    });
    if allowed {
        Ok(())
    } else {
        Err(ApiError::Forbidden(format!(
            "Permission '{required_permission}' required"
        )))
    }
}

fn permission_matches_prefix(granted: &str, required: &str) -> bool {
    granted
        .strip_suffix(":*")
        .is_some_and(|prefix| required.starts_with(&format!("{prefix}:")))
}

fn permission_registry() -> &'static [PermissionDescriptor] {
    &[
        PermissionDescriptor {
            key: ADMIN_PERMISSION,
            label: "全部权限",
            description: "系统超级管理员权限",
        },
        PermissionDescriptor {
            key: "workspace:own",
            label: "个人工作空间",
            description: "访问自己的 SSH 凭据、Git 项目和工作空间",
        },
        PermissionDescriptor {
            key: "admin:user:list",
            label: "用户列表",
            description: "查看用户列表",
        },
        PermissionDescriptor {
            key: "admin:user:read",
            label: "用户详情",
            description: "查看用户详情和同步状态",
        },
        PermissionDescriptor {
            key: "admin:user:update",
            label: "用户状态",
            description: "启用或禁用用户",
        },
        PermissionDescriptor {
            key: "admin:user:reset-password",
            label: "重置密码",
            description: "为用户重置本地登录密码",
        },
        PermissionDescriptor {
            key: "admin:user:sync-status",
            label: "同步状态",
            description: "查看或触发用户同步状态刷新",
        },
        PermissionDescriptor {
            key: "admin:role:list",
            label: "角色列表",
            description: "查看角色和权限标记",
        },
        PermissionDescriptor {
            key: "admin:role:read",
            label: "角色详情",
            description: "查看角色详情",
        },
        PermissionDescriptor {
            key: "admin:role:create",
            label: "创建角色",
            description: "创建或 upsert 角色",
        },
        PermissionDescriptor {
            key: "admin:role:update",
            label: "编辑角色",
            description: "编辑角色名称、状态和权限",
        },
        PermissionDescriptor {
            key: "admin:role:assign",
            label: "分配角色",
            description: "为用户分配或取消角色",
        },
        PermissionDescriptor {
            key: "admin:snapshot:gc",
            label: "Snapshot 清理",
            description: "执行 snapshot retention GC",
        },
    ]
}

async fn find_role_by_id(state: &WorkbenchRouterState, role_id: &str) -> Result<RoleRow, ApiError> {
    state
        .roles
        .list_roles()
        .await
        .map_err(db_error)?
        .into_iter()
        .find(|role| role.id == role_id)
        .ok_or_else(|| ApiError::NotFound(format!("Role '{role_id}' not found")))
}

async fn sync_system_roles(
    state: &WorkbenchRouterState,
    user: &PlatformUserRow,
    sync_type: i32,
    is_admin: bool,
) -> Result<(), ApiError> {
    if matches!(sync_type, 0 | 4) {
        return Ok(());
    }

    let default_role = state
        .roles
        .upsert_role(UpsertRoleParams {
            role_key: DEFAULT_USER_ROLE_KEY.to_owned(),
            role_name: "普通用户".to_owned(),
            status: "enabled".to_owned(),
            permissions: Value::Array(vec![Value::String("workspace:own".to_owned())]),
            sort_order: 1000,
            is_system: true,
        })
        .await
        .map_err(db_error)?;
    state
        .roles
        .assign_role(&user.id, &default_role.id)
        .await
        .map_err(db_error)?;

    if is_admin {
        let admin_role = state
            .roles
            .upsert_role(UpsertRoleParams {
                role_key: SUPER_ADMIN_ROLE_KEY.to_owned(),
                role_name: "超级管理员".to_owned(),
                status: "enabled".to_owned(),
                permissions: Value::Array(vec![Value::String(ADMIN_PERMISSION.to_owned())]),
                sort_order: 0,
                is_system: true,
            })
            .await
            .map_err(db_error)?;
        state
            .roles
            .assign_role(&user.id, &admin_role.id)
            .await
            .map_err(db_error)?;
    }

    Ok(())
}

async fn owned_workspace(
    state: &WorkbenchRouterState,
    user_id: &str,
    workspace_id: &str,
) -> Result<WorkspaceRow, ApiError> {
    let workspace = state
        .workspaces
        .find_by_id(workspace_id)
        .await
        .map_err(db_error)?
        .ok_or_else(|| ApiError::NotFound(format!("Workspace '{workspace_id}' not found")))?;
    if workspace.owner_user_id != user_id {
        return Err(ApiError::Forbidden("Workspace access denied".into()));
    }
    Ok(workspace)
}

async fn resolve_workspace_path(
    state: &WorkbenchRouterState,
    user_id: &str,
    workspace_id: &str,
    relative_path: &str,
    mode: WorkspaceResolveMode,
) -> Result<crate::workspace_resolver::ResolvedWorkspacePath, ApiError> {
    WorkspaceResolver::new(state.senmo_root.clone(), state.workspaces.clone())
        .resolve_for_user(user_id, workspace_id, relative_path, mode)
        .await
        .map_err(workspace_resolve_error)
}

fn workspace_resolve_error(error: WorkspaceResolveError) -> ApiError {
    match error {
        WorkspaceResolveError::Lookup(message) | WorkspaceResolveError::Internal(message) => {
            tracing::error!(error = %message, "workspace path resolution failed");
            ApiError::Internal("Workspace path resolution failed".into())
        }
        WorkspaceResolveError::NotFound(message) => ApiError::NotFound(message),
        WorkspaceResolveError::Forbidden(message) => ApiError::Forbidden(message),
        WorkspaceResolveError::BadPath(message) => ApiError::BadRequest(message),
    }
}

async fn resolve_workspace_runtime_context(
    state: &WorkbenchRouterState,
    user_id: &str,
    workspace_id: &str,
    relative_path: &str,
) -> Result<WorkspaceRuntimeContext, ApiError> {
    let resolved = resolve_workspace_path(
        state,
        user_id,
        workspace_id,
        relative_path,
        WorkspaceResolveMode::Existing,
    )
    .await?;
    let metadata = tokio::fs::metadata(&resolved.absolute_path)
        .await
        .map_err(|error| ApiError::BadRequest(format!("Failed to inspect workspace runtime cwd: {error}")))?;
    if !metadata.is_dir() {
        return Err(ApiError::BadRequest(
            "Runtime cwd must resolve to a workspace directory".into(),
        ));
    }
    Ok(WorkspaceRuntimeContext {
        cwd: resolved.absolute_path,
        response: WorkspaceRuntimeContextResponse {
            workspace_id: workspace_id.to_owned(),
            relative_path: resolved.relative_path.clone(),
            display_path: workspace_display_path(workspace_id, &resolved.relative_path),
        },
    })
}

fn workspace_display_path(workspace_id: &str, relative_path: &str) -> String {
    if relative_path.is_empty() {
        format!("workspace:{workspace_id}/")
    } else {
        format!("workspace:{workspace_id}/{relative_path}")
    }
}

fn build_saas_content_update_event(
    workspace_id: &str,
    relative_path: &str,
    operation: ContentUpdateOperation,
    content: Option<String>,
) -> ContentUpdateEvent {
    ContentUpdateEvent {
        file_path: String::new(),
        content,
        workspace_id: Some(workspace_id.to_owned()),
        workspace: String::new(),
        relative_path: relative_path.to_owned(),
        operation,
    }
}

fn build_saas_file_watch_event(workspace_id: &str, relative_path: &str, event_type: &str) -> FileWatchEvent {
    FileWatchEvent {
        file_path: String::new(),
        workspace_id: Some(workspace_id.to_owned()),
        relative_path: Some(relative_path.to_owned()),
        event_type: event_type.to_owned(),
    }
}

fn join_relative_display_path(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.to_owned()
    } else {
        format!("{}/{}", parent.trim_end_matches('/'), name)
    }
}

async fn create_snapshot_for_workspace(
    state: &WorkbenchRouterState,
    created_by: &str,
    workspace: WorkspaceRow,
    environment_metadata: Value,
    retention_policy: Value,
) -> Result<SnapshotRow, ApiError> {
    if workspace.status != "active" {
        return Err(ApiError::BadRequest("Only active workspaces can be snapshotted".into()));
    }
    let workspace_root =
        safe_join_under_root(&state.senmo_root, &workspace.root_rel_path).map_err(workspace_resolve_error)?;
    let snapshot_id = aionui_common::generate_prefixed_id("snap");
    let snapshot_rel_root = format!("snapshots/{snapshot_id}");
    let snapshot_abs_root =
        safe_join_under_root(&state.senmo_root, &snapshot_rel_root).map_err(workspace_resolve_error)?;
    let content_abs_root = snapshot_abs_root.join("content");
    let manifest_abs_path = snapshot_abs_root.join("manifest.json");
    tokio::fs::create_dir_all(&content_abs_root)
        .await
        .map_err(|error| ApiError::Internal(format!("Failed to create snapshot directory: {error}")))?;

    let sealed = freeze_workspace_snapshot(&workspace_root, &content_abs_root).await?;
    let sealed_at = aionui_common::now_ms();
    let expires_at = snapshot_expires_at(&retention_policy, sealed_at);
    let base_git_metadata = json!({
        "git_project_id": workspace.git_project_id,
        "branch_ref": workspace.branch_ref
    });
    let manifest = json!({
        "snapshot_id": snapshot_id,
        "workspace_id": workspace.id,
        "source_kind": workspace.source_type,
        "artifact_ref": format!("{snapshot_rel_root}/content"),
        "checksum": sealed.checksum,
        "size_bytes": sealed.size_bytes,
        "file_count": sealed.file_count,
        "base_git_metadata": base_git_metadata,
        "environment_metadata": environment_metadata,
        "retention_policy": retention_policy,
        "created_by": created_by,
        "sealed_at": sealed_at
    });
    tokio::fs::write(
        &manifest_abs_path,
        serde_json::to_vec_pretty(&manifest)
            .map_err(|error| ApiError::Internal(format!("Failed to serialize snapshot manifest: {error}")))?,
    )
    .await
    .map_err(|error| ApiError::Internal(format!("Failed to write snapshot manifest: {error}")))?;

    state
        .snapshots
        .create_snapshot(CreateSnapshotParams {
            workspace_id: manifest["workspace_id"].as_str().unwrap_or_default().to_owned(),
            created_by: created_by.to_owned(),
            source_kind: manifest["source_kind"].as_str().unwrap_or_default().to_owned(),
            artifact_ref: format!("{snapshot_rel_root}/content"),
            manifest_ref: format!("{snapshot_rel_root}/manifest.json"),
            checksum: sealed.checksum,
            size_bytes: sealed.size_bytes,
            base_git_metadata: manifest["base_git_metadata"].clone(),
            environment_metadata: manifest["environment_metadata"].clone(),
            retention_policy: manifest["retention_policy"].clone(),
            sealed_at,
            expires_at,
        })
        .await
        .map_err(db_error)
}

fn snapshot_expires_at(retention_policy: &Value, sealed_at: i64) -> Option<i64> {
    retention_policy.get("expires_at").and_then(Value::as_i64).or_else(|| {
        retention_policy
            .get("ttl_seconds")
            .and_then(Value::as_i64)
            .filter(|ttl| *ttl > 0)
            .map(|ttl| sealed_at.saturating_add(ttl.saturating_mul(1000)))
    })
}

async fn run_snapshot_gc(state: &WorkbenchRouterState, limit: i64) -> Result<SnapshotGcResponse, ApiError> {
    let now = aionui_common::now_ms();
    let candidates = state.snapshots.list_gc_candidates(now, limit).await.map_err(db_error)?;
    let scanned = candidates.len();
    let mut deleted_content = 0;
    let mut skipped_active = 0;
    let mut tombstoned = 0;

    for snapshot in candidates {
        if state
            .execution_runs
            .has_active_by_snapshot(&snapshot.id)
            .await
            .map_err(db_error)?
        {
            skipped_active += 1;
            continue;
        }

        let content_path =
            safe_join_under_root(&state.senmo_root, &snapshot.artifact_ref).map_err(workspace_resolve_error)?;
        match tokio::fs::remove_dir_all(&content_path).await {
            Ok(()) => {
                deleted_content += 1;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(ApiError::Internal(format!(
                    "Failed to remove snapshot content '{}': {error}",
                    snapshot.artifact_ref
                )));
            }
        }

        state
            .snapshots
            .mark_gc_tombstone(&snapshot.id, now)
            .await
            .map_err(db_error)?;
        tombstoned += 1;
    }

    Ok(SnapshotGcResponse {
        scanned,
        deleted_content,
        skipped_active,
        tombstoned,
    })
}

struct FrozenSnapshot {
    checksum: String,
    size_bytes: i64,
    file_count: usize,
}

const SNAPSHOT_MAX_FILES: usize = 20_000;
const SNAPSHOT_MAX_FILE_BYTES: u64 = 50 * 1024 * 1024;
const SNAPSHOT_MAX_TOTAL_BYTES: u64 = 500 * 1024 * 1024;

async fn freeze_workspace_snapshot(source: &FsPath, destination: &FsPath) -> Result<FrozenSnapshot, ApiError> {
    let source = source
        .canonicalize()
        .map_err(|error| ApiError::NotFound(format!("Workspace directory not found: {error}")))?;
    let mut files = Vec::new();
    collect_regular_files(&source, &source, &mut files).await?;
    files.sort();

    let mut hasher = Sha256::new();
    let mut size_bytes = 0_u64;
    for relative in &files {
        let from = source.join(relative);
        let to = destination.join(relative);
        let metadata = tokio::fs::metadata(&from)
            .await
            .map_err(|error| ApiError::Internal(format!("Failed to inspect workspace file: {error}")))?;
        let file_size = metadata.len();
        if file_size > SNAPSHOT_MAX_FILE_BYTES {
            return Err(ApiError::PayloadTooLarge(format!(
                "Workspace file '{}' exceeds the snapshot file size limit",
                relative.to_string_lossy()
            )));
        }
        size_bytes = size_bytes.saturating_add(file_size);
        if size_bytes > SNAPSHOT_MAX_TOTAL_BYTES {
            return Err(ApiError::PayloadTooLarge(
                "Workspace snapshot exceeds the total size limit".into(),
            ));
        }
        if let Some(parent) = to.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|error| ApiError::Internal(format!("Failed to create snapshot directory: {error}")))?;
        }
        let content = tokio::fs::read(&from)
            .await
            .map_err(|error| ApiError::Internal(format!("Failed to read workspace file: {error}")))?;
        hasher.update(relative.to_string_lossy().as_bytes());
        hasher.update([0]);
        hasher.update(&content);
        tokio::fs::write(&to, content)
            .await
            .map_err(|error| ApiError::Internal(format!("Failed to write snapshot file: {error}")))?;
    }

    Ok(FrozenSnapshot {
        checksum: format!("sha256:{}", hex::encode(hasher.finalize())),
        size_bytes: i64::try_from(size_bytes).unwrap_or(i64::MAX),
        file_count: files.len(),
    })
}

async fn collect_regular_files(root: &FsPath, current: &FsPath, files: &mut Vec<PathBuf>) -> Result<(), ApiError> {
    let mut entries = tokio::fs::read_dir(current)
        .await
        .map_err(|error| ApiError::Internal(format!("Failed to read workspace directory: {error}")))?;
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|error| ApiError::Internal(format!("Failed to read workspace directory entry: {error}")))?
    {
        let path = entry.path();
        let file_type = entry
            .file_type()
            .await
            .map_err(|error| ApiError::Internal(format!("Failed to inspect workspace entry: {error}")))?;
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            Box::pin(collect_regular_files(root, &path, files)).await?;
        } else if file_type.is_file() {
            if files.len() >= SNAPSHOT_MAX_FILES {
                return Err(ApiError::PayloadTooLarge(
                    "Workspace snapshot exceeds the file count limit".into(),
                ));
            }
            let relative = path
                .strip_prefix(root)
                .map_err(|error| ApiError::Internal(format!("Failed to relativize workspace file: {error}")))?;
            files.push(relative.to_path_buf());
        }
    }
    Ok(())
}

async fn owned_credential(
    state: &WorkbenchRouterState,
    user_id: &str,
    credential_id: &str,
) -> Result<GitSshCredentialRow, ApiError> {
    let credential = state
        .git_ssh_credentials
        .find_by_id(credential_id)
        .await
        .map_err(db_error)?
        .ok_or_else(|| ApiError::NotFound(format!("SSH credential '{credential_id}' not found")))?;
    if credential.owner_user_id != user_id {
        return Err(ApiError::Forbidden("SSH credential access denied".into()));
    }
    Ok(credential)
}

async fn owned_git_project(
    state: &WorkbenchRouterState,
    user_id: &str,
    project_id: &str,
) -> Result<GitProjectRow, ApiError> {
    let project = state
        .git_projects
        .find_by_id(project_id)
        .await
        .map_err(db_error)?
        .ok_or_else(|| ApiError::NotFound(format!("Git project '{project_id}' not found")))?;
    if project.owner_user_id != user_id {
        return Err(ApiError::Forbidden("Git project access denied".into()));
    }
    Ok(project)
}

async fn owned_execution(
    state: &WorkbenchRouterState,
    user_id: &str,
    execution_id: &str,
) -> Result<ExecutionRunRow, ApiError> {
    let execution = state
        .execution_runs
        .find_by_id(execution_id)
        .await
        .map_err(db_error)?
        .ok_or_else(|| ApiError::NotFound(format!("ExecutionRun '{execution_id}' not found")))?;
    if execution.owner_user_id != user_id {
        return Err(ApiError::Forbidden("ExecutionRun access denied".into()));
    }
    Ok(execution)
}

fn validate_execution_type(value: &str) -> Result<(), ApiError> {
    validate_status(value, &["test_run", "preview_env", "ci_job"])
}

fn normalized_execution_policy(req: &CreateExecutionRequest) -> Value {
    if req.execution_type != "preview_env" {
        return req.policy.clone();
    }
    let mut policy = req.policy.clone();
    if !policy.is_object() {
        policy = json!({ "value": policy });
    }
    let object = policy.as_object_mut().expect("policy object");
    let preview = object.entry("preview").or_insert_with(|| json!({}));
    if !preview.is_object() {
        *preview = json!({});
    }
    let preview = preview.as_object_mut().expect("preview object");
    preview.entry("ttl_seconds".to_owned()).or_insert_with(|| json!(7200));
    policy
}

fn validate_execution_status(value: &str) -> Result<(), ApiError> {
    validate_status(
        value,
        &[
            "created",
            "preparing",
            "snapshot_resolved",
            "provisioning",
            "running",
            "streaming",
            "collecting",
            "succeeded",
            "failed",
            "timeout",
            "cancelled",
            "cleanup",
            "cleanup_failed",
        ],
    )
}

fn is_terminal_execution_status(value: &str) -> bool {
    matches!(value, "succeeded" | "failed" | "timeout" | "cancelled")
}

async fn verify_git_project_access(
    state: &WorkbenchRouterState,
    credential: &GitSshCredentialRow,
    repo_ssh_url: &str,
) -> Result<i64, ApiError> {
    if !repo_ssh_url.starts_with("git@") && !repo_ssh_url.starts_with("ssh://") {
        return Err(ApiError::BadRequest("repo_ssh_url must be an SSH URL".into()));
    }
    let private_key = aionui_common::decrypt_string(&credential.encrypted_private_key, &state.encryption_key)
        .map_err(|error| ApiError::Internal(format!("Credential decryption failed: {error}")))?;
    let verify_dir = state
        .senmo_root
        .join("tmp")
        .join(aionui_common::generate_prefixed_id("gitverify"));
    tokio::fs::create_dir_all(&verify_dir)
        .await
        .map_err(|error| ApiError::Internal(format!("Failed to create git verify directory: {error}")))?;
    let key_path = verify_dir.join("id_key");
    tokio::fs::write(&key_path, private_key)
        .await
        .map_err(|error| ApiError::Internal(format!("Failed to materialize SSH key: {error}")))?;
    set_owner_only_permissions(&key_path).await;

    let ssh_command = format!(
        "ssh -i {} -o IdentitiesOnly=yes -o StrictHostKeyChecking=accept-new -o BatchMode=yes",
        shell_quote_path(&key_path)
    );
    let mut builder = CmdBuilder::clean_cli("git");
    builder
        .args(["ls-remote", "--heads", repo_ssh_url])
        .env("GIT_SSH_COMMAND", ssh_command)
        .env("GIT_TERMINAL_PROMPT", "0");
    let output = tokio::time::timeout(Duration::from_secs(20), builder.output())
        .await
        .map_err(|_| ApiError::BadRequest("Git verification timed out".into()))?
        .map_err(|error| ApiError::Internal(format!("git failed to start: {error}")))?;
    let _ = tokio::fs::remove_dir_all(&verify_dir).await;

    if output.status.success() {
        Ok(aionui_common::now_ms())
    } else {
        Err(ApiError::BadRequest(sanitize_process_error(&output.stderr)))
    }
}

async fn clone_git_project_workspace(
    state: &WorkbenchRouterState,
    user_id: &str,
    project: &GitProjectRow,
    branch_ref: Option<&str>,
    destination: &std::path::Path,
) -> Result<(), ApiError> {
    let credential = owned_credential(state, user_id, &project.credential_id).await?;
    let private_key = aionui_common::decrypt_string(&credential.encrypted_private_key, &state.encryption_key)
        .map_err(|error| ApiError::Internal(format!("Credential decryption failed: {error}")))?;
    let clone_tmp = state
        .senmo_root
        .join("tmp")
        .join(aionui_common::generate_prefixed_id("gitclone"));
    tokio::fs::create_dir_all(&clone_tmp)
        .await
        .map_err(|error| ApiError::Internal(format!("Failed to create git clone temp directory: {error}")))?;
    let key_path = clone_tmp.join("id_key");
    tokio::fs::write(&key_path, private_key)
        .await
        .map_err(|error| ApiError::Internal(format!("Failed to materialize SSH key: {error}")))?;
    set_owner_only_permissions(&key_path).await;
    if let Some(parent) = destination.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|error| ApiError::Internal(format!("Failed to create workspace parent directory: {error}")))?;
    }

    let ssh_command = format!(
        "ssh -i {} -o IdentitiesOnly=yes -o StrictHostKeyChecking=accept-new -o BatchMode=yes",
        shell_quote_path(&key_path)
    );
    let mut builder = CmdBuilder::clean_cli("git");
    builder.arg("clone");
    if let Some(branch) = branch_ref.filter(|value| !value.trim().is_empty()) {
        builder.args(["--branch", branch]);
    }
    builder
        .arg(&project.repo_ssh_url)
        .arg(destination.to_string_lossy().as_ref())
        .env("GIT_SSH_COMMAND", ssh_command)
        .env("GIT_TERMINAL_PROMPT", "0");
    let output = tokio::time::timeout(Duration::from_secs(120), builder.output())
        .await
        .map_err(|_| ApiError::BadRequest("Git clone timed out".into()))?
        .map_err(|error| ApiError::Internal(format!("git failed to start: {error}")))?;
    let _ = tokio::fs::remove_dir_all(&clone_tmp).await;

    if output.status.success() {
        Ok(())
    } else {
        let _ = tokio::fs::remove_dir_all(destination).await;
        Err(ApiError::BadRequest(sanitize_process_error(&output.stderr)))
    }
}

async fn set_owner_only_permissions(path: &std::path::Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(metadata) = tokio::fs::metadata(path).await {
            let mut permissions = metadata.permissions();
            permissions.set_mode(0o600);
            let _ = tokio::fs::set_permissions(path, permissions).await;
        }
    }
}

fn shell_quote_path(path: &std::path::Path) -> String {
    let value = path.to_string_lossy();
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn fingerprint_private_key(private_key: &str) -> String {
    let digest = Sha256::digest(private_key.as_bytes());
    format!("SHA256:{}", hex::encode(digest))
}

fn validate_ssh_private_key_format(private_key: &str) -> Result<(), ApiError> {
    let first = private_key.lines().next().unwrap_or_default().trim();
    let last = private_key.lines().last().unwrap_or_default().trim();
    let valid_boundary = first.starts_with("-----BEGIN ")
        && first.ends_with(" PRIVATE KEY-----")
        && last.starts_with("-----END ")
        && last.ends_with(" PRIVATE KEY-----");
    if !valid_boundary {
        return Err(ApiError::BadRequest(
            "private_key must be a PEM/OpenSSH private key".into(),
        ));
    }

    let body = private_key
        .lines()
        .skip(1)
        .take_while(|line| !line.trim_start().starts_with("-----END "))
        .map(str::trim)
        .collect::<String>();
    let looks_like_base64 = body.len() >= 64
        && body
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '+' | '/' | '='));
    if !looks_like_base64 {
        return Err(ApiError::BadRequest(
            "private_key body is not valid PEM/OpenSSH key data".into(),
        ));
    }

    Ok(())
}

fn sanitize_process_error(stderr: &[u8]) -> String {
    sanitize_process_error_with_default(stderr, "Git verification failed")
}

fn sanitize_process_error_with_default(stderr: &[u8], default_message: &str) -> String {
    let message = String::from_utf8_lossy(stderr);
    let first_line = message.lines().next().unwrap_or(default_message).trim();
    if first_line.is_empty() {
        default_message.into()
    } else {
        first_line.chars().take(240).collect()
    }
}

fn verify_sync_secret(state: &WorkbenchRouterState, headers: &HeaderMap) -> Result<(), ApiError> {
    let Some(config) = &state.gateway_auth else {
        return Err(ApiError::Unauthorized("Gateway sync secret is not configured".into()));
    };
    let app_id = headers
        .get("x-gateway-app-id")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| ApiError::Unauthorized("Missing gateway app id".into()))?;
    if !constant_time_eq(app_id.as_bytes(), config.app_id.as_bytes()) {
        return Err(ApiError::Unauthorized("Invalid gateway app id".into()));
    }
    let bearer = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .ok_or_else(|| ApiError::Unauthorized("Missing gateway app secret".into()))?;
    if constant_time_eq(bearer.as_bytes(), config.app_secret.as_bytes()) {
        Ok(())
    } else {
        Err(ApiError::Unauthorized("Invalid gateway app secret".into()))
    }
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let max_len = left.len().max(right.len());
    let mut diff = left.len() ^ right.len();
    for index in 0..max_len {
        let left_byte = left.get(index).copied().unwrap_or(0);
        let right_byte = right.get(index).copied().unwrap_or(0);
        diff |= usize::from(left_byte ^ right_byte);
    }
    diff == 0
}

fn validate_status(value: &str, allowed: &[&str]) -> Result<(), ApiError> {
    if allowed.contains(&value) {
        Ok(())
    } else {
        Err(ApiError::BadRequest(format!("Unsupported status '{value}'")))
    }
}

async fn audit(
    state: &WorkbenchRouterState,
    actor_user_id: Option<&str>,
    target_type: &str,
    target_id: Option<&str>,
    action: &str,
    metadata: Value,
) {
    if let Err(error) = state
        .audit_logs
        .create_log(CreateAuditLogParams {
            actor_user_id: actor_user_id.map(ToOwned::to_owned),
            target_type: target_type.to_owned(),
            target_id: target_id.map(ToOwned::to_owned),
            action: action.to_owned(),
            result: "success".into(),
            metadata,
            trace_id: None,
        })
        .await
    {
        tracing::warn!(error = %error, "workbench audit log write failed");
    }
}

fn workspace_response(row: WorkspaceRow) -> WorkspaceResponse {
    WorkspaceResponse {
        id: row.id,
        name: row.name,
        source_type: row.source_type,
        git_project_id: row.git_project_id,
        branch_ref: row.branch_ref,
        status: row.status,
        root_rel_path: row.root_rel_path,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

async fn admin_user_response(
    state: &WorkbenchRouterState,
    row: PlatformUserRow,
) -> Result<AdminUserResponse, ApiError> {
    let roles = state
        .roles
        .list_user_roles(&row.id)
        .await
        .map_err(db_error)?
        .into_iter()
        .map(|role| AdminUserRoleSummary {
            id: role.id,
            role_key: role.role_key,
            role_name: role.role_name,
        })
        .collect();
    let external_identities = state
        .external_identities
        .list_by_user(&row.id)
        .await
        .map_err(db_error)?
        .into_iter()
        .map(|identity| AdminUserExternalIdentitySummary {
            provider: identity.provider,
            app_id: identity.app_id,
            external_user_id: identity.external_user_id,
            phone_snapshot: identity.phone_snapshot,
            is_admin: identity.is_admin,
            last_synced_at: identity.last_synced_at,
        })
        .collect();

    Ok(AdminUserResponse {
        id: row.id,
        phone: row.phone,
        username: row.username,
        display_name: row.display_name,
        email: row.email,
        avatar_path: row.avatar_path,
        status: row.status,
        roles,
        external_identities,
        created_at: row.created_at,
        updated_at: row.updated_at,
        last_login: row.last_login,
    })
}

fn ssh_credential_response(row: GitSshCredentialRow) -> SshCredentialResponse {
    SshCredentialResponse {
        id: row.id,
        name: row.name,
        fingerprint: row.fingerprint,
        public_key: row.public_key,
        status: row.status,
        last_verified_at: row.last_verified_at,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

fn git_project_response(row: GitProjectRow) -> GitProjectResponse {
    GitProjectResponse {
        id: row.id,
        credential_id: row.credential_id,
        repo_ssh_url: row.repo_ssh_url,
        display_name: row.display_name,
        default_branch: row.default_branch,
        status: row.status,
        last_verified_at: row.last_verified_at,
        last_synced_at: row.last_synced_at,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

fn snapshot_response(row: SnapshotRow) -> SnapshotResponse {
    SnapshotResponse {
        id: row.id,
        workspace_id: row.workspace_id,
        source_kind: row.source_kind,
        artifact_ref: row.artifact_ref,
        manifest_ref: row.manifest_ref,
        checksum: row.checksum,
        size_bytes: row.size_bytes,
        sealed_at: row.sealed_at,
        expires_at: row.expires_at,
        gc_tombstone_at: row.gc_tombstone_at,
        created_at: row.created_at,
    }
}

fn execution_run_response(row: ExecutionRunRow) -> ExecutionRunResponse {
    ExecutionRunResponse {
        id: row.id,
        workspace_id: row.workspace_id,
        snapshot_id: row.snapshot_id,
        owner_user_id: row.owner_user_id,
        execution_type: row.execution_type,
        status: row.status,
        policy: row.policy,
        resource_profile: row.resource_profile,
        network_profile: row.network_profile,
        trace_id: row.trace_id,
        preview_url: row.preview_url,
        created_at: row.created_at,
        started_at: row.started_at,
        finished_at: row.finished_at,
        updated_at: row.updated_at,
    }
}

fn execution_artifact_response(row: ExecutionArtifactRow) -> ExecutionArtifactResponse {
    ExecutionArtifactResponse {
        id: row.id,
        execution_id: row.execution_id,
        artifact_type: row.artifact_type,
        ref_: row.ref_,
        metadata: row.metadata,
        immutable: row.immutable,
        created_at: row.created_at,
    }
}

fn enabled_status() -> String {
    "enabled".into()
}

fn blank_source_type() -> String {
    "blank".into()
}

fn default_sort_order() -> i32 {
    1000
}

fn db_error(error: aionui_db::DbError) -> ApiError {
    match error {
        aionui_db::DbError::NotFound(message) => ApiError::NotFound(message),
        aionui_db::DbError::Conflict(message) => ApiError::Conflict(message),
        aionui_db::DbError::Query(error) => ApiError::Internal(format!("Database error: {error}")),
        aionui_db::DbError::Migration(error) => ApiError::Internal(format!("Migration error: {error}")),
        aionui_db::DbError::Init(message) => ApiError::Internal(format!("Database init error: {message}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aionui_db::{
        AuditLogRow, CreateExecutionArtifactParams, CreateGitProjectParams, CreateGitSshCredentialParams, DbError,
        ExecutionArtifactRow, ExternalIdentityRow, UpsertExternalIdentityParams,
    };
    use axum::body::Body;
    use axum::extract::{Form, State};
    use axum::http::{HeaderMap, Request, StatusCode, header};
    use tokio::sync::RwLock;
    use tower::ServiceExt;

    #[test]
    fn snapshot_expires_at_prefers_explicit_timestamp() {
        let policy = json!({
            "expires_at": 10_000,
            "ttl_seconds": 60
        });

        assert_eq!(snapshot_expires_at(&policy, 1_000), Some(10_000));
    }

    #[test]
    fn snapshot_expires_at_uses_positive_ttl_seconds() {
        let policy = json!({
            "ttl_seconds": 60
        });

        assert_eq!(snapshot_expires_at(&policy, 1_000), Some(61_000));
    }

    #[test]
    fn permission_registry_contains_system_and_admin_flags() {
        let keys = permission_registry()
            .iter()
            .map(|permission| permission.key)
            .collect::<std::collections::HashSet<_>>();

        assert!(keys.contains("*"));
        assert!(keys.contains("workspace:own"));
        assert!(keys.contains("admin:user:list"));
        assert!(keys.contains("admin:role:list"));
        assert!(keys.contains("admin:role:assign"));
        assert!(keys.contains("admin:snapshot:gc"));
    }

    #[derive(Default)]
    struct WorkbenchTestRepos {
        users: RwLock<Vec<PlatformUserRow>>,
        identities: RwLock<Vec<ExternalIdentityRow>>,
        roles: RwLock<Vec<RoleRow>>,
        user_roles: RwLock<Vec<(String, String)>>,
        ssh_credentials: RwLock<Vec<GitSshCredentialRow>>,
        git_projects: RwLock<Vec<GitProjectRow>>,
        workspaces: RwLock<Vec<WorkspaceRow>>,
        snapshots: RwLock<Vec<SnapshotRow>>,
        execution_runs: RwLock<Vec<ExecutionRunRow>>,
        execution_artifacts: RwLock<Vec<ExecutionArtifactRow>>,
        audit_logs: RwLock<Vec<CreateAuditLogParams>>,
        active_git_projects: RwLock<std::collections::HashSet<String>>,
    }

    fn workbench_test_state(repos: Arc<WorkbenchTestRepos>) -> WorkbenchRouterState {
        WorkbenchRouterState {
            users: repos.clone(),
            external_identities: repos.clone(),
            roles: repos.clone(),
            git_ssh_credentials: repos.clone(),
            git_projects: repos.clone(),
            workspaces: repos.clone(),
            snapshots: repos.clone(),
            execution_runs: repos.clone(),
            execution_artifacts: repos.clone(),
            audit_logs: repos,
            senmo_root: std::path::PathBuf::from("/tmp/senmo-test"),
            gateway_auth: Some(GatewayAuthConfig {
                app_id: "app_aion".into(),
                app_secret: "secret_123".into(),
                provider: "modo_open_platform".into(),
                timestamp_skew_seconds: 300,
            }),
            encryption_key: [7; 32],
            git_ops: Arc::new(DefaultGitProjectOps),
        }
    }

    fn sync_headers() -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert("x-gateway-app-id", "app_aion".parse().unwrap());
        headers.insert(header::AUTHORIZATION, "Bearer secret_123".parse().unwrap());
        headers
    }

    fn sync_form(sync_type: i32, external_user_id: &str, phone: &str, is_admin: bool) -> UserSyncForm {
        UserSyncForm {
            sync_type,
            app_id: Some("app_aion".into()),
            external_user_id: external_user_id.into(),
            phone: Some(phone.into()),
            display_name: Some("同步用户".into()),
            email: None,
            is_admin: Some(is_admin),
        }
    }

    fn test_user_row(id: String, params: CreatePlatformUserParams) -> PlatformUserRow {
        PlatformUserRow {
            id,
            phone: params.phone,
            username: params.username,
            display_name: params.display_name,
            email: params.email,
            password_hash: params.password_hash,
            avatar_path: None,
            status: params.status,
            jwt_secret: None,
            created_at: 1,
            updated_at: 1,
            last_login: None,
        }
    }

    fn platform_test_user(id: &str, phone: &str, status: &str) -> PlatformUserRow {
        PlatformUserRow {
            id: id.to_owned(),
            phone: Some(phone.to_owned()),
            username: None,
            display_name: Some(format!("User {id}")),
            email: None,
            password_hash: None,
            avatar_path: None,
            status: status.to_owned(),
            jwt_secret: None,
            created_at: 1,
            updated_at: 1,
            last_login: None,
        }
    }

    fn workbench_test_role(role_key: &str, status: &str, permissions: Value) -> RoleRow {
        RoleRow {
            id: format!("role_{role_key}"),
            role_key: role_key.to_owned(),
            role_name: role_key.to_owned(),
            status: status.to_owned(),
            permissions,
            sort_order: 1,
            is_system: false,
            created_at: 1,
            updated_at: 1,
        }
    }

    fn current_user(id: &str) -> CurrentUser {
        CurrentUser {
            id: id.to_owned(),
            username: id.to_owned(),
        }
    }

    fn valid_private_key_fixture() -> String {
        format!(
            "-----BEGIN OPENSSH PRIVATE KEY-----\n{}\n-----END OPENSSH PRIVATE KEY-----",
            "A".repeat(80)
        )
    }

    #[derive(Default)]
    struct FakeGitProjectOps {
        fail_message: Option<String>,
        verifications: RwLock<Vec<(String, String)>>,
        clones: RwLock<Vec<(String, Option<String>)>>,
    }

    #[async_trait::async_trait]
    impl GitProjectOps for FakeGitProjectOps {
        async fn verify_access(
            &self,
            _state: &WorkbenchRouterState,
            credential: &GitSshCredentialRow,
            repo_ssh_url: &str,
        ) -> Result<i64, ApiError> {
            if let Some(message) = &self.fail_message {
                return Err(ApiError::BadRequest(message.clone()));
            }
            self.verifications
                .write()
                .await
                .push((credential.id.clone(), repo_ssh_url.to_owned()));
            Ok(42)
        }

        async fn clone_workspace(
            &self,
            _state: &WorkbenchRouterState,
            _user_id: &str,
            project: &GitProjectRow,
            branch_ref: Option<&str>,
            destination: &FsPath,
        ) -> Result<(), ApiError> {
            if let Some(message) = &self.fail_message {
                return Err(ApiError::BadRequest(message.clone()));
            }
            self.clones
                .write()
                .await
                .push((project.id.clone(), branch_ref.map(ToOwned::to_owned)));
            tokio::fs::create_dir_all(destination)
                .await
                .map_err(|error| ApiError::Internal(format!("fake clone failed: {error}")))?;
            tokio::fs::write(destination.join("README.md"), "cloned")
                .await
                .map_err(|error| ApiError::Internal(format!("fake clone failed: {error}")))?;
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl IPlatformUserRepository for WorkbenchTestRepos {
        async fn create_user(&self, params: CreatePlatformUserParams) -> Result<PlatformUserRow, DbError> {
            let mut users = self.users.write().await;
            let user = test_user_row(format!("user_{}", users.len() + 1), params);
            users.push(user.clone());
            Ok(user)
        }

        async fn find_by_id(&self, user_id: &str) -> Result<Option<PlatformUserRow>, DbError> {
            Ok(self.users.read().await.iter().find(|user| user.id == user_id).cloned())
        }

        async fn find_by_phone(&self, phone: &str) -> Result<Option<PlatformUserRow>, DbError> {
            Ok(self
                .users
                .read()
                .await
                .iter()
                .find(|user| user.phone.as_deref() == Some(phone))
                .cloned())
        }

        async fn update_from_sync(
            &self,
            user_id: &str,
            phone: Option<&str>,
            display_name: Option<&str>,
            email: Option<&str>,
            status: &str,
        ) -> Result<PlatformUserRow, DbError> {
            let mut users = self.users.write().await;
            let user = users
                .iter_mut()
                .find(|user| user.id == user_id)
                .ok_or_else(|| DbError::NotFound(format!("user {user_id} not found")))?;
            if let Some(phone) = phone {
                user.phone = Some(phone.to_owned());
            }
            if let Some(display_name) = display_name {
                user.display_name = Some(display_name.to_owned());
            }
            if let Some(email) = email {
                user.email = Some(email.to_owned());
            }
            user.status = status.to_owned();
            Ok(user.clone())
        }

        async fn update_status(&self, user_id: &str, status: &str) -> Result<(), DbError> {
            let mut users = self.users.write().await;
            let user = users
                .iter_mut()
                .find(|user| user.id == user_id)
                .ok_or_else(|| DbError::NotFound(format!("user {user_id} not found")))?;
            user.status = status.to_owned();
            Ok(())
        }

        async fn update_password_hash(&self, user_id: &str, password_hash: Option<&str>) -> Result<(), DbError> {
            let mut users = self.users.write().await;
            let user = users
                .iter_mut()
                .find(|user| user.id == user_id)
                .ok_or_else(|| DbError::NotFound(format!("user {user_id} not found")))?;
            user.password_hash = password_hash.map(ToOwned::to_owned);
            Ok(())
        }

        async fn update_last_login(&self, _user_id: &str) -> Result<(), DbError> {
            Ok(())
        }

        async fn list_users(&self, _limit: i64, _offset: i64) -> Result<Vec<PlatformUserRow>, DbError> {
            Ok(self.users.read().await.clone())
        }
    }

    #[async_trait::async_trait]
    impl IExternalIdentityRepository for WorkbenchTestRepos {
        async fn upsert_identity(&self, params: UpsertExternalIdentityParams) -> Result<ExternalIdentityRow, DbError> {
            let mut identities = self.identities.write().await;
            if let Some(identity) = identities.iter_mut().find(|identity| {
                identity.provider == params.provider
                    && identity.app_id == params.app_id
                    && identity.external_user_id == params.external_user_id
            }) {
                identity.user_id = params.user_id;
                identity.phone_snapshot = params.phone_snapshot;
                identity.external_role_ids = params.external_role_ids;
                identity.is_admin = params.is_admin;
                identity.raw_payload = params.raw_payload;
                identity.last_synced_at = params.last_synced_at;
                return Ok(identity.clone());
            }
            let identity = ExternalIdentityRow {
                id: format!("ext_{}", identities.len() + 1),
                user_id: params.user_id,
                provider: params.provider,
                app_id: params.app_id,
                external_user_id: params.external_user_id,
                phone_snapshot: params.phone_snapshot,
                external_role_ids: params.external_role_ids,
                is_admin: params.is_admin,
                raw_payload: params.raw_payload,
                last_synced_at: params.last_synced_at,
                created_at: 1,
                updated_at: 1,
            };
            identities.push(identity.clone());
            Ok(identity)
        }

        async fn find_identity(
            &self,
            provider: &str,
            app_id: &str,
            external_user_id: &str,
        ) -> Result<Option<ExternalIdentityRow>, DbError> {
            Ok(self
                .identities
                .read()
                .await
                .iter()
                .find(|identity| {
                    identity.provider == provider
                        && identity.app_id == app_id
                        && identity.external_user_id == external_user_id
                })
                .cloned())
        }

        async fn list_by_user(&self, user_id: &str) -> Result<Vec<ExternalIdentityRow>, DbError> {
            Ok(self
                .identities
                .read()
                .await
                .iter()
                .filter(|identity| identity.user_id == user_id)
                .cloned()
                .collect())
        }
    }

    #[async_trait::async_trait]
    impl IRoleRepository for WorkbenchTestRepos {
        async fn upsert_role(&self, params: UpsertRoleParams) -> Result<RoleRow, DbError> {
            let mut roles = self.roles.write().await;
            if let Some(role) = roles.iter_mut().find(|role| role.role_key == params.role_key) {
                role.role_name = params.role_name;
                role.status = params.status;
                role.permissions = params.permissions;
                role.sort_order = params.sort_order;
                role.is_system = params.is_system;
                return Ok(role.clone());
            }
            let role = RoleRow {
                id: format!("role_{}", params.role_key),
                role_key: params.role_key,
                role_name: params.role_name,
                status: params.status,
                permissions: params.permissions,
                sort_order: params.sort_order,
                is_system: params.is_system,
                created_at: 1,
                updated_at: 1,
            };
            roles.push(role.clone());
            Ok(role)
        }

        async fn list_roles(&self) -> Result<Vec<RoleRow>, DbError> {
            Ok(self.roles.read().await.clone())
        }

        async fn set_role_status(&self, role_id: &str, status: &str) -> Result<(), DbError> {
            let mut roles = self.roles.write().await;
            let role = roles
                .iter_mut()
                .find(|role| role.id == role_id)
                .ok_or_else(|| DbError::NotFound(format!("role {role_id} not found")))?;
            role.status = status.to_owned();
            Ok(())
        }

        async fn assign_role(&self, user_id: &str, role_id: &str) -> Result<(), DbError> {
            let mut user_roles = self.user_roles.write().await;
            let grant = (user_id.to_owned(), role_id.to_owned());
            if !user_roles.contains(&grant) {
                user_roles.push(grant);
            }
            Ok(())
        }

        async fn revoke_role(&self, user_id: &str, role_id: &str) -> Result<(), DbError> {
            self.user_roles
                .write()
                .await
                .retain(|grant| grant != &(user_id.to_owned(), role_id.to_owned()));
            Ok(())
        }

        async fn list_user_roles(&self, user_id: &str) -> Result<Vec<RoleRow>, DbError> {
            let grants = self.user_roles.read().await;
            let roles = self.roles.read().await;
            Ok(grants
                .iter()
                .filter(|(grant_user_id, _)| grant_user_id == user_id)
                .filter_map(|(_, role_id)| roles.iter().find(|role| &role.id == role_id).cloned())
                .collect())
        }
    }

    #[async_trait::async_trait]
    impl IAuditLogRepository for WorkbenchTestRepos {
        async fn create_log(&self, params: CreateAuditLogParams) -> Result<AuditLogRow, DbError> {
            let mut logs = self.audit_logs.write().await;
            let id = format!("audit_{}", logs.len() + 1);
            let row = AuditLogRow {
                id,
                actor_user_id: params.actor_user_id.clone(),
                target_type: params.target_type.clone(),
                target_id: params.target_id.clone(),
                action: params.action.clone(),
                result: params.result.clone(),
                metadata: params.metadata.clone(),
                trace_id: params.trace_id.clone(),
                created_at: 1,
            };
            logs.push(params);
            Ok(row)
        }

        async fn list_by_target(&self, target_type: &str, target_id: &str) -> Result<Vec<AuditLogRow>, DbError> {
            Ok(self
                .audit_logs
                .read()
                .await
                .iter()
                .enumerate()
                .filter(|(_, log)| log.target_type == target_type && log.target_id.as_deref() == Some(target_id))
                .map(|(index, log)| AuditLogRow {
                    id: format!("audit_{}", index + 1),
                    actor_user_id: log.actor_user_id.clone(),
                    target_type: log.target_type.clone(),
                    target_id: log.target_id.clone(),
                    action: log.action.clone(),
                    result: log.result.clone(),
                    metadata: log.metadata.clone(),
                    trace_id: log.trace_id.clone(),
                    created_at: 1,
                })
                .collect())
        }
    }

    #[async_trait::async_trait]
    impl IGitSshCredentialRepository for WorkbenchTestRepos {
        async fn create_credential(
            &self,
            params: CreateGitSshCredentialParams,
        ) -> Result<GitSshCredentialRow, DbError> {
            let mut credentials = self.ssh_credentials.write().await;
            let row = GitSshCredentialRow {
                id: format!("sshcred_{}", credentials.len() + 1),
                owner_user_id: params.owner_user_id,
                name: params.name,
                fingerprint: params.fingerprint,
                encrypted_private_key: params.encrypted_private_key,
                encrypted_passphrase: params.encrypted_passphrase,
                public_key: params.public_key,
                status: params.status,
                last_verified_at: None,
                created_at: 1,
                updated_at: 1,
            };
            credentials.push(row.clone());
            Ok(row)
        }

        async fn find_by_id(&self, credential_id: &str) -> Result<Option<GitSshCredentialRow>, DbError> {
            Ok(self
                .ssh_credentials
                .read()
                .await
                .iter()
                .find(|credential| credential.id == credential_id)
                .cloned())
        }

        async fn list_by_owner(&self, owner_user_id: &str) -> Result<Vec<GitSshCredentialRow>, DbError> {
            Ok(self
                .ssh_credentials
                .read()
                .await
                .iter()
                .filter(|credential| credential.owner_user_id == owner_user_id)
                .cloned()
                .collect())
        }

        async fn update_status(
            &self,
            credential_id: &str,
            status: &str,
            last_verified_at: Option<i64>,
        ) -> Result<(), DbError> {
            let mut credentials = self.ssh_credentials.write().await;
            let credential = credentials
                .iter_mut()
                .find(|credential| credential.id == credential_id)
                .ok_or_else(|| DbError::NotFound(format!("credential {credential_id} not found")))?;
            credential.status = status.to_owned();
            credential.last_verified_at = last_verified_at;
            credential.updated_at += 1;
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl IGitProjectRepository for WorkbenchTestRepos {
        async fn create_project(&self, params: CreateGitProjectParams) -> Result<GitProjectRow, DbError> {
            let mut projects = self.git_projects.write().await;
            let row = GitProjectRow {
                id: format!("gitproj_{}", projects.len() + 1),
                owner_user_id: params.owner_user_id,
                credential_id: params.credential_id,
                repo_ssh_url: params.repo_ssh_url,
                display_name: params.display_name,
                default_branch: params.default_branch,
                status: params.status,
                last_verified_at: None,
                last_synced_at: None,
                created_at: 1,
                updated_at: 1,
            };
            projects.push(row.clone());
            Ok(row)
        }

        async fn find_by_id(&self, project_id: &str) -> Result<Option<GitProjectRow>, DbError> {
            Ok(self
                .git_projects
                .read()
                .await
                .iter()
                .find(|project| project.id == project_id)
                .cloned())
        }

        async fn list_by_owner(&self, owner_user_id: &str) -> Result<Vec<GitProjectRow>, DbError> {
            Ok(self
                .git_projects
                .read()
                .await
                .iter()
                .filter(|project| project.owner_user_id == owner_user_id)
                .cloned()
                .collect())
        }

        async fn update_project(
            &self,
            project_id: &str,
            credential_id: &str,
            repo_ssh_url: &str,
            display_name: &str,
            default_branch: Option<&str>,
        ) -> Result<GitProjectRow, DbError> {
            let mut projects = self.git_projects.write().await;
            let project = projects
                .iter_mut()
                .find(|project| project.id == project_id)
                .ok_or_else(|| DbError::NotFound(format!("project {project_id} not found")))?;
            project.credential_id = credential_id.to_owned();
            project.repo_ssh_url = repo_ssh_url.to_owned();
            project.display_name = display_name.to_owned();
            project.default_branch = default_branch.map(ToOwned::to_owned);
            project.updated_at += 1;
            Ok(project.clone())
        }

        async fn update_status(
            &self,
            project_id: &str,
            status: &str,
            last_verified_at: Option<i64>,
            last_synced_at: Option<i64>,
        ) -> Result<(), DbError> {
            let mut projects = self.git_projects.write().await;
            let project = projects
                .iter_mut()
                .find(|project| project.id == project_id)
                .ok_or_else(|| DbError::NotFound(format!("project {project_id} not found")))?;
            project.status = status.to_owned();
            project.last_verified_at = last_verified_at;
            project.last_synced_at = last_synced_at;
            project.updated_at += 1;
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl IWorkspaceRepository for WorkbenchTestRepos {
        async fn create_workspace(&self, params: CreateWorkspaceParams) -> Result<WorkspaceRow, DbError> {
            let mut workspaces = self.workspaces.write().await;
            let row = WorkspaceRow {
                id: format!("ws_{}", workspaces.len() + 1),
                owner_user_id: params.owner_user_id,
                name: params.name,
                source_type: params.source_type,
                git_project_id: params.git_project_id,
                branch_ref: params.branch_ref,
                status: params.status,
                root_rel_path: params.root_rel_path,
                created_at: 1,
                updated_at: 1,
                deleted_at: None,
            };
            workspaces.push(row.clone());
            Ok(row)
        }

        async fn find_by_id(&self, workspace_id: &str) -> Result<Option<WorkspaceRow>, DbError> {
            Ok(self
                .workspaces
                .read()
                .await
                .iter()
                .find(|workspace| workspace.id == workspace_id)
                .cloned())
        }

        async fn list_by_owner(&self, owner_user_id: &str) -> Result<Vec<WorkspaceRow>, DbError> {
            Ok(self
                .workspaces
                .read()
                .await
                .iter()
                .filter(|workspace| workspace.owner_user_id == owner_user_id)
                .cloned()
                .collect())
        }

        async fn has_active_by_git_project(&self, git_project_id: &str) -> Result<bool, DbError> {
            if self.active_git_projects.read().await.contains(git_project_id) {
                return Ok(true);
            }
            Ok(self.workspaces.read().await.iter().any(|workspace| {
                workspace.git_project_id.as_deref() == Some(git_project_id)
                    && !matches!(workspace.status.as_str(), "archived" | "deleted")
            }))
        }

        async fn update_name(&self, workspace_id: &str, name: &str) -> Result<(), DbError> {
            let mut workspaces = self.workspaces.write().await;
            let workspace = workspaces
                .iter_mut()
                .find(|workspace| workspace.id == workspace_id)
                .ok_or_else(|| DbError::NotFound(format!("workspace {workspace_id} not found")))?;
            workspace.name = name.to_owned();
            workspace.updated_at += 1;
            Ok(())
        }

        async fn update_status(&self, workspace_id: &str, status: &str) -> Result<(), DbError> {
            let mut workspaces = self.workspaces.write().await;
            let workspace = workspaces
                .iter_mut()
                .find(|workspace| workspace.id == workspace_id)
                .ok_or_else(|| DbError::NotFound(format!("workspace {workspace_id} not found")))?;
            workspace.status = status.to_owned();
            workspace.updated_at += 1;
            Ok(())
        }

        async fn mark_deleted(&self, workspace_id: &str) -> Result<(), DbError> {
            let mut workspaces = self.workspaces.write().await;
            let workspace = workspaces
                .iter_mut()
                .find(|workspace| workspace.id == workspace_id)
                .ok_or_else(|| DbError::NotFound(format!("workspace {workspace_id} not found")))?;
            workspace.status = "deleted".to_owned();
            workspace.deleted_at = Some(1);
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl ISnapshotRepository for WorkbenchTestRepos {
        async fn create_snapshot(&self, params: CreateSnapshotParams) -> Result<SnapshotRow, DbError> {
            let mut snapshots = self.snapshots.write().await;
            let row = SnapshotRow {
                id: format!("snap_{}", snapshots.len() + 1),
                workspace_id: params.workspace_id,
                created_by: params.created_by,
                source_kind: params.source_kind,
                artifact_ref: params.artifact_ref,
                manifest_ref: params.manifest_ref,
                checksum: params.checksum,
                size_bytes: params.size_bytes,
                base_git_metadata: params.base_git_metadata,
                environment_metadata: params.environment_metadata,
                retention_policy: params.retention_policy,
                sealed_at: params.sealed_at,
                expires_at: params.expires_at,
                gc_tombstone_at: None,
                created_at: 1,
            };
            snapshots.push(row.clone());
            Ok(row)
        }

        async fn find_by_id(&self, snapshot_id: &str) -> Result<Option<SnapshotRow>, DbError> {
            Ok(self
                .snapshots
                .read()
                .await
                .iter()
                .find(|snapshot| snapshot.id == snapshot_id)
                .cloned())
        }

        async fn list_by_workspace(&self, workspace_id: &str) -> Result<Vec<SnapshotRow>, DbError> {
            Ok(self
                .snapshots
                .read()
                .await
                .iter()
                .filter(|snapshot| snapshot.workspace_id == workspace_id)
                .cloned()
                .collect())
        }

        async fn list_gc_candidates(&self, now_ms: i64, limit: i64) -> Result<Vec<SnapshotRow>, DbError> {
            Ok(self
                .snapshots
                .read()
                .await
                .iter()
                .filter(|snapshot| {
                    snapshot.gc_tombstone_at.is_none()
                        && snapshot.expires_at.is_some_and(|expires_at| expires_at <= now_ms)
                })
                .take(limit.max(0) as usize)
                .cloned()
                .collect())
        }

        async fn mark_gc_tombstone(&self, snapshot_id: &str, gc_tombstone_at: i64) -> Result<(), DbError> {
            let mut snapshots = self.snapshots.write().await;
            let snapshot = snapshots
                .iter_mut()
                .find(|snapshot| snapshot.id == snapshot_id)
                .ok_or_else(|| DbError::NotFound(format!("snapshot {snapshot_id} not found")))?;
            snapshot.gc_tombstone_at = Some(gc_tombstone_at);
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl IExecutionRunRepository for WorkbenchTestRepos {
        async fn create_execution_run(&self, params: CreateExecutionRunParams) -> Result<ExecutionRunRow, DbError> {
            let mut execution_runs = self.execution_runs.write().await;
            let row = ExecutionRunRow {
                id: format!("exec_{}", execution_runs.len() + 1),
                workspace_id: params.workspace_id,
                snapshot_id: params.snapshot_id,
                owner_user_id: params.owner_user_id,
                execution_type: params.execution_type,
                status: params.status,
                policy: params.policy,
                resource_profile: params.resource_profile,
                network_profile: params.network_profile,
                trace_id: params.trace_id,
                preview_url: None,
                created_at: 1,
                started_at: None,
                finished_at: None,
                updated_at: 1,
            };
            execution_runs.push(row.clone());
            Ok(row)
        }

        async fn find_by_id(&self, execution_id: &str) -> Result<Option<ExecutionRunRow>, DbError> {
            Ok(self
                .execution_runs
                .read()
                .await
                .iter()
                .find(|execution| execution.id == execution_id)
                .cloned())
        }

        async fn list_by_workspace(&self, workspace_id: &str) -> Result<Vec<ExecutionRunRow>, DbError> {
            Ok(self
                .execution_runs
                .read()
                .await
                .iter()
                .filter(|execution| execution.workspace_id == workspace_id)
                .cloned()
                .collect())
        }

        async fn has_active_by_snapshot(&self, snapshot_id: &str) -> Result<bool, DbError> {
            Ok(self.execution_runs.read().await.iter().any(|execution| {
                execution.snapshot_id == snapshot_id
                    && matches!(
                        execution.status.as_str(),
                        "created"
                            | "preparing"
                            | "snapshot_resolved"
                            | "provisioning"
                            | "running"
                            | "streaming"
                            | "collecting"
                    )
            }))
        }

        async fn update_status(
            &self,
            execution_id: &str,
            status: &str,
            started_at: Option<i64>,
            finished_at: Option<i64>,
        ) -> Result<(), DbError> {
            let mut execution_runs = self.execution_runs.write().await;
            let execution = execution_runs
                .iter_mut()
                .find(|execution| execution.id == execution_id)
                .ok_or_else(|| DbError::NotFound(format!("execution {execution_id} not found")))?;
            execution.status = status.to_owned();
            execution.started_at = started_at;
            execution.finished_at = finished_at;
            execution.updated_at += 1;
            Ok(())
        }

        async fn set_preview_url(&self, execution_id: &str, preview_url: Option<&str>) -> Result<(), DbError> {
            let mut execution_runs = self.execution_runs.write().await;
            let execution = execution_runs
                .iter_mut()
                .find(|execution| execution.id == execution_id)
                .ok_or_else(|| DbError::NotFound(format!("execution {execution_id} not found")))?;
            execution.preview_url = preview_url.map(ToOwned::to_owned);
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl IExecutionArtifactRepository for WorkbenchTestRepos {
        async fn create_artifact(
            &self,
            params: CreateExecutionArtifactParams,
        ) -> Result<ExecutionArtifactRow, DbError> {
            let mut artifacts = self.execution_artifacts.write().await;
            let row = ExecutionArtifactRow {
                id: format!("artifact_{}", artifacts.len() + 1),
                execution_id: params.execution_id,
                artifact_type: params.artifact_type,
                ref_: params.ref_,
                metadata: params.metadata,
                immutable: params.immutable,
                created_at: 1,
            };
            artifacts.push(row.clone());
            Ok(row)
        }

        async fn list_by_execution(&self, execution_id: &str) -> Result<Vec<ExecutionArtifactRow>, DbError> {
            Ok(self
                .execution_artifacts
                .read()
                .await
                .iter()
                .filter(|artifact| artifact.execution_id == execution_id)
                .cloned()
                .collect())
        }
    }

    #[tokio::test]
    async fn user_sync_is_idempotent_for_same_external_identity() {
        let repos = Arc::new(WorkbenchTestRepos::default());
        let state = workbench_test_state(repos.clone());

        for _ in 0..2 {
            let response = user_sync_handler(
                State(state.clone()),
                sync_headers(),
                Form(sync_form(1, "external_u1", "13800138000", true)),
            )
            .await
            .unwrap();
            assert_eq!(response.0["status"], 0);
        }

        assert_eq!(repos.users.read().await.len(), 1);
        assert_eq!(repos.identities.read().await.len(), 1);
        assert_eq!(repos.user_roles.read().await.len(), 2);
        let roles = repos.roles.read().await;
        assert!(roles.iter().any(|role| role.role_key == DEFAULT_USER_ROLE_KEY));
        assert!(roles.iter().any(|role| role.role_key == SUPER_ADMIN_ROLE_KEY));
    }

    #[tokio::test]
    async fn user_sync_requires_gateway_app_secret() {
        let repos = Arc::new(WorkbenchTestRepos::default());
        let state = workbench_test_state(repos.clone());
        let mut headers = HeaderMap::new();
        headers.insert("x-gateway-app-id", "app_aion".parse().unwrap());

        let error = user_sync_handler(
            State(state.clone()),
            headers,
            Form(sync_form(1, "external_u1", "13800138000", true)),
        )
        .await
        .unwrap_err();
        assert!(matches!(error, ApiError::Unauthorized(_)));
        assert!(repos.users.read().await.is_empty());

        let response = user_sync_handler(
            State(state),
            sync_headers(),
            Form(sync_form(1, "external_u1", "13800138000", true)),
        )
        .await
        .unwrap();
        assert_eq!(response.0["status"], 0);
    }

    #[tokio::test]
    async fn user_sync_public_route_requires_gateway_secret_before_handler() {
        let repos = Arc::new(WorkbenchTestRepos::default());
        let state = workbench_test_state(repos.clone());
        let app = workbench_public_routes(state.clone());
        let body = "type=1&app_id=app_aion&external_user_id=external_u1&phone=13800138000&display_name=sync-user&is_admin=true";

        let rejected = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/open-platform/user-sync")
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(rejected.status(), StatusCode::UNAUTHORIZED);
        assert!(repos.users.read().await.is_empty());

        let accepted = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/open-platform/user-sync")
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .header("x-gateway-app-id", "app_aion")
                    .header(header::AUTHORIZATION, "Bearer secret_123")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(accepted.status(), StatusCode::OK);
        assert_eq!(repos.users.read().await.len(), 1);
    }

    #[tokio::test]
    async fn user_sync_updates_phone_by_stable_external_identity() {
        let repos = Arc::new(WorkbenchTestRepos::default());
        let state = workbench_test_state(repos.clone());

        let _ = user_sync_handler(
            State(state.clone()),
            sync_headers(),
            Form(sync_form(1, "external_u1", "13800138000", false)),
        )
        .await
        .unwrap();
        let _ = user_sync_handler(
            State(state),
            sync_headers(),
            Form(sync_form(3, "external_u1", "13900139000", false)),
        )
        .await
        .unwrap();

        let users = repos.users.read().await;
        assert_eq!(users.len(), 1);
        assert_eq!(users[0].id, "user_1");
        assert_eq!(users[0].phone.as_deref(), Some("13900139000"));
        let identities = repos.identities.read().await;
        assert_eq!(identities.len(), 1);
        assert_eq!(identities[0].user_id, "user_1");
        assert_eq!(identities[0].phone_snapshot.as_deref(), Some("13900139000"));
    }

    #[tokio::test]
    async fn admin_api_rejects_current_user_without_aion_permission() {
        let repos = Arc::new(WorkbenchTestRepos::default());
        repos
            .users
            .write()
            .await
            .push(platform_test_user("user_1", "13800138000", "enabled"));
        repos.roles.write().await.push(workbench_test_role(
            "ordinary_user",
            "enabled",
            json!(["workspace:own"]),
        ));
        repos
            .user_roles
            .write()
            .await
            .push(("user_1".to_owned(), "role_ordinary_user".to_owned()));
        let state = workbench_test_state(repos);

        let err = list_users_handler(State(state), Extension(current_user("user_1")))
            .await
            .unwrap_err();

        assert!(matches!(err, ApiError::Forbidden(message) if message.contains("admin:user:list")));
    }

    #[tokio::test]
    async fn admin_api_rejects_disabled_role_permission() {
        let repos = Arc::new(WorkbenchTestRepos::default());
        repos
            .users
            .write()
            .await
            .push(platform_test_user("user_1", "13800138000", "enabled"));
        repos.roles.write().await.push(workbench_test_role(
            "admin_reader",
            "disabled",
            json!(["admin:user:list"]),
        ));
        repos
            .user_roles
            .write()
            .await
            .push(("user_1".to_owned(), "role_admin_reader".to_owned()));
        let state = workbench_test_state(repos);

        let err = list_users_handler(State(state), Extension(current_user("user_1")))
            .await
            .unwrap_err();

        assert!(matches!(err, ApiError::Forbidden(message) if message.contains("admin:user:list")));
    }

    #[tokio::test]
    async fn admin_api_allows_assigned_aion_permission() {
        let repos = Arc::new(WorkbenchTestRepos::default());
        let mut admin = platform_test_user("user_1", "13800138000", "enabled");
        admin.password_hash = Some("should-not-leak".to_owned());
        admin.jwt_secret = Some("should-not-leak".to_owned());
        repos.users.write().await.push(admin);
        repos
            .users
            .write()
            .await
            .push(platform_test_user("user_2", "13900139000", "enabled"));
        repos.roles.write().await.push(workbench_test_role(
            "admin_reader",
            "enabled",
            json!(["admin:user:list"]),
        ));
        repos
            .user_roles
            .write()
            .await
            .push(("user_1".to_owned(), "role_admin_reader".to_owned()));
        let state = workbench_test_state(repos);

        let response = list_users_handler(State(state), Extension(current_user("user_1")))
            .await
            .unwrap();

        let users = response.0.data.expect("users response data");
        assert_eq!(users.len(), 2);
        assert_eq!(users[0].roles.len(), 1);
        assert_eq!(users[0].roles[0].role_key, "admin_reader");
        let serialized = serde_json::to_value(&users).unwrap();
        assert!(serialized.to_string().contains("password_hash") == false);
        assert!(serialized.to_string().contains("jwt_secret") == false);
    }

    #[tokio::test]
    async fn frozen_snapshot_content_does_not_follow_workspace_changes() {
        let root = std::env::temp_dir().join(aionui_common::generate_prefixed_id("snapshot-freeze-test"));
        let workspace = root.join("workspace");
        let first_snapshot = root.join("snapshot_1");
        let second_snapshot = root.join("snapshot_2");
        tokio::fs::create_dir_all(workspace.join("src")).await.unwrap();
        tokio::fs::write(workspace.join("src/app.txt"), "v1").await.unwrap();

        let first = freeze_workspace_snapshot(&workspace, &first_snapshot).await.unwrap();
        tokio::fs::write(workspace.join("src/app.txt"), "v2").await.unwrap();
        let second = freeze_workspace_snapshot(&workspace, &second_snapshot).await.unwrap();

        assert_eq!(
            tokio::fs::read_to_string(first_snapshot.join("src/app.txt"))
                .await
                .unwrap(),
            "v1"
        );
        assert_eq!(
            tokio::fs::read_to_string(second_snapshot.join("src/app.txt"))
                .await
                .unwrap(),
            "v2"
        );
        assert_ne!(first.checksum, second.checksum);
        let _ = tokio::fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn artifacts_are_bound_to_execution_and_return_as_immutable() {
        let repos = Arc::new(WorkbenchTestRepos::default());
        let state = workbench_test_state(repos.clone());
        let execution = repos
            .create_execution_run(CreateExecutionRunParams {
                workspace_id: "ws_1".to_owned(),
                snapshot_id: "snap_1".to_owned(),
                owner_user_id: "user_1".to_owned(),
                execution_type: "test_run".to_owned(),
                status: "succeeded".to_owned(),
                policy: json!({}),
                resource_profile: json!({}),
                network_profile: None,
                trace_id: "trace_1".to_owned(),
            })
            .await
            .unwrap();
        repos
            .create_artifact(CreateExecutionArtifactParams {
                execution_id: execution.id.clone(),
                artifact_type: "structured_failure".to_owned(),
                ref_: "artifacts/trace.json".to_owned(),
                metadata: json!({ "reason": "button missing" }),
                immutable: true,
            })
            .await
            .unwrap();
        repos
            .create_artifact(CreateExecutionArtifactParams {
                execution_id: "exec_other".to_owned(),
                artifact_type: "log".to_owned(),
                ref_: "artifacts/other.log".to_owned(),
                metadata: json!({}),
                immutable: true,
            })
            .await
            .unwrap();

        let response =
            list_execution_artifacts_handler(State(state), Extension(current_user("user_1")), Path(execution.id))
                .await
                .unwrap();

        let artifacts = response.0.data.expect("artifact response data");
        assert_eq!(artifacts.len(), 1);
        assert_eq!(artifacts[0].artifact_type, "structured_failure");
        assert_eq!(artifacts[0].metadata["reason"], "button missing");
        assert!(artifacts[0].immutable);
    }

    #[tokio::test]
    async fn snapshot_gc_keeps_metadata_and_execution_history() {
        let repos = Arc::new(WorkbenchTestRepos::default());
        let root = std::env::temp_dir().join(aionui_common::generate_prefixed_id("snapshot-gc-test"));
        let mut state = workbench_test_state(repos.clone());
        state.senmo_root = root.clone();
        let snapshot = repos
            .create_snapshot(CreateSnapshotParams {
                workspace_id: "ws_1".to_owned(),
                created_by: "user_1".to_owned(),
                source_kind: "workspace_export".to_owned(),
                artifact_ref: "snapshots/snap_1/content".to_owned(),
                manifest_ref: "snapshots/snap_1/manifest.json".to_owned(),
                checksum: "sha256:abc".to_owned(),
                size_bytes: 10,
                base_git_metadata: json!({}),
                environment_metadata: json!({}),
                retention_policy: json!({ "expires_at": 1 }),
                sealed_at: 1,
                expires_at: Some(1),
            })
            .await
            .unwrap();
        tokio::fs::create_dir_all(root.join(&snapshot.artifact_ref))
            .await
            .unwrap();
        tokio::fs::write(root.join(&snapshot.artifact_ref).join("app.txt"), "sealed")
            .await
            .unwrap();
        let execution = repos
            .create_execution_run(CreateExecutionRunParams {
                workspace_id: snapshot.workspace_id.clone(),
                snapshot_id: snapshot.id.clone(),
                owner_user_id: "user_1".to_owned(),
                execution_type: "test_run".to_owned(),
                status: "succeeded".to_owned(),
                policy: json!({}),
                resource_profile: json!({}),
                network_profile: None,
                trace_id: "trace_1".to_owned(),
            })
            .await
            .unwrap();

        let gc = run_snapshot_gc(&state, 100).await.unwrap();

        assert_eq!(gc.deleted_content, 1);
        assert_eq!(gc.tombstoned, 1);
        assert!(!root.join(&snapshot.artifact_ref).exists());
        let tombstoned = ISnapshotRepository::find_by_id(&*repos, &snapshot.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(tombstoned.checksum, "sha256:abc");
        assert!(tombstoned.gc_tombstone_at.is_some());
        let history = IExecutionRunRepository::find_by_id(&*repos, &execution.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(history.snapshot_id, snapshot.id);
        assert_eq!(history.status, "succeeded");
        let _ = tokio::fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn ssh_credential_upload_encrypts_and_response_does_not_echo_secret() {
        let repos = Arc::new(WorkbenchTestRepos::default());
        let state = workbench_test_state(repos.clone());
        let private_key = valid_private_key_fixture();

        let response = create_ssh_credential_handler(
            State(state),
            Extension(current_user("user_1")),
            Json(CreateSshCredentialRequest {
                name: "main key".to_owned(),
                private_key: private_key.clone(),
                passphrase: Some("passphrase-secret".to_owned()),
            }),
        )
        .await
        .unwrap();

        let credential = repos.ssh_credentials.read().await[0].clone();
        assert_ne!(credential.encrypted_private_key, private_key);
        assert_ne!(credential.encrypted_passphrase.as_deref(), Some("passphrase-secret"));
        let body = serde_json::to_string(&response.0).unwrap();
        assert!(!body.contains(private_key.as_str()));
        assert!(!body.contains("passphrase-secret"));
        let returned = response.0.data.expect("credential response data");
        assert_eq!(returned.fingerprint, fingerprint_private_key(&private_key));
        assert_eq!(returned.status, "pending");
    }

    #[tokio::test]
    async fn ssh_credential_upload_rejects_invalid_private_key() {
        let repos = Arc::new(WorkbenchTestRepos::default());
        let state = workbench_test_state(repos.clone());

        let err = create_ssh_credential_handler(
            State(state),
            Extension(current_user("user_1")),
            Json(CreateSshCredentialRequest {
                name: "bad key".to_owned(),
                private_key: "-----BEGIN OPENSSH PRIVATE KEY-----\nreview-secret\n-----END OPENSSH PRIVATE KEY-----"
                    .to_owned(),
                passphrase: None,
            }),
        )
        .await
        .unwrap_err();

        assert!(matches!(err, ApiError::BadRequest(message) if message.contains("private_key body")));
        assert!(repos.ssh_credentials.read().await.is_empty());
    }

    #[tokio::test]
    async fn ssh_credential_generate_returns_public_key_without_private_key() {
        if std::process::Command::new("ssh-keygen").arg("-?").output().is_err() {
            return;
        }

        let repos = Arc::new(WorkbenchTestRepos::default());
        let root = std::env::temp_dir().join(aionui_common::generate_prefixed_id("ssh-generate-test"));
        let mut state = workbench_test_state(repos.clone());
        state.senmo_root = root.clone();

        let response = generate_ssh_credential_handler(
            State(state),
            Extension(current_user("user_1")),
            Json(GenerateSshCredentialRequest {
                name: "generated key".to_owned(),
                comment: Some("review@example.local".to_owned()),
            }),
        )
        .await
        .unwrap();

        let body = serde_json::to_string(&response.0).unwrap();
        assert!(body.contains("ssh-ed25519"));
        assert!(!body.contains("BEGIN OPENSSH PRIVATE KEY"));
        assert_eq!(repos.ssh_credentials.read().await.len(), 1);
        let _ = tokio::fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn git_project_create_and_sync_use_owner_credential_without_secret_echo() {
        let repos = Arc::new(WorkbenchTestRepos::default());
        let ops = Arc::new(FakeGitProjectOps::default());
        let mut state = workbench_test_state(repos.clone());
        state.git_ops = ops.clone();
        let encrypted_private_key = encrypt_string("private-key-secret", &state.encryption_key).unwrap();
        let credential = repos
            .create_credential(CreateGitSshCredentialParams {
                owner_user_id: "user_1".to_owned(),
                name: "main key".to_owned(),
                fingerprint: "SHA256:test".to_owned(),
                encrypted_private_key,
                encrypted_passphrase: None,
                public_key: None,
                status: "pending".to_owned(),
            })
            .await
            .unwrap();

        let created = create_git_project_handler(
            State(state.clone()),
            Extension(current_user("user_1")),
            Json(CreateGitProjectRequest {
                credential_id: credential.id.clone(),
                repo_ssh_url: "git@example.com:team/repo.git".to_owned(),
                display_name: "repo".to_owned(),
                default_branch: Some("main".to_owned()),
            }),
        )
        .await
        .unwrap()
        .0
        .data
        .expect("git project response data");

        assert_eq!(created.status, "active");
        assert_eq!(created.last_verified_at, Some(42));
        assert_eq!(created.last_synced_at, Some(42));
        assert_eq!(
            ops.verifications.read().await.as_slice(),
            &[(credential.id.clone(), "git@example.com:team/repo.git".to_owned())]
        );
        let body = serde_json::to_string(&created).unwrap();
        assert!(!body.contains("private-key-secret"));

        let synced = sync_git_project_handler(
            State(state),
            Extension(current_user("user_1")),
            Path(created.id.clone()),
        )
        .await
        .unwrap()
        .0
        .data
        .expect("git sync response data");

        assert_eq!(synced.status, "active");
        assert_eq!(synced.last_synced_at, Some(42));
        assert_eq!(ops.verifications.read().await.len(), 2);
        let stored_credential = IGitSshCredentialRepository::find_by_id(&*repos, &credential.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored_credential.status, "verified");
        assert_eq!(stored_credential.last_verified_at, Some(42));
    }

    #[tokio::test]
    async fn git_project_lists_only_owner_and_rejects_cross_owner_credential() {
        let repos = Arc::new(WorkbenchTestRepos::default());
        let ops = Arc::new(FakeGitProjectOps::default());
        let mut state = workbench_test_state(repos.clone());
        state.git_ops = ops;
        let user1_key = repos
            .create_credential(CreateGitSshCredentialParams {
                owner_user_id: "user_1".to_owned(),
                name: "user1".to_owned(),
                fingerprint: "SHA256:user1".to_owned(),
                encrypted_private_key: encrypt_string("user1-secret", &state.encryption_key).unwrap(),
                encrypted_passphrase: None,
                public_key: None,
                status: "verified".to_owned(),
            })
            .await
            .unwrap();
        let user2_key = repos
            .create_credential(CreateGitSshCredentialParams {
                owner_user_id: "user_2".to_owned(),
                name: "user2".to_owned(),
                fingerprint: "SHA256:user2".to_owned(),
                encrypted_private_key: encrypt_string("user2-secret", &state.encryption_key).unwrap(),
                encrypted_passphrase: None,
                public_key: None,
                status: "verified".to_owned(),
            })
            .await
            .unwrap();
        repos
            .create_project(CreateGitProjectParams {
                owner_user_id: "user_2".to_owned(),
                credential_id: user2_key.id.clone(),
                repo_ssh_url: "git@example.com:team/other.git".to_owned(),
                display_name: "other".to_owned(),
                default_branch: Some("main".to_owned()),
                status: "active".to_owned(),
            })
            .await
            .unwrap();

        let err = create_git_project_handler(
            State(state.clone()),
            Extension(current_user("user_1")),
            Json(CreateGitProjectRequest {
                credential_id: user2_key.id,
                repo_ssh_url: "git@example.com:team/repo.git".to_owned(),
                display_name: "repo".to_owned(),
                default_branch: None,
            }),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, ApiError::Forbidden(message) if message.contains("SSH credential access denied")));

        let _ = create_git_project_handler(
            State(state.clone()),
            Extension(current_user("user_1")),
            Json(CreateGitProjectRequest {
                credential_id: user1_key.id,
                repo_ssh_url: "git@example.com:team/repo.git".to_owned(),
                display_name: "repo".to_owned(),
                default_branch: None,
            }),
        )
        .await
        .unwrap();

        let listed = list_git_projects_handler(State(state), Extension(current_user("user_1")))
            .await
            .unwrap()
            .0
            .data
            .expect("git project list data");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].display_name, "repo");
    }

    #[tokio::test]
    async fn git_project_sync_failure_is_sanitized_and_delete_is_protected() {
        let repos = Arc::new(WorkbenchTestRepos::default());
        let mut state = workbench_test_state(repos.clone());
        state.git_ops = Arc::new(FakeGitProjectOps {
            fail_message: Some("Permission denied (publickey)".to_owned()),
            ..Default::default()
        });
        let credential = repos
            .create_credential(CreateGitSshCredentialParams {
                owner_user_id: "user_1".to_owned(),
                name: "main".to_owned(),
                fingerprint: "SHA256:test".to_owned(),
                encrypted_private_key: encrypt_string("private-key-secret", &state.encryption_key).unwrap(),
                encrypted_passphrase: None,
                public_key: None,
                status: "verified".to_owned(),
            })
            .await
            .unwrap();
        let project = repos
            .create_project(CreateGitProjectParams {
                owner_user_id: "user_1".to_owned(),
                credential_id: credential.id,
                repo_ssh_url: "git@example.com:team/repo.git".to_owned(),
                display_name: "repo".to_owned(),
                default_branch: Some("main".to_owned()),
                status: "active".to_owned(),
            })
            .await
            .unwrap();

        let err = sync_git_project_handler(
            State(state.clone()),
            Extension(current_user("user_1")),
            Path(project.id.clone()),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, ApiError::BadRequest(message) if message == "Permission denied (publickey)"));

        repos.active_git_projects.write().await.insert(project.id.clone());
        let err = delete_git_project_handler(State(state), Extension(current_user("user_1")), Path(project.id))
            .await
            .unwrap_err();
        assert!(matches!(err, ApiError::Conflict(message) if message.contains("active workspaces")));
    }

    #[tokio::test]
    async fn workspace_blank_create_archive_delete_and_audit_events() {
        let repos = Arc::new(WorkbenchTestRepos::default());
        let root = std::env::temp_dir().join(aionui_common::generate_prefixed_id("workspace-blank-test"));
        let mut state = workbench_test_state(repos.clone());
        state.senmo_root = root.clone();

        let created = create_workspace_handler(
            State(state.clone()),
            Extension(current_user("user_1")),
            Json(CreateWorkspaceRequest {
                name: "blank".to_owned(),
                source_type: "blank".to_owned(),
                git_project_id: None,
                branch_ref: None,
            }),
        )
        .await
        .unwrap()
        .0
        .data
        .expect("workspace response data");

        assert_eq!(created.source_type, "blank");
        assert!(root.join(&created.root_rel_path).exists());
        let write_response = write_workspace_file_handler(
            State(state.clone()),
            Extension(current_user("user_1")),
            Path(created.id.clone()),
            Json(WorkspaceFileWriteRequest {
                relative_path: "src/review.txt".to_owned(),
                content: "created workspace can write nested files".to_owned(),
            }),
        )
        .await
        .unwrap();
        assert_eq!(write_response.0.data, Some(true));
        assert_eq!(
            tokio::fs::read_to_string(root.join(&created.root_rel_path).join("src/review.txt"))
                .await
                .unwrap(),
            "created workspace can write nested files"
        );
        let _ = archive_workspace_handler(
            State(state.clone()),
            Extension(current_user("user_1")),
            Path(created.id.clone()),
        )
        .await
        .unwrap();
        let _ = delete_workspace_handler(
            State(state.clone()),
            Extension(current_user("user_1")),
            Path(created.id.clone()),
        )
        .await
        .unwrap();

        let logs = repos.audit_logs.read().await;
        assert!(logs.iter().any(|log| {
            log.target_type == "workspace"
                && log.target_id.as_deref() == Some(&created.id)
                && log.action == "create_blank"
        }));
        assert!(logs.iter().any(|log| {
            log.target_type == "workspace" && log.target_id.as_deref() == Some(&created.id) && log.action == "archive"
        }));
        assert!(logs.iter().any(|log| {
            log.target_type == "workspace" && log.target_id.as_deref() == Some(&created.id) && log.action == "delete"
        }));
        let _ = tokio::fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn workspace_git_backed_create_allows_same_project_multiple_workspaces() {
        let repos = Arc::new(WorkbenchTestRepos::default());
        let ops = Arc::new(FakeGitProjectOps::default());
        let root = std::env::temp_dir().join(aionui_common::generate_prefixed_id("workspace-git-test"));
        let mut state = workbench_test_state(repos.clone());
        state.senmo_root = root.clone();
        state.git_ops = ops.clone();
        let project = repos
            .create_project(CreateGitProjectParams {
                owner_user_id: "user_1".to_owned(),
                credential_id: "sshcred_1".to_owned(),
                repo_ssh_url: "git@example.com:team/repo.git".to_owned(),
                display_name: "repo".to_owned(),
                default_branch: Some("main".to_owned()),
                status: "active".to_owned(),
            })
            .await
            .unwrap();

        let first = create_workspace_handler(
            State(state.clone()),
            Extension(current_user("user_1")),
            Json(CreateWorkspaceRequest {
                name: "feature-a".to_owned(),
                source_type: "git_project".to_owned(),
                git_project_id: Some(project.id.clone()),
                branch_ref: Some("feature/a".to_owned()),
            }),
        )
        .await
        .unwrap()
        .0
        .data
        .expect("workspace response data");
        let second = create_workspace_handler(
            State(state),
            Extension(current_user("user_1")),
            Json(CreateWorkspaceRequest {
                name: "feature-b".to_owned(),
                source_type: "git_project".to_owned(),
                git_project_id: Some(project.id.clone()),
                branch_ref: Some("feature/b".to_owned()),
            }),
        )
        .await
        .unwrap()
        .0
        .data
        .expect("workspace response data");

        assert_ne!(first.id, second.id);
        assert_ne!(first.root_rel_path, second.root_rel_path);
        assert_eq!(first.git_project_id.as_deref(), Some(project.id.as_str()));
        assert_eq!(second.git_project_id.as_deref(), Some(project.id.as_str()));
        assert_eq!(ops.clones.read().await.len(), 2);
        let logs = repos.audit_logs.read().await;
        assert!(logs.iter().any(|log| {
            log.target_type == "workspace"
                && log.action == "create_git_project"
                && log.metadata["git_project_id"] == project.id
        }));
        let _ = tokio::fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn workspace_owner_isolation_and_state_limits_are_enforced() {
        let repos = Arc::new(WorkbenchTestRepos::default());
        let root = std::env::temp_dir().join(aionui_common::generate_prefixed_id("workspace-isolation-test"));
        let mut state = workbench_test_state(repos.clone());
        state.senmo_root = root.clone();
        tokio::fs::create_dir_all(root.join("u1/ws1")).await.unwrap();
        tokio::fs::create_dir_all(root.join("u2/ws2")).await.unwrap();
        tokio::fs::write(root.join("u1/ws1/README.md"), "owned").await.unwrap();
        let user_workspace = repos
            .create_workspace(CreateWorkspaceParams {
                owner_user_id: "user_1".to_owned(),
                name: "owned".to_owned(),
                source_type: "blank".to_owned(),
                git_project_id: None,
                branch_ref: None,
                status: "active".to_owned(),
                root_rel_path: "u1/ws1".to_owned(),
            })
            .await
            .unwrap();
        let other_workspace = repos
            .create_workspace(CreateWorkspaceParams {
                owner_user_id: "user_2".to_owned(),
                name: "other".to_owned(),
                source_type: "blank".to_owned(),
                git_project_id: None,
                branch_ref: None,
                status: "active".to_owned(),
                root_rel_path: "u2/ws2".to_owned(),
            })
            .await
            .unwrap();
        let archived = repos
            .create_workspace(CreateWorkspaceParams {
                owner_user_id: "user_1".to_owned(),
                name: "archived".to_owned(),
                source_type: "blank".to_owned(),
                git_project_id: None,
                branch_ref: None,
                status: "archived".to_owned(),
                root_rel_path: "u1/archived".to_owned(),
            })
            .await
            .unwrap();

        let listed = list_workspaces_handler(State(state.clone()), Extension(current_user("user_1")))
            .await
            .unwrap()
            .0
            .data
            .expect("workspace list data");
        assert_eq!(listed.len(), 2);
        assert!(listed.iter().all(|workspace| workspace.id != other_workspace.id));

        let err = write_workspace_file_handler(
            State(state.clone()),
            Extension(current_user("user_1")),
            Path(other_workspace.id),
            Json(WorkspaceFileWriteRequest {
                relative_path: "x.txt".to_owned(),
                content: "nope".to_owned(),
            }),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, ApiError::Forbidden(_)));

        let err = create_execution_handler(
            State(state.clone()),
            Extension(current_user("user_1")),
            Path(archived.id),
            Json(CreateExecutionRequest {
                snapshot_id: None,
                execution_type: "test_run".to_owned(),
                policy: json!({}),
                resource_profile: json!({}),
                network_profile: None,
            }),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, ApiError::BadRequest(message) if message.contains("active workspaces")));

        let execution = create_execution_handler(
            State(state),
            Extension(current_user("user_1")),
            Path(user_workspace.id.clone()),
            Json(CreateExecutionRequest {
                snapshot_id: None,
                execution_type: "test_run".to_owned(),
                policy: json!({}),
                resource_profile: json!({}),
                network_profile: None,
            }),
        )
        .await
        .unwrap()
        .0
        .data
        .expect("execution response data");
        assert_eq!(execution.workspace_id, user_workspace.id);
        assert_eq!(execution.execution_type, "test_run");
        assert_eq!(execution.status, "provisioning");
        assert!(execution.preview_url.is_none());
        assert!(execution.started_at.is_some());
        let logs = repos.audit_logs.read().await;
        assert!(logs.iter().any(|log| {
            log.target_type == "workspace"
                && log.target_id.as_deref() == Some(&user_workspace.id)
                && log.action == "runtime_start"
                && log.metadata["execution_id"] == execution.id
        }));
        let _ = tokio::fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn workspace_watch_terminal_and_git_contexts_use_relative_paths_without_leaking_root() {
        let repos = Arc::new(WorkbenchTestRepos::default());
        let root = std::env::temp_dir().join(aionui_common::generate_prefixed_id("workspace-runtime-context-test"));
        let mut state = workbench_test_state(repos.clone());
        state.senmo_root = root.clone();
        tokio::fs::create_dir_all(root.join("u1/ws1/src")).await.unwrap();
        tokio::fs::write(root.join("u1/ws1/src/main.rs"), "fn main() {}")
            .await
            .unwrap();
        let workspace = repos
            .create_workspace(CreateWorkspaceParams {
                owner_user_id: "user_1".to_owned(),
                name: "owned".to_owned(),
                source_type: "blank".to_owned(),
                git_project_id: None,
                branch_ref: None,
                status: "active".to_owned(),
                root_rel_path: "u1/ws1".to_owned(),
            })
            .await
            .unwrap();

        let payload = workspace_file_watch_payload_handler(
            State(state.clone()),
            Extension(current_user("user_1")),
            Path(workspace.id.clone()),
            Json(WorkspaceFilePathRequest {
                relative_path: "src/main.rs".to_owned(),
            }),
        )
        .await
        .unwrap()
        .0
        .data
        .expect("watch payload data");
        let payload_text = payload.to_string();
        assert_eq!(payload["content_update"]["workspace_id"], workspace.id);
        assert_eq!(payload["content_update"]["relative_path"], "src/main.rs");
        assert_eq!(payload["file_watch"]["workspace_id"], workspace.id);
        assert_eq!(payload["file_watch"]["relative_path"], "src/main.rs");
        assert!(!payload_text.contains(root.to_string_lossy().as_ref()));
        assert!(!payload_text.contains("/u1/ws1"));

        let terminal_context = workspace_terminal_context_handler(
            State(state.clone()),
            Extension(current_user("user_1")),
            Path(workspace.id.clone()),
            Json(WorkspaceFilePathRequest {
                relative_path: "src".to_owned(),
            }),
        )
        .await
        .unwrap()
        .0
        .data
        .expect("terminal context data");
        assert_eq!(terminal_context.workspace_id, workspace.id);
        assert_eq!(terminal_context.relative_path, "src");
        assert_eq!(terminal_context.display_path, format!("workspace:{}/src", workspace.id));
        assert!(
            !serde_json::to_string(&terminal_context)
                .unwrap()
                .contains(root.to_string_lossy().as_ref())
        );

        let git_context = resolve_workspace_runtime_context(&state, "user_1", &workspace.id, "")
            .await
            .unwrap();
        assert!(git_context.cwd.starts_with(root.canonicalize().unwrap()));
        assert_eq!(git_context.response.relative_path, "");
        assert!(
            !serde_json::to_string(&git_context.response)
                .unwrap()
                .contains(root.to_string_lossy().as_ref())
        );

        let err = workspace_git_context_handler(
            State(state.clone()),
            Extension(current_user("user_1")),
            Path(workspace.id.clone()),
            Json(WorkspaceFilePathRequest {
                relative_path: "src/main.rs".to_owned(),
            }),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, ApiError::BadRequest(message) if message.contains("Runtime cwd")));

        let err = workspace_terminal_context_handler(
            State(state),
            Extension(current_user("user_2")),
            Path(workspace.id),
            Json(WorkspaceFilePathRequest {
                relative_path: "src".to_owned(),
            }),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, ApiError::Forbidden(_)));
        let _ = tokio::fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn workspace_rejects_git_project_owned_by_another_user() {
        let repos = Arc::new(WorkbenchTestRepos::default());
        let root = std::env::temp_dir().join(aionui_common::generate_prefixed_id("workspace-cross-project-test"));
        let mut state = workbench_test_state(repos.clone());
        state.senmo_root = root.clone();
        let project = repos
            .create_project(CreateGitProjectParams {
                owner_user_id: "user_2".to_owned(),
                credential_id: "sshcred_2".to_owned(),
                repo_ssh_url: "git@example.com:team/other.git".to_owned(),
                display_name: "other".to_owned(),
                default_branch: Some("main".to_owned()),
                status: "active".to_owned(),
            })
            .await
            .unwrap();

        let err = create_workspace_handler(
            State(state),
            Extension(current_user("user_1")),
            Json(CreateWorkspaceRequest {
                name: "bad".to_owned(),
                source_type: "git_project".to_owned(),
                git_project_id: Some(project.id),
                branch_ref: Some("main".to_owned()),
            }),
        )
        .await
        .unwrap_err();

        assert!(matches!(err, ApiError::Forbidden(message) if message.contains("Git project access denied")));
        assert!(repos.workspaces.read().await.is_empty());
        let _ = tokio::fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn preview_env_uses_explicit_snapshot_without_live_workspace_sync() {
        let repos = Arc::new(WorkbenchTestRepos::default());
        let root = std::env::temp_dir().join(aionui_common::generate_prefixed_id("preview-snapshot-test"));
        let mut state = workbench_test_state(repos.clone());
        state.senmo_root = root.clone();
        tokio::fs::create_dir_all(root.join("u1/ws1")).await.unwrap();
        tokio::fs::write(root.join("u1/ws1/app.txt"), "before").await.unwrap();
        let workspace = repos
            .create_workspace(CreateWorkspaceParams {
                owner_user_id: "user_1".to_owned(),
                name: "preview".to_owned(),
                source_type: "blank".to_owned(),
                git_project_id: None,
                branch_ref: None,
                status: "active".to_owned(),
                root_rel_path: "u1/ws1".to_owned(),
            })
            .await
            .unwrap();
        let snapshot = create_snapshot_for_workspace(&state, "user_1", workspace.clone(), json!({}), json!({}))
            .await
            .unwrap();
        tokio::fs::write(root.join("u1/ws1/app.txt"), "after").await.unwrap();

        let execution = create_execution_handler(
            State(state.clone()),
            Extension(current_user("user_1")),
            Path(workspace.id.clone()),
            Json(CreateExecutionRequest {
                snapshot_id: Some(snapshot.id.clone()),
                execution_type: "preview_env".to_owned(),
                policy: json!({}),
                resource_profile: json!({}),
                network_profile: None,
            }),
        )
        .await
        .unwrap()
        .0
        .data
        .expect("execution response data");

        assert_eq!(execution.execution_type, "preview_env");
        assert_eq!(execution.status, "provisioning");
        assert_eq!(execution.snapshot_id, snapshot.id);
        assert_eq!(
            execution.preview_url.as_deref(),
            Some(format!("/api/executions/{}/preview", execution.id).as_str())
        );
        assert_eq!(execution.policy["preview"]["ttl_seconds"], 7200);
        assert!(execution.started_at.is_some());
        assert_eq!(repos.snapshots.read().await.len(), 1);
        let snapshot_content = tokio::fs::read_to_string(root.join(&snapshot.artifact_ref).join("app.txt"))
            .await
            .unwrap();
        assert_eq!(snapshot_content, "before");

        let redeployed = create_execution_handler(
            State(state.clone()),
            Extension(current_user("user_1")),
            Path(workspace.id),
            Json(CreateExecutionRequest {
                snapshot_id: None,
                execution_type: "preview_env".to_owned(),
                policy: json!({ "preview": { "ttl_seconds": 300 } }),
                resource_profile: json!({}),
                network_profile: None,
            }),
        )
        .await
        .unwrap()
        .0
        .data
        .expect("redeploy response data");
        assert_ne!(redeployed.id, execution.id);
        assert_ne!(redeployed.snapshot_id, execution.snapshot_id);
        assert_eq!(redeployed.policy["preview"]["ttl_seconds"], 300);
        assert!(redeployed.preview_url.is_some());
        assert_eq!(repos.snapshots.read().await.len(), 2);
        let redeploy_snapshot = ISnapshotRepository::find_by_id(&*repos, &redeployed.snapshot_id)
            .await
            .unwrap()
            .expect("redeploy snapshot");
        let redeploy_content = tokio::fs::read_to_string(root.join(&redeploy_snapshot.artifact_ref).join("app.txt"))
            .await
            .unwrap();
        assert_eq!(redeploy_content, "after");

        let cancelled = cancel_execution_handler(
            State(state),
            Extension(current_user("user_1")),
            Path(execution.id.clone()),
        )
        .await
        .unwrap()
        .0
        .data
        .expect("cancel response data");
        assert_eq!(cancelled.status, "cancelled");
        assert!(cancelled.preview_url.is_none());
        assert!(cancelled.finished_at.is_some());
        let _ = tokio::fs::remove_dir_all(root).await;
    }
}
