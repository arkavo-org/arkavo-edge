use std::env;
use std::fs;
use std::path::Path;

pub fn load_test_env() {
    let env_path = Path::new(".test.env");
    if env_path.exists() {
        let contents = fs::read_to_string(env_path).expect("Failed to read .test.env file");

        for line in contents.lines() {
            if line.trim().is_empty() || line.starts_with('#') {
                continue;
            }

            if let Some(eq_pos) = line.find('=') {
                let key = line[..eq_pos].trim();
                let value = line[eq_pos + 1..].trim();
                unsafe {
                    env::set_var(key, value);
                }
            }
        }
    }
}

pub fn ensure_api_key() -> String {
    load_test_env();
    env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY must be set in .test.env file")
}
