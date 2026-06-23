use crate::error::DbError;
use crate::models::{
    AuditLogRow, CreateAuditLogParams, CreateExecutionArtifactParams, CreateExecutionRunParams, CreateGitProjectParams,
    CreateGitSshCredentialParams, CreatePlatformUserParams, CreateSnapshotParams, CreateWorkspaceParams,
    ExecutionArtifactRow, ExecutionRunRow, ExternalIdentityRow, GitProjectRow, GitSshCredentialRow, PlatformUserRow,
    RoleRow, SnapshotRow, UpsertExternalIdentityParams, UpsertRoleParams, WorkspaceRow,
};

#[async_trait::async_trait]
pub trait IPlatformUserRepository: Send + Sync {
    async fn create_user(&self, params: CreatePlatformUserParams) -> Result<PlatformUserRow, DbError>;
    async fn find_by_id(&self, user_id: &str) -> Result<Option<PlatformUserRow>, DbError>;
    async fn find_by_phone(&self, phone: &str) -> Result<Option<PlatformUserRow>, DbError>;
    async fn update_from_sync(
        &self,
        user_id: &str,
        phone: Option<&str>,
        display_name: Option<&str>,
        email: Option<&str>,
        status: &str,
    ) -> Result<PlatformUserRow, DbError>;
    async fn update_status(&self, user_id: &str, status: &str) -> Result<(), DbError>;
    async fn update_password_hash(&self, user_id: &str, password_hash: Option<&str>) -> Result<(), DbError>;
    async fn update_last_login(&self, user_id: &str) -> Result<(), DbError>;
    async fn list_users(&self, limit: i64, offset: i64) -> Result<Vec<PlatformUserRow>, DbError>;
}

#[async_trait::async_trait]
pub trait IExternalIdentityRepository: Send + Sync {
    async fn upsert_identity(&self, params: UpsertExternalIdentityParams) -> Result<ExternalIdentityRow, DbError>;
    async fn find_identity(
        &self,
        provider: &str,
        app_id: &str,
        external_user_id: &str,
    ) -> Result<Option<ExternalIdentityRow>, DbError>;
    async fn list_by_user(&self, user_id: &str) -> Result<Vec<ExternalIdentityRow>, DbError>;
}

#[async_trait::async_trait]
pub trait IRoleRepository: Send + Sync {
    async fn upsert_role(&self, params: UpsertRoleParams) -> Result<RoleRow, DbError>;
    async fn list_roles(&self) -> Result<Vec<RoleRow>, DbError>;
    async fn set_role_status(&self, role_id: &str, status: &str) -> Result<(), DbError>;
    async fn assign_role(&self, user_id: &str, role_id: &str) -> Result<(), DbError>;
    async fn revoke_role(&self, user_id: &str, role_id: &str) -> Result<(), DbError>;
    async fn list_user_roles(&self, user_id: &str) -> Result<Vec<RoleRow>, DbError>;
}

#[async_trait::async_trait]
pub trait IGitSshCredentialRepository: Send + Sync {
    async fn create_credential(&self, params: CreateGitSshCredentialParams) -> Result<GitSshCredentialRow, DbError>;
    async fn find_by_id(&self, credential_id: &str) -> Result<Option<GitSshCredentialRow>, DbError>;
    async fn list_by_owner(&self, owner_user_id: &str) -> Result<Vec<GitSshCredentialRow>, DbError>;
    async fn update_status(
        &self,
        credential_id: &str,
        status: &str,
        last_verified_at: Option<i64>,
    ) -> Result<(), DbError>;
}

#[async_trait::async_trait]
pub trait IGitProjectRepository: Send + Sync {
    async fn create_project(&self, params: CreateGitProjectParams) -> Result<GitProjectRow, DbError>;
    async fn find_by_id(&self, project_id: &str) -> Result<Option<GitProjectRow>, DbError>;
    async fn list_by_owner(&self, owner_user_id: &str) -> Result<Vec<GitProjectRow>, DbError>;
    async fn update_project(
        &self,
        project_id: &str,
        credential_id: &str,
        repo_ssh_url: &str,
        display_name: &str,
        default_branch: Option<&str>,
    ) -> Result<GitProjectRow, DbError>;
    async fn update_status(
        &self,
        project_id: &str,
        status: &str,
        last_verified_at: Option<i64>,
        last_synced_at: Option<i64>,
    ) -> Result<(), DbError>;
}

#[async_trait::async_trait]
pub trait IWorkspaceRepository: Send + Sync {
    async fn create_workspace(&self, params: CreateWorkspaceParams) -> Result<WorkspaceRow, DbError>;
    async fn find_by_id(&self, workspace_id: &str) -> Result<Option<WorkspaceRow>, DbError>;
    async fn list_by_owner(&self, owner_user_id: &str) -> Result<Vec<WorkspaceRow>, DbError>;
    async fn has_active_by_git_project(&self, git_project_id: &str) -> Result<bool, DbError>;
    async fn update_name(&self, workspace_id: &str, name: &str) -> Result<(), DbError>;
    async fn update_status(&self, workspace_id: &str, status: &str) -> Result<(), DbError>;
    async fn mark_deleted(&self, workspace_id: &str) -> Result<(), DbError>;
}

#[async_trait::async_trait]
pub trait ISnapshotRepository: Send + Sync {
    async fn create_snapshot(&self, params: CreateSnapshotParams) -> Result<SnapshotRow, DbError>;
    async fn find_by_id(&self, snapshot_id: &str) -> Result<Option<SnapshotRow>, DbError>;
    async fn list_by_workspace(&self, workspace_id: &str) -> Result<Vec<SnapshotRow>, DbError>;
    async fn list_gc_candidates(&self, now_ms: i64, limit: i64) -> Result<Vec<SnapshotRow>, DbError>;
    async fn mark_gc_tombstone(&self, snapshot_id: &str, gc_tombstone_at: i64) -> Result<(), DbError>;
}

#[async_trait::async_trait]
pub trait IExecutionRunRepository: Send + Sync {
    async fn create_execution_run(&self, params: CreateExecutionRunParams) -> Result<ExecutionRunRow, DbError>;
    async fn find_by_id(&self, execution_id: &str) -> Result<Option<ExecutionRunRow>, DbError>;
    async fn list_by_workspace(&self, workspace_id: &str) -> Result<Vec<ExecutionRunRow>, DbError>;
    async fn has_active_by_snapshot(&self, snapshot_id: &str) -> Result<bool, DbError>;
    async fn update_status(
        &self,
        execution_id: &str,
        status: &str,
        started_at: Option<i64>,
        finished_at: Option<i64>,
    ) -> Result<(), DbError>;
    async fn set_preview_url(&self, execution_id: &str, preview_url: Option<&str>) -> Result<(), DbError>;
}

#[async_trait::async_trait]
pub trait IExecutionArtifactRepository: Send + Sync {
    async fn create_artifact(&self, params: CreateExecutionArtifactParams) -> Result<ExecutionArtifactRow, DbError>;
    async fn list_by_execution(&self, execution_id: &str) -> Result<Vec<ExecutionArtifactRow>, DbError>;
}

#[async_trait::async_trait]
pub trait IAuditLogRepository: Send + Sync {
    async fn create_log(&self, params: CreateAuditLogParams) -> Result<AuditLogRow, DbError>;
    async fn list_by_target(&self, target_type: &str, target_id: &str) -> Result<Vec<AuditLogRow>, DbError>;
}
