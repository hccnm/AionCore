use sqlx::PgPool;

use crate::error::DbError;
use crate::models::{
    AuditLogRow, CreateAuditLogParams, CreateExecutionArtifactParams, CreateExecutionRunParams, CreateGitProjectParams,
    CreateGitSshCredentialParams, CreatePlatformUserParams, CreateSnapshotParams, CreateWorkspaceParams,
    ExecutionArtifactRow, ExecutionRunRow, ExternalIdentityRow, GitProjectRow, GitSshCredentialRow, PlatformUserRow,
    RoleRow, SnapshotRow, UpsertExternalIdentityParams, UpsertRoleParams, WorkspaceRow,
};
use crate::repository::{
    IAuditLogRepository, IExecutionArtifactRepository, IExecutionRunRepository, IExternalIdentityRepository,
    IGitProjectRepository, IGitSshCredentialRepository, IPlatformUserRepository, IRoleRepository, ISnapshotRepository,
    IWorkspaceRepository,
};

#[derive(Clone, Debug)]
pub struct PgPlatformUserRepository {
    pool: PgPool,
}

impl PgPlatformUserRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl IPlatformUserRepository for PgPlatformUserRepository {
    async fn create_user(&self, params: CreatePlatformUserParams) -> Result<PlatformUserRow, DbError> {
        let id = aionui_common::generate_prefixed_id("user");
        let now = aionui_common::now_ms();

        sqlx::query_as::<_, PlatformUserRow>(
            "INSERT INTO users (
                id, phone, username, display_name, email, password_hash, status, created_at, updated_at
             ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
             RETURNING *",
        )
        .bind(id)
        .bind(params.phone)
        .bind(params.username)
        .bind(params.display_name)
        .bind(params.email)
        .bind(params.password_hash)
        .bind(params.status)
        .bind(now)
        .bind(now)
        .fetch_one(&self.pool)
        .await
        .map_err(map_pg_conflict("user already exists"))
    }

    async fn find_by_id(&self, user_id: &str) -> Result<Option<PlatformUserRow>, DbError> {
        Ok(
            sqlx::query_as::<_, PlatformUserRow>("SELECT * FROM users WHERE id = $1")
                .bind(user_id)
                .fetch_optional(&self.pool)
                .await?,
        )
    }

    async fn find_by_phone(&self, phone: &str) -> Result<Option<PlatformUserRow>, DbError> {
        Ok(
            sqlx::query_as::<_, PlatformUserRow>("SELECT * FROM users WHERE phone = $1")
                .bind(phone)
                .fetch_optional(&self.pool)
                .await?,
        )
    }

    async fn update_from_sync(
        &self,
        user_id: &str,
        phone: Option<&str>,
        display_name: Option<&str>,
        email: Option<&str>,
        status: &str,
    ) -> Result<PlatformUserRow, DbError> {
        sqlx::query_as::<_, PlatformUserRow>(
            "UPDATE users
             SET phone = $1,
                 display_name = $2,
                 email = $3,
                 status = $4,
                 updated_at = $5
             WHERE id = $6
             RETURNING *",
        )
        .bind(phone)
        .bind(display_name)
        .bind(email)
        .bind(status)
        .bind(aionui_common::now_ms())
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_pg_conflict("user already exists"))?
        .ok_or_else(|| DbError::NotFound(format!("User '{user_id}' not found")))
    }

    async fn update_status(&self, user_id: &str, status: &str) -> Result<(), DbError> {
        execute_required(
            sqlx::query("UPDATE users SET status = $1, updated_at = $2 WHERE id = $3")
                .bind(status)
                .bind(aionui_common::now_ms())
                .bind(user_id)
                .execute(&self.pool)
                .await?,
            format!("User '{user_id}' not found"),
        )
    }

    async fn update_password_hash(&self, user_id: &str, password_hash: Option<&str>) -> Result<(), DbError> {
        execute_required(
            sqlx::query("UPDATE users SET password_hash = $1, updated_at = $2 WHERE id = $3")
                .bind(password_hash)
                .bind(aionui_common::now_ms())
                .bind(user_id)
                .execute(&self.pool)
                .await?,
            format!("User '{user_id}' not found"),
        )
    }

    async fn update_last_login(&self, user_id: &str) -> Result<(), DbError> {
        let now = aionui_common::now_ms();
        execute_required(
            sqlx::query("UPDATE users SET last_login = $1, updated_at = $1 WHERE id = $2")
                .bind(now)
                .bind(user_id)
                .execute(&self.pool)
                .await?,
            format!("User '{user_id}' not found"),
        )
    }

    async fn list_users(&self, limit: i64, offset: i64) -> Result<Vec<PlatformUserRow>, DbError> {
        Ok(
            sqlx::query_as::<_, PlatformUserRow>("SELECT * FROM users ORDER BY created_at DESC LIMIT $1 OFFSET $2")
                .bind(limit)
                .bind(offset)
                .fetch_all(&self.pool)
                .await?,
        )
    }
}

#[derive(Clone, Debug)]
pub struct PgExternalIdentityRepository {
    pool: PgPool,
}

impl PgExternalIdentityRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl IExternalIdentityRepository for PgExternalIdentityRepository {
    async fn upsert_identity(&self, params: UpsertExternalIdentityParams) -> Result<ExternalIdentityRow, DbError> {
        let id = aionui_common::generate_prefixed_id("extid");
        let now = aionui_common::now_ms();

        Ok(sqlx::query_as::<_, ExternalIdentityRow>(
            "INSERT INTO external_identities (
                id, user_id, provider, app_id, external_user_id, phone_snapshot,
                external_role_ids, is_admin, raw_payload, last_synced_at, created_at, updated_at
             ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
             ON CONFLICT (provider, app_id, external_user_id)
             DO UPDATE SET
                user_id = EXCLUDED.user_id,
                phone_snapshot = EXCLUDED.phone_snapshot,
                external_role_ids = EXCLUDED.external_role_ids,
                is_admin = EXCLUDED.is_admin,
                raw_payload = EXCLUDED.raw_payload,
                last_synced_at = EXCLUDED.last_synced_at,
                updated_at = EXCLUDED.updated_at
             RETURNING *",
        )
        .bind(id)
        .bind(params.user_id)
        .bind(params.provider)
        .bind(params.app_id)
        .bind(params.external_user_id)
        .bind(params.phone_snapshot)
        .bind(params.external_role_ids)
        .bind(params.is_admin)
        .bind(params.raw_payload)
        .bind(params.last_synced_at)
        .bind(now)
        .bind(now)
        .fetch_one(&self.pool)
        .await?)
    }

    async fn find_identity(
        &self,
        provider: &str,
        app_id: &str,
        external_user_id: &str,
    ) -> Result<Option<ExternalIdentityRow>, DbError> {
        Ok(sqlx::query_as::<_, ExternalIdentityRow>(
            "SELECT * FROM external_identities
             WHERE provider = $1 AND app_id = $2 AND external_user_id = $3",
        )
        .bind(provider)
        .bind(app_id)
        .bind(external_user_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    async fn list_by_user(&self, user_id: &str) -> Result<Vec<ExternalIdentityRow>, DbError> {
        Ok(sqlx::query_as::<_, ExternalIdentityRow>(
            "SELECT * FROM external_identities WHERE user_id = $1 ORDER BY updated_at DESC",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?)
    }
}

#[derive(Clone, Debug)]
pub struct PgRoleRepository {
    pool: PgPool,
}

impl PgRoleRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl IRoleRepository for PgRoleRepository {
    async fn upsert_role(&self, params: UpsertRoleParams) -> Result<RoleRow, DbError> {
        let id = aionui_common::generate_prefixed_id("role");
        let now = aionui_common::now_ms();

        Ok(sqlx::query_as::<_, RoleRow>(
            "INSERT INTO roles (
                id, role_key, role_name, status, permissions, sort_order, is_system, created_at, updated_at
             ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
             ON CONFLICT (role_key)
             DO UPDATE SET
                role_name = EXCLUDED.role_name,
                status = EXCLUDED.status,
                permissions = EXCLUDED.permissions,
                sort_order = EXCLUDED.sort_order,
                is_system = EXCLUDED.is_system,
                updated_at = EXCLUDED.updated_at
             RETURNING *",
        )
        .bind(id)
        .bind(params.role_key)
        .bind(params.role_name)
        .bind(params.status)
        .bind(params.permissions)
        .bind(params.sort_order)
        .bind(params.is_system)
        .bind(now)
        .bind(now)
        .fetch_one(&self.pool)
        .await?)
    }

    async fn list_roles(&self) -> Result<Vec<RoleRow>, DbError> {
        Ok(
            sqlx::query_as::<_, RoleRow>("SELECT * FROM roles ORDER BY sort_order ASC, created_at ASC")
                .fetch_all(&self.pool)
                .await?,
        )
    }

    async fn set_role_status(&self, role_id: &str, status: &str) -> Result<(), DbError> {
        execute_required(
            sqlx::query("UPDATE roles SET status = $1, updated_at = $2 WHERE id = $3")
                .bind(status)
                .bind(aionui_common::now_ms())
                .bind(role_id)
                .execute(&self.pool)
                .await?,
            format!("Role '{role_id}' not found"),
        )
    }

    async fn assign_role(&self, user_id: &str, role_id: &str) -> Result<(), DbError> {
        sqlx::query(
            "INSERT INTO user_roles (user_id, role_id, created_at)
             VALUES ($1, $2, $3)
             ON CONFLICT (user_id, role_id) DO NOTHING",
        )
        .bind(user_id)
        .bind(role_id)
        .bind(aionui_common::now_ms())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn revoke_role(&self, user_id: &str, role_id: &str) -> Result<(), DbError> {
        sqlx::query("DELETE FROM user_roles WHERE user_id = $1 AND role_id = $2")
            .bind(user_id)
            .bind(role_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn list_user_roles(&self, user_id: &str) -> Result<Vec<RoleRow>, DbError> {
        Ok(sqlx::query_as::<_, RoleRow>(
            "SELECT r.*
             FROM roles r
             JOIN user_roles ur ON ur.role_id = r.id
             WHERE ur.user_id = $1
             ORDER BY r.sort_order ASC, r.created_at ASC",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?)
    }
}

#[derive(Clone, Debug)]
pub struct PgGitSshCredentialRepository {
    pool: PgPool,
}

impl PgGitSshCredentialRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl IGitSshCredentialRepository for PgGitSshCredentialRepository {
    async fn create_credential(&self, params: CreateGitSshCredentialParams) -> Result<GitSshCredentialRow, DbError> {
        let id = aionui_common::generate_prefixed_id("sshcred");
        let now = aionui_common::now_ms();

        Ok(sqlx::query_as::<_, GitSshCredentialRow>(
            "INSERT INTO git_ssh_credentials (
                id, owner_user_id, name, fingerprint, encrypted_private_key,
                encrypted_passphrase, public_key, status, created_at, updated_at
             ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
             RETURNING *",
        )
        .bind(id)
        .bind(params.owner_user_id)
        .bind(params.name)
        .bind(params.fingerprint)
        .bind(params.encrypted_private_key)
        .bind(params.encrypted_passphrase)
        .bind(params.public_key)
        .bind(params.status)
        .bind(now)
        .bind(now)
        .fetch_one(&self.pool)
        .await?)
    }

    async fn find_by_id(&self, credential_id: &str) -> Result<Option<GitSshCredentialRow>, DbError> {
        Ok(
            sqlx::query_as::<_, GitSshCredentialRow>("SELECT * FROM git_ssh_credentials WHERE id = $1")
                .bind(credential_id)
                .fetch_optional(&self.pool)
                .await?,
        )
    }

    async fn list_by_owner(&self, owner_user_id: &str) -> Result<Vec<GitSshCredentialRow>, DbError> {
        Ok(sqlx::query_as::<_, GitSshCredentialRow>(
            "SELECT * FROM git_ssh_credentials WHERE owner_user_id = $1 ORDER BY created_at DESC",
        )
        .bind(owner_user_id)
        .fetch_all(&self.pool)
        .await?)
    }

    async fn update_status(
        &self,
        credential_id: &str,
        status: &str,
        last_verified_at: Option<i64>,
    ) -> Result<(), DbError> {
        execute_required(
            sqlx::query(
                "UPDATE git_ssh_credentials
                 SET status = $1, last_verified_at = $2, updated_at = $3
                 WHERE id = $4",
            )
            .bind(status)
            .bind(last_verified_at)
            .bind(aionui_common::now_ms())
            .bind(credential_id)
            .execute(&self.pool)
            .await?,
            format!("SSH credential '{credential_id}' not found"),
        )
    }
}

#[derive(Clone, Debug)]
pub struct PgGitProjectRepository {
    pool: PgPool,
}

impl PgGitProjectRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl IGitProjectRepository for PgGitProjectRepository {
    async fn create_project(&self, params: CreateGitProjectParams) -> Result<GitProjectRow, DbError> {
        let id = aionui_common::generate_prefixed_id("gitproj");
        let now = aionui_common::now_ms();

        sqlx::query_as::<_, GitProjectRow>(
            "INSERT INTO git_projects (
                id, owner_user_id, credential_id, repo_ssh_url, display_name,
                default_branch, status, created_at, updated_at
             ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
             RETURNING *",
        )
        .bind(id)
        .bind(params.owner_user_id)
        .bind(params.credential_id)
        .bind(params.repo_ssh_url)
        .bind(params.display_name)
        .bind(params.default_branch)
        .bind(params.status)
        .bind(now)
        .bind(now)
        .fetch_one(&self.pool)
        .await
        .map_err(map_pg_conflict("git project already exists"))
    }

    async fn find_by_id(&self, project_id: &str) -> Result<Option<GitProjectRow>, DbError> {
        Ok(
            sqlx::query_as::<_, GitProjectRow>("SELECT * FROM git_projects WHERE id = $1")
                .bind(project_id)
                .fetch_optional(&self.pool)
                .await?,
        )
    }

    async fn list_by_owner(&self, owner_user_id: &str) -> Result<Vec<GitProjectRow>, DbError> {
        Ok(sqlx::query_as::<_, GitProjectRow>(
            "SELECT * FROM git_projects
             WHERE owner_user_id = $1 AND status != 'deleted'
             ORDER BY created_at DESC",
        )
        .bind(owner_user_id)
        .fetch_all(&self.pool)
        .await?)
    }

    async fn update_project(
        &self,
        project_id: &str,
        credential_id: &str,
        repo_ssh_url: &str,
        display_name: &str,
        default_branch: Option<&str>,
    ) -> Result<GitProjectRow, DbError> {
        sqlx::query_as::<_, GitProjectRow>(
            "UPDATE git_projects
             SET credential_id = $1,
                 repo_ssh_url = $2,
                 display_name = $3,
                 default_branch = $4,
                 updated_at = $5
             WHERE id = $6
             RETURNING *",
        )
        .bind(credential_id)
        .bind(repo_ssh_url)
        .bind(display_name)
        .bind(default_branch)
        .bind(aionui_common::now_ms())
        .bind(project_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_pg_conflict("git project already exists"))?
        .ok_or_else(|| DbError::NotFound(format!("Git project '{project_id}' not found")))
    }

    async fn update_status(
        &self,
        project_id: &str,
        status: &str,
        last_verified_at: Option<i64>,
        last_synced_at: Option<i64>,
    ) -> Result<(), DbError> {
        execute_required(
            sqlx::query(
                "UPDATE git_projects
                 SET status = $1, last_verified_at = $2, last_synced_at = $3, updated_at = $4
                 WHERE id = $5",
            )
            .bind(status)
            .bind(last_verified_at)
            .bind(last_synced_at)
            .bind(aionui_common::now_ms())
            .bind(project_id)
            .execute(&self.pool)
            .await?,
            format!("Git project '{project_id}' not found"),
        )
    }
}

#[derive(Clone, Debug)]
pub struct PgWorkspaceRepository {
    pool: PgPool,
}

impl PgWorkspaceRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl IWorkspaceRepository for PgWorkspaceRepository {
    async fn create_workspace(&self, params: CreateWorkspaceParams) -> Result<WorkspaceRow, DbError> {
        let id = aionui_common::generate_prefixed_id("ws");
        let now = aionui_common::now_ms();

        sqlx::query_as::<_, WorkspaceRow>(
            "INSERT INTO workspaces (
                id, owner_user_id, name, source_type, git_project_id, branch_ref,
                status, root_rel_path, created_at, updated_at
             ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
             RETURNING *",
        )
        .bind(id)
        .bind(params.owner_user_id)
        .bind(params.name)
        .bind(params.source_type)
        .bind(params.git_project_id)
        .bind(params.branch_ref)
        .bind(params.status)
        .bind(params.root_rel_path)
        .bind(now)
        .bind(now)
        .fetch_one(&self.pool)
        .await
        .map_err(map_pg_conflict("workspace already exists"))
    }

    async fn find_by_id(&self, workspace_id: &str) -> Result<Option<WorkspaceRow>, DbError> {
        Ok(
            sqlx::query_as::<_, WorkspaceRow>("SELECT * FROM workspaces WHERE id = $1")
                .bind(workspace_id)
                .fetch_optional(&self.pool)
                .await?,
        )
    }

    async fn list_by_owner(&self, owner_user_id: &str) -> Result<Vec<WorkspaceRow>, DbError> {
        Ok(sqlx::query_as::<_, WorkspaceRow>(
            "SELECT * FROM workspaces
             WHERE owner_user_id = $1 AND status != 'deleted'
             ORDER BY created_at DESC",
        )
        .bind(owner_user_id)
        .fetch_all(&self.pool)
        .await?)
    }

    async fn has_active_by_git_project(&self, git_project_id: &str) -> Result<bool, DbError> {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*)
             FROM workspaces
             WHERE git_project_id = $1 AND status NOT IN ('archived', 'deleted')",
        )
        .bind(git_project_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(count > 0)
    }

    async fn update_name(&self, workspace_id: &str, name: &str) -> Result<(), DbError> {
        execute_required(
            sqlx::query("UPDATE workspaces SET name = $1, updated_at = $2 WHERE id = $3")
                .bind(name)
                .bind(aionui_common::now_ms())
                .bind(workspace_id)
                .execute(&self.pool)
                .await?,
            format!("Workspace '{workspace_id}' not found"),
        )
    }

    async fn update_status(&self, workspace_id: &str, status: &str) -> Result<(), DbError> {
        execute_required(
            sqlx::query("UPDATE workspaces SET status = $1, updated_at = $2 WHERE id = $3")
                .bind(status)
                .bind(aionui_common::now_ms())
                .bind(workspace_id)
                .execute(&self.pool)
                .await?,
            format!("Workspace '{workspace_id}' not found"),
        )
    }

    async fn mark_deleted(&self, workspace_id: &str) -> Result<(), DbError> {
        let now = aionui_common::now_ms();
        execute_required(
            sqlx::query("UPDATE workspaces SET status = 'deleted', deleted_at = $1, updated_at = $1 WHERE id = $2")
                .bind(now)
                .bind(workspace_id)
                .execute(&self.pool)
                .await?,
            format!("Workspace '{workspace_id}' not found"),
        )
    }
}

#[derive(Clone, Debug)]
pub struct PgSnapshotRepository {
    pool: PgPool,
}

impl PgSnapshotRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl ISnapshotRepository for PgSnapshotRepository {
    async fn create_snapshot(&self, params: CreateSnapshotParams) -> Result<SnapshotRow, DbError> {
        let id = aionui_common::generate_prefixed_id("snap");
        let now = aionui_common::now_ms();

        Ok(sqlx::query_as::<_, SnapshotRow>(
            "INSERT INTO snapshots (
                id, workspace_id, created_by, source_kind, artifact_ref, manifest_ref,
                checksum, size_bytes, base_git_metadata, environment_metadata,
                retention_policy, sealed_at, expires_at, created_at
             ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
             RETURNING *",
        )
        .bind(id)
        .bind(params.workspace_id)
        .bind(params.created_by)
        .bind(params.source_kind)
        .bind(params.artifact_ref)
        .bind(params.manifest_ref)
        .bind(params.checksum)
        .bind(params.size_bytes)
        .bind(params.base_git_metadata)
        .bind(params.environment_metadata)
        .bind(params.retention_policy)
        .bind(params.sealed_at)
        .bind(params.expires_at)
        .bind(now)
        .fetch_one(&self.pool)
        .await?)
    }

    async fn find_by_id(&self, snapshot_id: &str) -> Result<Option<SnapshotRow>, DbError> {
        Ok(
            sqlx::query_as::<_, SnapshotRow>("SELECT * FROM snapshots WHERE id = $1")
                .bind(snapshot_id)
                .fetch_optional(&self.pool)
                .await?,
        )
    }

    async fn list_by_workspace(&self, workspace_id: &str) -> Result<Vec<SnapshotRow>, DbError> {
        Ok(
            sqlx::query_as::<_, SnapshotRow>(
                "SELECT * FROM snapshots WHERE workspace_id = $1 ORDER BY created_at DESC",
            )
            .bind(workspace_id)
            .fetch_all(&self.pool)
            .await?,
        )
    }

    async fn list_gc_candidates(&self, now_ms: i64, limit: i64) -> Result<Vec<SnapshotRow>, DbError> {
        Ok(sqlx::query_as::<_, SnapshotRow>(
            "SELECT *
             FROM snapshots
             WHERE expires_at IS NOT NULL
               AND expires_at <= $1
               AND gc_tombstone_at IS NULL
             ORDER BY expires_at ASC, created_at ASC
             LIMIT $2",
        )
        .bind(now_ms)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?)
    }

    async fn mark_gc_tombstone(&self, snapshot_id: &str, gc_tombstone_at: i64) -> Result<(), DbError> {
        execute_required(
            sqlx::query("UPDATE snapshots SET gc_tombstone_at = $1 WHERE id = $2")
                .bind(gc_tombstone_at)
                .bind(snapshot_id)
                .execute(&self.pool)
                .await?,
            format!("Snapshot '{snapshot_id}' not found"),
        )
    }
}

#[derive(Clone, Debug)]
pub struct PgExecutionRunRepository {
    pool: PgPool,
}

impl PgExecutionRunRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl IExecutionRunRepository for PgExecutionRunRepository {
    async fn create_execution_run(&self, params: CreateExecutionRunParams) -> Result<ExecutionRunRow, DbError> {
        let id = aionui_common::generate_prefixed_id("exec");
        let now = aionui_common::now_ms();

        Ok(sqlx::query_as::<_, ExecutionRunRow>(
            "INSERT INTO execution_runs (
                id, workspace_id, snapshot_id, owner_user_id, execution_type, status,
                policy, resource_profile, network_profile, trace_id, created_at, updated_at
             ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
             RETURNING *",
        )
        .bind(id)
        .bind(params.workspace_id)
        .bind(params.snapshot_id)
        .bind(params.owner_user_id)
        .bind(params.execution_type)
        .bind(params.status)
        .bind(params.policy)
        .bind(params.resource_profile)
        .bind(params.network_profile)
        .bind(params.trace_id)
        .bind(now)
        .bind(now)
        .fetch_one(&self.pool)
        .await?)
    }

    async fn find_by_id(&self, execution_id: &str) -> Result<Option<ExecutionRunRow>, DbError> {
        Ok(
            sqlx::query_as::<_, ExecutionRunRow>("SELECT * FROM execution_runs WHERE id = $1")
                .bind(execution_id)
                .fetch_optional(&self.pool)
                .await?,
        )
    }

    async fn list_by_workspace(&self, workspace_id: &str) -> Result<Vec<ExecutionRunRow>, DbError> {
        Ok(sqlx::query_as::<_, ExecutionRunRow>(
            "SELECT * FROM execution_runs WHERE workspace_id = $1 ORDER BY created_at DESC",
        )
        .bind(workspace_id)
        .fetch_all(&self.pool)
        .await?)
    }

    async fn has_active_by_snapshot(&self, snapshot_id: &str) -> Result<bool, DbError> {
        let count: (i64,) = sqlx::query_as(
            "SELECT COUNT(*)
             FROM execution_runs
             WHERE snapshot_id = $1
               AND status IN (
                 'created', 'preparing', 'snapshot_resolved', 'provisioning',
                 'running', 'streaming', 'collecting', 'cleanup', 'cleanup_failed'
               )",
        )
        .bind(snapshot_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(count.0 > 0)
    }

    async fn update_status(
        &self,
        execution_id: &str,
        status: &str,
        started_at: Option<i64>,
        finished_at: Option<i64>,
    ) -> Result<(), DbError> {
        execute_required(
            sqlx::query(
                "UPDATE execution_runs
                 SET status = $1,
                     started_at = COALESCE($2, started_at),
                     finished_at = COALESCE($3, finished_at),
                     updated_at = $4
                 WHERE id = $5",
            )
            .bind(status)
            .bind(started_at)
            .bind(finished_at)
            .bind(aionui_common::now_ms())
            .bind(execution_id)
            .execute(&self.pool)
            .await?,
            format!("ExecutionRun '{execution_id}' not found"),
        )
    }

    async fn set_preview_url(&self, execution_id: &str, preview_url: Option<&str>) -> Result<(), DbError> {
        execute_required(
            sqlx::query("UPDATE execution_runs SET preview_url = $1, updated_at = $2 WHERE id = $3")
                .bind(preview_url)
                .bind(aionui_common::now_ms())
                .bind(execution_id)
                .execute(&self.pool)
                .await?,
            format!("ExecutionRun '{execution_id}' not found"),
        )
    }
}

#[derive(Clone, Debug)]
pub struct PgExecutionArtifactRepository {
    pool: PgPool,
}

impl PgExecutionArtifactRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl IExecutionArtifactRepository for PgExecutionArtifactRepository {
    async fn create_artifact(&self, params: CreateExecutionArtifactParams) -> Result<ExecutionArtifactRow, DbError> {
        let id = aionui_common::generate_prefixed_id("artifact");
        let now = aionui_common::now_ms();

        Ok(sqlx::query_as::<_, ExecutionArtifactRow>(
            "INSERT INTO execution_artifacts (
                id, execution_id, artifact_type, ref, metadata, immutable, created_at
             ) VALUES ($1, $2, $3, $4, $5, $6, $7)
             RETURNING id, execution_id, artifact_type, ref AS ref_, metadata, immutable, created_at",
        )
        .bind(id)
        .bind(params.execution_id)
        .bind(params.artifact_type)
        .bind(params.ref_)
        .bind(params.metadata)
        .bind(params.immutable)
        .bind(now)
        .fetch_one(&self.pool)
        .await?)
    }

    async fn list_by_execution(&self, execution_id: &str) -> Result<Vec<ExecutionArtifactRow>, DbError> {
        Ok(sqlx::query_as::<_, ExecutionArtifactRow>(
            "SELECT id, execution_id, artifact_type, ref AS ref_, metadata, immutable, created_at
             FROM execution_artifacts
             WHERE execution_id = $1
             ORDER BY created_at ASC",
        )
        .bind(execution_id)
        .fetch_all(&self.pool)
        .await?)
    }
}

#[derive(Clone, Debug)]
pub struct PgAuditLogRepository {
    pool: PgPool,
}

impl PgAuditLogRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl IAuditLogRepository for PgAuditLogRepository {
    async fn create_log(&self, params: CreateAuditLogParams) -> Result<AuditLogRow, DbError> {
        let id = aionui_common::generate_prefixed_id("audit");
        let now = aionui_common::now_ms();

        Ok(sqlx::query_as::<_, AuditLogRow>(
            "INSERT INTO audit_logs (
                id, actor_user_id, target_type, target_id, action, result, metadata, trace_id, created_at
             ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
             RETURNING *",
        )
        .bind(id)
        .bind(params.actor_user_id)
        .bind(params.target_type)
        .bind(params.target_id)
        .bind(params.action)
        .bind(params.result)
        .bind(params.metadata)
        .bind(params.trace_id)
        .bind(now)
        .fetch_one(&self.pool)
        .await?)
    }

    async fn list_by_target(&self, target_type: &str, target_id: &str) -> Result<Vec<AuditLogRow>, DbError> {
        Ok(sqlx::query_as::<_, AuditLogRow>(
            "SELECT * FROM audit_logs
             WHERE target_type = $1 AND target_id = $2
             ORDER BY created_at DESC",
        )
        .bind(target_type)
        .bind(target_id)
        .fetch_all(&self.pool)
        .await?)
    }
}

fn execute_required(result: sqlx::postgres::PgQueryResult, not_found: String) -> Result<(), DbError> {
    if result.rows_affected() == 0 {
        return Err(DbError::NotFound(not_found));
    }
    Ok(())
}

fn map_pg_conflict(message: &'static str) -> impl FnOnce(sqlx::Error) -> DbError {
    move |error| match &error {
        sqlx::Error::Database(db_err) if db_err.code().is_some_and(|code| code == "23505") => {
            DbError::Conflict(message.to_string())
        }
        _ => DbError::Query(error),
    }
}
