/// Simple logging utilities for arkavo-cli
///
/// This module provides basic logging functions that output structured logs
/// to stderr. In the future, this can be replaced with a full logging framework
/// like tracing or env_logger.

#[allow(dead_code)]
pub fn info(component: &str, message: &str) {
    eprintln!("[INFO] {} {}", component, message);
}

#[allow(dead_code)]
pub fn warn(component: &str, message: &str) {
    eprintln!("[WARN] {} {}", component, message);
}

#[allow(dead_code)]
pub fn error(component: &str, message: &str) {
    eprintln!("[ERROR] {} {}", component, message);
}

#[allow(dead_code)]
pub fn debug(component: &str, message: &str) {
    if std::env::var("ARKAVO_DEBUG").is_ok() {
        eprintln!("[DEBUG] {} {}", component, message);
    }
}

/// Log structured data for telemetry
#[allow(dead_code)]
pub fn telemetry(event: &str, fields: &[(&str, &str)]) {
    let fields_str = fields
        .iter()
        .map(|(k, v)| format!("{}={}", k, v))
        .collect::<Vec<_>>()
        .join(" ");
    eprintln!("[INFO] {} {}", event, fields_str);
}
