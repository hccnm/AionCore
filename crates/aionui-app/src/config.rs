//! Application configuration parsed from CLI arguments + key derivation.

use std::path::PathBuf;
use std::str::FromStr;

use sha2::{Digest, Sha256};

/// Application configuration parsed from CLI arguments.
#[derive(Debug, Clone)]
pub struct AppConfig {
    pub host: String,
    pub port: u16,
    pub data_dir: PathBuf,
    pub work_dir: PathBuf,
    pub app_version: String,
    /// Run in local embedded mode (skip authentication, use system_default_user).
    pub local: bool,
    /// Deployment mode controls persistence and workspace isolation behavior.
    pub deployment_mode: DeploymentMode,
    /// PostgreSQL connection string required by SaaS mode.
    pub database_url: Option<String>,
    /// Fixed workspace root required by SaaS mode.
    pub senmo_workspace_root: Option<PathBuf>,
    /// app_gateway downstream application id. When absent, gateway auth is disabled.
    pub gateway_app_id: Option<String>,
    /// app_gateway downstream application secret. Required when gateway auth is enabled.
    pub gateway_app_secret: Option<String>,
    /// External identity provider key used to resolve gateway user ids.
    pub gateway_provider: String,
    /// Allowed gateway timestamp skew.
    pub gateway_timestamp_skew_seconds: u64,
    /// Optional bootstrap administrator phone number for SaaS deployments.
    pub senmo_initial_admin_phone: Option<String>,
    /// Optional bootstrap administrator password for SaaS deployments.
    pub senmo_initial_admin_password: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeploymentMode {
    Desktop,
    Local,
    Saas,
}

impl FromStr for DeploymentMode {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "" | "desktop" => Ok(Self::Desktop),
            "local" => Ok(Self::Local),
            "saas" => Ok(Self::Saas),
            other => Err(format!("unsupported deployment mode '{other}'")),
        }
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ConfigValidationError {
    #[error("DEPLOYMENT_MODE=saas requires DATABASE_URL")]
    MissingDatabaseUrl,
    #[error("DEPLOYMENT_MODE=saas requires SENMO_WORKSPACE_ROOT")]
    MissingSenmoWorkspaceRoot,
    #[error("SENMO_WORKSPACE_ROOT must be creatable and a directory: {0}")]
    InvalidSenmoWorkspaceRoot(String),
    #[error("SENMO_INITIAL_ADMIN_PHONE and SENMO_INITIAL_ADMIN_PASSWORD must be configured together")]
    IncompleteInitialAdminSeed,
}

impl AppConfig {
    /// Format as `host:port` for socket binding.
    pub fn socket_addr(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    /// Path to the SQLite database file.
    pub fn database_path(&self) -> PathBuf {
        self.data_dir.join("aionui-backend.db")
    }

    pub fn effective_deployment_mode(&self) -> DeploymentMode {
        if self.local {
            DeploymentMode::Local
        } else {
            self.deployment_mode
        }
    }

    pub fn validate_saas_startup(&self) -> Result<(), ConfigValidationError> {
        if self.effective_deployment_mode() != DeploymentMode::Saas {
            return Ok(());
        }

        let database_url = self.database_url.as_deref().unwrap_or_default().trim();
        if database_url.is_empty() {
            return Err(ConfigValidationError::MissingDatabaseUrl);
        }

        let Some(root) = &self.senmo_workspace_root else {
            return Err(ConfigValidationError::MissingSenmoWorkspaceRoot);
        };

        std::fs::create_dir_all(root)
            .map_err(|error| ConfigValidationError::InvalidSenmoWorkspaceRoot(error.to_string()))?;
        let metadata = std::fs::metadata(root)
            .map_err(|error| ConfigValidationError::InvalidSenmoWorkspaceRoot(error.to_string()))?;
        if !metadata.is_dir() {
            return Err(ConfigValidationError::InvalidSenmoWorkspaceRoot(
                "path is not a directory".to_string(),
            ));
        }

        let has_admin_phone = self
            .senmo_initial_admin_phone
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty());
        let has_admin_password = self
            .senmo_initial_admin_password
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty());
        if has_admin_phone != has_admin_password {
            return Err(ConfigValidationError::IncompleteInitialAdminSeed);
        }

        Ok(())
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            host: aionui_common::constants::DEFAULT_HOST.to_string(),
            port: aionui_common::constants::DEFAULT_PORT,
            data_dir: PathBuf::from("data"),
            work_dir: PathBuf::from("data"),
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            local: false,
            deployment_mode: DeploymentMode::Desktop,
            database_url: None,
            senmo_workspace_root: None,
            gateway_app_id: None,
            gateway_app_secret: None,
            gateway_provider: "modo_open_platform".to_string(),
            gateway_timestamp_skew_seconds: 300,
            senmo_initial_admin_phone: None,
            senmo_initial_admin_password: None,
        }
    }
}

/// Derive a 32-byte encryption key from the JWT secret using SHA-256.
pub fn derive_encryption_key(jwt_secret: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"aionui-encryption-key:");
    hasher.update(jwt_secret.as_bytes());
    hasher.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_app_config_default() {
        let config = AppConfig::default();
        assert_eq!(config.host, "127.0.0.1");
        assert_eq!(config.port, 25808);
        assert_eq!(config.data_dir, PathBuf::from("data"));
        assert_eq!(config.app_version, env!("CARGO_PKG_VERSION"));
        assert_eq!(config.effective_deployment_mode(), DeploymentMode::Desktop);
    }

    #[test]
    fn test_app_config_socket_addr() {
        let config = AppConfig {
            host: "0.0.0.0".to_string(),
            port: 3000,
            ..Default::default()
        };
        assert_eq!(config.socket_addr(), "0.0.0.0:3000");
    }

    #[test]
    fn test_app_config_database_path() {
        let config = AppConfig {
            data_dir: PathBuf::from("/tmp/aionui"),
            ..Default::default()
        };
        assert_eq!(config.database_path(), PathBuf::from("/tmp/aionui/aionui-backend.db"));
    }

    #[test]
    fn local_flag_promotes_effective_mode() {
        let config = AppConfig {
            local: true,
            deployment_mode: DeploymentMode::Saas,
            ..Default::default()
        };

        assert_eq!(config.effective_deployment_mode(), DeploymentMode::Local);
        assert!(config.validate_saas_startup().is_ok());
    }

    #[test]
    fn saas_requires_database_url() {
        let config = AppConfig {
            deployment_mode: DeploymentMode::Saas,
            senmo_workspace_root: Some(PathBuf::from("target/test-senmo-root")),
            ..Default::default()
        };

        assert_eq!(
            config.validate_saas_startup(),
            Err(ConfigValidationError::MissingDatabaseUrl)
        );
    }

    #[test]
    fn saas_requires_senmo_workspace_root() {
        let config = AppConfig {
            deployment_mode: DeploymentMode::Saas,
            database_url: Some("postgres://localhost/aion".to_string()),
            ..Default::default()
        };

        assert_eq!(
            config.validate_saas_startup(),
            Err(ConfigValidationError::MissingSenmoWorkspaceRoot)
        );
    }

    #[test]
    fn saas_accepts_database_url_and_creatable_senmo_root() {
        let temp = tempfile::TempDir::new().unwrap();
        let root = temp.path().join("senmo");
        let config = AppConfig {
            deployment_mode: DeploymentMode::Saas,
            database_url: Some("postgres://localhost/aion".to_string()),
            senmo_workspace_root: Some(root.clone()),
            ..Default::default()
        };

        config.validate_saas_startup().unwrap();
        assert!(root.is_dir());
    }

    #[test]
    fn saas_rejects_incomplete_initial_admin_seed() {
        let temp = tempfile::TempDir::new().unwrap();
        let config = AppConfig {
            deployment_mode: DeploymentMode::Saas,
            database_url: Some("postgres://localhost/aion".to_string()),
            senmo_workspace_root: Some(temp.path().join("senmo")),
            senmo_initial_admin_phone: Some("13800138000".to_string()),
            senmo_initial_admin_password: None,
            ..Default::default()
        };

        assert_eq!(
            config.validate_saas_startup(),
            Err(ConfigValidationError::IncompleteInitialAdminSeed)
        );
    }

    #[test]
    fn deployment_mode_parses_supported_values() {
        assert_eq!("desktop".parse::<DeploymentMode>().unwrap(), DeploymentMode::Desktop);
        assert_eq!("local".parse::<DeploymentMode>().unwrap(), DeploymentMode::Local);
        assert_eq!("saas".parse::<DeploymentMode>().unwrap(), DeploymentMode::Saas);
        assert_eq!("".parse::<DeploymentMode>().unwrap(), DeploymentMode::Desktop);
    }

    #[test]
    fn deployment_mode_rejects_unknown_value() {
        assert!("prod".parse::<DeploymentMode>().is_err());
    }
}
