#!/bin/bash

# Generate test certificates for mTLS testing
# Uses rcgen (Rust certificate generator) instead of OpenSSL

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
CERTS_DIR="$SCRIPT_DIR/../tests/certs"

echo "Creating certificates directory..."
mkdir -p "$CERTS_DIR"

echo "Building certificate generator..."
# Create a temporary Cargo project for the certificate generator
TEMP_DIR=$(mktemp -d)

cat > "$TEMP_DIR/cert_gen.rs" << 'EOF'
use rcgen::{Certificate, CertificateParams, DistinguishedName, DnType, KeyPair};
use std::fs;
use std::path::Path;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let certs_dir = std::env::args().nth(1).expect("certs directory required");
    let certs_path = Path::new(&certs_dir);
    
    // Generate CA certificate
    println!("Generating CA certificate...");
    let mut ca_params = CertificateParams::default();
    ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    ca_params.distinguished_name = DistinguishedName::new();
    ca_params.distinguished_name.push(DnType::CommonName, "Test CA");
    ca_params.distinguished_name.push(DnType::OrganizationName, "Arkavo Test");
    ca_params.distinguished_name.push(DnType::CountryName, "US");
    
    let ca_key = KeyPair::generate()?;
    let ca_cert = ca_params.self_signed(&ca_key)?;
    
    fs::write(certs_path.join("ca.crt"), ca_cert.pem())?;
    fs::write(certs_path.join("ca.key"), ca_key.serialize_pem())?;
    
    // Generate server certificate
    println!("Generating server certificate...");
    let mut server_params = CertificateParams::new(vec!["localhost".to_string()])?;
    server_params.distinguished_name = DistinguishedName::new();
    server_params.distinguished_name.push(DnType::CommonName, "localhost");
    server_params.distinguished_name.push(DnType::OrganizationName, "Arkavo Test Server");
    
    let server_key = KeyPair::generate()?;
    let server_cert_pem = server_params.signed_by(&server_key, &ca_cert, &ca_key)?;
    
    fs::write(certs_path.join("server.crt"), server_cert_pem.pem())?;
    fs::write(certs_path.join("server.key"), server_key.serialize_pem())?;
    
    // Generate client certificate
    println!("Generating client certificate...");
    let mut client_params = CertificateParams::default();
    client_params.distinguished_name = DistinguishedName::new();
    client_params.distinguished_name.push(DnType::CommonName, "Test Client");
    client_params.distinguished_name.push(DnType::OrganizationName, "Arkavo Test Client");
    
    let client_key = KeyPair::generate()?;
    let client_cert_pem = client_params.signed_by(&client_key, &ca_cert, &ca_key)?;
    
    fs::write(certs_path.join("client.crt"), client_cert_pem.pem())?;
    fs::write(certs_path.join("client.key"), client_key.serialize_pem())?;
    
    // Generate invalid client certificate (self-signed, not by CA)
    println!("Generating invalid client certificate...");
    let mut invalid_params = CertificateParams::default();
    invalid_params.distinguished_name = DistinguishedName::new();
    invalid_params.distinguished_name.push(DnType::CommonName, "Invalid Client");
    
    let invalid_key = KeyPair::generate()?;
    let invalid_cert = invalid_params.self_signed(&invalid_key)?;
    
    fs::write(certs_path.join("invalid_client.crt"), invalid_cert.pem())?;
    fs::write(certs_path.join("invalid_client.key"), invalid_key.serialize_pem())?;
    
    println!("Certificates generated successfully!");
    println!("  CA: ca.crt, ca.key");
    println!("  Server: server.crt, server.key");
    println!("  Client: client.crt, client.key");
    println!("  Invalid: invalid_client.crt, invalid_client.key");
    
    Ok(())
}
EOF

# Change to temporary directory
cd "$TEMP_DIR"

cat > Cargo.toml << EOF
[package]
name = "cert-gen"
version = "0.1.0"
edition = "2021"

[dependencies]
rcgen = { version = "0.13", features = ["pem"] }
EOF

mkdir src
mv cert_gen.rs src/main.rs

echo "Running certificate generator..."
cargo run --quiet -- "$CERTS_DIR"

# Clean up
cd "$SCRIPT_DIR"
rm -rf "$TEMP_DIR"

echo "Test certificates generated in: $CERTS_DIR"