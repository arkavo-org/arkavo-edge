#![allow(clippy::disallowed_methods)]
#![allow(clippy::field_reassign_with_default)]

#[cfg(test)]
mod tests {
    use arkavo_protocol::config::BufferConfig;

    #[test]
    fn test_buffer_config_valid() {
        let config = BufferConfig::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_buffer_config_too_small() {
        let mut config = BufferConfig::default();
        config.chat_delta_buffer_size = 0;

        // Validation now only warns, doesn't fail
        let result = config.validate();
        assert!(result.is_ok());
    }

    #[test]
    fn test_buffer_config_too_large() {
        let mut config = BufferConfig::default();
        config.metrics_broadcast_buffer_size = 20_000;

        // Validation now only warns, doesn't fail
        let result = config.validate();
        assert!(result.is_ok());
    }

    #[test]
    fn test_buffer_config_all_valid_boundaries() {
        let mut config = BufferConfig {
            chat_delta_buffer_size: 1,
            metrics_broadcast_buffer_size: 10_000,
            telemetry_channel_buffer_size: 5_000,
            agui_broadcast_buffer_size: 100,
        };

        assert!(config.validate().is_ok());

        // Test upper boundary
        config.chat_delta_buffer_size = 10_000;
        assert!(config.validate().is_ok());
    }
}
