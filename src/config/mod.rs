//! Configuration module - loads and validates settings from TOML

use p256::{ecdsa::SigningKey, elliptic_curve::sec1::FromEncodedPoint, EncodedPoint, PublicKey};
use rand::{rngs::OsRng, RngCore};
use serde::{Deserialize, Serialize};
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("Failed to read config file: {0}")]
    ReadError(#[from] std::io::Error),

    #[error("Failed to parse config: {0}")]
    ParseError(#[from] toml::de::Error),

    #[error("Invalid config: {0}")]
    ValidationError(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    pub security: SecurityConfig,
    pub network: NetworkConfig,
    pub display: DisplayConfig,
    pub logging: LoggingConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    /// TCP port for connections
    pub port: u16,

    /// Session timeout in seconds (0 = never)
    #[serde(default)]
    pub session_timeout_secs: u64,

    /// Maximum FPS for screen updates
    #[serde(default = "default_max_fps")]
    pub max_fps: u32,

    /// JPEG quality (1-100)
    #[serde(default = "default_jpeg_quality")]
    pub jpeg_quality: u8,
}

fn default_max_fps() -> u32 {
    10
}

fn default_jpeg_quality() -> u8 {
    70
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityConfig {
    /// 16-byte discovery cookie (hex encoded)
    pub discovery_cookie: String,

    /// Private key for ECDH (secp256r1, hex encoded, 32 bytes)
    pub private_key: String,

    /// Expected Cardputer public key (compressed, hex encoded, 33 bytes)
    pub cardputer_public_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkConfig {
    /// mDNS service name
    #[serde(default = "default_mdns_service_name")]
    pub mdns_service_name: String,

    /// Device name shown during discovery
    pub device_name: String,

    /// Bind address
    #[serde(default = "default_bind_address")]
    pub bind_address: String,
}

fn default_mdns_service_name() -> String {
    "cardputer-remote".to_string()
}

fn default_bind_address() -> String {
    "127.0.0.1".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisplayConfig {
    /// Target width for Cardputer display
    #[serde(default = "default_target_width")]
    pub target_width: u32,

    /// Target height for Cardputer display
    #[serde(default = "default_target_height")]
    pub target_height: u32,

    /// Optional capture region [x, y, width, height]
    pub capture_region: Option<[u32; 4]>,
}

fn default_target_width() -> u32 {
    240
}

fn default_target_height() -> u32 {
    135
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingConfig {
    /// Log level
    #[serde(default = "default_log_level")]
    pub level: String,

    /// Log file path (None = stdout only)
    pub log_file: Option<String>,

    /// Use JSON format
    #[serde(default)]
    pub json_format: bool,
}

fn default_log_level() -> String {
    "info".to_string()
}

impl Config {
    /// Load configuration from a TOML file
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, ConfigError> {
        let content = std::fs::read_to_string(path)?;
        let config: Config = toml::from_str(&content)?;
        config.validate()?;
        Ok(config)
    }

    /// Load configuration from string
    pub fn from_str(content: &str) -> Result<Self, ConfigError> {
        let config: Config = toml::from_str(content)?;
        config.validate()?;
        Ok(config)
    }

    /// Validate configuration values
    fn validate(&self) -> Result<(), ConfigError> {
        // Validate discovery cookie (16 bytes = 32 hex chars)
        let cookie_bytes = hex::decode(&self.security.discovery_cookie)
            .map_err(|_| ConfigError::ValidationError("Invalid discovery_cookie hex".into()))?;
        if cookie_bytes.len() != 16 {
            return Err(ConfigError::ValidationError(
                "discovery_cookie must be 16 bytes (32 hex chars)".into(),
            ));
        }
        if is_all_zero(&cookie_bytes) {
            return Err(ConfigError::ValidationError(
                "discovery_cookie must not be all zero; run `cargo run --bin keygen`".into(),
            ));
        }

        // Validate private key (32 bytes = 64 hex chars)
        let key_bytes = hex::decode(&self.security.private_key)
            .map_err(|_| ConfigError::ValidationError("Invalid private_key hex".into()))?;
        if key_bytes.len() != 32 {
            return Err(ConfigError::ValidationError(
                "private_key must be 32 bytes (64 hex chars)".into(),
            ));
        }
        if is_all_zero(&key_bytes) {
            return Err(ConfigError::ValidationError(
                "private_key must not be all zero; run `cargo run --bin keygen`".into(),
            ));
        }
        SigningKey::from_bytes(key_bytes.as_slice().into()).map_err(|_| {
            ConfigError::ValidationError("private_key is not a valid P-256 signing key".into())
        })?;

        // Validate Cardputer public key (33 bytes compressed = 66 hex chars)
        let pubkey_bytes = hex::decode(&self.security.cardputer_public_key)
            .map_err(|_| ConfigError::ValidationError("Invalid cardputer_public_key hex".into()))?;
        if pubkey_bytes.len() != 33 {
            return Err(ConfigError::ValidationError(
                "cardputer_public_key must be 33 bytes compressed (66 hex chars)".into(),
            ));
        }
        if is_all_zero(&pubkey_bytes) {
            return Err(ConfigError::ValidationError(
                "cardputer_public_key must not be all zero; run `cargo run --bin keygen`".into(),
            ));
        }
        let encoded_point = EncodedPoint::from_bytes(&pubkey_bytes).map_err(|_| {
            ConfigError::ValidationError("cardputer_public_key is not a valid SEC1 point".into())
        })?;
        let public_key = PublicKey::from_encoded_point(&encoded_point);
        if Option::<PublicKey>::from(public_key).is_none() {
            return Err(ConfigError::ValidationError(
                "cardputer_public_key is not a valid P-256 public key".into(),
            ));
        }

        // Validate port
        if self.server.port == 0 {
            return Err(ConfigError::ValidationError("port cannot be 0".into()));
        }

        // Validate JPEG quality
        if self.server.jpeg_quality == 0 || self.server.jpeg_quality > 100 {
            return Err(ConfigError::ValidationError(
                "jpeg_quality must be 1-100".into(),
            ));
        }

        // Validate max FPS
        if self.server.max_fps == 0 || self.server.max_fps > 60 {
            return Err(ConfigError::ValidationError("max_fps must be 1-60".into()));
        }

        // Validate display dimensions
        if self.display.target_width == 0 || self.display.target_height == 0 {
            return Err(ConfigError::ValidationError(
                "Display dimensions must be positive".into(),
            ));
        }

        Ok(())
    }

    /// Get discovery cookie as bytes
    pub fn get_discovery_cookie(&self) -> [u8; 16] {
        let mut cookie = [0u8; 16];
        if let Ok(bytes) = hex::decode(&self.security.discovery_cookie) {
            if bytes.len() == cookie.len() {
                cookie.copy_from_slice(&bytes);
            }
        }
        cookie
    }

    /// Get minimum frame interval based on max_fps
    pub fn get_frame_interval_ms(&self) -> u64 {
        1000 / self.server.max_fps as u64
    }
}

fn is_all_zero(bytes: &[u8]) -> bool {
    bytes.iter().all(|byte| *byte == 0)
}

fn random_cookie_hex() -> String {
    let mut cookie = [0u8; 16];
    OsRng.fill_bytes(&mut cookie);
    hex::encode(cookie)
}

fn random_private_key_hex() -> String {
    let signing_key = SigningKey::random(&mut OsRng);
    hex::encode(signing_key.to_bytes())
}

fn random_public_key_hex() -> String {
    let signing_key = SigningKey::random(&mut OsRng);
    hex::encode(
        signing_key
            .verifying_key()
            .to_encoded_point(true)
            .as_bytes(),
    )
}

impl Default for Config {
    fn default() -> Self {
        Self {
            server: ServerConfig {
                port: 19847,
                session_timeout_secs: 0,
                max_fps: 10,
                jpeg_quality: 70,
            },
            security: SecurityConfig {
                discovery_cookie: random_cookie_hex(),
                private_key: random_private_key_hex(),
                cardputer_public_key: random_public_key_hex(),
            },
            network: NetworkConfig {
                mdns_service_name: "CardputerRemote".to_string(),
                device_name: "PC".to_string(),
                bind_address: default_bind_address(),
            },
            display: DisplayConfig {
                target_width: 240,
                target_height: 135,
                capture_region: None,
            },
            logging: LoggingConfig {
                level: "info".to_string(),
                log_file: None,
                json_format: false,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn public_key_hex(private_key_hex: &str) -> String {
        let private_key_bytes = hex::decode(private_key_hex).expect("private key hex");
        let signing_key =
            SigningKey::from_bytes(private_key_bytes.as_slice().into()).expect("signing key");
        hex::encode(
            signing_key
                .verifying_key()
                .to_encoded_point(true)
                .as_bytes(),
        )
    }

    #[test]
    fn test_config_load() {
        let private_key = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let cardputer_public_key = public_key_hex(private_key);
        let toml = r#"
[server]
port = 19847
session_timeout_secs = 60
max_fps = 10
jpeg_quality = 70

[security]
discovery_cookie = "a1b2c3d4e5f6789012345678deadbeef"
private_key = "$PRIVATE_KEY"
cardputer_public_key = "$CARDPUTER_PUBLIC_KEY"

[network]
mdns_service_name = "CardputerRemote"
device_name = "TestPC"
bind_address = "0.0.0.0"

[display]
target_width = 240
target_height = 135

[logging]
level = "debug"
"#
        .replace("$PRIVATE_KEY", private_key)
        .replace("$CARDPUTER_PUBLIC_KEY", &cardputer_public_key);

        let config = Config::from_str(&toml).unwrap();
        assert_eq!(config.server.port, 19847);
        assert_eq!(config.server.session_timeout_secs, 60);
        assert_eq!(config.network.device_name, "TestPC");
    }

    #[test]
    fn test_config_rejects_all_zero_security_values() {
        let toml = r#"
[server]
port = 19847
max_fps = 10
jpeg_quality = 70

[security]
discovery_cookie = "00000000000000000000000000000000"
private_key = "0000000000000000000000000000000000000000000000000000000000000000"
cardputer_public_key = "000000000000000000000000000000000000000000000000000000000000000000"

[network]
device_name = "TestPC"

[display]
target_width = 240
target_height = 135

[logging]
level = "debug"
"#;

        let err = Config::from_str(toml).unwrap_err();
        assert!(
            err.to_string()
                .contains("discovery_cookie must not be all zero"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn test_config_rejects_invalid_public_key_point() {
        let private_key = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let toml = format!(
            r#"
[server]
port = 19847
max_fps = 10
jpeg_quality = 70

[security]
discovery_cookie = "a1b2c3d4e5f6789012345678deadbeef"
private_key = "{private_key}"
cardputer_public_key = "02ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"

[network]
device_name = "TestPC"

[display]
target_width = 240
target_height = 135

[logging]
level = "debug"
"#
        );

        let err = Config::from_str(&toml).unwrap_err();
        assert!(
            err.to_string()
                .contains("cardputer_public_key is not a valid"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn test_default_config_uses_valid_nonzero_security_values() {
        let config = Config::default();
        config.validate().expect("default config validates");

        assert_ne!(config.security.discovery_cookie, "0".repeat(32));
        assert_ne!(config.security.private_key, "0".repeat(64));
        assert_ne!(config.network.bind_address, "0.0.0.0");
    }
}
