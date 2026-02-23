//! Cryptography module - ECDH key exchange and AES-128-GCM encryption
//!
//! Security properties:
//! - ECDH on secp256r1 for key exchange
//! - HKDF-SHA256 for key derivation
//! - AES-128-GCM for authenticated encryption
//! - Nonce: 12 bytes (96 bits) - 4 byte counter + 8 byte random
//! - Mutual authentication via ECDSA signatures

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes128Gcm, Nonce,
};
use hkdf::Hkdf;
use p256::{
    ecdh::EphemeralSecret,
    ecdsa::{signature::Signer, signature::Verifier, Signature, SigningKey, VerifyingKey},
    elliptic_curve::sec1::{FromEncodedPoint, ToEncodedPoint},
    EncodedPoint, PublicKey,
};
use rand::{rngs::OsRng, RngCore};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::protocol::{NONCE_SIZE, TAG_SIZE};

#[derive(Debug, Error)]
pub enum CryptoError {
    #[error("Invalid key format")]
    InvalidKeyFormat,

    #[error("Key derivation failed")]
    KeyDerivationFailed,

    #[error("Encryption failed")]
    EncryptionFailed,

    #[error("Decryption failed - message may be tampered")]
    DecryptionFailed,

    #[error("Signature verification failed")]
    SignatureVerificationFailed,

    #[error("Invalid public key")]
    InvalidPublicKey,

    #[error("Nonce overflow - session must be renegotiated")]
    NonceOverflow,
}

/// Session keys derived from ECDH
pub struct SessionKeys {
    /// AES-128 key for encryption (client -> server)
    pub client_to_server_key: [u8; 16],
    /// AES-128 key for encryption (server -> client)
    pub server_to_client_key: [u8; 16],
    /// HMAC key for additional authentication
    pub hmac_key: [u8; 32],
}

/// Cryptographic context for a session
pub struct CryptoContext {
    /// Our static signing key
    signing_key: SigningKey,
    /// Our static public key (for identification)
    pub our_public_key: PublicKey,
    /// Peer's static public key (for verification)
    peer_public_key: Option<PublicKey>,
    /// Current session keys
    session_keys: Option<SessionKeys>,
    /// AES-GCM cipher for outgoing messages
    outgoing_cipher: Option<Aes128Gcm>,
    /// AES-GCM cipher for incoming messages
    incoming_cipher: Option<Aes128Gcm>,
    /// Nonce counter for outgoing messages
    outgoing_nonce_counter: u32,
    /// Nonce random part for outgoing messages
    outgoing_nonce_random: [u8; 8],
    /// Last seen incoming nonce counter (for replay protection)
    last_incoming_nonce: Option<u32>,
    /// Expected incoming nonce random part
    incoming_nonce_random: Option<[u8; 8]>,
    /// Are we the server (PC) or client (Cardputer)?
    is_server: bool,
}

impl CryptoContext {
    /// Create a new crypto context from a hex-encoded private key
    pub fn new(private_key_hex: &str, is_server: bool) -> Result<Self, CryptoError> {
        let private_key_bytes =
            hex::decode(private_key_hex).map_err(|_| CryptoError::InvalidKeyFormat)?;

        if private_key_bytes.len() != 32 {
            return Err(CryptoError::InvalidKeyFormat);
        }

        let signing_key = SigningKey::from_bytes(private_key_bytes.as_slice().into())
            .map_err(|_| CryptoError::InvalidKeyFormat)?;

        let our_public_key = *signing_key.verifying_key().as_affine();
        let our_public_key =
            PublicKey::from_affine(our_public_key).map_err(|_| CryptoError::InvalidKeyFormat)?;

        // Generate random part for nonce
        let mut outgoing_nonce_random = [0u8; 8];
        OsRng.fill_bytes(&mut outgoing_nonce_random);

        Ok(Self {
            signing_key,
            our_public_key,
            peer_public_key: None,
            session_keys: None,
            outgoing_cipher: None,
            incoming_cipher: None,
            outgoing_nonce_counter: 0,
            outgoing_nonce_random,
            last_incoming_nonce: None,
            incoming_nonce_random: None,
            is_server,
        })
    }

    /// Set the expected peer's public key (for mutual authentication)
    pub fn set_peer_public_key(&mut self, public_key_hex: &str) -> Result<(), CryptoError> {
        let public_key_bytes =
            hex::decode(public_key_hex).map_err(|_| CryptoError::InvalidKeyFormat)?;

        let encoded_point = EncodedPoint::from_bytes(&public_key_bytes)
            .map_err(|_| CryptoError::InvalidPublicKey)?;

        let public_key = Option::<PublicKey>::from(PublicKey::from_encoded_point(&encoded_point))
            .ok_or(CryptoError::InvalidPublicKey)?;

        self.peer_public_key = Some(public_key);
        Ok(())
    }

    /// Generate ephemeral keypair for ECDH
    pub fn generate_ephemeral_keypair(&self) -> (EphemeralSecret, [u8; 33]) {
        let secret = EphemeralSecret::random(&mut OsRng);
        let public_key = secret.public_key();
        let encoded = public_key.to_encoded_point(true);
        let mut compressed = [0u8; 33];
        compressed.copy_from_slice(encoded.as_bytes());
        (secret, compressed)
    }

    /// Generate a random 32-byte nonce
    pub fn generate_nonce() -> [u8; 32] {
        let mut nonce = [0u8; 32];
        OsRng.fill_bytes(&mut nonce);
        nonce
    }

    /// Sign data with our static key
    pub fn sign(&self, data: &[u8]) -> [u8; 64] {
        let signature: Signature = self.signing_key.sign(data);
        let mut sig_bytes = [0u8; 64];
        sig_bytes.copy_from_slice(&signature.to_bytes());
        sig_bytes
    }

    /// Verify signature from peer
    pub fn verify_peer_signature(
        &self,
        data: &[u8],
        signature: &[u8; 64],
    ) -> Result<(), CryptoError> {
        let peer_public = self
            .peer_public_key
            .as_ref()
            .ok_or(CryptoError::InvalidPublicKey)?;

        let verifying_key = VerifyingKey::from(peer_public);
        let sig = Signature::from_bytes(signature.into())
            .map_err(|_| CryptoError::SignatureVerificationFailed)?;

        verifying_key
            .verify(data, &sig)
            .map_err(|_| CryptoError::SignatureVerificationFailed)
    }

    /// Verify signature from an ephemeral public key during handshake
    pub fn verify_ephemeral_signature(
        &self,
        ephemeral_public_key: &[u8; 33],
        data: &[u8],
        signature: &[u8; 64],
    ) -> Result<PublicKey, CryptoError> {
        let encoded_point = EncodedPoint::from_bytes(ephemeral_public_key)
            .map_err(|_| CryptoError::InvalidPublicKey)?;

        let public_key = Option::<PublicKey>::from(PublicKey::from_encoded_point(&encoded_point))
            .ok_or(CryptoError::InvalidPublicKey)?;

        // For handshake, we verify against the stored peer public key, not the ephemeral one
        // The signature is made with the static key
        self.verify_peer_signature(data, signature)?;

        Ok(public_key)
    }

    /// Perform ECDH key exchange and derive session keys
    pub fn derive_session_keys(
        &mut self,
        our_ephemeral_secret: EphemeralSecret,
        peer_ephemeral_public: &[u8; 33],
        our_nonce: &[u8; 32],
        peer_nonce: &[u8; 32],
    ) -> Result<(), CryptoError> {
        // Parse peer's ephemeral public key
        let encoded_point = EncodedPoint::from_bytes(peer_ephemeral_public)
            .map_err(|_| CryptoError::InvalidPublicKey)?;

        let peer_public = Option::<PublicKey>::from(PublicKey::from_encoded_point(&encoded_point))
            .ok_or(CryptoError::InvalidPublicKey)?;

        // Perform ECDH
        let shared_secret = our_ephemeral_secret.diffie_hellman(&peer_public);

        // Derive keys using HKDF
        // Salt = SHA256(client_nonce || server_nonce)
        let (client_nonce, server_nonce) = if self.is_server {
            (peer_nonce, our_nonce)
        } else {
            (our_nonce, peer_nonce)
        };

        let mut hasher = Sha256::new();
        hasher.update(client_nonce);
        hasher.update(server_nonce);
        let salt = hasher.finalize();

        // Info string includes protocol identifier
        let info = b"cardputer-remote-v1-session-keys";

        let hk = Hkdf::<Sha256>::new(Some(&salt), shared_secret.raw_secret_bytes());

        // Derive 64 bytes: 16 (c2s key) + 16 (s2c key) + 32 (hmac key)
        let mut okm = [0u8; 64];
        hk.expand(info, &mut okm)
            .map_err(|_| CryptoError::KeyDerivationFailed)?;

        let mut client_to_server_key = [0u8; 16];
        client_to_server_key.copy_from_slice(&okm[0..16]);
        let mut server_to_client_key = [0u8; 16];
        server_to_client_key.copy_from_slice(&okm[16..32]);
        let mut hmac_key = [0u8; 32];
        hmac_key.copy_from_slice(&okm[32..64]);

        let session_keys = SessionKeys {
            client_to_server_key,
            server_to_client_key,
            hmac_key,
        };

        // Initialize ciphers
        let (outgoing_key, incoming_key) = if self.is_server {
            (
                &session_keys.server_to_client_key,
                &session_keys.client_to_server_key,
            )
        } else {
            (
                &session_keys.client_to_server_key,
                &session_keys.server_to_client_key,
            )
        };

        self.outgoing_cipher = Some(
            Aes128Gcm::new_from_slice(outgoing_key)
                .map_err(|_| CryptoError::KeyDerivationFailed)?,
        );
        self.incoming_cipher = Some(
            Aes128Gcm::new_from_slice(incoming_key)
                .map_err(|_| CryptoError::KeyDerivationFailed)?,
        );
        self.session_keys = Some(session_keys);

        // Reset nonce counters
        self.outgoing_nonce_counter = 0;
        self.last_incoming_nonce = None;

        // Use handshake nonces as deterministic random parts for message nonces.
        self.outgoing_nonce_random.copy_from_slice(&our_nonce[0..8]);

        // Set incoming nonce random from peer
        let mut incoming_random = [0u8; 8];
        incoming_random.copy_from_slice(&peer_nonce[0..8]);
        self.incoming_nonce_random = Some(incoming_random);

        Ok(())
    }

    /// Get the next nonce for outgoing messages
    fn next_outgoing_nonce(&mut self) -> Result<[u8; NONCE_SIZE], CryptoError> {
        if self.outgoing_nonce_counter == u32::MAX {
            return Err(CryptoError::NonceOverflow);
        }

        let mut nonce = [0u8; NONCE_SIZE];
        nonce[0..4].copy_from_slice(&self.outgoing_nonce_counter.to_be_bytes());
        nonce[4..12].copy_from_slice(&self.outgoing_nonce_random);

        self.outgoing_nonce_counter += 1;
        Ok(nonce)
    }

    /// Validate and extract counter from incoming nonce
    fn validate_incoming_nonce(&mut self, nonce: &[u8; NONCE_SIZE]) -> Result<(), CryptoError> {
        let counter = u32::from_be_bytes([nonce[0], nonce[1], nonce[2], nonce[3]]);

        // Check for replay/reordering (must be strictly greater than last seen)
        if let Some(last_counter) = self.last_incoming_nonce {
            if counter <= last_counter {
                return Err(CryptoError::DecryptionFailed);
            }
        }

        // Verify random part matches expected
        if let Some(expected_random) = &self.incoming_nonce_random {
            if &nonce[4..12] != expected_random {
                return Err(CryptoError::DecryptionFailed);
            }
        }

        self.last_incoming_nonce = Some(counter);
        Ok(())
    }

    /// Encrypt a message
    /// Returns (ciphertext, nonce, tag)
    pub fn encrypt(
        &mut self,
        plaintext: &[u8],
    ) -> Result<(Vec<u8>, [u8; NONCE_SIZE], [u8; TAG_SIZE]), CryptoError> {
        // Get nonce first (requires mutable borrow)
        let nonce_bytes = self.next_outgoing_nonce()?;

        // Then get cipher reference
        let cipher = self
            .outgoing_cipher
            .as_ref()
            .ok_or(CryptoError::EncryptionFailed)?;
        let nonce = Nonce::from(nonce_bytes);

        let ciphertext = cipher
            .encrypt(&nonce, plaintext)
            .map_err(|_| CryptoError::EncryptionFailed)?;

        // AES-GCM appends tag to ciphertext, we need to split it
        if ciphertext.len() < TAG_SIZE {
            return Err(CryptoError::EncryptionFailed);
        }

        let tag_start = ciphertext.len() - TAG_SIZE;
        let mut tag = [0u8; TAG_SIZE];
        tag.copy_from_slice(&ciphertext[tag_start..]);

        let ciphertext_only = ciphertext[..tag_start].to_vec();

        Ok((ciphertext_only, nonce_bytes, tag))
    }

    /// Decrypt a message
    pub fn decrypt(
        &mut self,
        ciphertext: &[u8],
        nonce: &[u8; NONCE_SIZE],
        tag: &[u8; TAG_SIZE],
    ) -> Result<Vec<u8>, CryptoError> {
        // Validate nonce first (replay protection)
        self.validate_incoming_nonce(nonce)?;

        let cipher = self
            .incoming_cipher
            .as_ref()
            .ok_or(CryptoError::DecryptionFailed)?;
        let nonce_obj = Nonce::from(*nonce);

        // Reconstruct ciphertext with tag appended
        let mut ciphertext_with_tag = Vec::with_capacity(ciphertext.len() + TAG_SIZE);
        ciphertext_with_tag.extend_from_slice(ciphertext);
        ciphertext_with_tag.extend_from_slice(tag);

        let plaintext = cipher
            .decrypt(&nonce_obj, ciphertext_with_tag.as_slice())
            .map_err(|_| CryptoError::DecryptionFailed)?;

        Ok(plaintext)
    }

    /// Compute HMAC-SHA256 for transcript verification
    pub fn compute_transcript_mac(&self, transcript: &[u8]) -> Result<[u8; 32], CryptoError> {
        use hmac::{digest::KeyInit, Hmac, Mac};
        use sha2::Sha256;

        type HmacSha256 = Hmac<Sha256>;

        let session_keys = self
            .session_keys
            .as_ref()
            .ok_or(CryptoError::KeyDerivationFailed)?;

        let mut mac = <HmacSha256 as KeyInit>::new_from_slice(&session_keys.hmac_key)
            .map_err(|_| CryptoError::KeyDerivationFailed)?;
        mac.update(transcript);
        Ok(mac.finalize().into_bytes().into())
    }

    /// Get our public key as compressed bytes
    pub fn get_our_public_key_compressed(&self) -> [u8; 33] {
        let encoded = self.our_public_key.to_encoded_point(true);
        let mut compressed = [0u8; 33];
        compressed.copy_from_slice(encoded.as_bytes());
        compressed
    }

    /// Check if session is established
    pub fn is_session_established(&self) -> bool {
        self.session_keys.is_some()
    }
}

/// Constant-time comparison for cryptographic values
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }

    let mut result = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        result |= x ^ y;
    }
    result == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_key_derivation() {
        // Test that two parties derive the same keys
        let server_key = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let client_key = "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";

        let mut server = CryptoContext::new(server_key, true).unwrap();
        let mut client = CryptoContext::new(client_key, false).unwrap();

        // Exchange public keys
        let server_pub = hex::encode(server.get_our_public_key_compressed());
        let client_pub = hex::encode(client.get_our_public_key_compressed());

        server.set_peer_public_key(&client_pub).unwrap();
        client.set_peer_public_key(&server_pub).unwrap();

        // Generate ephemeral keys
        let (server_eph_secret, server_eph_pub) = server.generate_ephemeral_keypair();
        let (client_eph_secret, client_eph_pub) = client.generate_ephemeral_keypair();

        // Generate nonces
        let server_nonce = CryptoContext::generate_nonce();
        let client_nonce = CryptoContext::generate_nonce();

        // Derive keys
        server
            .derive_session_keys(
                server_eph_secret,
                &client_eph_pub,
                &server_nonce,
                &client_nonce,
            )
            .unwrap();
        client
            .derive_session_keys(
                client_eph_secret,
                &server_eph_pub,
                &client_nonce,
                &server_nonce,
            )
            .unwrap();

        // Test encryption/decryption
        let message = b"Hello, Cardputer!";
        let (ciphertext, nonce, tag) = server.encrypt(message).unwrap();
        let decrypted = client.decrypt(&ciphertext, &nonce, &tag).unwrap();

        assert_eq!(message.as_slice(), decrypted.as_slice());
    }

    #[test]
    fn test_nonce_replay_protection_allows_sequential_and_blocks_replay() {
        let server_key = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let client_key = "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";

        let mut server = CryptoContext::new(server_key, true).unwrap();
        let mut client = CryptoContext::new(client_key, false).unwrap();

        let server_pub = hex::encode(server.get_our_public_key_compressed());
        let client_pub = hex::encode(client.get_our_public_key_compressed());
        server.set_peer_public_key(&client_pub).unwrap();
        client.set_peer_public_key(&server_pub).unwrap();

        let (server_eph_secret, server_eph_pub) = server.generate_ephemeral_keypair();
        let (client_eph_secret, client_eph_pub) = client.generate_ephemeral_keypair();
        let server_nonce = CryptoContext::generate_nonce();
        let client_nonce = CryptoContext::generate_nonce();

        server
            .derive_session_keys(
                server_eph_secret,
                &client_eph_pub,
                &server_nonce,
                &client_nonce,
            )
            .unwrap();
        client
            .derive_session_keys(
                client_eph_secret,
                &server_eph_pub,
                &client_nonce,
                &server_nonce,
            )
            .unwrap();

        let (ciphertext_1, nonce_1, tag_1) = server.encrypt(b"msg-1").unwrap();
        let (ciphertext_2, nonce_2, tag_2) = server.encrypt(b"msg-2").unwrap();

        let msg_1 = client.decrypt(&ciphertext_1, &nonce_1, &tag_1).unwrap();
        let msg_2 = client.decrypt(&ciphertext_2, &nonce_2, &tag_2).unwrap();

        assert_eq!(msg_1, b"msg-1");
        assert_eq!(msg_2, b"msg-2");

        let replay_result = client.decrypt(&ciphertext_1, &nonce_1, &tag_1);
        assert!(matches!(replay_result, Err(CryptoError::DecryptionFailed)));
    }

    #[test]
    fn test_transcript_mac_without_session_returns_error() {
        let key = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let crypto = CryptoContext::new(key, true).unwrap();

        let result = crypto.compute_transcript_mac(b"test-transcript");
        assert!(matches!(result, Err(CryptoError::KeyDerivationFailed)));
    }
}
