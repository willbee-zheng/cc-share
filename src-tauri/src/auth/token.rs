//! Auth token persistence — stores auth state in the client_config KV table.

use crate::database::ShareDb;

/// Key used in client_config KV table for auth state.
const AUTH_STATE_KEY: &str = "auth_state_v1";

/// Authentication state persisted in client_config KV store.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AuthState {
    pub user_id: String,
    pub email: String,
    pub display_name: String,
    pub role: String,
    pub access_token: String,
    pub refresh_token: String,
    pub access_expires_at: i64, // unix timestamp
}

/// Save auth state to the database.
pub fn save_auth_state(db: &ShareDb, state: &AuthState) -> Result<(), String> {
    let json = serde_json::to_string(state).map_err(|e| format!("serialize auth state: {e}"))?;
    db.set_config(AUTH_STATE_KEY, &json).map_err(|e| e.to_string())
}

/// Load auth state from the database.
pub fn load_auth_state(db: &ShareDb) -> Result<Option<AuthState>, String> {
    let json = match db.get_config(AUTH_STATE_KEY).map_err(|e| e.to_string())? {
        Some(v) => v,
        None => return Ok(None),
    };
    let state: AuthState =
        serde_json::from_str(&json).map_err(|e| format!("deserialize auth state: {e}"))?;
    Ok(Some(state))
}

/// Clear auth state from the database (used on logout).
pub fn clear_auth_state(db: &ShareDb) -> Result<(), String> {
    db.delete_config(AUTH_STATE_KEY).map_err(|e| e.to_string())
}

/// Check if the access token is expired or will expire within `within_secs` seconds.
pub fn is_token_expired(state: &AuthState, within_secs: i64) -> bool {
    let now = chrono::Utc::now().timestamp();
    state.access_expires_at - now < within_secs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_auth_state_round_trip() {
        let state = AuthState {
            user_id: "test-user-id".to_string(),
            email: "test@example.com".to_string(),
            display_name: "Test User".to_string(),
            role: "user".to_string(),
            access_token: "jwt-token-here".to_string(),
            refresh_token: "rt-refresh-here".to_string(),
            access_expires_at: 1750000000,
        };
        let json = serde_json::to_string(&state).unwrap();
        let decoded: AuthState = serde_json::from_str(&json).unwrap();
        assert_eq!(state.user_id, decoded.user_id);
        assert_eq!(state.email, decoded.email);
        assert_eq!(state.access_token, decoded.access_token);
        assert_eq!(state.refresh_token, decoded.refresh_token);
        assert_eq!(state.access_expires_at, decoded.access_expires_at);
    }

    #[test]
    fn test_is_token_expired_with_valid_token() {
        let state = AuthState {
            user_id: String::new(),
            email: String::new(),
            display_name: String::new(),
            role: String::new(),
            access_token: String::new(),
            refresh_token: String::new(),
            access_expires_at: chrono::Utc::now().timestamp() + 3600,
        };
        assert!(!is_token_expired(&state, 300));
    }

    #[test]
    fn test_is_token_expired_with_expiring_token() {
        let state = AuthState {
            user_id: String::new(),
            email: String::new(),
            display_name: String::new(),
            role: String::new(),
            access_token: String::new(),
            refresh_token: String::new(),
            access_expires_at: chrono::Utc::now().timestamp() + 120,
        };
        assert!(is_token_expired(&state, 300));
    }

    #[test]
    fn test_is_token_expired_with_expired_token() {
        let state = AuthState {
            user_id: String::new(),
            email: String::new(),
            display_name: String::new(),
            role: String::new(),
            access_token: String::new(),
            refresh_token: String::new(),
            access_expires_at: chrono::Utc::now().timestamp() - 60,
        };
        assert!(is_token_expired(&state, 300));
    }
}