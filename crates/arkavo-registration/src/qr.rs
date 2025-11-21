use crate::{AgentDescriptor, RegistrationError};
use qrcode::render::unicode;
use qrcode::QrCode;

pub fn generate_qr_string(descriptor: &AgentDescriptor) -> Result<String, RegistrationError> {
    let url = descriptor.to_url();
    let code = QrCode::new(url.as_bytes())
        .map_err(|e| RegistrationError::QrCodeGeneration(e.to_string()))?;

    let short_sha = &descriptor.agent_id_short_sha;
    let overlay = if short_sha.len() <= 7 {
        short_sha.clone()
    } else {
        short_sha[..7].to_string()
    };

    let image = code
        .render::<unicode::Dense1x2>()
        .dark_color(unicode::Dense1x2::Light)
        .light_color(unicode::Dense1x2::Dark)
        .build();

    let lines: Vec<&str> = image.lines().collect();
    let height = lines.len();
    let width_chars = if height > 0 {
        lines[0].chars().count()
    } else {
        0
    };

    if height < 5 || width_chars < overlay.len() {
        return Ok(image);
    }

    let center_row = height / 2;
    let center_col = (width_chars - overlay.len()) / 2;

    let mut modified_lines: Vec<String> = lines.iter().map(|s| s.to_string()).collect();

    if center_row < modified_lines.len() {
        let line = &mut modified_lines[center_row];
        let mut chars: Vec<char> = line.chars().collect();

        for (i, ch) in overlay.chars().enumerate() {
            let pos = center_col + i;
            if pos < chars.len() {
                chars[pos] = ch;
            }
        }

        modified_lines[center_row] = chars.into_iter().collect();
    }

    Ok(modified_lines.join("\n"))
}

pub fn display_qr(descriptor: &AgentDescriptor) -> Result<(), RegistrationError> {
    let qr_string = generate_qr_string(descriptor)?;
    println!("\n{}\n", qr_string);
    println!("Agent ID: {}", descriptor.agent_id_short_sha);
    println!("Endpoint: {}", descriptor.endpoint);
    if let Some(mdns) = &descriptor.mdns_service {
        println!("mDNS Service: {}", mdns);
    }
    println!();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_crypto::AgentKeypair;

    #[test]
    fn test_qr_generation() {
        let keypair = AgentKeypair::generate();
        let public_key = keypair.public_key();
        let descriptor = AgentDescriptor::new(
            public_key,
            "http://localhost:8342".to_string(),
            Some("arkavo._tcp.local.".to_string()),
            "abc1234".to_string(),
        );

        let qr_string = generate_qr_string(&descriptor).unwrap();
        assert!(!qr_string.is_empty());
        // Short-SHA should be embedded in the visual QR code
        assert!(qr_string.contains("abc1234"));
        // URL should be encoded in the QR code data
        let url = descriptor.to_url();
        assert!(url.contains("arkavo://agent?public_key="));
        assert!(url.contains("arkavo._tcp.local."));
    }

    #[test]
    fn test_qr_with_short_sha() {
        let keypair = AgentKeypair::generate();
        let public_key = keypair.public_key();
        let descriptor = AgentDescriptor::new(
            public_key,
            "http://192.168.1.1:8342".to_string(),
            None,
            "a1b".to_string(),
        );

        let qr_string = generate_qr_string(&descriptor).unwrap();
        assert!(!qr_string.is_empty());
    }

    #[test]
    fn test_display_qr() {
        let keypair = AgentKeypair::generate();
        let public_key = keypair.public_key();
        let descriptor = AgentDescriptor::new(
            public_key,
            "http://localhost:8342".to_string(),
            Some("arkavo._tcp.local.".to_string()),
            "test123".to_string(),
        );

        let result = display_qr(&descriptor);
        assert!(result.is_ok());
    }
}
