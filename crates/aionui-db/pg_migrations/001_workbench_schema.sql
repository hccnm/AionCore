-- Phase 2 PostgreSQL schema for SaaS workspaces.

CREATE TABLE IF NOT EXISTS users (
    id TEXT PRIMARY KEY NOT NULL,
    phone TEXT UNIQUE,
    username TEXT UNIQUE,
    display_name TEXT,
    email TEXT UNIQUE,
    password_hash TEXT,
    avatar_path TEXT,
    status TEXT NOT NULL DEFAULT 'enabled' CHECK (status IN ('enabled', 'disabled')),
    jwt_secret TEXT,
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL,
    last_login BIGINT
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_users_phone_unique ON users(phone) WHERE phone IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_users_status ON users(status);

CREATE TABLE IF NOT EXISTS external_identities (
    id TEXT PRIMARY KEY NOT NULL,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider TEXT NOT NULL,
    app_id TEXT NOT NULL,
    external_user_id TEXT NOT NULL,
    phone_snapshot TEXT,
    external_role_ids JSONB NOT NULL DEFAULT '[]'::jsonb,
    is_admin BOOLEAN NOT NULL DEFAULT FALSE,
    raw_payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    last_synced_at BIGINT NOT NULL,
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL,
    UNIQUE (provider, app_id, external_user_id)
);
CREATE INDEX IF NOT EXISTS idx_external_identities_user ON external_identities(user_id);

CREATE TABLE IF NOT EXISTS roles (
    id TEXT PRIMARY KEY NOT NULL,
    role_key TEXT NOT NULL UNIQUE,
    role_name TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'enabled' CHECK (status IN ('enabled', 'disabled')),
    permissions JSONB NOT NULL DEFAULT '[]'::jsonb,
    sort_order INTEGER NOT NULL DEFAULT 1000,
    is_system BOOLEAN NOT NULL DEFAULT FALSE,
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_roles_status ON roles(status);

CREATE TABLE IF NOT EXISTS user_roles (
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    role_id TEXT NOT NULL REFERENCES roles(id) ON DELETE CASCADE,
    created_at BIGINT NOT NULL,
    PRIMARY KEY (user_id, role_id)
);

CREATE TABLE IF NOT EXISTS git_ssh_credentials (
    id TEXT PRIMARY KEY NOT NULL,
    owner_user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    encrypted_private_key TEXT NOT NULL,
    encrypted_passphrase TEXT,
    public_key TEXT,
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'verified', 'failed', 'disabled')),
    last_verified_at BIGINT,
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_git_ssh_credentials_owner ON git_ssh_credentials(owner_user_id);

CREATE TABLE IF NOT EXISTS git_projects (
    id TEXT PRIMARY KEY NOT NULL,
    owner_user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    credential_id TEXT NOT NULL REFERENCES git_ssh_credentials(id),
    repo_ssh_url TEXT NOT NULL,
    display_name TEXT NOT NULL,
    default_branch TEXT,
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'verified', 'failed', 'archived')),
    last_verified_at BIGINT,
    last_synced_at BIGINT,
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL,
    UNIQUE (owner_user_id, repo_ssh_url)
);
CREATE INDEX IF NOT EXISTS idx_git_projects_owner ON git_projects(owner_user_id);
CREATE INDEX IF NOT EXISTS idx_git_projects_credential ON git_projects(credential_id);

CREATE TABLE IF NOT EXISTS workspaces (
    id TEXT PRIMARY KEY NOT NULL,
    owner_user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    source_type TEXT NOT NULL CHECK (source_type IN ('blank', 'git_project')),
    git_project_id TEXT REFERENCES git_projects(id),
    branch_ref TEXT,
    status TEXT NOT NULL DEFAULT 'provisioning'
        CHECK (status IN ('provisioning', 'active', 'archived', 'failed', 'deleted')),
    root_rel_path TEXT NOT NULL UNIQUE,
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL,
    deleted_at BIGINT,
    CHECK (
        (source_type = 'blank' AND git_project_id IS NULL)
        OR (source_type = 'git_project' AND git_project_id IS NOT NULL)
    )
);
CREATE INDEX IF NOT EXISTS idx_workspaces_owner ON workspaces(owner_user_id);
CREATE INDEX IF NOT EXISTS idx_workspaces_git_project ON workspaces(git_project_id);
CREATE INDEX IF NOT EXISTS idx_workspaces_status ON workspaces(status);

CREATE TABLE IF NOT EXISTS snapshots (
    id TEXT PRIMARY KEY NOT NULL,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id),
    created_by TEXT NOT NULL REFERENCES users(id),
    source_kind TEXT NOT NULL,
    artifact_ref TEXT NOT NULL,
    manifest_ref TEXT NOT NULL,
    checksum TEXT NOT NULL,
    size_bytes BIGINT NOT NULL DEFAULT 0,
    base_git_metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    environment_metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    retention_policy JSONB NOT NULL DEFAULT '{}'::jsonb,
    sealed_at BIGINT NOT NULL,
    expires_at BIGINT,
    gc_tombstone_at BIGINT,
    created_at BIGINT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_snapshots_workspace ON snapshots(workspace_id);

CREATE TABLE IF NOT EXISTS execution_runs (
    id TEXT PRIMARY KEY NOT NULL,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id),
    snapshot_id TEXT NOT NULL REFERENCES snapshots(id),
    owner_user_id TEXT NOT NULL REFERENCES users(id),
    execution_type TEXT NOT NULL CHECK (execution_type IN ('test_run', 'preview_env', 'ci_job')),
    status TEXT NOT NULL CHECK (
        status IN (
            'created', 'preparing', 'snapshot_resolved', 'provisioning', 'running',
            'streaming', 'collecting', 'succeeded', 'failed', 'timeout', 'cancelled',
            'cleanup', 'cleanup_failed'
        )
    ),
    policy JSONB NOT NULL DEFAULT '{}'::jsonb,
    resource_profile JSONB NOT NULL DEFAULT '{}'::jsonb,
    network_profile TEXT,
    trace_id TEXT NOT NULL,
    preview_url TEXT,
    created_at BIGINT NOT NULL,
    started_at BIGINT,
    finished_at BIGINT,
    updated_at BIGINT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_execution_runs_workspace ON execution_runs(workspace_id);
CREATE INDEX IF NOT EXISTS idx_execution_runs_owner ON execution_runs(owner_user_id);
CREATE INDEX IF NOT EXISTS idx_execution_runs_trace ON execution_runs(trace_id);
CREATE INDEX IF NOT EXISTS idx_execution_runs_status ON execution_runs(status);

CREATE TABLE IF NOT EXISTS execution_artifacts (
    id TEXT PRIMARY KEY NOT NULL,
    execution_id TEXT NOT NULL REFERENCES execution_runs(id) ON DELETE CASCADE,
    artifact_type TEXT NOT NULL,
    ref TEXT NOT NULL,
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    immutable BOOLEAN NOT NULL DEFAULT TRUE,
    created_at BIGINT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_execution_artifacts_execution ON execution_artifacts(execution_id);

CREATE TABLE IF NOT EXISTS audit_logs (
    id TEXT PRIMARY KEY NOT NULL,
    actor_user_id TEXT REFERENCES users(id),
    target_type TEXT NOT NULL,
    target_id TEXT,
    action TEXT NOT NULL,
    result TEXT NOT NULL CHECK (result IN ('success', 'failure')),
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    trace_id TEXT,
    created_at BIGINT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_audit_logs_actor ON audit_logs(actor_user_id);
CREATE INDEX IF NOT EXISTS idx_audit_logs_target ON audit_logs(target_type, target_id);
CREATE INDEX IF NOT EXISTS idx_audit_logs_trace ON audit_logs(trace_id);
