//! Plugin code safety scanning for dangerous patterns
//!
//! Scans plugin/tool source code for potentially dangerous imports,
//! function calls, and code patterns before loading.

use regex::Regex;
use std::sync::LazyLock;

/// Severity level for detected patterns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// Must be blocked — no exceptions.
    Critical,
    /// Should be logged but may be allowed.
    Warning,
}

/// A detected dangerous pattern.
#[derive(Debug, Clone)]
pub struct ScanFinding {
    pub severity: Severity,
    pub pattern_name: String,
    pub matched_text: String,
    pub line_number: usize,
}

/// Result of a code scan.
#[derive(Debug, Clone)]
pub struct ScanResult {
    pub findings: Vec<ScanFinding>,
}

impl ScanResult {
    /// Whether any critical findings were detected.
    pub fn has_critical(&self) -> bool {
        self.findings
            .iter()
            .any(|f| f.severity == Severity::Critical)
    }

    /// Whether any findings were detected.
    pub fn has_findings(&self) -> bool {
        !self.findings.is_empty()
    }

    /// Count of critical findings.
    pub fn critical_count(&self) -> usize {
        self.findings
            .iter()
            .filter(|f| f.severity == Severity::Critical)
            .count()
    }

    /// Count of warning findings.
    pub fn warning_count(&self) -> usize {
        self.findings
            .iter()
            .filter(|f| f.severity == Severity::Warning)
            .count()
    }
}

// Pre-compiled regexes for dangerous patterns
static RE_PROCESS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:child_process|subprocess|exec|spawn|popen|system)\s*\(").expect("valid")
});
static RE_EVAL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:eval|Function)\s*\(").expect("valid"));
static RE_FS_WRITE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:writeFile|writeFileSync|unlink|rmdir|rename)\s*\(").expect("valid")
});
static RE_NET_LISTEN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:createServer|listen|bind)\s*\(").expect("valid"));
static RE_DYN_IMPORT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?:require|import)\s*\(\s*[^'"]"#).expect("valid"));
static RE_NET_REQ: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:fetch|axios|http[.]get|https[.]get|request)\s*\(").expect("valid")
});
static RE_ENV: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"process[.]env|std::env::var|os[.]environ").expect("valid"));
static RE_CRYPTO: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:createCipher|createDecipher|crypto[.]subtle)").expect("valid")
});

static PATTERNS: [(&str, &LazyLock<Regex>, Severity); 8] = [
    ("process_spawn", &RE_PROCESS, Severity::Critical),
    ("code_eval", &RE_EVAL, Severity::Critical),
    ("fs_write", &RE_FS_WRITE, Severity::Critical),
    ("network_listen", &RE_NET_LISTEN, Severity::Critical),
    ("dynamic_import", &RE_DYN_IMPORT, Severity::Critical),
    ("network_request", &RE_NET_REQ, Severity::Warning),
    ("env_access", &RE_ENV, Severity::Warning),
    ("crypto_ops", &RE_CRYPTO, Severity::Warning),
];

/// Scans plugin/tool source code for dangerous patterns.
pub struct CodeScanner;

impl CodeScanner {
    /// Scan source code for dangerous patterns.
    pub fn scan(source: &str) -> ScanResult {
        let mut findings = Vec::new();

        for (line_num, line) in source.lines().enumerate() {
            let trimmed = line.trim();
            if trimmed.starts_with("//") || trimmed.starts_with('#') || trimmed.starts_with("/*") {
                continue;
            }

            for &(name, regex, severity) in &PATTERNS {
                if regex.is_match(line) {
                    findings.push(ScanFinding {
                        severity,
                        pattern_name: name.to_string(),
                        matched_text: trimmed.to_string(),
                        line_number: line_num + 1,
                    });
                }
            }
        }

        ScanResult { findings }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    #[spec("MCPD-005")]
    #[test]
    fn test_detects_process_spawn() {
        let result = CodeScanner::scan("const child = child_process.exec('ls')");
        assert!(result.has_critical());
        assert_eq!(result.findings[0].pattern_name, "process_spawn");
    }

    #[spec("MCPD-005")]
    #[test]
    fn test_detects_eval() {
        let result = CodeScanner::scan("eval('malicious code')");
        assert!(result.has_critical());
        assert_eq!(result.findings[0].pattern_name, "code_eval");
    }

    #[spec("MCPD-005")]
    #[test]
    fn test_detects_fs_write() {
        let result = CodeScanner::scan("fs.writeFileSync('/etc/hosts', data)");
        assert!(result.has_critical());
    }

    #[spec("MCPD-005")]
    #[test]
    fn test_detects_network_listen() {
        let result = CodeScanner::scan("http.createServer(handler)");
        assert!(result.has_critical());
    }

    #[spec("MCPD-005")]
    #[test]
    fn test_detects_dynamic_import() {
        let result = CodeScanner::scan("const mod = require(userInput)");
        assert!(result.has_critical());
    }

    #[spec("MCPD-005")]
    #[test]
    fn test_warns_on_fetch() {
        let result = CodeScanner::scan("const data = fetch('https://api.example.com')");
        assert!(result.has_findings());
        assert!(!result.has_critical());
        assert_eq!(result.warning_count(), 1);
    }

    #[spec("MCPD-005")]
    #[test]
    fn test_warns_on_env_access() {
        let result = CodeScanner::scan("const key = process.env.API_KEY");
        assert!(result.has_findings());
    }

    #[spec("MCPD-005")]
    #[test]
    fn test_clean_code_passes() {
        let code = "function add(a, b) {\n    return a + b;\n}\nconst result = add(1, 2);";
        let result = CodeScanner::scan(code);
        assert!(!result.has_findings());
    }

    #[spec("MCPD-005")]
    #[test]
    fn test_skips_comments() {
        let result = CodeScanner::scan("// eval('this is a comment')\nconst x = 1;");
        assert!(!result.has_findings());
    }

    #[spec("MCPD-005")]
    #[test]
    fn test_multiple_findings() {
        let result = CodeScanner::scan("eval('x')\nchild_process.exec('ls')\nfetch('url')");
        assert_eq!(result.critical_count(), 2);
        assert_eq!(result.warning_count(), 1);
    }

    #[spec("MCPD-005")]
    #[test]
    fn test_line_numbers() {
        let result = CodeScanner::scan("safe line\nunsafe eval('x')");
        assert_eq!(result.findings[0].line_number, 2);
    }
}
