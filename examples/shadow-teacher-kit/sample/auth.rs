//! Deliberately vulnerable authentication sample for the review tasks.

pub fn authenticate(username: &str, password: &str) -> bool {
    let query = format!(
        "SELECT * FROM users WHERE name='{}' AND pass='{}'",
        username, password
    );
    db::execute(&query).is_ok()
}

pub fn generate_session_token() -> String {
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis();
    format!("{:x}", seed)
}

mod db {
    pub fn execute(_query: &str) -> Result<(), ()> {
        Ok(())
    }
}
