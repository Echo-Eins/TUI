//! Cardputer Remote Desktop Server
//!
//! A secure remote desktop server for M5Stack Cardputer devices.
//!
//! ## Features
//!
//! - ECDH key exchange with secp256r1
//! - AES-128-GCM authenticated encryption
//! - Mutual authentication
//! - mDNS discovery
//! - JPEG screen capture with delta detection
//! - Mouse and keyboard simulation
//! - Event-driven architecture
//!
//! ## Security
//!
//! - All communication after handshake is encrypted with AES-128-GCM
//! - Nonce includes counter (replay protection) + random component
//! - HKDF for key derivation from ECDH shared secret
//! - Mutual authentication via ECDSA signatures
//! - Constant-time comparison for cryptographic values

pub mod capture;
pub mod config;
pub mod crypto;
pub mod input;
pub mod network;
pub mod protocol;

pub use config::Config;
pub use network::{Session, Server};

/// Application version
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Protocol version
pub const PROTOCOL_VERSION: u8 = crate::protocol::PROTOCOL_VERSION;
