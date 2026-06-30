# SharePlan Desktop

<div align="center">

**Share idle AI subscriptions, earn credits, access more models**

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](./LICENSE)
[![Platform: macOS](https://img.shields.io/badge/platform-macOS-blue)](https://github.com/datavii/cc-share/releases)
[![Platform: Windows](https://img.shields.io/badge/platform-Windows-blue)](https://github.com/datavii/cc-share/releases)
[![Platform: Linux](https://img.shields.io/badge/platform-Linux-blue)](https://github.com/datavii/cc-share/releases)

</div>

SharePlan Desktop is the client application for the SharePlan P2P AI sharing platform. It connects to a SharePlan cloud server to relay tasks between suppliers and consumers, while all LLM calls execute locally through [cc-switch](https://github.com/farion1231/cc-switch).

**Your API keys never leave your machine.** cc-switch holds them and makes all upstream calls. SharePlan Desktop only relays task payloads and results.

## How It Works

```
                        SharePlan Cloud Server
                               │
              ┌────────────────┼────────────────┐
              │                │                │
         Supplier A       Supplier B       Consumer
              │                │                │
    ┌─────────┴──────┐  ┌─────┴──────┐  ┌──────┴──────────┐
    │  SharePlan     │  │ SharePlan  │  │  SharePlan      │
    │  Desktop       │  │ Desktop    │  │  Desktop        │
    │       │        │  │     │      │  │       │         │
    │  cc-switch     │  │ cc-switch  │  │  Local Server   │
    │  :15721        │  │ :15721     │  │  :8081          │
    │       │        │  │     │      │  │       │         │
    └───────┼────────┘  └─────┼──────┘  └───────┼─────────┘
            │                 │                  │
    ┌───────┴──────┐  ┌─────┴──────┐   Any OpenAI client
    │  Claude API  │  │  GPT-4o    │   (cc-switch, Cursor,
    │  Gemini API  │  │  Gemini    │    Continue, curl...)
    └──────────────┘  └────────────┘
```

**Supplier** — Your cc-switch proxy receives tasks from the cloud, calls upstream APIs with your keys, and sends results back. You earn credits per token.

**Consumer** — The local OpenAI-compatible server at `:8081` forwards your requests to the cloud, which dispatches them to the best available supplier. You spend credits per token.

## Features

- **Supplier mode** — Share unused AI subscription time and earn credits (90% of token cost goes to you)
- **Consumer mode** — Access any model in the pool via a local OpenAI-compatible server
- **Browser extension bridge** — Share ChatGPT/Claude web sessions without exposing cookies
- **SSE streaming** — Real-time streaming responses for both supplier and consumer
- **Activity mutex** — Automatically pauses sharing when you're actively using your own subscription
- **HMAC-SHA256 signing** — Cryptographic request signing with nonce-based replay protection
- **Device fingerprint binding** — Each node is tied to a machine fingerprint
- **Content safety filter** — Configurable keyword rules for content moderation
- **Cross-platform** — macOS, Windows, Linux (Tauri v2)

## Prerequisites

- [cc-switch](https://github.com/farion1231/cc-switch) — Install from releases and configure at least one provider (Anthropic, OpenAI, or Gemini). Enable the local proxy (default: `127.0.0.1:15721`).
- A SharePlan cloud server — Either self-host ([shareplan-cloud](https://github.com/datavii/shareplan-cloud)) or use a public instance.

## Install

### Download Release (Recommended)

Download the latest release from [GitHub Releases](https://github.com/datavii/cc-share/releases):

| Platform | File |
|----------|------|
| macOS | `.dmg` |
| Windows | `.msi` or `.exe` |
| Linux | `.AppImage` or `.deb` |

### Build from Source

```bash
# Prerequisites: Rust 1.85+ (stable), Node 20+, pnpm 9+
git clone https://github.com/datavii/cc-share.git
cd cc-share
pnpm install
npx vite build           # Build frontend
cd src-tauri
cargo build --release --bin cc-share

# Or create a full installer:
# cd cc-share && npx tauri build
```

## Getting Started

### 1. First-Time Setup

1. Launch SharePlan Desktop
2. Go to **Settings** → enter your server address (e.g., `api.shareplan.com`)
   - The app auto-detects the protocol: plain domains use WSS/HTTPS, `IP:port` uses WS/HTTP
   - Force HTTP during testing by prefixing with `http://`
3. Click **Sign in with browser** — this opens the web dashboard for authentication

### 2. Share Your Subscription (Supplier)

1. Make sure cc-switch is running with at least one active provider
2. Go to **Providers** tab → **Refresh** — your cc-switch providers should appear
3. Switch to **Share** tab → review the available models
4. Click **Start Sharing** — status should change to **Connected**
5. Earn credits for every token processed through your subscription

While sharing:
- The **activity mutex** automatically pauses sharing when you're actively using your own subscription
- The **Logs** tab shows real-time task processing
- The **Wallet** tab shows your accumulated credits and token statistics

### 3. Use the Pool (Consumer)

1. Switch to **Consume** tab → select **Consumer** role (this stops any running supplier)
2. Click **Start Local Server** — starts at `127.0.0.1:8081`
3. Add a custom provider in cc-switch or any OpenAI-compatible client:
   - **Base URL**: `http://127.0.0.1:8081/v1`
   - **API Key**: any non-empty string (auth is handled server-side)
4. Send requests as usual — they'll be routed to the best available supplier

Test with curl:
```bash
curl http://127.0.0.1:8081/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{"model":"claude-sonnet-4","messages":[{"role":"user","content":"hello"}],"stream":false}'
```

### 4. Browser Extension (Optional)

For sharing ChatGPT/Claude web sessions, install the [SharePlan browser extension](https://github.com/datavii/shareplan-extension).

## Configuration

SharePlan Desktop stores all settings in a local SQLite database. Key configurations:

| Setting | Description | Default |
|---------|-------------|---------|
| Server address | Cloud server domain or `host:port` | — |
| Role | `Supplier`, `Consumer`, or `Idle` | `Idle` |
| Local server port | Consumer mode OpenAI-compatible port | `8081` |
| Heartbeat interval | How often to send heartbeats to cloud (seconds) | `30` |
| Reconnect interval | Delay between reconnection attempts (seconds) | `5` |
| Use HTTPS | Force HTTPS/WSS for all cloud connections | auto-detected |

### Data Storage

Local data is stored in platform-specific directories:

| Platform | Path |
|----------|------|
| macOS | `~/Library/Application Support/com.shareplan.desktop/` |
| Windows | `%APPDATA%\com.shareplan.desktop\` |
| Linux | `~/.local/share/com.shareplan.desktop/` |

The `share.db` SQLite file contains sharing settings, wallet data, task logs, and auth state. Delete this directory to reset.

## Tech Stack

| Layer | Technology |
|-------|-----------|
| Backend | Rust (Tauri v2), SQLite (rusqlite), tokio, reqwest, tokio-tungstenite |
| Frontend | React 18, TypeScript, shadcn/ui components, Tailwind CSS |
| Protocol | WebSocket (cloud connection), HTTP (local server + cc-switch proxy) |
| Auth | JWT + HMAC-SHA256, device fingerprint binding |

## Project Structure

```
src-tauri/src/
├── auth/              Cloud server auth (login, register, token refresh, API keys)
├── ccswitch/          HTTP client for cc-switch local proxy
│   ├── proxy_client.rs     HTTP client with streaming support
│   ├── proxy_executor.rs   Task execution via cc-switch (Anthropic/OpenAI/Gemini formats)
│   └── provider_registry.rs Provider discovery from cc-switch /status endpoint
├── commands/          Tauri IPC command handlers
├── content_filter/    Keyword-based content moderation
├── credits/           Per-token pricing, settlement, wallet
├── database/          Independent share.db (schema v4, DAOs)
├── local_server/      OpenAI-compatible HTTP server (:8081) for consumer mode
├── share/             P2P core
│   ├── client.rs           WebSocket client with reconnect
│   ├── daemon.rs           Daemon lifecycle management
│   ├── supplier.rs         Task execution with content filter + mutex
│   ├── consumer.rs         Cloud dispatch HTTP client
│   ├── protocol.rs         Message types (NodeStatus, TaskPayload, TaskResult)
│   ├── signing.rs           HMAC-SHA256 client-side signing
│   ├── fingerprint.rs      Device fingerprint generation
│   ├── web_bridge.rs       Local WS server for browser extension (:19829)
│   └── web_executor.rs     Routes web: provider tasks to extension
└── system_log/        Log pipeline with batch writer + Tauri event emission

src/
├── auth/              Login, register, API key management UI
├── share/             Supplier panel, config form, earnings calculator
├── consume/           Consumer panel, local server controls
├── wallet/            Wallet panel with trend chart
├── providers/         Provider discovery and diagnostics
├── settings/          Server config, auth state
├── system_log/        Real-time log viewer
└── lib/              API client, events, error mapping
```

## Testing

```bash
# Rust tests (150+ tests)
cd src-tauri && cargo test

# TypeScript type check
cd .. && npx tsc --noEmit

# Frontend build
npx vite build
```

## Troubleshooting

### "cc-switch proxy unreachable" in Providers tab

- Make sure cc-switch is running with the local proxy enabled
- Verify: `curl http://127.0.0.1:15721/status`

### "Connecting..." stays stuck

- Check server address format — enter just the domain (e.g., `api.shareplan.com`)
- Re-authenticate if your token has expired
- Check cloud server logs for connection attempts

### Local server returns errors

- Make sure the local server is started (Consume tab → Start Local Server)
- Verify your auth state in Settings
- Check that the cloud server has available suppliers for your requested model

## Related Projects

- [shareplan-cloud](https://github.com/datavii/shareplan-cloud) — Cloud dispatch, auth, and billing server
- [shareplan-dashboard](https://github.com/datavii/shareplan-dashboard) — Admin web UI
- [shareplan-extension](https://github.com/datavii/shareplan-extension) — Chrome browser extension
- [cc-switch](https://github.com/farion1231/cc-switch) — Local AI provider proxy (separate project)

## License

[MIT](./LICENSE)