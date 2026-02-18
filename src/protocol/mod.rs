//! Protocol module - defines wire format for Cardputer Remote
//!
//! Packet format:
//! [1 byte version][1 byte type][2 bytes length (BE)][payload][16 bytes AES-GCM tag]

use serde::{Deserialize, Serialize};
use serde_with::{serde_as, Bytes};
use thiserror::Error;

/// Protocol version
pub const PROTOCOL_VERSION: u8 = 1;
pub const MAX_PAYLOAD_SIZE: usize = 65515;
pub const HEADER_SIZE: usize = 4;
pub const TAG_SIZE: usize = 16;
pub const NONCE_SIZE: usize = 12;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PacketType {
    DiscoveryRequest = 0x00,
    DiscoveryResponse = 0x01,
    HandshakeInit = 0x02,
    HandshakeResponse = 0x03,
    HandshakeComplete = 0x04,
    SessionStart = 0x10,
    SessionEnd = 0x11,
    SessionTimeout = 0x12,
    Heartbeat = 0x13,
    HeartbeatAck = 0x14,
    ScreenFrame = 0x20,
    ScreenDelta = 0x21,
    ScreenRequest = 0x22,
    MouseMove = 0x30,
    MouseClick = 0x31,
    KeyPress = 0x32,
    KeyRelease = 0x33,
    KeyType = 0x34,
    ModeSwitch = 0x40,
    ModeAck = 0x41,
    ErrorPacket = 0xF0,
}

impl TryFrom<u8> for PacketType {
    type Error = ProtocolError;

    fn try_from(value: u8) -> Result<Self, ProtocolError> {
        match value {
            0x00 => Ok(PacketType::DiscoveryRequest),
            0x01 => Ok(PacketType::DiscoveryResponse),
            0x02 => Ok(PacketType::HandshakeInit),
            0x03 => Ok(PacketType::HandshakeResponse),
            0x04 => Ok(PacketType::HandshakeComplete),
            0x10 => Ok(PacketType::SessionStart),
            0x11 => Ok(PacketType::SessionEnd),
            0x12 => Ok(PacketType::SessionTimeout),
            0x13 => Ok(PacketType::Heartbeat),
            0x14 => Ok(PacketType::HeartbeatAck),
            0x20 => Ok(PacketType::ScreenFrame),
            0x21 => Ok(PacketType::ScreenDelta),
            0x22 => Ok(PacketType::ScreenRequest),
            0x30 => Ok(PacketType::MouseMove),
            0x31 => Ok(PacketType::MouseClick),
            0x32 => Ok(PacketType::KeyPress),
            0x33 => Ok(PacketType::KeyRelease),
            0x34 => Ok(PacketType::KeyType),
            0x40 => Ok(PacketType::ModeSwitch),
            0x41 => Ok(PacketType::ModeAck),
            0xF0 => Ok(PacketType::ErrorPacket),
            _ => Err(ProtocolError::InvalidPacketType(value)),
        }
    }
}

#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("Invalid protocol version: {0}")]
    InvalidVersion(u8),
    #[error("Invalid packet type: 0x{0:02X}")]
    InvalidPacketType(u8),
    #[error("Payload too large: {0}")]
    PayloadTooLarge(usize),
    #[error("Incomplete packet: expected {expected}, got {got}")]
    IncompletePacket { expected: usize, got: usize },
    #[error("Decryption failed")]
    DecryptionFailed,
    #[error("Invalid nonce")]
    InvalidNonce,
    #[error("Serialization error: {0}")]
    SerializationError(String),
    #[error("Invalid data length")]
    InvalidDataLength,
}

#[derive(Debug, Clone, Copy)]
pub struct PacketHeader {
    pub version: u8,
    pub packet_type: PacketType,
    pub length: u16,
}

impl PacketHeader {
    pub fn new(packet_type: PacketType, payload_len: usize) -> Result<Self, ProtocolError> {
        if payload_len > MAX_PAYLOAD_SIZE {
            return Err(ProtocolError::PayloadTooLarge(payload_len));
        }
        Ok(Self {
            version: PROTOCOL_VERSION,
            packet_type,
            length: payload_len as u16,
        })
    }

    pub fn to_bytes(&self) -> [u8; HEADER_SIZE] {
        [
            self.version,
            self.packet_type as u8,
            (self.length >> 8) as u8,
            (self.length & 0xFF) as u8,
        ]
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ProtocolError> {
        if bytes.len() < HEADER_SIZE {
            return Err(ProtocolError::IncompletePacket {
                expected: HEADER_SIZE,
                got: bytes.len(),
            });
        }
        let version = bytes[0];
        if version != PROTOCOL_VERSION {
            return Err(ProtocolError::InvalidVersion(version));
        }
        let packet_type = PacketType::try_from(bytes[1])?;
        let length = ((bytes[2] as u16) << 8) | (bytes[3] as u16);
        Ok(Self { version, packet_type, length })
    }

    pub fn total_size(&self) -> usize {
        HEADER_SIZE + self.length as usize + TAG_SIZE
    }
}

// Using serde_as with Bytes for byte arrays
#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveryRequest {
    #[serde_as(as = "Bytes")]
    pub cookie: Vec<u8>,
    pub cardputer_name: String,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveryResponse {
    #[serde_as(as = "Bytes")]
    pub cookie: Vec<u8>,
    pub device_name: String,
    pub server_port: u16,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandshakeInit {
    #[serde_as(as = "Bytes")]
    pub ephemeral_public_key: Vec<u8>,
    #[serde_as(as = "Bytes")]
    pub nonce: Vec<u8>,
    #[serde_as(as = "Bytes")]
    pub signature: Vec<u8>,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandshakeResponse {
    #[serde_as(as = "Bytes")]
    pub ephemeral_public_key: Vec<u8>,
    #[serde_as(as = "Bytes")]
    pub nonce: Vec<u8>,
    #[serde_as(as = "Bytes")]
    pub signature: Vec<u8>,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandshakeComplete {
    #[serde_as(as = "Bytes")]
    pub transcript_mac: Vec<u8>,
}

impl HandshakeInit {
    pub fn get_ephemeral_public_key(&self) -> Result<[u8; 33], ProtocolError> {
        self.ephemeral_public_key.as_slice().try_into().map_err(|_| ProtocolError::InvalidDataLength)
    }
    pub fn get_nonce(&self) -> Result<[u8; 32], ProtocolError> {
        self.nonce.as_slice().try_into().map_err(|_| ProtocolError::InvalidDataLength)
    }
    pub fn get_signature(&self) -> Result<[u8; 64], ProtocolError> {
        self.signature.as_slice().try_into().map_err(|_| ProtocolError::InvalidDataLength)
    }
}

impl HandshakeResponse {
    pub fn get_ephemeral_public_key(&self) -> Result<[u8; 33], ProtocolError> {
        self.ephemeral_public_key.as_slice().try_into().map_err(|_| ProtocolError::InvalidDataLength)
    }
    pub fn get_nonce(&self) -> Result<[u8; 32], ProtocolError> {
        self.nonce.as_slice().try_into().map_err(|_| ProtocolError::InvalidDataLength)
    }
    pub fn get_signature(&self) -> Result<[u8; 64], ProtocolError> {
        self.signature.as_slice().try_into().map_err(|_| ProtocolError::InvalidDataLength)
    }
}

impl HandshakeComplete {
    pub fn get_transcript_mac(&self) -> Result<[u8; 32], ProtocolError> {
        self.transcript_mac.as_slice().try_into().map_err(|_| ProtocolError::InvalidDataLength)
    }
}

#[derive(Debug, Clone)]
pub struct ScreenFrame {
    pub sequence: u32,
    pub timestamp: u32,
    pub jpeg_data: Vec<u8>,
}

impl ScreenFrame {
    pub fn serialize(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(8 + self.jpeg_data.len());
        buf.extend_from_slice(&self.sequence.to_be_bytes());
        buf.extend_from_slice(&self.timestamp.to_be_bytes());
        buf.extend_from_slice(&self.jpeg_data);
        buf
    }

    pub fn deserialize(data: &[u8]) -> Result<Self, ProtocolError> {
        if data.len() < 8 {
            return Err(ProtocolError::IncompletePacket { expected: 8, got: data.len() });
        }
        let sequence = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
        let timestamp = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
        Ok(Self { sequence, timestamp, jpeg_data: data[8..].to_vec() })
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct MouseMove {
    pub dx: i8,
    pub dy: i8,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct MouseClick {
    pub button: MouseButton,
    pub action: ClickAction,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[repr(u8)]
pub enum MouseButton { Left = 0, Right = 1, Middle = 2 }

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[repr(u8)]
pub enum ClickAction { Press = 0, Release = 1, Click = 2, DoubleClick = 3 }

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct KeyEvent {
    pub keycode: u8,
    pub modifiers: u8,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ModeSwitch {
    pub mode: InputMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum InputMode { Mouse = 0, Keyboard = 1 }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorPacket {
    pub code: u16,
    pub message: String,
}

pub struct Packet {
    pub header: PacketHeader,
    pub payload: Vec<u8>,
    pub tag: [u8; TAG_SIZE],
}

impl Packet {
    pub fn new(packet_type: PacketType, encrypted_payload: Vec<u8>, tag: [u8; TAG_SIZE]) -> Result<Self, ProtocolError> {
        let header = PacketHeader::new(packet_type, encrypted_payload.len())?;
        Ok(Self { header, payload: encrypted_payload, tag })
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(self.header.total_size());
        buf.extend_from_slice(&self.header.to_bytes());
        buf.extend_from_slice(&self.payload);
        buf.extend_from_slice(&self.tag);
        buf
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ProtocolError> {
        let header = PacketHeader::from_bytes(bytes)?;
        let total_size = header.total_size();
        if bytes.len() < total_size {
            return Err(ProtocolError::IncompletePacket { expected: total_size, got: bytes.len() });
        }
        let payload_end = HEADER_SIZE + header.length as usize;
        let payload = bytes[HEADER_SIZE..payload_end].to_vec();
        let mut tag = [0u8; TAG_SIZE];
        tag.copy_from_slice(&bytes[payload_end..payload_end + TAG_SIZE]);
        Ok(Self { header, payload, tag })
    }
}
