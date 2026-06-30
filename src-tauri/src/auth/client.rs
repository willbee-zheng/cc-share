//! Auth HTTP client for the SharePlan cloud server.

use crate::auth::token::AuthState;
use crate::auth::{AuthError, LoginRequest, RefreshRequest, RegisterRequest};
use crate::auth::{ApiKeyInfo, ChangePasswordRequest, CreateKeyResponse, UserProfile};
use reqwest::Client;
use serde::Deserialize;

/// HTTP client for the SharePlan cloud auth API.
pub struct AuthClient {
    http: Client,
    base_url: String,
}

impl AuthClient {
    /// Create a new auth client targeting the given cloud server URL.
    pub fn new(base_url: &str) -> Self {
        let http = crate::http_client::shareplan_client_builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .unwrap_or_default();
        Self {
            http,
            base_url: base_url.trim_end_matches('/').to_string(),
        }
    }

    /// Register a new user account.
    pub async fn register(
        &self,
        email: &str,
        password: &str,
        display_name: Option<&str>,
    ) -> Result<(AuthState, UserProfile), AuthError> {
        let body = RegisterRequest {
            email: email.to_string(),
            password: password.to_string(),
            display_name: display_name.map(|s| s.to_string()),
        };
        let resp = self
            .http
            .post(format!("{}/api/v1/auth/register", self.base_url))
            .json(&body)
            .send()
            .await
            .map_err(|e| AuthError::Network(e.to_string()))?;

        match resp.status().as_u16() {
            201 => {
                let data: RegisterResponse = resp
                    .json()
                    .await
                    .map_err(|e| AuthError::Server(e.to_string()))?;
                let state = AuthState {
                    user_id: data.user.id.clone(),
                    email: data.user.email.clone(),
                    display_name: data.user.display_name.clone(),
                    role: data.user.role.clone(),
                    access_token: data.access_token,
                    refresh_token: data.refresh_token,
                    access_expires_at: data.access_expires_at,
                };
                Ok((state, data.user))
            }
            409 => Err(AuthError::EmailExists),
            400 => {
                let err: ApiErrorResponse = resp.json().await.unwrap_or(ApiErrorResponse {
                    error: "bad request".into(),
                });
                Err(AuthError::Validation(err.error))
            }
            status => {
                let err: ApiErrorResponse = resp.json().await.unwrap_or(ApiErrorResponse {
                    error: format!("server error {status}"),
                });
                Err(AuthError::Server(err.error))
            }
        }
    }

    /// Login with email and password. Supports both new (email/password) and
    /// legacy (user_id/secret) modes — the server auto-detects the format.
    pub async fn login(&self, email: &str, password: &str) -> Result<AuthState, AuthError> {
        let body = LoginRequest {
            email: email.to_string(),
            password: password.to_string(),
        };
        let resp = self
            .http
            .post(format!("{}/api/v1/auth/login", self.base_url))
            .json(&body)
            .send()
            .await
            .map_err(|e| AuthError::Network(e.to_string()))?;

        match resp.status().as_u16() {
            200 => {
                let data: LoginResponse = resp
                    .json()
                    .await
                    .map_err(|e| AuthError::Server(e.to_string()))?;
                Ok(data.into_auth_state())
            }
            401 => Err(AuthError::InvalidCredentials),
            status => {
                let err: ApiErrorResponse = resp.json().await.unwrap_or(ApiErrorResponse {
                    error: format!("server error {status}"),
                });
                Err(AuthError::Server(err.error))
            }
        }
    }

    /// Refresh an expired access token using a refresh token.
    pub async fn refresh_token(&self, refresh_token: &str) -> Result<AuthState, AuthError> {
        let body = RefreshRequest {
            refresh_token: refresh_token.to_string(),
        };
        let resp = self
            .http
            .post(format!("{}/api/v1/auth/refresh", self.base_url))
            .json(&body)
            .send()
            .await
            .map_err(|e| AuthError::Network(e.to_string()))?;

        match resp.status().as_u16() {
            200 => {
                let data: RefreshResponse = resp
                    .json()
                    .await
                    .map_err(|e| AuthError::Server(e.to_string()))?;
                Ok(AuthState {
                    user_id: String::new(), // refreshed token keeps same user
                    email: String::new(),
                    display_name: String::new(),
                    role: String::new(),
                    access_token: data.access_token,
                    refresh_token: data.refresh_token,
                    access_expires_at: data.access_expires_at,
                })
            }
            401 => Err(AuthError::TokenExpired),
            status => {
                let err: ApiErrorResponse = resp.json().await.unwrap_or(ApiErrorResponse {
                    error: format!("server error {status}"),
                });
                Err(AuthError::Server(err.error))
            }
        }
    }

    /// Logout by revoking the refresh token.
    pub async fn logout(&self, access_token: &str, refresh_token: &str) -> Result<(), AuthError> {
        let body = RefreshRequest {
            refresh_token: refresh_token.to_string(),
        };
        let resp = self
            .http
            .post(format!("{}/api/v1/auth/logout", self.base_url))
            .header("Authorization", format!("Bearer {access_token}"))
            .json(&body)
            .send()
            .await
            .map_err(|e| AuthError::Network(e.to_string()))?;

        if resp.status().is_success() {
            Ok(())
        } else {
            Err(AuthError::Server(format!("logout failed: {}", resp.status())))
        }
    }

    /// Change the user's password.
    pub async fn change_password(
        &self,
        access_token: &str,
        current_password: &str,
        new_password: &str,
    ) -> Result<(), AuthError> {
        let body = ChangePasswordRequest {
            current_password: current_password.to_string(),
            new_password: new_password.to_string(),
        };
        let resp = self
            .http
            .post(format!("{}/api/v1/auth/change-password", self.base_url))
            .header("Authorization", format!("Bearer {access_token}"))
            .json(&body)
            .send()
            .await
            .map_err(|e| AuthError::Network(e.to_string()))?;

        match resp.status().as_u16() {
            200 => Ok(()),
            401 => Err(AuthError::InvalidCredentials),
            status => {
                let err: ApiErrorResponse = resp.json().await.unwrap_or(ApiErrorResponse {
                    error: format!("server error {status}"),
                });
                Err(AuthError::Server(err.error))
            }
        }
    }

    /// Get the authenticated user's profile.
    pub async fn get_profile(&self, access_token: &str) -> Result<UserProfile, AuthError> {
        let resp = self
            .http
            .get(format!("{}/api/v1/user/profile", self.base_url))
            .header("Authorization", format!("Bearer {access_token}"))
            .send()
            .await
            .map_err(|e| AuthError::Network(e.to_string()))?;

        match resp.status().as_u16() {
            200 => resp.json().await.map_err(|e| AuthError::Server(e.to_string())),
            401 => Err(AuthError::Unauthorized("invalid token".into())),
            status => {
                let err: ApiErrorResponse = resp.json().await.unwrap_or(ApiErrorResponse {
                    error: format!("server error {status}"),
                });
                Err(AuthError::Server(err.error))
            }
        }
    }

    /// Create a new API key. The full key is returned only once.
    pub async fn create_api_key(
        &self,
        access_token: &str,
        name: &str,
        permissions: Vec<String>,
    ) -> Result<CreateKeyResponse, AuthError> {
        let body = serde_json::json!({
            "name": name,
            "permissions": permissions,
        });
        let resp = self
            .http
            .post(format!("{}/api/v1/user/keys", self.base_url))
            .header("Authorization", format!("Bearer {access_token}"))
            .json(&body)
            .send()
            .await
            .map_err(|e| AuthError::Network(e.to_string()))?;

        match resp.status().as_u16() {
            201 => resp.json().await.map_err(|e| AuthError::Server(e.to_string())),
            401 => Err(AuthError::Unauthorized("invalid token".into())),
            status => {
                let err: ApiErrorResponse = resp.json().await.unwrap_or(ApiErrorResponse {
                    error: format!("server error {status}"),
                });
                Err(AuthError::Server(err.error))
            }
        }
    }

    /// List all API keys for the authenticated user.
    pub async fn list_api_keys(&self, access_token: &str) -> Result<Vec<ApiKeyInfo>, AuthError> {
        let resp = self
            .http
            .get(format!("{}/api/v1/user/keys", self.base_url))
            .header("Authorization", format!("Bearer {access_token}"))
            .send()
            .await
            .map_err(|e| AuthError::Network(e.to_string()))?;

        match resp.status().as_u16() {
            200 => {
                let data: KeysListResponse = resp
                    .json()
                    .await
                    .map_err(|e| AuthError::Server(e.to_string()))?;
                Ok(data.keys)
            }
            401 => Err(AuthError::Unauthorized("invalid token".into())),
            status => {
                let err: ApiErrorResponse = resp.json().await.unwrap_or(ApiErrorResponse {
                    error: format!("server error {status}"),
                });
                Err(AuthError::Server(err.error))
            }
        }
    }

    /// Revoke (delete) an API key.
    pub async fn revoke_api_key(&self, access_token: &str, key_id: &str) -> Result<(), AuthError> {
        let resp = self
            .http
            .delete(format!("{}/api/v1/user/keys/{key_id}", self.base_url))
            .header("Authorization", format!("Bearer {access_token}"))
            .send()
            .await
            .map_err(|e| AuthError::Network(e.to_string()))?;

        match resp.status().as_u16() {
            200 | 204 => Ok(()),
            401 => Err(AuthError::Unauthorized("invalid token".into())),
            404 => Err(AuthError::Validation("API key not found".into())),
            status => {
                let err: ApiErrorResponse = resp.json().await.unwrap_or(ApiErrorResponse {
                    error: format!("server error {status}"),
                });
                Err(AuthError::Server(err.error))
            }
        }
    }
}

// --- Response types ---

#[derive(Debug, Deserialize)]
struct LoginResponse {
    user: Option<UserProfile>,
    token: Option<String>,      // legacy format
    expires_at: Option<i64>,    // legacy format
    access_token: Option<String>,
    access_expires_at: Option<i64>,
    refresh_token: Option<String>,
}

impl LoginResponse {
    fn into_auth_state(self) -> AuthState {
        // Handle both new format (with user object) and legacy format (user_id/secret)
        let (access_token, refresh_token, expires_at) = if self.access_token.is_some() {
            (
                self.access_token.unwrap_or_default(),
                self.refresh_token.unwrap_or_default(),
                self.access_expires_at.unwrap_or(0),
            )
        } else {
            (
                self.token.unwrap_or_default(),
                String::new(),
                self.expires_at.unwrap_or(0),
            )
        };
        AuthState {
            user_id: self.user.as_ref().map(|u| u.id.clone()).unwrap_or_default(),
            email: self.user.as_ref().map(|u| u.email.clone()).unwrap_or_default(),
            display_name: self.user.as_ref().map(|u| u.display_name.clone()).unwrap_or_default(),
            role: self.user.as_ref().map(|u| u.role.clone()).unwrap_or_default(),
            access_token,
            refresh_token,
            access_expires_at: expires_at,
        }
    }
}

#[derive(Debug, Deserialize)]
struct RegisterResponse {
    user: UserProfile,
    access_token: String,
    access_expires_at: i64,
    refresh_token: String,
}

#[derive(Debug, Deserialize)]
struct RefreshResponse {
    access_token: String,
    access_expires_at: i64,
    refresh_token: String,
}

#[derive(Debug, Deserialize)]
struct KeysListResponse {
    keys: Vec<ApiKeyInfo>,
}

#[derive(Debug, Deserialize)]
struct ApiErrorResponse {
    error: String,
}