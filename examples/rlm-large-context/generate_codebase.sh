#!/bin/bash
# Generate a synthetic large codebase for RLM demo
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="$SCRIPT_DIR/synthetic_repo"

echo "━━━━━━ Generating Synthetic Codebase ━━━━━━"
echo ""

# Clean previous
rm -rf "$REPO_DIR"
mkdir -p "$REPO_DIR/src/auth" "$REPO_DIR/src/db" "$REPO_DIR/src/api"

# Generate main.rs (~4K tokens)
cat > "$REPO_DIR/src/main.rs" << 'EOF'
//! Main entry point for the synthetic application
//! This is a demo codebase for RLM testing

use auth::AuthService;
use db::Database;
use api::Server;

mod auth;
mod db;
mod api;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::init();

    // Connect to database
    let db = Database::connect("postgres://localhost/demo").await?;

    // Initialize auth service
    let auth = AuthService::new(db.clone());

    // Start API server
    let server = Server::new(auth, db);
    server.run("0.0.0.0:8080").await?;

    Ok(())
}
EOF

# Generate auth/mod.rs
cat > "$REPO_DIR/src/auth/mod.rs" << 'EOF'
//! Authentication module
//! Handles user login, registration, and session management

pub mod login;
pub mod password;
pub mod session;
pub mod tokens;

use crate::db::Database;
use std::sync::Arc;

pub struct AuthService {
    db: Arc<Database>,
    secret_key: String,
}

impl AuthService {
    pub fn new(db: Arc<Database>) -> Self {
        // WARNING: Hardcoded secret key - security vulnerability!
        let secret_key = "super_secret_key_123".to_string();
        Self { db, secret_key }
    }

    pub async fn authenticate(&self, username: &str, password: &str) -> Result<User, AuthError> {
        let user = self.db.get_user_by_username(username).await?;

        if password::verify(password, &user.password_hash) {
            Ok(user)
        } else {
            Err(AuthError::InvalidCredentials)
        }
    }
}

#[derive(Debug)]
pub struct User {
    pub id: i64,
    pub username: String,
    pub email: String,
    pub password_hash: String,
}

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("Invalid credentials")]
    InvalidCredentials,
    #[error("User not found")]
    UserNotFound,
    #[error("Database error: {0}")]
    Database(#[from] DbError),
}
EOF

# Generate auth/password.rs with INTENTIONAL VULNERABILITY
cat > "$REPO_DIR/src/auth/password.rs" << 'EOF'
//! Password hashing utilities
//!
//! SECURITY REVIEW NEEDED: This module handles sensitive password operations

use md5;  // WARNING: MD5 is cryptographically broken!

/// Hash a password for storage
///
/// # Security Warning
/// This implementation uses MD5 which is NOT secure for password hashing.
/// Should use bcrypt, argon2, or scrypt instead.
pub fn hash(password: &str) -> String {
    // VULNERABILITY: Using MD5 for password hashing
    let digest = md5::compute(password.as_bytes());
    format!("{:x}", digest)
}

/// Verify a password against its hash
pub fn verify(password: &str, hash: &str) -> bool {
    // VULNERABILITY: Timing attack possible with direct comparison
    hash(password) == hash
}

/// Generate a random salt
///
/// Note: Not actually used because MD5 implementation doesn't use salt
pub fn generate_salt() -> String {
    // VULNERABILITY: Not using salt at all
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_password() {
        let hash = hash("password123");
        assert!(!hash.is_empty());
    }

    #[test]
    fn test_verify_password() {
        let hashed = hash("password123");
        assert!(verify("password123", &hashed));
    }
}
EOF

# Generate db/mod.rs
cat > "$REPO_DIR/src/db/mod.rs" << 'EOF'
//! Database module
//! Handles all database operations

pub mod queries;
pub mod migrations;
pub mod pool;

use sqlx::PgPool;
use std::sync::Arc;

pub struct Database {
    pool: PgPool,
}

impl Database {
    pub async fn connect(url: &str) -> Result<Arc<Self>, DbError> {
        let pool = PgPool::connect(url).await?;
        Ok(Arc::new(Self { pool }))
    }

    pub async fn get_user_by_username(&self, username: &str) -> Result<User, DbError> {
        queries::get_user_by_username(&self.pool, username).await
    }

    pub async fn get_user_by_id(&self, id: i64) -> Result<User, DbError> {
        queries::get_user_by_id(&self.pool, id).await
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("Database connection error: {0}")]
    Connection(#[from] sqlx::Error),
    #[error("Record not found")]
    NotFound,
}
EOF

# Generate db/queries.rs with INTENTIONAL SQL INJECTION
cat > "$REPO_DIR/src/db/queries.rs" << 'EOF'
//! Database queries
//!
//! WARNING: This module contains SQL injection vulnerabilities for demo purposes

use sqlx::PgPool;
use crate::auth::User;
use super::DbError;

/// Get user by username
///
/// # Security Warning
/// This query is vulnerable to SQL injection!
pub async fn get_user_by_username(pool: &PgPool, username: &str) -> Result<User, DbError> {
    // VULNERABILITY: SQL Injection - string concatenation instead of parameterized query
    let query = format!(
        "SELECT id, username, email, password_hash FROM users WHERE username = '{}'",
        username  // NOT SANITIZED!
    );

    let row = sqlx::query_as::<_, UserRow>(&query)
        .fetch_one(pool)
        .await?;

    Ok(User {
        id: row.id,
        username: row.username,
        email: row.email,
        password_hash: row.password_hash,
    })
}

/// Get user by ID - this one is safe
pub async fn get_user_by_id(pool: &PgPool, id: i64) -> Result<User, DbError> {
    // SAFE: Using parameterized query
    let row = sqlx::query_as!(
        UserRow,
        "SELECT id, username, email, password_hash FROM users WHERE id = $1",
        id
    )
    .fetch_one(pool)
    .await?;

    Ok(User {
        id: row.id,
        username: row.username,
        email: row.email,
        password_hash: row.password_hash,
    })
}

/// Search users - also vulnerable
pub async fn search_users(pool: &PgPool, search_term: &str) -> Result<Vec<User>, DbError> {
    // VULNERABILITY: SQL Injection in LIKE clause
    let query = format!(
        "SELECT id, username, email, password_hash FROM users WHERE username LIKE '%{}%'",
        search_term  // NOT SANITIZED!
    );

    let rows = sqlx::query_as::<_, UserRow>(&query)
        .fetch_all(pool)
        .await?;

    Ok(rows.into_iter().map(|r| User {
        id: r.id,
        username: r.username,
        email: r.email,
        password_hash: r.password_hash,
    }).collect())
}

#[derive(sqlx::FromRow)]
struct UserRow {
    id: i64,
    username: String,
    email: String,
    password_hash: String,
}
EOF

# Generate api/mod.rs
cat > "$REPO_DIR/src/api/mod.rs" << 'EOF'
//! API module
//! HTTP server and endpoint handlers

pub mod auth;
pub mod users;
pub mod middleware;

use crate::auth::AuthService;
use crate::db::Database;
use axum::{Router, routing::{get, post}};
use std::sync::Arc;

pub struct Server {
    auth: Arc<AuthService>,
    db: Arc<Database>,
}

impl Server {
    pub fn new(auth: AuthService, db: Arc<Database>) -> Self {
        Self {
            auth: Arc::new(auth),
            db,
        }
    }

    pub async fn run(&self, addr: &str) -> Result<(), Box<dyn std::error::Error>> {
        let app = Router::new()
            .route("/health", get(|| async { "OK" }))
            .route("/api/login", post(auth::login))
            .route("/api/register", post(auth::register))
            .route("/api/users", get(users::list))
            .route("/api/users/:id", get(users::get))
            .with_state(AppState {
                auth: self.auth.clone(),
                db: self.db.clone(),
            });

        let listener = tokio::net::TcpListener::bind(addr).await?;
        axum::serve(listener, app).await?;

        Ok(())
    }
}

#[derive(Clone)]
pub struct AppState {
    pub auth: Arc<AuthService>,
    pub db: Arc<Database>,
}
EOF

# Generate api/auth.rs with MISSING RATE LIMITING
cat > "$REPO_DIR/src/api/auth.rs" << 'EOF'
//! Authentication API endpoints
//!
//! SECURITY REVIEW: Missing rate limiting on login endpoint

use axum::{Json, extract::State};
use serde::{Deserialize, Serialize};
use super::AppState;

#[derive(Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Serialize)]
pub struct LoginResponse {
    pub token: String,
    pub expires_in: u64,
}

/// Login endpoint
///
/// # Security Warning
/// This endpoint has NO rate limiting, making it vulnerable to:
/// - Brute force attacks
/// - Credential stuffing
/// - Denial of service
pub async fn login(
    State(state): State<AppState>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, ApiError> {
    // VULNERABILITY: No rate limiting!
    // An attacker can try unlimited passwords

    let user = state.auth.authenticate(&req.username, &req.password).await?;

    // Generate JWT token
    let token = generate_token(&user)?;

    Ok(Json(LoginResponse {
        token,
        expires_in: 3600,
    }))
}

/// Register endpoint - also missing rate limiting
pub async fn register(
    State(state): State<AppState>,
    Json(req): Json<RegisterRequest>,
) -> Result<Json<RegisterResponse>, ApiError> {
    // VULNERABILITY: No rate limiting on registration
    // Could be used to enumerate usernames or spam accounts

    // Also missing: email verification, CAPTCHA, etc.

    let user = state.db.create_user(&req.username, &req.email, &req.password).await?;

    Ok(Json(RegisterResponse {
        id: user.id,
        username: user.username,
    }))
}

#[derive(Deserialize)]
pub struct RegisterRequest {
    pub username: String,
    pub email: String,
    pub password: String,
}

#[derive(Serialize)]
pub struct RegisterResponse {
    pub id: i64,
    pub username: String,
}

fn generate_token(user: &User) -> Result<String, ApiError> {
    // Token generation logic
    Ok(format!("token_for_{}", user.id))
}

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("Authentication failed")]
    AuthFailed,
    #[error("Database error")]
    Database,
}
EOF

# Generate additional filler files to reach ~100K tokens
echo "Generating additional source files..."

for i in {1..20}; do
    cat > "$REPO_DIR/src/utils_$i.rs" << EOF
//! Utility module $i
//! Contains helper functions and common utilities

use std::collections::HashMap;
use std::sync::Arc;

/// Configuration for module $i
pub struct Config$i {
    pub enabled: bool,
    pub max_retries: u32,
    pub timeout_ms: u64,
    pub cache_size: usize,
}

impl Default for Config$i {
    fn default() -> Self {
        Self {
            enabled: true,
            max_retries: 3,
            timeout_ms: 5000,
            cache_size: 1000,
        }
    }
}

/// Process data for module $i
pub async fn process_data_$i(input: &str) -> Result<String, Error$i> {
    // Simulate some processing
    let mut result = String::new();
    for line in input.lines() {
        if !line.is_empty() {
            result.push_str(&line.to_uppercase());
            result.push('\n');
        }
    }
    Ok(result)
}

/// Cache implementation for module $i
pub struct Cache$i<K, V> {
    data: HashMap<K, V>,
    max_size: usize,
}

impl<K: std::hash::Hash + Eq, V> Cache$i<K, V> {
    pub fn new(max_size: usize) -> Self {
        Self {
            data: HashMap::new(),
            max_size,
        }
    }

    pub fn get(&self, key: &K) -> Option<&V> {
        self.data.get(key)
    }

    pub fn insert(&mut self, key: K, value: V) {
        if self.data.len() >= self.max_size {
            // Simple eviction: remove first entry
            if let Some(k) = self.data.keys().next().cloned() {
                self.data.remove(&k);
            }
        }
        self.data.insert(key, value);
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error$i {
    #[error("Processing error in module $i")]
    Processing,
    #[error("Cache error in module $i")]
    Cache,
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_default_$i() {
        let config = Config$i::default();
        assert!(config.enabled);
        assert_eq!(config.max_retries, 3);
    }

    #[test]
    fn test_cache_$i() {
        let mut cache = Cache$i::new(10);
        cache.insert("key1", "value1");
        assert_eq!(cache.get(&"key1"), Some(&"value1"));
    }
}
EOF
done

# Count approximate tokens
TOTAL_BYTES=$(find "$REPO_DIR" -name "*.rs" -exec cat {} \; | wc -c)
APPROX_TOKENS=$((TOTAL_BYTES / 4))

echo ""
echo "━━━━━━ Codebase Generated ━━━━━━"
echo ""
echo "Location: $REPO_DIR"
echo "Files: $(find "$REPO_DIR" -name "*.rs" | wc -l | tr -d ' ')"
echo "Total size: $TOTAL_BYTES bytes"
echo "Approx tokens: ~$APPROX_TOKENS"
echo ""
echo "Security vulnerabilities planted:"
echo "  - auth/password.rs: MD5 password hashing (CRITICAL)"
echo "  - db/queries.rs: SQL injection (HIGH)"
echo "  - api/auth.rs: Missing rate limiting (MEDIUM)"
echo ""
echo "Ready for RLM analysis!"
