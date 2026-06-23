//! Shared application services for dependency injection.

use std::path::PathBuf;
use std::sync::Arc;

use aionui_ai_agent::{
    AcpSessionSyncService, AcpSkillManager, AgentFactoryDeps, AgentRegistry, IWorkerTaskManager, WorkerTaskManagerImpl,
    build_agent_factory,
};
use aionui_api_types::GuideMcpConfig;
use aionui_auth::{CookieConfig, GatewayAuthConfig, JwtService, QrTokenStore, hash_password, resolve_jwt_secret};
use aionui_common::OnConversationDelete;
use aionui_conversation::runtime_state::ConversationRuntimeStateService;
use aionui_db::{
    CreatePlatformUserParams, Database, IAcpSessionRepository, IAgentMetadataRepository, IAuditLogRepository,
    IConversationRepository, IExecutionArtifactRepository, IExecutionRunRepository, IExternalIdentityRepository,
    IGitProjectRepository, IGitSshCredentialRepository, IMcpServerRepository, IPlatformUserRepository, IRoleRepository,
    ISnapshotRepository, IUserRepository, IWorkspaceRepository, PgAuditLogRepository, PgExecutionArtifactRepository,
    PgExecutionRunRepository, PgExternalIdentityRepository, PgGitProjectRepository, PgGitSshCredentialRepository,
    PgPlatformUserRepository, PgRoleRepository, PgSnapshotRepository, PgWorkspaceRepository, PostgresDatabase,
    SqliteAcpSessionRepository, SqliteAgentMetadataRepository, SqliteConversationRepository, SqliteMcpServerRepository,
    SqliteProviderRepository, SqliteUserRepository, UpsertRoleParams,
};
use aionui_realtime::{BroadcastEventBus, WebSocketManager};
use aionui_team::GuideMcpServer;
use serde_json::{Value, json};

use crate::config::{AppConfig, DeploymentMode, derive_encryption_key};

pub struct WorkbenchRepositories {
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
}

const SUPER_ADMIN_ROLE_KEY: &str = "super_admin";
const DEFAULT_USER_ROLE_KEY: &str = "ordinary_user";
const ADMIN_PERMISSION: &str = "*";

pub struct AppServices {
    pub database: Database,
    pub postgres_database: Option<PostgresDatabase>,
    pub workbench: Option<WorkbenchRepositories>,
    pub jwt_service: Arc<JwtService>,
    pub gateway_auth: Option<GatewayAuthConfig>,
    pub user_repo: Arc<dyn IUserRepository>,
    pub cookie_config: Arc<CookieConfig>,
    pub qr_token_store: Arc<QrTokenStore>,
    pub ws_manager: Arc<WebSocketManager>,
    pub event_bus: Arc<BroadcastEventBus>,
    pub worker_task_manager: Arc<dyn IWorkerTaskManager>,
    pub conversation_runtime_state: Arc<ConversationRuntimeStateService>,
    /// Same instance as `worker_task_manager`, exposed through the
    /// `OnConversationDelete` trait so `ConversationService::with_delete_hook`
    /// can wire it up. Optional because tests construct `AppServices` with a
    /// mock `worker_task_manager` that does not implement the trait.
    pub task_manager_delete_hook: Option<Arc<dyn OnConversationDelete>>,
    pub agent_registry: Arc<AgentRegistry>,
    pub conversation_repo: Arc<dyn IConversationRepository>,
    pub acp_session_sync: Arc<AcpSessionSyncService>,
    /// Raw JWT secret string, used to derive encryption keys.
    pub jwt_secret_raw: String,
    pub data_dir: PathBuf,
    pub work_dir: PathBuf,
    pub senmo_workspace_root: Option<PathBuf>,
    /// When `true`, skip JWT authentication and use a fixed default user.
    pub local: bool,
    pub app_version: String,
    /// Resolved skill paths. Shared with the `ConversationService` for
    /// snapshot resolution at create time.
    pub skill_paths: Arc<aionui_extension::SkillPaths>,
    /// Guide MCP server config (port, token, binary_path).
    /// `None` when the server failed to start (graceful degradation).
    pub guide_mcp_config: Option<GuideMcpConfig>,
    /// Guide MCP server instance kept alive for the app lifetime.
    pub(crate) _guide_server: Option<GuideMcpServer>,
}

impl AppServices {
    /// Replace the worker task manager after construction.
    ///
    /// Primarily used by tests to inject mock implementations.
    pub fn with_worker_task_manager(mut self, wtm: Arc<dyn IWorkerTaskManager>) -> Self {
        self.worker_task_manager = wtm;
        self
    }

    /// Wire the TeamSessionService into the Guide MCP server so
    /// `aion_create_team` requests can call `service.create_team(...)`.
    /// Called from `create_router` after `build_module_states`.
    pub(crate) async fn inject_guide_service(&self, service: std::sync::Weak<aionui_team::TeamSessionService>) {
        if let Some(server) = &self._guide_server {
            server.set_service(service).await;
        }
    }

    pub async fn from_config(database: Database, config: &AppConfig) -> anyhow::Result<Self> {
        let data_dir = config.data_dir.clone();
        let work_dir = config.work_dir.clone();
        let senmo_workspace_root = config.senmo_workspace_root.clone();
        let local = config.local;
        let app_version = config.app_version.clone();
        let (postgres_database, workbench) = init_platform_repositories(config).await?;
        let gateway_auth = gateway_auth_config(config);
        let user_repo: Arc<dyn IUserRepository> = Arc::new(SqliteUserRepository::new(database.pool().clone()));

        // Resolve JWT secret: env var → system user db field → random generation
        let env_secret = std::env::var("JWT_SECRET").ok();
        let system_user = user_repo
            .get_system_user()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to get system user: {e}"))?;

        let db_secret = system_user
            .as_ref()
            .and_then(|u| u.jwt_secret.as_deref())
            .filter(|s| !s.is_empty());

        let (secret, is_new) = resolve_jwt_secret(env_secret.as_deref(), db_secret);

        // Persist newly generated secret to database
        if is_new && let Some(user) = &system_user {
            user_repo
                .update_jwt_secret(&user.id, &secret)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to persist JWT secret: {e}"))?;
            tracing::info!("Generated and persisted new JWT secret");
        }

        let encryption_key = derive_encryption_key(&secret);

        let provider_repo = Arc::new(SqliteProviderRepository::new(database.pool().clone()));
        let event_bus = Arc::new(BroadcastEventBus::new(256));
        // User-configured MCP servers — injected into ACP `session/new`
        // so the agent gets the operator's tools (ELECTRON-1JG fix).
        let mcp_server_repo: Arc<dyn IMcpServerRepository> =
            Arc::new(SqliteMcpServerRepository::new(database.pool().clone()));

        let agent_metadata_repo: Arc<dyn IAgentMetadataRepository> =
            Arc::new(SqliteAgentMetadataRepository::new(database.pool().clone()));
        let agent_registry = AgentRegistry::new(agent_metadata_repo);
        agent_registry
            .hydrate()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to hydrate agent registry: {e}"))?;

        let acp_session_repo: Arc<dyn IAcpSessionRepository> =
            Arc::new(SqliteAcpSessionRepository::new(database.pool().clone()));
        let acp_agent_service = AcpSessionSyncService::new(acp_session_repo.clone());

        let conversation_repo: Arc<dyn IConversationRepository> =
            Arc::new(SqliteConversationRepository::new(database.pool().clone()));

        // Skill paths need app resource dir (for builtin rules) + data dir
        // (for user skills + materialized views). AcpSkillManager uses these
        // for first-message skill index/body loading.
        let app_resource_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.canonicalize().ok())
            .and_then(|p| p.parent().map(|pp| pp.to_path_buf()))
            .unwrap_or_else(|| std::path::PathBuf::from("."));
        let skill_paths = Arc::new(aionui_extension::resolve_skill_paths(&app_resource_dir, &data_dir));

        // Absolute path to this process's binary. Reused as the `command` for
        // the stdio MCP bridge spawned by ACP CLIs when a team session is
        // attached to a conversation in the single-binary model.
        let backend_binary_path =
            Arc::new(std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("aioncore")));

        // Start Guide MCP server. Failure is non-fatal: solo agents simply
        // won't get the `aion_create_team` tool.
        let (guide_server, guide_mcp_config) = match GuideMcpServer::start().await {
            Ok(srv) => {
                let config = GuideMcpConfig {
                    port: srv.http_port(),
                    token: srv.auth_token().to_owned(),
                    binary_path: backend_binary_path.to_string_lossy().to_string(),
                };
                tracing::info!(port = config.port, "Guide MCP server started");
                (Some(srv), Some(config))
            }
            Err(e) => {
                tracing::warn!(
                    code = "BOOTSTRAP_DEGRADED_GUIDE_MCP",
                    stage = "guide_mcp.start",
                    error = %e,
                    "Guide MCP server failed to start; solo create-team disabled"
                );
                (None, None)
            }
        };

        let factory = build_agent_factory(AgentFactoryDeps {
            skill_manager: AcpSkillManager::new(skill_paths.clone()),
            provider_repo,
            encryption_key,
            agent_registry: agent_registry.clone(),
            acp_agent_service: acp_agent_service.clone(),
            data_dir: data_dir.clone(),
            broadcaster: event_bus.clone(),
            backend_binary_path: backend_binary_path.clone(),
            guide_mcp_config: guide_mcp_config.clone(),
            mcp_server_repo: Some(mcp_server_repo),
        });

        // Agent factory is now wired. Future extension/custom agents
        // that get written to `agent_metadata` will show up after the
        // relevant service calls `AgentRegistry::hydrate`.
        let task_manager_concrete = Arc::new(WorkerTaskManagerImpl::new(factory));
        let worker_task_manager: Arc<dyn IWorkerTaskManager> = task_manager_concrete.clone();
        let task_manager_delete_hook: Arc<dyn OnConversationDelete> = task_manager_concrete;
        let conversation_runtime_state = Arc::new(ConversationRuntimeStateService::default());

        Ok(Self {
            database,
            postgres_database,
            workbench,
            jwt_service: Arc::new(JwtService::new(secret.clone())),
            gateway_auth,
            user_repo,
            cookie_config: Arc::new(CookieConfig::from_env()),
            qr_token_store: Arc::new(QrTokenStore::new()),
            ws_manager: Arc::new(WebSocketManager::new()),
            event_bus,
            worker_task_manager,
            conversation_runtime_state,
            task_manager_delete_hook: Some(task_manager_delete_hook),
            agent_registry,
            conversation_repo,
            acp_session_sync: acp_agent_service,
            jwt_secret_raw: secret,
            data_dir,
            work_dir,
            senmo_workspace_root,
            local,
            app_version,
            skill_paths,
            guide_mcp_config: guide_mcp_config.clone(),
            _guide_server: guide_server,
        })
    }
}

fn gateway_auth_config(config: &AppConfig) -> Option<GatewayAuthConfig> {
    let app_id = config.gateway_app_id.as_deref()?.trim();
    let app_secret = config.gateway_app_secret.as_deref()?.trim();
    if app_id.is_empty() || app_secret.is_empty() {
        return None;
    }
    Some(GatewayAuthConfig {
        app_id: app_id.to_string(),
        app_secret: app_secret.to_string(),
        provider: config.gateway_provider.clone(),
        timestamp_skew_seconds: config.gateway_timestamp_skew_seconds,
    })
}

async fn init_platform_repositories(
    config: &AppConfig,
) -> anyhow::Result<(Option<PostgresDatabase>, Option<WorkbenchRepositories>)> {
    if config.effective_deployment_mode() != DeploymentMode::Saas {
        return Ok((None, None));
    }

    let database_url = config
        .database_url
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("DEPLOYMENT_MODE=saas requires DATABASE_URL"))?;

    let postgres_database = aionui_db::init_postgres_database_staged(database_url)
        .await
        .map_err(|error| anyhow::anyhow!("Failed to initialize SaaS PostgreSQL database: {error}"))?;
    let pool = postgres_database.pool().clone();
    let repositories = WorkbenchRepositories {
        users: Arc::new(PgPlatformUserRepository::new(pool.clone())),
        external_identities: Arc::new(PgExternalIdentityRepository::new(pool.clone())),
        roles: Arc::new(PgRoleRepository::new(pool.clone())),
        git_ssh_credentials: Arc::new(PgGitSshCredentialRepository::new(pool.clone())),
        git_projects: Arc::new(PgGitProjectRepository::new(pool.clone())),
        workspaces: Arc::new(PgWorkspaceRepository::new(pool.clone())),
        snapshots: Arc::new(PgSnapshotRepository::new(pool.clone())),
        execution_runs: Arc::new(PgExecutionRunRepository::new(pool.clone())),
        execution_artifacts: Arc::new(PgExecutionArtifactRepository::new(pool.clone())),
        audit_logs: Arc::new(PgAuditLogRepository::new(pool)),
    };

    seed_senmo_initial_admin(config, &repositories).await?;

    Ok((Some(postgres_database), Some(repositories)))
}

async fn seed_senmo_initial_admin(config: &AppConfig, repositories: &WorkbenchRepositories) -> anyhow::Result<()> {
    let Some(phone) = config
        .senmo_initial_admin_phone
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(());
    };
    let password = config
        .senmo_initial_admin_password
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("SENMO_INITIAL_ADMIN_PASSWORD is required when initial admin phone is set"))?;
    let user = if let Some(existing) = repositories
        .users
        .find_by_phone(phone)
        .await
        .map_err(|error| anyhow::anyhow!("Failed to query initial admin user: {error}"))?
    {
        repositories
            .users
            .update_status(&existing.id, "enabled")
            .await
            .map_err(|error| anyhow::anyhow!("Failed to enable initial admin user: {error}"))?;
        repositories
            .users
            .find_by_id(&existing.id)
            .await
            .map_err(|error| anyhow::anyhow!("Failed to reload initial admin user: {error}"))?
            .unwrap_or(existing)
    } else {
        let password = password.to_owned();
        let password_hash = tokio::task::spawn_blocking(move || hash_password(&password))
            .await
            .map_err(|error| anyhow::anyhow!("Failed to join initial admin password hashing task: {error}"))?
            .map_err(|error| anyhow::anyhow!("Failed to hash initial admin password: {error}"))?;
        repositories
            .users
            .create_user(CreatePlatformUserParams {
                phone: Some(phone.to_owned()),
                username: None,
                display_name: Some("Initial Administrator".to_owned()),
                email: None,
                password_hash: Some(password_hash),
                status: "enabled".to_owned(),
            })
            .await
            .map_err(|error| anyhow::anyhow!("Failed to create initial admin user: {error}"))?
    };

    assign_system_roles(repositories, &user.id, true).await
}

async fn assign_system_roles(
    repositories: &WorkbenchRepositories,
    user_id: &str,
    include_super_admin: bool,
) -> anyhow::Result<()> {
    let default_role = ensure_system_role(
        repositories,
        UpsertRoleParams {
            role_key: DEFAULT_USER_ROLE_KEY.to_owned(),
            role_name: "普通用户".to_owned(),
            status: "enabled".to_owned(),
            permissions: Value::Array(vec![Value::String("workspace:own".to_owned())]),
            sort_order: 1000,
            is_system: true,
        },
        "default user",
    )
    .await?;
    repositories
        .roles
        .assign_role(user_id, &default_role.id)
        .await
        .map_err(|error| anyhow::anyhow!("Failed to assign default user role: {error}"))?;

    if include_super_admin {
        let admin_role = ensure_system_role(
            repositories,
            UpsertRoleParams {
                role_key: SUPER_ADMIN_ROLE_KEY.to_owned(),
                role_name: "超级管理员".to_owned(),
                status: "enabled".to_owned(),
                permissions: json!([ADMIN_PERMISSION]),
                sort_order: 0,
                is_system: true,
            },
            "super admin",
        )
        .await?;
        repositories
            .roles
            .assign_role(user_id, &admin_role.id)
            .await
            .map_err(|error| anyhow::anyhow!("Failed to assign super admin role: {error}"))?;
    }

    Ok(())
}

async fn ensure_system_role(
    repositories: &WorkbenchRepositories,
    params: UpsertRoleParams,
    label: &str,
) -> anyhow::Result<aionui_db::RoleRow> {
    if let Some(existing) = repositories
        .roles
        .list_roles()
        .await
        .map_err(|error| anyhow::anyhow!("Failed to list roles before seeding {label} role: {error}"))?
        .into_iter()
        .find(|role| role.role_key == params.role_key)
    {
        return Ok(existing);
    }

    repositories
        .roles
        .upsert_role(params)
        .await
        .map_err(|error| anyhow::anyhow!("Failed to seed {label} role: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_app_services_from_memory_db() {
        let db = aionui_db::init_database_memory().await.unwrap();
        let services = AppServices::from_config(db, &AppConfig::default()).await.unwrap();

        assert!(services.postgres_database.is_none());
        assert!(services.workbench.is_none());

        // JWT service should be functional
        let token = services.jwt_service.sign("test_user", "testuser").unwrap();
        let payload = services.jwt_service.verify(&token).unwrap();
        assert_eq!(payload.user_id, "test_user");

        // User repo should have system user
        let has_users = services.user_repo.has_users().await.unwrap();
        assert!(!has_users); // system user has empty password → not counted

        services.database.close().await;
    }

    #[tokio::test]
    async fn test_saas_services_require_database_url() {
        let db = aionui_db::init_database_memory().await.unwrap();
        let config = AppConfig {
            deployment_mode: DeploymentMode::Saas,
            senmo_workspace_root: Some(tempfile::tempdir().unwrap().path().join("senmo")),
            ..Default::default()
        };

        let error = match AppServices::from_config(db.clone(), &config).await {
            Ok(_) => panic!("SaaS services should require DATABASE_URL"),
            Err(error) => error,
        };

        assert!(error.to_string().contains("DATABASE_URL"));
        db.close().await;
    }

    #[tokio::test]
    async fn desktop_services_start_without_postgres() {
        let db = aionui_db::init_database_memory().await.unwrap();
        let config = AppConfig {
            deployment_mode: DeploymentMode::Desktop,
            database_url: None,
            senmo_workspace_root: None,
            ..Default::default()
        };

        let services = AppServices::from_config(db, &config).await.unwrap();

        assert!(services.postgres_database.is_none());
        assert!(services.workbench.is_none());
        services.database.close().await;
    }

    #[tokio::test]
    async fn local_services_start_without_postgres_even_when_saas_requested() {
        let db = aionui_db::init_database_memory().await.unwrap();
        let config = AppConfig {
            local: true,
            deployment_mode: DeploymentMode::Saas,
            database_url: None,
            senmo_workspace_root: None,
            ..Default::default()
        };

        let services = AppServices::from_config(db, &config).await.unwrap();

        assert!(services.postgres_database.is_none());
        assert!(services.workbench.is_none());
        services.database.close().await;
    }

    #[tokio::test]
    async fn test_jwt_secret_persisted_to_db() {
        let db = aionui_db::init_database_memory().await.unwrap();
        let services = AppServices::from_config(db, &AppConfig::default()).await.unwrap();

        // System user should now have a jwt_secret persisted
        let system_user = services.user_repo.get_system_user().await.unwrap();
        let jwt_secret = system_user.unwrap().jwt_secret;
        assert!(jwt_secret.is_some());
        assert!(!jwt_secret.unwrap().is_empty());

        services.database.close().await;
    }

    #[tokio::test]
    async fn test_app_services_uses_supplied_app_version() {
        let db = aionui_db::init_database_memory().await.unwrap();
        let config = AppConfig {
            app_version: "9.9.9".to_string(),
            ..Default::default()
        };
        let services = AppServices::from_config(db, &config).await.unwrap();

        assert_eq!(services.app_version, "9.9.9");

        services.database.close().await;
    }
}
