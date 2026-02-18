//! Network module - TCP server and mDNS discovery
//!
//! Full PKI authentication required for all clients.
//! Handshake format: pubkey(33) + nonce(32) + signature(64) = 129 bytes

use crate::config::Config;
use crate::crypto::{constant_time_eq, CryptoContext, CryptoError};
use crate::protocol::{
    DiscoveryResponse, Packet, PacketHeader, PacketType, HEADER_SIZE, NONCE_SIZE, TAG_SIZE,
};
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

/// Binary handshake format sizes (same for all clients)
const PUBKEY_SIZE: usize = 33;        // Compressed secp256r1
const HANDSHAKE_NONCE_SIZE: usize = 32;
const SIGNATURE_SIZE: usize = 64;     // ECDSA r||s format
const HANDSHAKE_MSG_SIZE: usize = PUBKEY_SIZE + HANDSHAKE_NONCE_SIZE + SIGNATURE_SIZE;  // 129 bytes
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

pub mod session;
pub use session::Session;

#[derive(Debug, Error)]
pub enum NetworkError {
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
    #[error("mDNS error: {0}")]
    MdnsError(String),
    #[error("Protocol error: {0}")]
    ProtocolError(#[from] crate::protocol::ProtocolError),
    #[error("Crypto error: {0}")]
    CryptoError(#[from] CryptoError),
    #[error("Connection closed")]
    ConnectionClosed,
    #[error("Handshake failed: {0}")]
    HandshakeFailed(String),
    #[error("Invalid cookie")]
    InvalidCookie,
    #[error("Timeout")]
    Timeout,
    #[error("Session expired")]
    SessionExpired,
}

pub struct DiscoveryService {
    daemon: ServiceDaemon,
    service_type: String,
    device_name: String,
    cookie: [u8; 16],
    port: u16,
    running: Arc<AtomicBool>,
}

impl DiscoveryService {
    pub fn new(config: &Config) -> Result<Self, NetworkError> {
        let daemon = ServiceDaemon::new().map_err(|e| NetworkError::MdnsError(e.to_string()))?;
        let service_type = format!("_{}._tcp.local.", config.network.mdns_service_name.to_lowercase());

        Ok(Self {
            daemon,
            service_type,
            device_name: config.network.device_name.clone(),
            cookie: config.get_discovery_cookie(),
            port: config.server.port,
            running: Arc::new(AtomicBool::new(false)),
        })
    }

    pub async fn start(&self) -> Result<(), NetworkError> {
        self.running.store(true, Ordering::Relaxed);
        let receiver = self.daemon.browse(&self.service_type)
            .map_err(|e| NetworkError::MdnsError(e.to_string()))?;

        info!("mDNS discovery started for {}", self.service_type);

        let service_info = ServiceInfo::new(
            &self.service_type, &self.device_name,
            &format!("{}.local.", self.device_name), "", self.port, None,
        ).map_err(|e| NetworkError::MdnsError(e.to_string()))?;

        self.daemon.register(service_info).map_err(|e| NetworkError::MdnsError(e.to_string()))?;

        let running = self.running.clone();
        tokio::spawn(async move {
            while running.load(Ordering::Relaxed) {
                match receiver.recv_timeout(Duration::from_millis(100)) {
                    Ok(ServiceEvent::ServiceResolved(info)) => debug!("Discovered: {:?}", info),
                    Ok(ServiceEvent::SearchStarted(_)) => debug!("mDNS search started"),
                    _ => {}
                }
            }
        });
        Ok(())
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::Relaxed);
        let _ = self.daemon.shutdown();
    }

    pub fn validate_cookie(&self, request_cookie: &[u8]) -> bool {
        constant_time_eq(&self.cookie, request_cookie)
    }

    pub fn create_response(&self) -> DiscoveryResponse {
        DiscoveryResponse {
            cookie: self.cookie.to_vec(),
            device_name: self.device_name.clone(),
            server_port: self.port,
        }
    }
}

pub struct Server {
    listener: TcpListener,
    config: Arc<Config>,
    running: Arc<AtomicBool>,
}

impl Server {
    pub async fn new(config: Arc<Config>) -> Result<Self, NetworkError> {
        let addr = format!("{}:{}", config.network.bind_address, config.server.port);
        let listener = TcpListener::bind(&addr).await?;
        info!("TCP server listening on {}", addr);
        Ok(Self { listener, config, running: Arc::new(AtomicBool::new(true)) })
    }

    pub async fn run(&self, session_tx: mpsc::Sender<Session>) -> Result<(), NetworkError> {
        while self.running.load(Ordering::Relaxed) {
            tokio::select! {
                result = self.listener.accept() => {
                    match result {
                        Ok((stream, addr)) => {
                            info!("New connection from {}", addr);
                            let config = self.config.clone();
                            let tx = session_tx.clone();
                            tokio::spawn(async move {
                                match Self::handle_connection(stream, addr, config).await {
                                    Ok(session) => { let _ = tx.send(session).await; }
                                    Err(e) => warn!("Connection {} failed: {}", addr, e),
                                }
                            });
                        }
                        Err(e) => error!("Accept error: {}", e),
                    }
                }
            }
        }
        Ok(())
    }

    async fn handle_connection(
        mut stream: TcpStream, addr: SocketAddr, config: Arc<Config>,
    ) -> Result<Session, NetworkError> {
        let mut crypto = CryptoContext::new(&config.security.private_key, true)?;

        // Set expected client public key for signature verification
        crypto.set_peer_public_key(&config.security.cardputer_public_key)?;

        // ===== RECEIVE FIRST PACKET (DiscoveryRequest or HandshakeInit) =====
        let first_packet = Self::receive_packet_with_timeout(&mut stream).await?;

        // Handle DiscoveryRequest if sent
        if first_packet.header.packet_type == PacketType::DiscoveryRequest {
            info!("Received DiscoveryRequest from {}", addr);

            // Validate cookie (first 16 bytes of payload)
            if first_packet.payload.len() < 16 {
                return Err(NetworkError::HandshakeFailed("DiscoveryRequest too short".into()));
            }

            let request_cookie = &first_packet.payload[..16];
            let expected_cookie = config.get_discovery_cookie();

            if !constant_time_eq(request_cookie, &expected_cookie) {
                warn!("Invalid discovery cookie from {}", addr);
                // Send error response
                Self::send_unencrypted_packet(&mut stream, PacketType::ErrorPacket, b"Invalid cookie").await?;
                return Err(NetworkError::InvalidCookie);
            }

            info!("Discovery cookie validated from {}", addr);

            // Send DiscoveryResponse: cookie(16) + device_name + port(2)
            let device_name = config.network.device_name.as_bytes();
            let mut response = Vec::with_capacity(16 + device_name.len() + 2);
            response.extend_from_slice(&expected_cookie);
            response.extend_from_slice(device_name);
            response.push((config.server.port >> 8) as u8);
            response.push((config.server.port & 0xFF) as u8);

            Self::send_unencrypted_packet(&mut stream, PacketType::DiscoveryResponse, &response).await?;
            info!("Sent DiscoveryResponse to {}", addr);

            // Now wait for HandshakeInit
            let init_packet = Self::receive_packet_with_timeout(&mut stream).await?;
            if init_packet.header.packet_type != PacketType::HandshakeInit {
                return Err(NetworkError::HandshakeFailed(
                    format!("Expected HandshakeInit after discovery, got {:?}", init_packet.header.packet_type)
                ));
            }

            return Self::process_handshake(stream, addr, config, crypto, init_packet).await;
        }

        // Direct HandshakeInit (without discovery)
        if first_packet.header.packet_type != PacketType::HandshakeInit {
            return Err(NetworkError::HandshakeFailed(
                format!("Expected HandshakeInit or DiscoveryRequest, got {:?}", first_packet.header.packet_type)
            ));
        }

        Self::process_handshake(stream, addr, config, crypto, first_packet).await
    }

    async fn process_handshake(
        mut stream: TcpStream, addr: SocketAddr, config: Arc<Config>,
        mut crypto: CryptoContext, init_packet: Packet,
    ) -> Result<Session, NetworkError> {
        // ===== PROCESS HANDSHAKE INIT =====
        // Format: pubkey(33) + nonce(32) + signature(64) = 129 bytes

        if init_packet.payload.len() != HANDSHAKE_MSG_SIZE {
            return Err(NetworkError::HandshakeFailed(
                format!("HandshakeInit wrong size: {} (expected {})", init_packet.payload.len(), HANDSHAKE_MSG_SIZE)
            ));
        }

        // Parse components
        let mut init_pubkey = [0u8; PUBKEY_SIZE];
        let mut init_nonce = [0u8; HANDSHAKE_NONCE_SIZE];
        let mut init_signature = [0u8; SIGNATURE_SIZE];
        init_pubkey.copy_from_slice(&init_packet.payload[..PUBKEY_SIZE]);
        init_nonce.copy_from_slice(&init_packet.payload[PUBKEY_SIZE..PUBKEY_SIZE + HANDSHAKE_NONCE_SIZE]);
        init_signature.copy_from_slice(&init_packet.payload[PUBKEY_SIZE + HANDSHAKE_NONCE_SIZE..]);

        // ===== VERIFY CLIENT SIGNATURE =====
        // Client signs: ephemeral_pubkey || nonce
        let mut sign_data = Vec::with_capacity(PUBKEY_SIZE + HANDSHAKE_NONCE_SIZE);
        sign_data.extend_from_slice(&init_pubkey);
        sign_data.extend_from_slice(&init_nonce);

        crypto.verify_peer_signature(&sign_data, &init_signature)?;
        info!("Client signature verified from {}", addr);

        // ===== GENERATE OUR RESPONSE =====
        let (our_ephemeral_secret, our_ephemeral_public) = crypto.generate_ephemeral_keypair();
        let our_nonce = CryptoContext::generate_nonce();

        // Sign: our_ephemeral_pubkey || client_nonce || our_nonce
        let mut response_sign_data = Vec::with_capacity(PUBKEY_SIZE + HANDSHAKE_NONCE_SIZE + HANDSHAKE_NONCE_SIZE);
        response_sign_data.extend_from_slice(&our_ephemeral_public);
        response_sign_data.extend_from_slice(&init_nonce);
        response_sign_data.extend_from_slice(&our_nonce);
        let response_signature = crypto.sign(&response_sign_data);

        // Build response: pubkey(33) + nonce(32) + signature(64)
        let mut response_bytes = Vec::with_capacity(HANDSHAKE_MSG_SIZE);
        response_bytes.extend_from_slice(&our_ephemeral_public);
        response_bytes.extend_from_slice(&our_nonce);
        response_bytes.extend_from_slice(&response_signature);

        Self::send_unencrypted_packet(&mut stream, PacketType::HandshakeResponse, &response_bytes).await?;
        info!("Sent signed HandshakeResponse to {}", addr);

        // ===== DERIVE SESSION KEYS =====
        crypto.derive_session_keys(our_ephemeral_secret, &init_pubkey, &our_nonce, &init_nonce)?;
        info!("Session keys derived for {}", addr);

        // ===== RECEIVE ENCRYPTED HANDSHAKE COMPLETE =====
        let complete_packet = Self::receive_packet_with_timeout(&mut stream).await?;
        if complete_packet.header.packet_type != PacketType::HandshakeComplete {
            return Err(NetworkError::HandshakeFailed("Expected HandshakeComplete".into()));
        }

        if complete_packet.payload.len() < NONCE_SIZE {
            return Err(NetworkError::HandshakeFailed("HandshakeComplete too short".into()));
        }

        // Decrypt
        let mut nonce = [0u8; NONCE_SIZE];
        nonce.copy_from_slice(&complete_packet.payload[..NONCE_SIZE]);
        let ciphertext = &complete_packet.payload[NONCE_SIZE..];

        let plaintext = crypto.decrypt(ciphertext, &nonce, &complete_packet.tag)?;

        // Verify transcript MAC (plaintext should be 32-byte MAC)
        if plaintext.len() != 32 {
            return Err(NetworkError::HandshakeFailed("Invalid transcript MAC size".into()));
        }

        // Compute expected MAC: HMAC(hmac_key, client_pub || server_pub || client_nonce || server_nonce)
        let mut transcript = Vec::with_capacity(PUBKEY_SIZE * 2 + HANDSHAKE_NONCE_SIZE * 2);
        transcript.extend_from_slice(&init_pubkey);
        transcript.extend_from_slice(&our_ephemeral_public);
        transcript.extend_from_slice(&init_nonce);
        transcript.extend_from_slice(&our_nonce);

        let expected_mac = crypto.compute_transcript_mac(&transcript)?;
        if !constant_time_eq(&plaintext, &expected_mac) {
            return Err(NetworkError::HandshakeFailed("Transcript MAC verification failed".into()));
        }

        info!("Transcript MAC verified for {}", addr);

        info!("Handshake complete with {} - secure channel established", addr);
        Ok(Session::new(stream, addr, crypto, config))
    }

    async fn receive_packet_with_timeout(stream: &mut TcpStream) -> Result<Packet, NetworkError> {
        match tokio::time::timeout(HANDSHAKE_TIMEOUT, Self::receive_packet(stream)).await {
            Ok(result) => result,
            Err(_) => Err(NetworkError::Timeout),
        }
    }

    /// Receive packet: header(4) + payload + tag(16)
    async fn receive_packet(stream: &mut TcpStream) -> Result<Packet, NetworkError> {
        let mut header_buf = [0u8; HEADER_SIZE];
        stream.read_exact(&mut header_buf).await?;
        let header = PacketHeader::from_bytes(&header_buf)?;

        let mut payload = vec![0u8; header.length as usize];
        stream.read_exact(&mut payload).await?;

        let mut tag = [0u8; TAG_SIZE];
        stream.read_exact(&mut tag).await?;

        Ok(Packet { header, payload, tag })
    }

    /// Send unencrypted packet: header(4) + payload + zero_tag(16)
    async fn send_unencrypted_packet(stream: &mut TcpStream, packet_type: PacketType, payload: &[u8]) -> Result<(), NetworkError> {
        let header = PacketHeader::new(packet_type, payload.len())?;
        stream.write_all(&header.to_bytes()).await?;
        stream.write_all(payload).await?;
        stream.write_all(&[0u8; TAG_SIZE]).await?;
        stream.flush().await?;
        Ok(())
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::Relaxed);
    }
}

pub struct Connection {
    stream: TcpStream,
    crypto: CryptoContext,
}

impl Connection {
    pub fn new(stream: TcpStream, crypto: CryptoContext) -> Self {
        Self { stream, crypto }
    }

    pub async fn send(&mut self, packet_type: PacketType, payload: &[u8]) -> Result<(), NetworkError> {
        let (ciphertext, nonce, tag) = self.crypto.encrypt(payload)?;

        let mut full_payload = Vec::with_capacity(NONCE_SIZE + ciphertext.len());
        full_payload.extend_from_slice(&nonce);
        full_payload.extend_from_slice(&ciphertext);

        let header = PacketHeader::new(packet_type, full_payload.len())?;
        self.stream.write_all(&header.to_bytes()).await?;
        self.stream.write_all(&full_payload).await?;
        self.stream.write_all(&tag).await?;
        self.stream.flush().await?;
        Ok(())
    }

    pub async fn receive(&mut self) -> Result<(PacketType, Vec<u8>), NetworkError> {
        let mut header_buf = [0u8; HEADER_SIZE];
        match self.stream.read_exact(&mut header_buf).await {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                return Err(NetworkError::ConnectionClosed);
            }
            Err(e) => return Err(e.into()),
        }

        let header = PacketHeader::from_bytes(&header_buf)?;
        let payload_size = header.length as usize;

        if payload_size < NONCE_SIZE {
            return Err(NetworkError::ProtocolError(
                crate::protocol::ProtocolError::IncompletePacket { expected: NONCE_SIZE, got: payload_size }
            ));
        }

        let mut payload = vec![0u8; payload_size];
        self.stream.read_exact(&mut payload).await?;

        let mut tag = [0u8; TAG_SIZE];
        self.stream.read_exact(&mut tag).await?;

        let mut nonce = [0u8; NONCE_SIZE];
        nonce.copy_from_slice(&payload[..NONCE_SIZE]);
        let ciphertext = &payload[NONCE_SIZE..];

        let plaintext = self.crypto.decrypt(ciphertext, &nonce, &tag)?;
        Ok((header.packet_type, plaintext))
    }

    pub fn is_encrypted(&self) -> bool {
        self.crypto.is_session_established()
    }
}
