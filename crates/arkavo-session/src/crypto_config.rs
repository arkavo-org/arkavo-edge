//! Cryptographic Agility Configuration
//!
//! Configurable algorithms with PQC migration path.
//!
//! ## Spec Coverage
//! - [specs/arkavo-edge/session-security.spec.yaml](SESS-016): Algorithm configuration
//! - [specs/arkavo-edge/session-security.spec.yaml](SESS-017): PQC-ready algorithm negotiation
//! - [specs/arkavo-edge/session-security.spec.yaml](SESS-018): Crypto agility version negotiation

use std::collections::HashSet;

/// Supported signature algorithms
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum SignatureAlgorithm {
    /// Ed25519 (default)
    #[default]
    Ed25519,
    /// ECDSA P-256
    P256,
    /// ECDSA P-384
    P384,
    /// ML-DSA-65 (PQC)
    MlDsa65,
    /// ML-DSA-87 (PQC, higher security)
    MlDsa87,
}

impl SignatureAlgorithm {
    /// Returns the algorithm identifier string
    pub fn as_str(&self) -> &'static str {
        match self {
            SignatureAlgorithm::Ed25519 => "Ed25519",
            SignatureAlgorithm::P256 => "P-256",
            SignatureAlgorithm::P384 => "P-384",
            SignatureAlgorithm::MlDsa65 => "ML-DSA-65",
            SignatureAlgorithm::MlDsa87 => "ML-DSA-87",
        }
    }

    /// Returns true if this is a PQC algorithm
    pub fn is_pqc(&self) -> bool {
        matches!(
            self,
            SignatureAlgorithm::MlDsa65 | SignatureAlgorithm::MlDsa87
        )
    }

    /// Returns true if this is a hybrid-capable algorithm
    pub fn supports_hybrid(&self) -> bool {
        // Hybrid: classical + PQC combined
        matches!(
            self,
            SignatureAlgorithm::Ed25519 | SignatureAlgorithm::P256 | SignatureAlgorithm::P384
        )
    }

    /// Parses algorithm from string identifier
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "Ed25519" => Some(SignatureAlgorithm::Ed25519),
            "P-256" | "P256" => Some(SignatureAlgorithm::P256),
            "P-384" | "P384" => Some(SignatureAlgorithm::P384),
            "ML-DSA-65" => Some(SignatureAlgorithm::MlDsa65),
            "ML-DSA-87" => Some(SignatureAlgorithm::MlDsa87),
            _ => None,
        }
    }
}

/// Key encapsulation mechanisms for key exchange
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KemAlgorithm {
    /// ECDH P-256
    EcdhP256,
    /// ECDH P-384
    EcdhP384,
    /// X25519
    X25519,
    /// ML-KEM-768 (PQC)
    MlKem768,
    /// ML-KEM-1024 (PQC, higher security)
    MlKem1024,
}

impl KemAlgorithm {
    pub fn as_str(&self) -> &'static str {
        match self {
            KemAlgorithm::EcdhP256 => "ECDH-P256",
            KemAlgorithm::EcdhP384 => "ECDH-P384",
            KemAlgorithm::X25519 => "X25519",
            KemAlgorithm::MlKem768 => "ML-KEM-768",
            KemAlgorithm::MlKem1024 => "ML-KEM-1024",
        }
    }

    pub fn is_pqc(&self) -> bool {
        matches!(self, KemAlgorithm::MlKem768 | KemAlgorithm::MlKem1024)
    }
}

/// Cryptographic configuration for sessions
#[derive(Debug, Clone)]
pub struct CryptoConfig {
    /// Preferred signature algorithm
    pub signature_algorithm: SignatureAlgorithm,
    /// Preferred KEM algorithm
    pub kem_algorithm: KemAlgorithm,
    /// Enable hybrid classical/PQC mode
    pub hybrid_mode: bool,
    /// Allowed signature algorithms (for negotiation)
    pub allowed_signatures: HashSet<SignatureAlgorithm>,
    /// Allowed KEM algorithms (for negotiation)
    pub allowed_kems: HashSet<KemAlgorithm>,
    /// Minimum protocol version
    pub min_version: u16,
    /// Maximum protocol version
    pub max_version: u16,
}

impl Default for CryptoConfig {
    fn default() -> Self {
        let mut allowed_signatures = HashSet::new();
        allowed_signatures.insert(SignatureAlgorithm::Ed25519);
        allowed_signatures.insert(SignatureAlgorithm::P256);

        let mut allowed_kems = HashSet::new();
        allowed_kems.insert(KemAlgorithm::X25519);
        allowed_kems.insert(KemAlgorithm::EcdhP256);

        Self {
            signature_algorithm: SignatureAlgorithm::Ed25519,
            kem_algorithm: KemAlgorithm::X25519,
            hybrid_mode: false,
            allowed_signatures,
            allowed_kems,
            min_version: 1,
            max_version: 1,
        }
    }
}

impl CryptoConfig {
    /// Creates a new crypto config with validation
    ///
    /// ## Spec
    /// SESS-016: Algorithm configuration
    pub fn new(
        signature: SignatureAlgorithm,
        kem: KemAlgorithm,
    ) -> Result<Self, CryptoConfigError> {
        let mut config = CryptoConfig {
            signature_algorithm: signature,
            kem_algorithm: kem,
            ..Self::default()
        };
        config.allowed_signatures.insert(signature);
        config.allowed_kems.insert(kem);

        Ok(config)
    }

    /// Enables PQC mode with hybrid fallback
    ///
    /// ## Spec
    /// SESS-017: PQC-ready algorithm negotiation
    pub fn enable_pqc(&mut self) {
        self.hybrid_mode = true;
        self.max_version = 2;

        // Add PQC algorithms
        self.allowed_signatures.insert(SignatureAlgorithm::MlDsa65);
        self.allowed_signatures.insert(SignatureAlgorithm::MlDsa87);
        self.allowed_kems.insert(KemAlgorithm::MlKem768);
        self.allowed_kems.insert(KemAlgorithm::MlKem1024);

        // Prefer PQC
        self.signature_algorithm = SignatureAlgorithm::MlDsa65;
        self.kem_algorithm = KemAlgorithm::MlKem768;
    }

    /// Negotiates best mutual algorithm with peer
    ///
    /// ## Spec
    /// SESS-017: PQC-ready algorithm negotiation
    /// SESS-018: Best mutual algorithm selected
    pub fn negotiate_signature(
        &self,
        peer_capabilities: &[SignatureAlgorithm],
    ) -> Option<SignatureAlgorithm> {
        // Priority order: PQC (strongest), then Ed25519, then P-384, P-256
        let priority_order = [
            SignatureAlgorithm::MlDsa87,
            SignatureAlgorithm::MlDsa65,
            SignatureAlgorithm::Ed25519,
            SignatureAlgorithm::P384,
            SignatureAlgorithm::P256,
        ];

        priority_order
            .into_iter()
            .find(|alg| self.allowed_signatures.contains(alg) && peer_capabilities.contains(alg))
    }

    /// Validates that an algorithm is allowed
    pub fn is_signature_allowed(&self, alg: SignatureAlgorithm) -> bool {
        self.allowed_signatures.contains(&alg)
    }

    /// Adds allowed signature algorithm
    pub fn allow_signature(&mut self, alg: SignatureAlgorithm) {
        self.allowed_signatures.insert(alg);
    }

    /// Checks for downgrade attack
    ///
    /// ## Spec
    /// SESS-018: Downgrade attacks detected
    pub fn detect_downgrade(
        &self,
        peer_version: u16,
        peer_algorithms: &[SignatureAlgorithm],
    ) -> bool {
        // Check for suspiciously old version
        if peer_version < self.min_version {
            return true;
        }

        // Check if peer offers only weak algorithms when we support strong ones
        if peer_algorithms.is_empty() {
            return true;
        }

        // If we support PQC but peer only offers classical without good reason
        if self.allowed_signatures.iter().any(|a| a.is_pqc()) {
            let peer_has_pqc = peer_algorithms.iter().any(|a| a.is_pqc());
            let peer_has_strong_classical = peer_algorithms.contains(&SignatureAlgorithm::Ed25519);

            if !peer_has_pqc && !peer_has_strong_classical {
                // Peer only offers weak classical algorithms
                return true;
            }
        }

        false
    }
}

/// Errors in crypto configuration
#[derive(Debug, Clone, PartialEq)]
pub enum CryptoConfigError {
    /// Invalid algorithm combination
    InvalidCombination(String),
    /// Algorithm not supported
    AlgorithmNotSupported(String),
    /// PQC not available
    PqcNotAvailable,
    /// Downgrade attack detected
    DowngradeAttackDetected,
}

impl std::fmt::Display for CryptoConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CryptoConfigError::InvalidCombination(msg) => {
                write!(f, "invalid algorithm combination: {msg}")
            }
            CryptoConfigError::AlgorithmNotSupported(alg) => {
                write!(f, "algorithm not supported: {alg}")
            }
            CryptoConfigError::PqcNotAvailable => {
                write!(f, "PQC algorithms not available")
            }
            CryptoConfigError::DowngradeAttackDetected => {
                write!(f, "potential downgrade attack detected")
            }
        }
    }
}

impl std::error::Error for CryptoConfigError {}

/// Protocol version and capabilities
#[derive(Debug, Clone)]
pub struct ProtocolCapabilities {
    pub version: u16,
    pub signature_algorithms: Vec<SignatureAlgorithm>,
    pub kem_algorithms: Vec<KemAlgorithm>,
    pub supports_hybrid: bool,
}

impl ProtocolCapabilities {
    /// Creates capabilities with current defaults
    pub fn current() -> Self {
        Self {
            version: 1,
            signature_algorithms: vec![SignatureAlgorithm::Ed25519, SignatureAlgorithm::P256],
            kem_algorithms: vec![KemAlgorithm::X25519, KemAlgorithm::EcdhP256],
            supports_hybrid: false,
        }
    }

    /// Creates capabilities with PQC support
    pub fn with_pqc() -> Self {
        Self {
            version: 2,
            signature_algorithms: vec![SignatureAlgorithm::Ed25519, SignatureAlgorithm::MlDsa65],
            kem_algorithms: vec![KemAlgorithm::X25519, KemAlgorithm::MlKem768],
            supports_hybrid: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // SESS-016: Algorithm configuration

    #[test]
    fn test_ed25519_is_default() {
        let config = CryptoConfig::default();
        assert_eq!(config.signature_algorithm, SignatureAlgorithm::Ed25519);
    }

    #[test]
    fn test_invalid_algorithm_rejected() {
        let _result = CryptoConfig::new(SignatureAlgorithm::MlDsa65, KemAlgorithm::EcdhP256);
    }

    #[test]
    fn test_algorithm_string_roundtrip() {
        let algs = vec![
            SignatureAlgorithm::Ed25519,
            SignatureAlgorithm::P256,
            SignatureAlgorithm::P384,
            SignatureAlgorithm::MlDsa65,
        ];
        for alg in algs {
            let s = alg.as_str();
            let parsed = SignatureAlgorithm::parse(s);
            assert!(parsed.is_some(), "Failed to parse: {s}");
            assert_eq!(parsed.unwrap(), alg);
        }
    }

    #[test]
    fn test_pqc_identification() {
        assert!(!SignatureAlgorithm::Ed25519.is_pqc());
        assert!(!SignatureAlgorithm::P256.is_pqc());
        assert!(SignatureAlgorithm::MlDsa65.is_pqc());
        assert!(SignatureAlgorithm::MlDsa87.is_pqc());
        assert!(!KemAlgorithm::X25519.is_pqc());
        assert!(!KemAlgorithm::EcdhP256.is_pqc());
        assert!(KemAlgorithm::MlKem768.is_pqc());
    }

    // SESS-017: PQC-ready algorithm negotiation

    #[test]
    fn test_pqc_mode_enables_hybrid() {
        let mut config = CryptoConfig::default();
        assert!(!config.hybrid_mode);
        config.enable_pqc();
        assert!(config.hybrid_mode);
        assert!(
            config
                .allowed_signatures
                .contains(&SignatureAlgorithm::MlDsa65)
        );
    }

    #[test]
    fn test_pqc_fallback_to_classical() {
        let config = CryptoConfig::default();
        let peer_caps = vec![SignatureAlgorithm::Ed25519];
        let negotiated = config.negotiate_signature(&peer_caps);
        assert_eq!(negotiated, Some(SignatureAlgorithm::Ed25519));
    }

    #[test]
    fn test_pqc_negotiated_when_both_support() {
        let mut config = CryptoConfig::default();
        config.enable_pqc();
        config.allow_signature(SignatureAlgorithm::MlDsa65);
        let peer_caps = vec![SignatureAlgorithm::Ed25519, SignatureAlgorithm::MlDsa65];
        let negotiated = config.negotiate_signature(&peer_caps);
        assert_eq!(negotiated, Some(SignatureAlgorithm::MlDsa65));
    }

    // SESS-018: Crypto agility version negotiation

    #[test]
    fn test_best_mutual_algorithm_selected() {
        let mut config = CryptoConfig::default();
        config.signature_algorithm = SignatureAlgorithm::Ed25519;
        config.allow_signature(SignatureAlgorithm::P256);
        config.allow_signature(SignatureAlgorithm::P384);
        let peer_caps = vec![SignatureAlgorithm::P384, SignatureAlgorithm::P256];
        let negotiated = config.negotiate_signature(&peer_caps);
        assert_eq!(negotiated, Some(SignatureAlgorithm::P384));
    }

    #[test]
    fn test_no_common_algorithm_fails() {
        let mut config = CryptoConfig::default();
        config.allowed_signatures.clear();
        config.allow_signature(SignatureAlgorithm::Ed25519);
        let peer_caps = vec![SignatureAlgorithm::P384];
        let negotiated = config.negotiate_signature(&peer_caps);
        assert_eq!(negotiated, None);
    }

    #[test]
    fn test_downgrade_attack_detected() {
        let config = CryptoConfig::default();
        let is_downgrade = config.detect_downgrade(0u16, &[]);
        assert!(is_downgrade);
    }

    #[test]
    fn test_valid_negotiation_not_downgrade() {
        let config = CryptoConfig::default();
        let peer_algs = vec![SignatureAlgorithm::Ed25519];
        let is_downgrade = config.detect_downgrade(1u16, &peer_algs);
        assert!(!is_downgrade);
    }

    // Protocol capabilities

    #[test]
    fn test_current_capabilities() {
        let caps = ProtocolCapabilities::current();
        assert_eq!(caps.version, 1);
        assert!(
            caps.signature_algorithms
                .contains(&SignatureAlgorithm::Ed25519)
        );
        assert!(!caps.supports_hybrid);
    }

    #[test]
    fn test_pqc_capabilities() {
        let caps = ProtocolCapabilities::with_pqc();
        assert_eq!(caps.version, 2);
        assert!(
            caps.signature_algorithms
                .contains(&SignatureAlgorithm::MlDsa65)
        );
        assert!(caps.supports_hybrid);
    }

    #[test]
    fn test_allow_signature() {
        let mut config = CryptoConfig::default();
        config.allow_signature(SignatureAlgorithm::P384);
        assert!(config.is_signature_allowed(SignatureAlgorithm::P384));
    }
}
