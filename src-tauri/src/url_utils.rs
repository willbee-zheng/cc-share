//! URL helpers for deriving HTTP/WS URLs from user-provided host strings.
//!
//! The user enters a `server_host` in settings (e.g. `api.cc-share.com`,
//! `192.168.1.60:8080`, `https://api.cc-share.com`). These helpers produce
//! the correct full URL for each protocol:
//!
//! - If the user prefixes with `http://` or `https://`, that scheme is respected.
//! - If no prefix, **default to HTTP/WS** (testing environments don't have certs).
//!   For production, use an explicit `https://` or `wss://` prefix.

/// Parsed result of a user-provided host string.
struct ParsedHost {
    scheme: Option<&'static str>,
    authority: String,
}

/// Parse a user-provided host string, detecting an explicit scheme if present.
///
/// - `http://test.local`   → scheme=Some("http"), authority="test.local"
/// - `https://api.cc.com`  → scheme=Some("https"), authority="api.cc.com"
/// - `192.168.1.60:8080`   → scheme=None, authority="192.168.1.60:8080"
/// - `api.cc-share.com`    → scheme=None, authority="api.cc-share.com"
/// - `wss://api.cc.com`    → scheme=Some("wss"), authority="api.cc.com"
fn parse_host(host: &str) -> ParsedHost {
    let trimmed = host.trim();
    if trimmed.is_empty() {
        return ParsedHost {
            scheme: None,
            authority: String::new(),
        };
    }

    // Strip known scheme prefixes and record which one was used.
    if let Some(rest) = trimmed.strip_prefix("https://") {
        let authority = rest.split('/').next().unwrap_or(rest);
        return ParsedHost {
            scheme: Some("https"),
            authority: authority.to_string(),
        };
    }
    if let Some(rest) = trimmed.strip_prefix("http://") {
        let authority = rest.split('/').next().unwrap_or(rest);
        return ParsedHost {
            scheme: Some("http"),
            authority: authority.to_string(),
        };
    }
    if let Some(rest) = trimmed.strip_prefix("wss://") {
        let authority = rest.split('/').next().unwrap_or(rest);
        return ParsedHost {
            scheme: Some("wss"),
            authority: authority.to_string(),
        };
    }
    if let Some(rest) = trimmed.strip_prefix("ws://") {
        let authority = rest.split('/').next().unwrap_or(rest);
        return ParsedHost {
            scheme: Some("ws"),
            authority: authority.to_string(),
        };
    }

    // No scheme prefix — extract authority (host[:port]) from any path portion.
    let authority = trimmed.split('/').next().unwrap_or(trimmed);
    ParsedHost {
        scheme: None,
        authority: authority.to_string(),
    }
}

/// Build an HTTP base URL (e.g. `http://api.cc-share.com` or `http://192.168.1.60:8080`).
///
/// - If the user prefixed with `http://`, the result uses `http://`.
/// - If the user prefixed with `https://`, the result uses `https://`.
/// - If no prefix and `use_https` is true, the result uses `https://`.
/// - If no prefix and `use_https` is false (default), uses `http://`.
/// - WS/WSS prefixes map to HTTP/HTTPS respectively.
pub fn build_http_base(host: &str) -> String {
    build_http_base_with_tls(host, false)
}

/// Build an HTTP base URL with explicit TLS preference.
pub fn build_http_base_with_tls(host: &str, use_https: bool) -> String {
    let parsed = parse_host(host);
    if parsed.authority.is_empty() {
        return String::new();
    }
    let scheme = match parsed.scheme {
        Some("http") | Some("ws") => "http",
        Some("https") | Some("wss") => "https",
        _ => {
            if use_https {
                "https"
            } else {
                "http"
            }
        }
    };
    format!("{scheme}://{authority}", authority = parsed.authority)
}

/// Build a WebSocket base URL (e.g. `ws://api.cc-share.com` or `ws://192.168.1.60:8080`).
///
/// - If the user prefixed with `ws://`, the result uses `ws://`.
/// - If the user prefixed with `wss://`, the result uses `wss://`.
/// - If no prefix and `use_https` is true, defaults to `wss://`.
/// - If no prefix and `use_https` is false (default), defaults to `ws://`.
/// - HTTP/HTTPS prefixes map to WS/WSS respectively.
pub fn build_ws_base(host: &str) -> String {
    build_ws_base_with_tls(host, false)
}

/// Build a WebSocket base URL with explicit TLS preference.
pub fn build_ws_base_with_tls(host: &str, use_https: bool) -> String {
    let parsed = parse_host(host);
    if parsed.authority.is_empty() {
        return String::new();
    }
    let scheme = match parsed.scheme {
        Some("ws") | Some("http") => "ws",
        Some("wss") | Some("https") => "wss",
        _ => {
            if use_https {
                "wss"
            } else {
                "ws"
            }
        }
    };
    format!("{scheme}://{authority}", authority = parsed.authority)
}

/// Build the dashboard (web) base URL for browser-based login.
///
/// Derives the dashboard host from `server_host` by stripping the `api.`
/// prefix. The dashboard URL always defaults to HTTP (no HTTPS certs in
/// testing). If `server_host` has an explicit `https://` prefix, HTTPS
/// is carried over.
///
/// Examples:
/// - "api.shareplan.com"        → "http://shareplan.com"
/// - "http://api.shareplan.com"  → "http://shareplan.com"
/// - "https://api.shareplan.com" → "https://shareplan.com"
/// - "192.168.1.60:8080"        → "http://192.168.1.60:8080"
pub fn build_dashboard_base(server_host: &str) -> String {
    let parsed = parse_host(server_host);
    if parsed.authority.is_empty() {
        return String::new();
    }
    let authority = &parsed.authority;
    // Case-insensitive strip of "api." prefix — users may type "Api.shareplan.com"
    // or "API.shareplan.com", all of which should resolve to the dashboard host.
    let derived_authority = if authority.len() > 4 && authority[..4].eq_ignore_ascii_case("api.") {
        &authority[4..]
    } else {
        authority
    };

    // If server_host had an explicit scheme, carry it over to the
    // derived dashboard URL (http/ws → http, https/wss → https).
    if let Some(scheme) = parsed.scheme {
        let http_scheme = match scheme {
            "http" | "ws" => "http",
            _ => "https",
        };
        return format!("{http_scheme}://{derived_authority}");
    }

    // No explicit scheme — default to HTTP for dashboard.
    format!("http://{derived_authority}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_http_base_explicit_http() {
        assert_eq!(build_http_base("http://test.local"), "http://test.local");
    }

    #[test]
    fn test_build_http_base_explicit_https() {
        assert_eq!(build_http_base("https://api.cc-share.com"), "https://api.cc-share.com");
    }

    #[test]
    fn test_build_http_base_no_scheme_with_port() {
        assert_eq!(build_http_base("192.168.1.60:8080"), "http://192.168.1.60:8080");
    }

    #[test]
    fn test_build_http_base_no_scheme_defaults_http() {
        // Plain domain without scheme defaults to HTTP (not HTTPS)
        assert_eq!(build_http_base("api.cc-share.com"), "http://api.cc-share.com");
    }

    #[test]
    fn test_build_http_base_empty() {
        assert_eq!(build_http_base(""), "");
    }

    #[test]
    fn test_build_http_base_ws_prefix() {
        assert_eq!(build_http_base("ws://test.local"), "http://test.local");
    }

    #[test]
    fn test_build_http_base_wss_prefix() {
        assert_eq!(build_http_base("wss://api.cc-share.com"), "https://api.cc-share.com");
    }

    #[test]
    fn test_build_ws_base_explicit_ws() {
        assert_eq!(build_ws_base("ws://test.local"), "ws://test.local");
    }

    #[test]
    fn test_build_ws_base_explicit_wss() {
        assert_eq!(build_ws_base("wss://api.cc-share.com"), "wss://api.cc-share.com");
    }

    #[test]
    fn test_build_ws_base_no_scheme_with_port() {
        assert_eq!(build_ws_base("192.168.1.60:8080"), "ws://192.168.1.60:8080");
    }

    #[test]
    fn test_build_ws_base_no_scheme_defaults_ws() {
        // Plain domain without scheme defaults to WS (not WSS)
        assert_eq!(build_ws_base("api.cc-share.com"), "ws://api.cc-share.com");
    }

    #[test]
    fn test_build_ws_base_http_prefix() {
        assert_eq!(build_ws_base("http://test.local"), "ws://test.local");
    }

    #[test]
    fn test_build_ws_base_https_prefix() {
        assert_eq!(build_ws_base("https://api.cc-share.com"), "wss://api.cc-share.com");
    }

    #[test]
    fn test_build_http_base_strips_path() {
        assert_eq!(build_http_base("https://api.cc-share.com/api/v1"), "https://api.cc-share.com");
    }

    #[test]
    fn test_build_http_base_with_path_and_port() {
        assert_eq!(build_http_base("http://192.168.1.60:8080/some/path"), "http://192.168.1.60:8080");
    }

    // --- build_dashboard_base tests ---

    #[test]
    fn test_dashboard_strips_api_prefix_defaults_http() {
        assert_eq!(
            build_dashboard_base("api.shareplan.com"),
            "http://shareplan.com"
        );
    }

    #[test]
    fn test_dashboard_carries_over_http_scheme() {
        assert_eq!(
            build_dashboard_base("http://api.shareplan.com"),
            "http://shareplan.com"
        );
    }

    #[test]
    fn test_dashboard_carries_over_https_scheme() {
        assert_eq!(
            build_dashboard_base("https://api.shareplan.com"),
            "https://shareplan.com"
        );
    }

    #[test]
    fn test_dashboard_carries_over_ws_scheme() {
        assert_eq!(
            build_dashboard_base("ws://api.shareplan.com"),
            "http://shareplan.com"
        );
    }

    #[test]
    fn test_dashboard_no_api_prefix_defaults_http() {
        assert_eq!(
            build_dashboard_base("192.168.1.60:8080"),
            "http://192.168.1.60:8080"
        );
    }

    #[test]
    fn test_dashboard_plain_domain_no_api_defaults_http() {
        assert_eq!(
            build_dashboard_base("myserver.local"),
            "http://myserver.local"
        );
    }

    #[test]
    fn test_dashboard_empty() {
        assert_eq!(build_dashboard_base(""), "");
    }

    #[test]
    fn test_dashboard_strips_api_prefix_case_insensitive() {
        assert_eq!(
            build_dashboard_base("Api.shareplan.com"),
            "http://shareplan.com"
        );
        assert_eq!(
            build_dashboard_base("API.shareplan.com"),
            "http://shareplan.com"
        );
        assert_eq!(
            build_dashboard_base("aPi.shareplan.com"),
            "http://shareplan.com"
        );
    }

    #[test]
    fn test_dashboard_case_insensitive_with_scheme() {
        assert_eq!(
            build_dashboard_base("https://Api.shareplan.com"),
            "https://shareplan.com"
        );
        assert_eq!(
            build_dashboard_base("http://API.shareplan.com"),
            "http://shareplan.com"
        );
    }
}