//! Diagnostic warnings surfaced to the UI.
//!
//! Non-fatal conditions (cc-switch proxy down, no active providers, etc.)
//! are collected here so the frontend can render them without crashing
//! the supplier daemon.

use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Clone, serde::Serialize)]
pub struct DiagnosticWarning {
    pub code: String,
    pub message: String,
    /// "info" | "warn" | "error"
    pub severity: String,
}

#[derive(Default)]
pub struct Diagnostics {
    warnings: RwLock<Vec<DiagnosticWarning>>,
}

impl Diagnostics {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            warnings: RwLock::new(Vec::new()),
        })
    }

    pub async fn all(&self) -> Vec<DiagnosticWarning> {
        self.warnings.read().await.clone()
    }

    pub async fn replace(&self, warnings: Vec<DiagnosticWarning>) {
        *self.warnings.write().await = warnings;
    }

    pub async fn clear(&self) {
        self.warnings.write().await.clear();
    }
}
