//! SharePlan standalone Tauri app entry.
//!
//! cc-share is now an independent desktop application (not a cc-switch plugin).
//! It reads `~/.cc-switch/cc-switch.db` read-only to discover providers,
//! runs a built-in LLM client to forward supplier tasks, and exposes a local
//! OpenAI-compatible HTTP server for consumers.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    shareplan_lib::run();
}
