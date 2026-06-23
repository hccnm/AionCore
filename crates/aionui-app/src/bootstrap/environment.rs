//! Bootstrap layers shared by non-MCP subcommands.

use std::time::Instant;

use tracing::info;

use aionui_app::{AppConfig, DeploymentMode};
use aionui_db::Database;

use crate::cli::Cli;

use super::builtin_skills::materialize_builtin_skills;
use super::tracing_init::{LogGuards, init_tracing};
use super::work_dir::resolve_work_dir;
use super::{BootstrapError, BootstrapErrorCode};

/// Resolved environment needed by all non-MCP subcommands.
pub struct ServerEnvironment {
    /// Must be held alive for the process lifetime to flush log buffers.
    pub _log_guard: LogGuards,
    pub config: AppConfig,
}

/// Layer 1: Logging + config resolution.
///
/// Cheap, synchronous, no IO beyond creating the log directory.
/// All subcommands that need logging and config should call this first.
pub fn init_environment(cli: &Cli, merged_path: &str) -> Result<ServerEnvironment, BootstrapError> {
    load_dotenv_if_present();

    let log_dir = cli.log_dir.clone().unwrap_or_else(|| cli.data_dir.join("logs"));
    let log_guard = init_tracing(&log_dir, cli.log_level.as_deref())?;

    info!(
        path_segments = merged_path.split(if cfg!(windows) { ';' } else { ':' }).count(),
        path_len = merged_path.len(),
        "startup: PATH ready"
    );

    let work_dir = resolve_work_dir(cli.work_dir.clone(), &cli.data_dir);

    // SAFETY: called before any service initialization; no concurrent reads.
    unsafe {
        std::env::set_var("AIONUI_WORK_DIR", &work_dir);
    }

    let deployment_mode = std::env::var("DEPLOYMENT_MODE")
        .ok()
        .as_deref()
        .unwrap_or("desktop")
        .parse::<DeploymentMode>()
        .map_err(|error| {
            BootstrapError::new(
                BootstrapErrorCode::ConfigInvalid,
                "config.deployment_mode",
                "invalid startup configuration",
            )
            .with_field("error", error)
        })?;

    let config = AppConfig {
        host: cli.host.clone(),
        port: cli.port,
        data_dir: cli.data_dir.clone(),
        work_dir,
        app_version: cli.app_version.clone(),
        local: cli.local,
        deployment_mode,
        database_url: std::env::var("DATABASE_URL").ok(),
        senmo_workspace_root: std::env::var_os("SENMO_WORKSPACE_ROOT").map(std::path::PathBuf::from),
        gateway_app_id: std::env::var("APP_GATEWAY_APP_ID").ok(),
        gateway_app_secret: std::env::var("APP_GATEWAY_APP_SECRET").ok(),
        gateway_provider: std::env::var("APP_GATEWAY_PROVIDER")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "modo_open_platform".to_string()),
        gateway_timestamp_skew_seconds: std::env::var("APP_GATEWAY_TIMESTAMP_SKEW_SECONDS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(300),
        senmo_initial_admin_phone: std::env::var("SENMO_INITIAL_ADMIN_PHONE").ok(),
        senmo_initial_admin_password: std::env::var("SENMO_INITIAL_ADMIN_PASSWORD").ok(),
    };
    config.validate_saas_startup().map_err(|error| {
        BootstrapError::new(
            BootstrapErrorCode::ConfigInvalid,
            "config.saas",
            "invalid startup configuration",
        )
        .with_field("error", error.to_string())
    })?;
    info!(
        "Running in {:?} deployment mode — authentication is {}",
        config.effective_deployment_mode(),
        if config.local { "disabled" } else { "enabled" }
    );

    Ok(ServerEnvironment {
        _log_guard: log_guard,
        config,
    })
}

fn load_dotenv_if_present() {
    match dotenvy::dotenv() {
        Ok(_) => {}
        Err(dotenvy::Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            eprintln!("warning: failed to load .env: {error}");
        }
    }
}

/// Layer 2: Materialize builtin skills + initialize the database.
///
/// Requires only `data_dir`. Subcommands that need persistent state
/// (database, skill files) should call this after `init_environment`.
pub async fn init_data_layer(config: &AppConfig) -> Result<Database, BootstrapError> {
    let boot = Instant::now();

    materialize_builtin_skills(&config.data_dir).await.map_err(|e| {
        BootstrapError::new(
            BootstrapErrorCode::DataInitFailed,
            "data.builtin_skills",
            "failed to initialize application data",
        )
        .with_source(e)
        .with_field("dataDir", config.data_dir.display().to_string())
    })?;
    info!(
        elapsed_ms = boot.elapsed().as_millis(),
        "startup: builtin skills materialized"
    );

    let db_path = config.database_path();
    aionui_db::maybe_copy_legacy_database(&db_path).map_err(|e| {
        BootstrapError::new(
            BootstrapErrorCode::DataInitFailed,
            "data.legacy_db",
            "failed to initialize application data",
        )
        .with_source(e)
        .with_field("databasePath", db_path.display().to_string())
    })?;
    info!("Initializing database at {}", db_path.display());
    let database = aionui_db::init_database_staged(&db_path).await.map_err(|e| {
        let stage = e.stage();
        BootstrapError::new(
            BootstrapErrorCode::DataInitFailed,
            stage,
            "failed to initialize application data",
        )
        .with_source(e.into_source())
        .with_field("databasePath", db_path.display().to_string())
    })?;
    info!(elapsed_ms = boot.elapsed().as_millis(), "startup: database initialized");

    Ok(database)
}

#[cfg(test)]
mod tests {
    #[test]
    fn database_stage_comes_from_db_boundary_error() {
        let err = aionui_db::DatabaseInitError::new(
            "database.migration",
            aionui_db::DbError::Migration(sqlx::migrate::MigrateError::VersionMismatch(42)),
        );

        assert_eq!(err.stage(), "database.migration");
    }
}
