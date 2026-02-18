//! Key generation utility for Cardputer Remote
//!
//! Generates ECDH keypairs and discovery cookie for secure communication.
//!
//! Usage:
//!   keygen [OPTIONS]
//!
//! Options:
//!   --pc          Generate keypair for PC (server)
//!   --cardputer   Generate keypair for Cardputer (client)
//!   --both        Generate keypairs for both (default)
//!   --cookie      Generate a random discovery cookie
//!   --output DIR  Output binary files to directory (for SD card)
//!   -h, --help    Show help

use p256::ecdsa::SigningKey;
use rand::rngs::OsRng;
use std::fs;
use std::path::Path;

fn main() {
    let mut args = std::env::args().skip(1);
    let mut gen_pc = false;
    let mut gen_cardputer = false;
    let mut gen_cookie = false;
    let mut output_dir: Option<String> = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--pc" => gen_pc = true,
            "--cardputer" => gen_cardputer = true,
            "--both" => {
                gen_pc = true;
                gen_cardputer = true;
            }
            "--cookie" => gen_cookie = true,
            "--output" | "-o" => {
                output_dir = args.next();
                if output_dir.is_none() {
                    eprintln!("Error: --output requires a directory path");
                    std::process::exit(1);
                }
            }
            "-h" | "--help" => {
                print_help();
                return;
            }
            _ => {
                eprintln!("Unknown argument: {}", arg);
                print_help();
                std::process::exit(1);
            }
        }
    }

    // Default to all if nothing specified
    if !gen_pc && !gen_cardputer && !gen_cookie {
        gen_pc = true;
        gen_cardputer = true;
        gen_cookie = true;
    }

    println!("╔══════════════════════════════════════════════════════════════╗");
    println!("║         Cardputer Remote Key Generator v1.0                  ║");
    println!("╚══════════════════════════════════════════════════════════════╝\n");

    let mut pc_private: Option<Vec<u8>> = None;
    let mut pc_public: Option<Vec<u8>> = None;
    let mut cardputer_private: Option<Vec<u8>> = None;
    let mut cardputer_public: Option<Vec<u8>> = None;
    let mut cookie_bytes: Option<Vec<u8>> = None;

    if gen_cookie {
        cookie_bytes = Some(generate_cookie());
        println!();
    }

    if gen_pc {
        let (priv_key, pub_key) = generate_keypair("PC (Server)");
        pc_private = Some(priv_key);
        pc_public = Some(pub_key);
        println!();
    }

    if gen_cardputer {
        let (priv_key, pub_key) = generate_keypair("Cardputer (Client)");
        cardputer_private = Some(priv_key);
        cardputer_public = Some(pub_key);
        println!();
    }

    // Output binary files if requested
    if let Some(ref dir) = output_dir {
        output_binary_files(
            dir,
            cardputer_private.as_ref(),
            cardputer_public.as_ref(),
            pc_public.as_ref(),
            cookie_bytes.as_ref(),
        );
        println!();
    }

    // Print config instructions
    println!("╔══════════════════════════════════════════════════════════════╗");
    println!("║                   CONFIGURATION GUIDE                        ║");
    println!("╚══════════════════════════════════════════════════════════════╝\n");

    // PC config.toml
    if gen_pc {
        println!("┌─────────────────────────────────────────────────────────────┐");
        println!("│ PC SERVER: config.toml                                      │");
        println!("└─────────────────────────────────────────────────────────────┘");
        println!("[security]");
        if let Some(ref cookie) = cookie_bytes {
            println!("discovery_cookie = \"{}\"", hex::encode(cookie));
        }
        if let Some(ref priv_key) = pc_private {
            println!("private_key = \"{}\"", hex::encode(priv_key));
        }
        if let Some(ref pub_key) = cardputer_public {
            println!("cardputer_public_key = \"{}\"", hex::encode(pub_key));
        }
        println!();
    }

    // ESP32 SD card instructions
    if gen_cardputer {
        println!("┌─────────────────────────────────────────────────────────────┐");
        println!("│ ESP32 CARDPUTER: SD Card /rd_keys/ directory                │");
        println!("└─────────────────────────────────────────────────────────────┘");
        println!();

        if output_dir.is_some() {
            println!("✓ Binary files created! Copy the rd_keys folder to SD card root.");
            println!();
        } else {
            println!("Create folder: SD:\\rd_keys\\");
            println!();

            // PowerShell commands for Windows users
            println!("=== PowerShell Commands (Windows) ===");
            println!();

            if let Some(ref priv_key) = cardputer_private {
                println!("# client.key (32 bytes) - Cardputer private key");
                print_powershell_command("client.key", priv_key);
                println!();
            }

            if let Some(ref pub_key) = cardputer_public {
                println!("# client.pub (33 bytes) - Cardputer public key");
                print_powershell_command("client.pub", pub_key);
                println!();
            }

            if let Some(ref pub_key) = pc_public {
                println!("# server.pub (33 bytes) - PC server public key");
                print_powershell_command("server.pub", pub_key);
                println!();
            }

            if let Some(ref cookie) = cookie_bytes {
                println!("# cookie (16 bytes) - Discovery cookie");
                print_powershell_command("cookie", cookie);
                println!();
            }

            println!("=== Linux/macOS Commands ===");
            println!();

            if let Some(ref priv_key) = cardputer_private {
                println!("# client.key");
                println!("echo '{}' | xxd -r -p > rd_keys/client.key", hex::encode(priv_key));
            }
            if let Some(ref pub_key) = cardputer_public {
                println!("# client.pub");
                println!("echo '{}' | xxd -r -p > rd_keys/client.pub", hex::encode(pub_key));
            }
            if let Some(ref pub_key) = pc_public {
                println!("# server.pub");
                println!("echo '{}' | xxd -r -p > rd_keys/server.pub", hex::encode(pub_key));
            }
            if let Some(ref cookie) = cookie_bytes {
                println!("# cookie");
                println!("echo '{}' | xxd -r -p > rd_keys/cookie", hex::encode(cookie));
            }
            println!();
        }
    }

    println!("┌─────────────────────────────────────────────────────────────┐");
    println!("│ ⚠️  SECURITY WARNING                                         │");
    println!("├─────────────────────────────────────────────────────────────┤");
    println!("│ • Keep private keys SECRET - never share them!              │");
    println!("│ • Store them securely on each device                        │");
    println!("│ • Generate new keys if compromised                          │");
    println!("│ • NEVER commit keys to version control!                     │");
    println!("└─────────────────────────────────────────────────────────────┘");
}

fn print_help() {
    println!("Cardputer Remote Key Generator");
    println!();
    println!("Usage: keygen [OPTIONS]");
    println!();
    println!("Options:");
    println!("  --pc          Generate keypair for PC (server)");
    println!("  --cardputer   Generate keypair for Cardputer (client)");
    println!("  --both        Generate keypairs for both (default)");
    println!("  --cookie      Generate a random discovery cookie");
    println!("  --output DIR  Output binary files to DIR/rd_keys/ (for SD card)");
    println!("  -h, --help    Show this help");
    println!();
    println!("Examples:");
    println!("  keygen                    # Generate all keys, show commands");
    println!("  keygen --output D:\\      # Generate and write to D:\\rd_keys\\");
    println!("  keygen --output /mnt/sd   # Generate and write to /mnt/sd/rd_keys/");
}

fn generate_keypair(name: &str) -> (Vec<u8>, Vec<u8>) {
    let signing_key = SigningKey::random(&mut OsRng);
    let verifying_key = signing_key.verifying_key();

    // Get private key bytes
    let private_bytes = signing_key.to_bytes().to_vec();
    let private_hex = hex::encode(&private_bytes);

    // Get compressed public key
    let public_point = verifying_key.to_encoded_point(true);
    let public_bytes = public_point.as_bytes().to_vec();
    let public_hex = hex::encode(&public_bytes);

    println!("┌─ {} ─┐", name);
    println!("│");
    println!("│ Private Key ({} bytes) - KEEP SECRET!", private_bytes.len());
    println!("│   {}", private_hex);
    println!("│");
    println!("│ Public Key ({} bytes) - share with peer", public_bytes.len());
    println!("│   {}", public_hex);
    println!("│");

    (private_bytes, public_bytes)
}

fn generate_cookie() -> Vec<u8> {
    use rand::RngCore;

    let mut cookie = [0u8; 16];
    OsRng.fill_bytes(&mut cookie);

    println!("┌─ Discovery Cookie ─┐");
    println!("│");
    println!("│ Cookie ({} bytes) - same on BOTH devices", cookie.len());
    println!("│   {}", hex::encode(cookie));
    println!("│");

    cookie.to_vec()
}

fn print_powershell_command(filename: &str, data: &[u8]) {
    // Format bytes as PowerShell array
    let bytes_str: Vec<String> = data.iter().map(|b| format!("0x{:02X}", b)).collect();

    // Split into multiple lines if too long
    let chunk_size = 12;
    let chunks: Vec<String> = bytes_str
        .chunks(chunk_size)
        .map(|chunk| chunk.join(","))
        .collect();

    println!("$bytes = [byte[]]@(");
    for (i, chunk) in chunks.iter().enumerate() {
        if i < chunks.len() - 1 {
            println!("  {},", chunk);
        } else {
            println!("  {}", chunk);
        }
    }
    println!(")");
    println!("[IO.File]::WriteAllBytes(\"rd_keys\\{}\", $bytes)", filename);
}

fn output_binary_files(
    base_dir: &str,
    client_key: Option<&Vec<u8>>,
    client_pub: Option<&Vec<u8>>,
    server_pub: Option<&Vec<u8>>,
    cookie: Option<&Vec<u8>>,
) {
    let rd_keys_dir = Path::new(base_dir).join("rd_keys");

    // Create directory
    if let Err(e) = fs::create_dir_all(&rd_keys_dir) {
        eprintln!("Error creating directory {:?}: {}", rd_keys_dir, e);
        return;
    }

    println!("┌─────────────────────────────────────────────────────────────┐");
    println!("│ Writing binary files to: {:?}", rd_keys_dir);
    println!("└─────────────────────────────────────────────────────────────┘");

    if let Some(data) = client_key {
        write_file(&rd_keys_dir.join("client.key"), data, "client.key");
    }

    if let Some(data) = client_pub {
        write_file(&rd_keys_dir.join("client.pub"), data, "client.pub");
    }

    if let Some(data) = server_pub {
        write_file(&rd_keys_dir.join("server.pub"), data, "server.pub");
    }

    if let Some(data) = cookie {
        write_file(&rd_keys_dir.join("cookie"), data, "cookie");
    }
}

fn write_file(path: &Path, data: &[u8], name: &str) {
    match fs::write(path, data) {
        Ok(_) => println!("  ✓ {} ({} bytes)", name, data.len()),
        Err(e) => eprintln!("  ✗ {} - Error: {}", name, e),
    }
}
