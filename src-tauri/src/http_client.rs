//! Shared HTTP client builder with browser-like default headers.
//!
//! All outgoing HTTP requests must use `shareplan_client()` (or a builder
//! derived from it) so that cloud-server WAFs (1Panel, Alibaba Cloud, etc.)
//! recognise the traffic as legitimate application requests rather than
//! bot/library traffic.
//!
//! The default `reqwest::Client` sends `reqwest/<version>` as User-Agent and
//! omits common browser headers (Accept-Language, Accept-Encoding), which
//! many WAFs block with a 403 "Request forbidden by administrative rules".
//!
//! This module also disables system proxy by default (`.no_proxy()`). Desktop
//! clients connect directly to the cloud server; going through a system proxy
//! is rarely desired and can trigger WAF blocks on intermediate proxies.

use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, ACCEPT_LANGUAGE};

/// User-Agent string identifying cc-share desktop client requests.
const USER_AGENT: &str = "SharePlan/1.0 (Desktop; Tauri)";

/// Build default headers common to all cc-share API requests.
fn default_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    // WAFs often block requests that lack Accept / Accept-Language headers.
    headers.insert(ACCEPT, HeaderValue::from_static("application/json, */*"));
    headers.insert(ACCEPT_LANGUAGE, HeaderValue::from_static("zh-CN,zh;q=0.9,en;q=0.8"));
    headers
}

/// Return a `reqwest::ClientBuilder` pre-configured for cc-share API calls.
///
/// Applies: User-Agent, Accept, Accept-Language, `.no_proxy()`.
/// Callers can chain additional options (timeouts, etc.) before `.build()`.
/// For a ready-made client, use `shareplan_client()`.
pub fn shareplan_client_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .default_headers(default_headers())
        .no_proxy()
}

/// Return a ready-made `reqwest::Client` pre-configured for cc-share API calls.
///
/// Equivalent to `shareplan_client_builder().build().unwrap()`.
pub fn shareplan_client() -> reqwest::Client {
    shareplan_client_builder()
        .build()
        .expect("reqwest client with cc-share defaults")
}