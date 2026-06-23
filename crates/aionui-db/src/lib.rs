#![warn(clippy::disallowed_types)]

//! Database layer: init, migrations, repository traits, and implementations.
mod database;
mod error;
pub mod models;
mod repository;

pub use database::{
    Database, DatabaseInitError, PostgresDatabase, init_database, init_database_memory, init_database_staged,
    init_postgres_database, init_postgres_database_staged, maybe_copy_legacy_database,
};
pub use error::DbError;
pub use models::{
    AgentMetadataRow, AssistantOverrideRow, AssistantRow, AuditLogRow, ConversationArtifactRow, CreateAssistantParams,
    CreateAuditLogParams, CreateExecutionArtifactParams, CreateExecutionRunParams, CreateGitProjectParams,
    CreateGitSshCredentialParams, CreatePlatformUserParams, CreateSnapshotParams, CreateWorkspaceParams,
    ExecutionArtifactRow, ExecutionRunRow, ExternalIdentityRow, GitProjectRow, GitSshCredentialRow, PlatformUserRow,
    RoleRow, SnapshotRow, UpdateAgentHandshakeParams, UpdateAssistantParams, UpsertAgentMetadataParams,
    UpsertExternalIdentityParams, UpsertOverrideParams, UpsertRoleParams, WorkspaceRow,
};
pub use repository::channel::UpdatePluginStatusParams;
pub use repository::conversation::{
    ConversationFilters, ConversationRowUpdate, MessageRowUpdate, MessageSearchRow, SortOrder,
};
pub use repository::cron::UpdateCronJobParams;
pub use repository::mcp_server::{CreateMcpServerParams, UpdateMcpServerParams};
pub use repository::oauth_token::UpsertOAuthTokenParams;
pub use repository::provider::{CreateProviderParams, UpdateProviderParams};
pub use repository::remote_agent::{CreateRemoteAgentParams, UpdateRemoteAgentParams};
pub use repository::team::{UpdateTaskParams, UpdateTeamParams};
pub use repository::{
    CreateAcpSessionParams, IAcpSessionRepository, IAgentMetadataRepository, IAssistantOverrideRepository,
    IAssistantRepository, IAuditLogRepository, IChannelRepository, IClientPreferenceRepository,
    IConversationRepository, ICronRepository, IExecutionArtifactRepository, IExecutionRunRepository,
    IExternalIdentityRepository, IGitProjectRepository, IGitSshCredentialRepository, IMcpServerRepository,
    IOAuthTokenRepository, IPlatformUserRepository, IProviderRepository, IRemoteAgentRepository, IRoleRepository,
    ISettingsRepository, ISnapshotRepository, ITeamRepository, IUserRepository, IWorkspaceRepository,
    PersistedSessionState, PgAuditLogRepository, PgExecutionArtifactRepository, PgExecutionRunRepository,
    PgExternalIdentityRepository, PgGitProjectRepository, PgGitSshCredentialRepository, PgPlatformUserRepository,
    PgRoleRepository, PgSnapshotRepository, PgWorkspaceRepository, SaveRuntimeStateParams, SqliteAcpSessionRepository,
    SqliteAgentMetadataRepository, SqliteAssistantOverrideRepository, SqliteAssistantRepository,
    SqliteChannelRepository, SqliteClientPreferenceRepository, SqliteConversationRepository, SqliteCronRepository,
    SqliteMcpServerRepository, SqliteOAuthTokenRepository, SqliteProviderRepository, SqliteRemoteAgentRepository,
    SqliteSettingsRepository, SqliteTeamRepository, SqliteUserRepository,
};

// Re-export sqlx pool types for downstream crates.
pub use sqlx::{PgPool, SqlitePool};
