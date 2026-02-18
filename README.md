# Cardputer Remote Desktop

Secure remote desktop system for M5Stack Cardputer with full PKI mutual authentication.

## Security Features

- **ECDSA Mutual Authentication** - Both client and server verify each other's identity
- **ECDH Forward Secrecy** - Ephemeral keys ensure past sessions remain secure if keys leak
- **AES-128-GCM Encryption** - Authenticated encryption for all session data
- **Replay Protection** - Monotonic nonce counters prevent replay attacks
- **HKDF Key Derivation** - RFC 5869 compliant key derivation

## Quick Start

### 1. Generate Keys

```bash
cd cardputer-remote
cargo run --bin keygen
```

This generates:
- PC private/public keypair
- Cardputer private/public keypair
- Discovery cookie

### 2. Configure PC Server

Create `config.toml`:

```toml
[server]
port = 19847
session_timeout_secs = 300
max_fps = 10
jpeg_quality = 70

[security]
discovery_cookie = "<generated cookie>"
private_key = "<PC private key>"
cardputer_public_key = "<Cardputer public key>"

[network]
mdns_service_name = "CardputerRemote"
device_name = "MyPC"
bind_address = "0.0.0.0"

[display]
target_width = 240
target_height = 135

[logging]
level = "info"
```

### 3. Configure Cardputer (ESP32)

Create key files on SD card:

```bash
# Create directory
mkdir /sd/rd_keys

# Convert hex keys to binary and copy to SD card
echo '<Cardputer private key hex>' | xxd -r -p > /sd/rd_keys/client.key
echo '<Cardputer public key hex>' | xxd -r -p > /sd/rd_keys/client.pub
echo '<PC public key hex>' | xxd -r -p > /sd/rd_keys/server.pub
```

Or use the ESP32's key generation (via Serial):
```
[RD] Generated public key (add to server config): 02abc123...
```

### 4. Run

**PC Server:**
```bash
cargo run --release
```

**Cardputer:**
1. Flash firmware with Remote Desktop module
2. Connect to WiFi
3. Select "Remote Desktop" from menu
4. Press ENTER to connect

## Controls

| Key | Action |
|-----|--------|
| FN + ; | Mouse up |
| FN + . | Mouse down |
| FN + , | Mouse left |
| FN + / | Mouse right |
| FN + ENTER | Left click |
| FN + BACKSPACE | Disconnect |
| A-Z, 0-9 | Type characters |

## File Structure

```
/sd/
├── rd_keys/
│   ├── client.key    # ECDSA private key (32 bytes)
│   ├── client.pub    # ECDSA public key (33 bytes)
│   └── server.pub    # Server's public key (33 bytes)
└── remote_desktop.json  # Optional config
```

## Troubleshooting

### "Failed to load keys"
- Ensure all three key files exist in `/sd/rd_keys/`
- Check file sizes: client.key=32, client.pub=33, server.pub=33

### "Server signature verification failed"
- Verify server.pub matches PC's public key
- Regenerate keys if compromised

### "Handshake timeout"
- Check WiFi connectivity
- Verify PC server is running
- Check firewall allows port 19847

## Security Considerations

1. **Key Storage**: Private keys are stored unencrypted on SD card. Physical security of the Cardputer is essential.

2. **Network**: Initial TCP connection is unencrypted. Use on trusted networks only.

3. **Updates**: Keep both ESP32 and PC components updated together.
