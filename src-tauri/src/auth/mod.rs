//! Auth module — cloud service authentication and API key management.
//!
//! Handles registration, login, token refresh, and API key CRUD via the
//! SharePlan cloud server REST API.

pub mod callback;
pub mod client;
pub mod refresh;
pub mod token;

pub use client::AuthClient;
pub use token::AuthState;

use serde::{Deserialize, Serialize};

/// Errors that can occur during auth operations.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AuthError {
    Network(String),
    Unauthorized(String),
    EmailExists,
    InvalidCredentials,
    TokenExpired,
    Validation(String),
    Server(String),
}

impl std::fmt::Display for AuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuthError::Network(msg) => write!(f, "network error: {msg}"),
            AuthError::Unauthorized(msg) => write!(f, "unauthorized: {msg}"),
            AuthError::EmailExists => write!(f, "email already registered"),
            AuthError::InvalidCredentials => write!(f, "invalid email or password"),
            AuthError::TokenExpired => write!(f, "token expired"),
            AuthError::Validation(msg) => write!(f, "validation error: {msg}"),
            AuthError::Server(msg) => write!(f, "server error: {msg}"),
        }
    }
}

impl std::error::Error for AuthError {}

/// Login request body.
#[derive(Debug, Serialize)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

/// Register request body.
#[derive(Debug, Serialize)]
pub struct RegisterRequest {
    pub email: String,
    pub password: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

/// Refresh token request body.
#[derive(Debug, Serialize)]
pub struct RefreshRequest {
    pub refresh_token: String,
}

/// Change password request body.
#[derive(Debug, Serialize)]
pub struct ChangePasswordRequest {
    pub current_password: String,
    pub new_password: String,
}

/// API key info returned by the server (without the full secret).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeyInfo {
    pub id: String,
    pub name: String,
    pub key_prefix: String,
    pub permissions: Vec<String>,
    pub status: String,
    #[serde(default)]
    pub last_used_at: Option<String>,
    pub created_at: String,
}

/// Create API key response (includes the full key, shown only once).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateKeyResponse {
    pub id: String,
    pub name: String,
    pub key: String,
    pub key_prefix: String,
    pub permissions: Vec<String>,
    pub created_at: String,
}

/// User profile from the server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserProfile {
    pub id: String,
    pub email: String,
    pub display_name: String,
    pub role: String,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub last_login_at: Option<String>,
}

/// Generic API error response.
#[derive(Debug, Deserialize)]
pub struct ApiError {
    pub error: String,
}