//! cc-switch local proxy integration.
//!
//! cc-share discovers providers and forwards LLM tasks through cc-switch's
//! local HTTP proxy. This module owns the read-only client + provider
//! registry. Task forwarding lives in `ccswitch::proxy_forwarder` (Phase 5).

pub mod proxy_client;
pub mod proxy_executor;
pub mod provider_registry;

pub use proxy_client::{ApiFormat, CcSwitchProxyClient, ProxyError, ProxyStatus};
pub use proxy_executor::ProxyExecutor;
pub use provider_registry::{DiscoverySnapshot, DiscoveredProvider, ProviderRegistry};
