# SharePlan Desktop (cc-share)

> Share idle AI subscriptions, earn credits, access more models — all running locally on your machine.

SharePlan Desktop is the client application for the [SharePlan](https://github.com/datavii) P2P AI subscription sharing platform. It connects to a SharePlan cloud server to relay tasks between suppliers and consumers, while all LLM calls execute locally through [cc-switch](https://github.com/farion1231/cc-switch).

**Key principle: Your API keys never leave your machine.** cc-switch holds them and makes all upstream calls. SharePlan Desktop only relays task payloads and results.

## Features

- **Supplier mode** — Share your unused AI subscriptions and earn credits
- **Consumer mode** — Access any model in the pool via a local OpenAI-compatible server (`127.0.0.1:8081`)
- **Browser extension bridge** — Share ChatGPT/Claude web sessions without exposing cookies
- **Stream support** — Full SSE streaming for real-time responses
- **Activity mutex** — Automatically pauses sharing when you're actively using your own subscription
- **HMAC-SHA256 signing** — Cryptographic request signing with nonce-based replay protection
- **Device fingerprint binding** — Nodes are tied to a specific machine
- **Content safety filter** — Configurable keyword rules for content moderation
- **Cross-platform** — macOS, Windows, Linux (Tauri v2)

## Architecture

```
SharePlan Desktop
├── Supplier path:  cc-switch proxy → upstream API (OpenAI/Claude/Gemini)
├── Consumer path:  Local OpenAI server (:8081) → cloud server dispatch → supplier
└── Browser bridge: Local WS server (:19829) → Chrome extension → ChatGPT/Claude web
```

## Prerequisites

- [cc-switch](https://github.com/farion1231/cc-switch) installed and running with at least one provider configured
- A SharePlan cloud server (self-hosted or public instance)

## Install

### Download Release

Download the latest release for your platform from [Releases](https://github.com/datavii/cc-share/releases):

- **macOS**: `.dmg`
- **Windows**: `.msi` or `.exe`
- **Linux**: `.AppImage` or `.deb`

### Build from Source

```bash
# Prerequisites: Rust 1.85+ (stable), Node 20+, pnpm 9+
cd cc-share
pnpm install
npx vite build           # Build frontend
cd src-tauri
cargo build --release --bin cc-share
# Or create an installer:
# npx tauri build
```

## Quick Start

1. Launch SharePlan Desktop
2. Go to **Settings** → enter your server address (e.g., `api.shareplan.com`)
3. Sign in with your browser
4. **Providers** tab → Refresh to see your cc-switch providers
5. **Share** tab → Start Sharing to earn credits
6. **Consume** tab → Start Local Server, then add `http://127.0.0.1:8081/v1` as a custom provider in cc-switch or any OpenAI-compatible client

## Tech Stack

- **Backend**: Rust (Tauri v2), SQLite (rusqlite), tokio, reqwest, tokio-tungstenite
- **Frontend**: React 18, TypeScript, shadcn/ui, Tailwind CSS, react-i18next
- **Protocol**: WebSocket for cloud connection, HTTP for local server and cc-switch proxy

## Project Structure

```
src-tauri/src/
├── auth/              Cloud server auth (login, register, token refresh, API key)
├── ccswitch/          HTTP client for cc-switch local proxy
├── commands/          Tauri IPC command handlers
├── content_filter/    Content moderation rules
├── credits/           Credit pricing, settlement, wallet
├── database/          Independent share.db (schema, DAOs)
├── local_server/      OpenAI-compatible HTTP server (:8081) for consumer mode
├── share/             P2P core (WebSocket, daemon, supplier, consumer, protocol, signing)
└── system_log/        Log pipeline with batch writer
```

## Testing

```bash
# Rust tests
cd src-tauri && cargo test

# TypeScript type check
cd .. && npx tsc --noEmit
```

## Related Projects

- [shareplan-cloud](https://github.com/datavii/shareplan-cloud) — Cloud dispatch, auth, and billing server
- [shareplan-dashboard](https://github.com/datavii/shareplan-dashboard) — Admin web UI
- [shareplan-extension](https://github.com/datavii/shareplan-extension) — Chrome browser extension
- [cc-switch](https://github.com/farion1231/cc-switch) — Local AI provider proxy (separate project)

## License

[MIT](./LICENSE)