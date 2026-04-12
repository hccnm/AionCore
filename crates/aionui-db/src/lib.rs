mod database;
mod error;

pub use database::{init_database, init_database_memory, Database};
pub use error::DbError;

// Re-export sqlx pool type for downstream crates
pub use sqlx::SqlitePool;
