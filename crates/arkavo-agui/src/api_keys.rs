use regex::Regex;
use std::collections::HashMap;

/// Extract API keys from text and return (cleaned_text, extracted_keys)
pub fn extract_api_keys(text: &str) -> (String, HashMap<String, String>) {
    let mut keys = HashMap::new();
    let mut cleaned_text = text.to_string();

    // Pattern: KEY_NAME=value (value can have alphanumeric, hyphens, underscores)
    let key_pattern = Regex::new(r"([A-Z_]+_API_KEY)=([A-Za-z0-9\-_]+)").unwrap();

    for cap in key_pattern.captures_iter(text) {
        if let (Some(key_name), Some(key_value)) = (cap.get(1), cap.get(2)) {
            keys.insert(
                key_name.as_str().to_string(),
                key_value.as_str().to_string(),
            );

            // Remove the key=value pair from the text
            let full_match = cap.get(0).unwrap().as_str();
            cleaned_text = cleaned_text.replace(full_match, "");
        }
    }

    // Clean up extra whitespace
    cleaned_text = cleaned_text.trim().to_string();

    (cleaned_text, keys)
}

/// Set API keys in the environment
pub fn set_api_keys(keys: &HashMap<String, String>) {
    for (key, value) in keys {
        println!("AG-UI: Setting {} (value hidden)", key);
        unsafe {
            std::env::set_var(key, value);
        }
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_single_key() {
        let text = "GEMINI_API_KEY=abc123 create a counter";
        let (cleaned, keys) = extract_api_keys(text);

        assert_eq!(cleaned, "create a counter");
        assert_eq!(keys.get("GEMINI_API_KEY"), Some(&"abc123".to_string()));
    }

    #[test]
    fn test_extract_multiple_keys() {
        let text = "GEMINI_API_KEY=abc123 OPENAI_API_KEY=xyz789 create a counter";
        let (cleaned, keys) = extract_api_keys(text);

        assert_eq!(cleaned, "create a counter");
        assert_eq!(keys.get("GEMINI_API_KEY"), Some(&"abc123".to_string()));
        assert_eq!(keys.get("OPENAI_API_KEY"), Some(&"xyz789".to_string()));
    }

    #[test]
    fn test_no_keys() {
        let text = "create a counter";
        let (cleaned, keys) = extract_api_keys(text);

        assert_eq!(cleaned, "create a counter");
        assert!(keys.is_empty());
    }

    #[test]
    fn test_key_with_hyphens() {
        let text = "GEMINI_API_KEY=AIza-SyB18-nz_J7f create app";
        let (cleaned, keys) = extract_api_keys(text);

        assert_eq!(cleaned, "create app");
        assert_eq!(
            keys.get("GEMINI_API_KEY"),
            Some(&"AIza-SyB18-nz_J7f".to_string())
        );
    }
}
