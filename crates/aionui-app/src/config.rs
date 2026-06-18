//! Application configuration parsed from CLI arguments + key derivation.

use std::path::PathBuf;
use std::str::FromStr;

use sha2::{Digest, Sha256};

/// Runtime deployment profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeploymentMode {
    /// Embedded/local mode skips remote auth and CORS/CSRF protections.
    Local,
    /// Desktop/web-host compatibility mode keeps cookie fallback and CSRF.
    Desktop,
    /// SaaS mode enables remote frontend CORS and Bearer-first auth.
    Saas,
}

impl DeploymentMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Desktop => "desktop",
            Self::Saas => "saas",
        }
    }

    pub fn is_local(self) -> bool {
        matches!(self, Self::Local)
    }

    pub fn is_saas(self) -> bool {
        matches!(self, Self::Saas)
    }

    /// Resolve from `DEPLOYMENT_MODE` when set, otherwise fall back to the
    /// legacy `--local` flag for backward compatibility.
    pub fn from_env_or_local(local: bool) -> Result<Self, DeploymentModeParseError> {
        match std::env::var("DEPLOYMENT_MODE")
            .ok()
            .filter(|value| !value.trim().is_empty())
        {
            Some(value) => value.parse(),
            None if local => Ok(Self::Local),
            None => Ok(Self::Desktop),
        }
    }
}

impl FromStr for DeploymentMode {
    type Err = DeploymentModeParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "local" => Ok(Self::Local),
            "desktop" => Ok(Self::Desktop),
            "saas" => Ok(Self::Saas),
            _ => Err(DeploymentModeParseError(value.to_owned())),
        }
    }
}

impl std::fmt::Display for DeploymentMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Invalid `DEPLOYMENT_MODE` value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeploymentModeParseError(String);

impl DeploymentModeParseError {
    pub fn value(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for DeploymentModeParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "invalid deployment mode {:?}; expected one of: local, desktop, saas",
            self.0
        )
    }
}

impl std::error::Error for DeploymentModeParseError {}

/// Application configuration parsed from CLI arguments.
#[derive(Debug, Clone)]
pub struct AppConfig {
    pub host: String,
    pub port: u16,
    pub data_dir: PathBuf,
    pub config_path: Option<PathBuf>,
    pub public_base_url: Option<String>,
    pub work_dir: PathBuf,
    pub app_version: String,
    pub deployment_mode: DeploymentMode,
    /// Run in local embedded mode (skip authentication, use system_default_user).
    pub local: bool,
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

    /// Deployment mode after applying the legacy `local` compatibility flag.
    pub fn effective_deployment_mode(&self) -> DeploymentMode {
        if self.local {
            DeploymentMode::Local
        } else {
            self.deployment_mode
        }
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            host: aionui_common::constants::DEFAULT_HOST.to_string(),
            port: aionui_common::constants::DEFAULT_PORT,
            data_dir: PathBuf::from("data"),
            config_path: None,
            public_base_url: None,
            work_dir: PathBuf::from("data"),
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            deployment_mode: DeploymentMode::Desktop,
            local: false,
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
        assert_eq!(config.config_path, None);
        assert_eq!(config.public_base_url, None);
        assert_eq!(config.app_version, env!("CARGO_PKG_VERSION"));
        assert_eq!(config.deployment_mode, DeploymentMode::Desktop);
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
    fn deployment_mode_parses_supported_values() {
        assert_eq!("local".parse::<DeploymentMode>().unwrap(), DeploymentMode::Local);
        assert_eq!("desktop".parse::<DeploymentMode>().unwrap(), DeploymentMode::Desktop);
        assert_eq!("saas".parse::<DeploymentMode>().unwrap(), DeploymentMode::Saas);
        assert_eq!("SAAS".parse::<DeploymentMode>().unwrap(), DeploymentMode::Saas);
    }

    #[test]
    fn deployment_mode_rejects_unknown_value() {
        let err = "production".parse::<DeploymentMode>().unwrap_err();
        assert_eq!(err.value(), "production");
        assert!(err.to_string().contains("local, desktop, saas"));
    }

    #[test]
    fn local_flag_promotes_effective_mode() {
        let config = AppConfig {
            local: true,
            ..Default::default()
        };
        assert_eq!(config.effective_deployment_mode(), DeploymentMode::Local);
    }
}
