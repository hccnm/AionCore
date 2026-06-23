use aionui_common::TimestampMs;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct PlatformUserRow {
    pub id: String,
    pub phone: Option<String>,
    pub username: Option<String>,
    pub display_name: Option<String>,
    pub email: Option<String>,
    pub password_hash: Option<String>,
    pub avatar_path: Option<String>,
    pub status: String,
    pub jwt_secret: Option<String>,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
    pub last_login: Option<TimestampMs>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreatePlatformUserParams {
    pub phone: Option<String>,
    pub username: Option<String>,
    pub display_name: Option<String>,
    pub email: Option<String>,
    pub password_hash: Option<String>,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpsertExternalIdentityParams {
    pub provider: String,
    pub app_id: String,
    pub external_user_id: String,
    pub user_id: String,
    pub phone_snapshot: Option<String>,
    pub external_role_ids: Value,
    pub is_admin: bool,
    pub raw_payload: Value,
    pub last_synced_at: TimestampMs,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ExternalIdentityRow {
    pub id: String,
    pub user_id: String,
    pub provider: String,
    pub app_id: String,
    pub external_user_id: String,
    pub phone_snapshot: Option<String>,
    pub external_role_ids: Value,
    pub is_admin: bool,
    pub raw_payload: Value,
    pub last_synced_at: TimestampMs,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct RoleRow {
    pub id: String,
    pub role_key: String,
    pub role_name: String,
    pub status: String,
    pub permissions: Value,
    pub sort_order: i32,
    pub is_system: bool,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpsertRoleParams {
    pub role_key: String,
    pub role_name: String,
    pub status: String,
    pub permissions: Value,
    pub sort_order: i32,
    pub is_system: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct GitSshCredentialRow {
    pub id: String,
    pub owner_user_id: String,
    pub name: String,
    pub fingerprint: String,
    pub encrypted_private_key: String,
    pub encrypted_passphrase: Option<String>,
    pub public_key: Option<String>,
    pub status: String,
    pub last_verified_at: Option<TimestampMs>,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateGitSshCredentialParams {
    pub owner_user_id: String,
    pub name: String,
    pub fingerprint: String,
    pub encrypted_private_key: String,
    pub encrypted_passphrase: Option<String>,
    pub public_key: Option<String>,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct GitProjectRow {
    pub id: String,
    pub owner_user_id: String,
    pub credential_id: String,
    pub repo_ssh_url: String,
    pub display_name: String,
    pub default_branch: Option<String>,
    pub status: String,
    pub last_verified_at: Option<TimestampMs>,
    pub last_synced_at: Option<TimestampMs>,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateGitProjectParams {
    pub owner_user_id: String,
    pub credential_id: String,
    pub repo_ssh_url: String,
    pub display_name: String,
    pub default_branch: Option<String>,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct WorkspaceRow {
    pub id: String,
    pub owner_user_id: String,
    pub name: String,
    pub source_type: String,
    pub git_project_id: Option<String>,
    pub branch_ref: Option<String>,
    pub status: String,
    pub root_rel_path: String,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
    pub deleted_at: Option<TimestampMs>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateWorkspaceParams {
    pub owner_user_id: String,
    pub name: String,
    pub source_type: String,
    pub git_project_id: Option<String>,
    pub branch_ref: Option<String>,
    pub status: String,
    pub root_rel_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct SnapshotRow {
    pub id: String,
    pub workspace_id: String,
    pub created_by: String,
    pub source_kind: String,
    pub artifact_ref: String,
    pub manifest_ref: String,
    pub checksum: String,
    pub size_bytes: i64,
    pub base_git_metadata: Value,
    pub environment_metadata: Value,
    pub retention_policy: Value,
    pub sealed_at: TimestampMs,
    pub expires_at: Option<TimestampMs>,
    pub gc_tombstone_at: Option<TimestampMs>,
    pub created_at: TimestampMs,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSnapshotParams {
    pub workspace_id: String,
    pub created_by: String,
    pub source_kind: String,
    pub artifact_ref: String,
    pub manifest_ref: String,
    pub checksum: String,
    pub size_bytes: i64,
    pub base_git_metadata: Value,
    pub environment_metadata: Value,
    pub retention_policy: Value,
    pub sealed_at: TimestampMs,
    pub expires_at: Option<TimestampMs>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ExecutionRunRow {
    pub id: String,
    pub workspace_id: String,
    pub snapshot_id: String,
    pub owner_user_id: String,
    pub execution_type: String,
    pub status: String,
    pub policy: Value,
    pub resource_profile: Value,
    pub network_profile: Option<String>,
    pub trace_id: String,
    pub preview_url: Option<String>,
    pub created_at: TimestampMs,
    pub started_at: Option<TimestampMs>,
    pub finished_at: Option<TimestampMs>,
    pub updated_at: TimestampMs,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateExecutionRunParams {
    pub workspace_id: String,
    pub snapshot_id: String,
    pub owner_user_id: String,
    pub execution_type: String,
    pub status: String,
    pub policy: Value,
    pub resource_profile: Value,
    pub network_profile: Option<String>,
    pub trace_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ExecutionArtifactRow {
    pub id: String,
    pub execution_id: String,
    pub artifact_type: String,
    pub ref_: String,
    pub metadata: Value,
    pub immutable: bool,
    pub created_at: TimestampMs,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateExecutionArtifactParams {
    pub execution_id: String,
    pub artifact_type: String,
    pub ref_: String,
    pub metadata: Value,
    pub immutable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct AuditLogRow {
    pub id: String,
    pub actor_user_id: Option<String>,
    pub target_type: String,
    pub target_id: Option<String>,
    pub action: String,
    pub result: String,
    pub metadata: Value,
    pub trace_id: Option<String>,
    pub created_at: TimestampMs,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateAuditLogParams {
    pub actor_user_id: Option<String>,
    pub target_type: String,
    pub target_id: Option<String>,
    pub action: String,
    pub result: String,
    pub metadata: Value,
    pub trace_id: Option<String>,
}
