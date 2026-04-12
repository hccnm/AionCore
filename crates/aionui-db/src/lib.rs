mod database;
mod error;
pub mod models;
mod repository;

pub use database::{init_database, init_database_memory, Database};
pub use error::DbError;
pub use repository::{
    IClientPreferenceRepository, IConversationRepository, IProviderRepository,
    ISettingsRepository, IUserRepository, SqliteClientPreferenceRepository,
    SqliteConversationRepository, SqliteProviderRepository, SqliteSettingsRepository,
    SqliteUserRepository,
};
pub use repository::conversation::{
    ConversationFilters, ConversationRowUpdate, MessageRowUpdate, MessageSearchRow, SortOrder,
};
pub use repository::provider::{CreateProviderParams, UpdateProviderParams};

// Re-export sqlx pool type for downstream crates
pub use sqlx::SqlitePool;
