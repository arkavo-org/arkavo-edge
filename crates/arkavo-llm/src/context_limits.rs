/// Centralized context limit management for all LLM providers
///
/// Determines effective context limits based on:
/// 1. Device capabilities (Arduino, Raspberry Pi, Desktop)
/// 2. Provider API limits (Gemini, local models, etc.)
/// 3. Manual overrides via environment variables

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeviceType {
    /// Arduino UNO Q (QRB2210) - Adreno GPU detected
    ArduinoUnoQ,
    /// Raspberry Pi or similar low-power device (≤4 cores)
    RaspberryPi,
    /// Generic edge device
    EdgeDevice,
    /// Desktop/laptop with adequate resources
    Desktop,
    /// Manual override via environment variable
    Custom,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceProfile {
    pub max_context_tokens: u32,
    pub device_type: DeviceType,
    pub num_cores: usize,
}

impl DeviceProfile {
    /// Detect device capabilities and determine appropriate context limit
    pub fn detect() -> Self {
        // Priority 1: Manual override via ARKAVO_N_CTX
        if let Some(ctx) = std::env::var("ARKAVO_N_CTX")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
        {
            return Self {
                max_context_tokens: ctx,
                device_type: DeviceType::Custom,
                num_cores: Self::get_num_cores(),
            };
        }

        let num_cores = Self::get_num_cores();

        // Priority 2: Detect Arduino UNO Q (Qualcomm Adreno GPU)
        if Self::is_arduino_uno_q() {
            return Self {
                max_context_tokens: 2_048,
                device_type: DeviceType::ArduinoUnoQ,
                num_cores,
            };
        }

        // Priority 3: Detect Raspberry Pi or low-power devices
        if Self::is_low_power_device(num_cores) {
            return Self {
                max_context_tokens: 2_048,
                device_type: DeviceType::RaspberryPi,
                num_cores,
            };
        }

        // Priority 4: Desktop/laptop with full capabilities
        Self {
            max_context_tokens: 32_768,  // 32K for desktop systems
            device_type: DeviceType::Desktop,
            num_cores,
        }
    }

    /// Get the effective context limit for a given provider API limit
    /// Returns the minimum of device capability and provider API limit
    pub fn effective_limit(&self, provider_api_limit: u32) -> u32 {
        self.max_context_tokens.min(provider_api_limit)
    }

    fn get_num_cores() -> usize {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(8)
    }

    fn is_arduino_uno_q() -> bool {
        // Detect Qualcomm Adreno GPU (Arduino UNO Q uses QRB2210 with Adreno)
        // Exclude Apple Silicon which is also aarch64
        let is_apple_silicon = cfg!(all(target_arch = "aarch64", target_os = "macos"));

        // Check for Adreno-specific environment variable or ARM64 non-Apple
        std::env::var("GGML_VK_MAX_BATCH").is_ok()
            || (cfg!(target_arch = "aarch64") && !is_apple_silicon)
    }

    fn is_low_power_device(num_cores: usize) -> bool {
        // Explicit Raspberry Pi detection
        std::env::var("ARKAVO_RASPBERRY_PI")
            .map(|v| v == "1" || v.to_lowercase() == "true")
            .unwrap_or(false)
        // Or auto-detect based on core count (≤4 cores = likely edge device)
        || num_cores <= 4
    }

    /// Get a description of the detected device
    pub fn description(&self) -> &str {
        match self.device_type {
            DeviceType::ArduinoUnoQ => "Arduino UNO Q (Qualcomm QRB2210)",
            DeviceType::RaspberryPi => "Raspberry Pi or edge device",
            DeviceType::EdgeDevice => "Edge device",
            DeviceType::Desktop => "Desktop/laptop",
            DeviceType::Custom => "Custom (manual override)",
        }
    }
}

/// Provider-specific API context limits
#[derive(Debug, Clone, Copy)]
pub enum ProviderLimit {
    /// Gemini 2.5 Pro: 2M tokens
    GeminiPro,
    /// Gemini 2.0 Flash: 1M tokens
    GeminiFlash,
    /// Local Gemma models: 8K tokens
    LocalGemma,
    /// DeepSeek Coder: 16K tokens
    LocalDeepSeek,
    /// Custom provider with specified limit
    Custom(u32),
}

impl ProviderLimit {
    pub fn api_limit(&self) -> u32 {
        match self {
            Self::GeminiPro => 2_097_152,
            Self::GeminiFlash => 1_048_576,
            Self::LocalGemma => 8_192,
            Self::LocalDeepSeek => 16_384,
            Self::Custom(limit) => *limit,
        }
    }
}

/// Get the effective context limit considering both device and provider
pub fn get_effective_context_limit(provider: ProviderLimit) -> u32 {
    let device_profile = DeviceProfile::detect();
    let provider_limit = provider.api_limit();

    let effective = device_profile.effective_limit(provider_limit);

    tracing::debug!(
        "Context limit: {} (device: {} @ {}k, provider: {}k)",
        effective,
        device_profile.description(),
        device_profile.max_context_tokens / 1024,
        provider_limit / 1024
    );

    effective
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_device_profile_detection() {
        let profile = DeviceProfile::detect();
        assert!(profile.max_context_tokens > 0);
        assert!(profile.num_cores > 0);
    }

    #[test]
    fn test_effective_limit_respects_device() {
        // Simulate Arduino device with 2K limit
        let profile = DeviceProfile {
            max_context_tokens: 2_048,
            device_type: DeviceType::ArduinoUnoQ,
            num_cores: 4,
        };

        // Even though Gemini supports 2M, effective limit is 2K due to device
        assert_eq!(profile.effective_limit(2_097_152), 2_048);
    }

    #[test]
    fn test_effective_limit_respects_provider() {
        // Desktop device with 32K limit
        let profile = DeviceProfile {
            max_context_tokens: 32_768,
            device_type: DeviceType::Desktop,
            num_cores: 8,
        };

        // Local Gemma only supports 8K, so effective limit is 8K
        assert_eq!(profile.effective_limit(8_192), 8_192);
    }

    #[test]
    fn test_provider_limits() {
        assert_eq!(ProviderLimit::GeminiPro.api_limit(), 2_097_152);
        assert_eq!(ProviderLimit::GeminiFlash.api_limit(), 1_048_576);
        assert_eq!(ProviderLimit::LocalGemma.api_limit(), 8_192);
        assert_eq!(ProviderLimit::LocalDeepSeek.api_limit(), 16_384);
    }
}
