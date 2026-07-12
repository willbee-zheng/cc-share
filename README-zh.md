# SharePlan 桌面客户端

<div align="center">

**共享闲置 AI 订阅，赚取积分，使用更多模型**

[English](./README.md) | 中文

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](./LICENSE)
[![Platform: macOS](https://img.shields.io/badge/platform-macOS-blue)](https://github.com/willbee-zheng/cc-share/releases)
[![Platform: Windows](https://img.shields.io/badge/platform-Windows-blue)](https://github.com/willbee-zheng/cc-share/releases)
[![Platform: Linux](https://img.shields.io/badge/platform-Linux-blue)](https://github.com/willbee-zheng/cc-share/releases)

</div>

SharePlan 桌面客户端是 SharePlan P2P AI 订阅共享平台的应用。它把闲置的 ChatGPT Plus、Claude Pro、API 额度聚合成一个模型池：你可以把空闲时段的订阅共享出去赚积分，也能花积分调用池子里任何人的任何模型。所有 LLM 调用都在**供应者本机**通过 [cc-switch](https://github.com/farion1231/cc-switch) 执行 —— 你的 API Key 永不离开设备。

**你的 API Key 永远不会离开本机。** cc-switch 持有密钥并执行所有上游调用，SharePlan 桌面端仅中继任务载荷和结果。

## 为什么选择 SharePlan？

- **密钥本地化** —— API Key 永不离开设备，所有上游调用由 cc-switch 完成。
- **闲置变现** —— 你的 ChatGPT Plus 大部分时间在吃灰，共享出去换取你真正需要的其他模型额度。
- **规避封号** —— 请求从供应者自己的机器、用自己的订阅发出，与正常使用无异。
- **公平计费** —— 10+ 模型透明按 token 定价，90% 积分归供应者。
- **P2P 直连** —— NAT 打洞让消费者与供应者点对点直连，无法直连时回退到云端中继。

## 工作原理

```
                        SharePlan 云端服务器
                               │
              ┌────────────────┼────────────────┐
              │                │                │
          供应者 A          供应者 B          消费者
              │                │                │
    ┌─────────┴──────┐  ┌─────┴──────┐  ┌──────┴──────────┐
    │  SharePlan     │  │ SharePlan  │  │  SharePlan      │
    │  桌面端        │  │ 桌面端     │  │  桌面端         │
    │       │        │  │     │      │  │       │         │
    │  cc-switch     │  │ cc-switch  │  │  本地服务       │
    │  :15721        │  │ :15721     │  │  :18081         │
    │       │        │  │     │      │  │       │         │
    └───────┼────────┘  └─────┼──────┘  └───────┼─────────┘
            │                 │                  │
    ┌───────┴──────┐  ┌─────┴──────┐   任何 OpenAI 客户端
    │  Claude API  │  │  GPT-4o    │   (cc-switch, Cursor,
    │  Gemini API  │  │  Gemini    │    Continue, curl...)
    └──────────────┘  └────────────┘
```

**供应者** —— 你的 cc-switch 代理接收来自云端的任务，用你的密钥调用上游 API，再把结果返回。你按 token 数赚取积分。

**消费者** —— 本地 OpenAI 兼容服务（`:18081`）将请求转发到云端，云端调度给最优供应者。你按 token 数消耗积分。

云端只负责调度、计费和签名校验 —— **永远不经手** API Key 或 LLM 内容。消费者与供应者也可通过 NAT 打洞直连，云端中继作为回退。

## 核心特性

- **供应者模式** —— 共享闲置 AI 订阅时间赚取积分（90% 的 token 费用归你）
- **消费者模式** —— 通过本地 OpenAI 兼容服务访问池中任意模型
- **P2P 直连** —— 节点间 NAT 打洞，自动回退到中继
- **浏览器扩展桥接** —— 共享 ChatGPT/Claude 网页会话，无需暴露 Cookie
- **SSE 流式传输** —— 供应者和消费者两端均支持实时流式响应
- **活动互斥** —— 当你正在使用自己的订阅时，自动暂停共享
- **HMAC-SHA256 签名** —— 加密请求签名 + nonce 防重放
- **设备指纹绑定** —— 每个节点绑定到特定机器指纹
- **内容安全过滤** —— 可配置的关键词规则
- **跨平台** —— macOS、Windows、Linux（Tauri v2）

## 前置要求

- [cc-switch](https://github.com/farion1231/cc-switch) —— 从 Releases 下载安装，配置至少一个 Provider（Anthropic、OpenAI 或 Gemini），并开启本地代理（默认 `127.0.0.1:15721`）
- SharePlan 云端服务器 —— 自建（[shareplan-cloud](https://github.com/willbee-zheng/shareplan-cloud)）或使用公共实例

## 安装

### 下载安装包（推荐）

从 [GitHub Releases](https://github.com/willbee-zheng/cc-share/releases) 下载最新版本：

| 平台 | 文件 |
|------|------|
| macOS | `.dmg` |
| Windows | `.msi` 或 `.exe` |
| Linux | `.AppImage` 或 `.deb` |

### 从源码构建

```bash
# 前置：Rust 1.85+ (stable)、Node 20+、pnpm 9+
git clone https://github.com/willbee-zheng/cc-share.git
cd cc-share
pnpm install
npx vite build           # 构建前端
cd src-tauri
cargo build --release --bin cc-share

# 或生成安装包：
# cd cc-share && npx tauri build
```

## 快速开始

### 1. 首次配置

1. 启动 SharePlan 桌面端
2. 进入 **设置** → 填写服务器地址（如 `api.shareplan.com`）
   - 应用自动检测协议：纯域名使用 WSS/HTTPS，`IP:端口` 使用 WS/HTTP
   - 测试环境可加 `http://` 前缀强制使用 HTTP
3. 点击 **浏览器登录** —— 打开网页仪表板完成认证

### 2. 作为供应者共享

1. 确保 cc-switch 在运行且至少有一个 Provider 激活
2. 切换到 **供应者** 标签页 → 点击 **刷新** —— 应看到 cc-switch 的 Provider
3. 切换到 **共享** 标签页 → 查看可共享的模型列表
4. 点击 **开始共享** —— 状态应变为 **已连接**
5. 每个经过你订阅的 token 都会为你赚取积分

共享期间：
- **活动互斥** 在你正在使用自己的订阅时自动暂停共享
- **日志** 标签页实时显示任务处理情况
- **钱包** 标签页显示累计积分和 token 统计

### 3. 作为消费者使用

1. 切换到 **消费** 标签页 → 选择 **消费者** 角色（会停止正在运行的供应者）
2. 点击 **启动本地服务** —— 在 `127.0.0.1:18081` 启动
3. 在 cc-switch 或任何 OpenAI 兼容客户端中添加自定义 Provider：
   - **Base URL**：`http://127.0.0.1:18081/v1`
   - **API Key**：任意非空字符串（鉴权由云端处理）
4. 像往常一样发送请求 —— 会被路由到最优供应者

curl 测试：
```bash
curl http://127.0.0.1:18081/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{"model":"claude-sonnet-4","messages":[{"role":"user","content":"你好"}],"stream":false}'
```

### 4. 浏览器扩展（可选）

共享 ChatGPT/Claude 网页会话，请安装 [SharePlan 浏览器扩展](https://github.com/willbee-zheng/shareplan-extension)。

## 配置

SharePlan 桌面端将所有设置存储在本地 SQLite 数据库中：

| 设置 | 说明 | 默认值 |
|------|------|--------|
| 服务器地址 | 云端服务器域名或 `主机:端口` | - |
| 角色 | `供应者`、`消费者` 或 `空闲` | `空闲` |
| 本地服务端口 | 消费者模式 OpenAI 兼容端口 | `18081` |
| 心跳间隔 | 向云端发送心跳的频率（秒） | `30` |
| 重连间隔 | 重连尝试间隔（秒） | `5` |
| 使用 HTTPS | 强制所有云端连接使用 HTTPS/WSS | 自动检测 |

### 数据存储

本地数据存储在平台特定目录：

| 平台 | 路径 |
|------|------|
| macOS | `~/Library/Application Support/com.shareplan.desktop/` |
| Windows | `%APPDATA%\com.shareplan.desktop\` |
| Linux | `~/.local/share/com.shareplan.desktop/` |

`share.db` SQLite 文件包含共享设置、钱包数据、任务日志、P2P 会话日志和认证状态。删除此目录可重置所有数据。

## 技术栈

| 层 | 技术 |
|----|------|
| 后端 | Rust (Tauri v2)、SQLite (rusqlite)、tokio、reqwest、tokio-tungstenite、Axum |
| 前端 | React 18、TypeScript、shadcn/ui 组件、Tailwind CSS |
| 协议 | WebSocket（云端连接）、HTTP（本地服务 + cc-switch 代理） |
| 认证 | JWT + HMAC-SHA256、设备指纹绑定 |

## 项目结构

```
src-tauri/src/
├── auth/              云端认证（登录、注册、Token 刷新、API Key 管理）
├── ccswitch/          cc-switch 本地代理 HTTP 客户端
├── commands/          Tauri IPC 命令处理器
├── content_filter/    基于关键词的内容审核
├── credits/           按 Token 定价、钱包
├── database/          独立 share.db（Schema v7、DAO）
├── diagnostics/       运行时诊断
├── local_server/      OpenAI 兼容 HTTP 服务器（:18081）消费者模式
├── p2p/               P2P 直连（NAT 打洞、STUN、加密）
├── share/             P2P 核心（WebSocket 客户端、daemon、supplier、consumer、
│                      protocol、signing、mutex、fingerprint、web bridge）
├── stats/             每日统计同步到云端
└── system_log/        日志管道（批量写入 + Tauri 事件推送）

src/
├── auth/              登录、注册、API Key 管理 UI
├── share/             供应者面板、配置表单、收益计算器
├── consume/           消费者面板、本地服务控制
├── wallet/            钱包面板（趋势图）
├── providers/         Provider 发现与诊断
├── settings/          服务器配置、认证状态
├── system_log/        实时日志查看器
└── lib/               API 客户端、事件、错误映射
```

## 测试

```bash
# Rust 测试（190+ 测试）
cd src-tauri && cargo test

# TypeScript 类型检查
cd .. && npx tsc --noEmit

# 前端构建
npx vite build
```

## 故障排除

### 供应者标签页显示 "cc-switch 代理不可达"

- 确保 cc-switch 在运行且本地代理已开启
- 验证：`curl http://127.0.0.1:15721/status`

### "连接中..." 一直不变

- 检查服务器地址格式 —— 只填域名即可（如 `api.shareplan.com`）
- 如果 Token 过期，重新登录
- 检查云端服务器日志中的连接尝试记录

### 本地服务返回错误

- 确保本地服务已启动（消费标签页 → 启动本地服务）
- 在设置中检查认证状态
- 确认云端服务器有供应者提供你请求的模型

## 相关项目

- [shareplan-cloud](https://github.com/willbee-zheng/shareplan-cloud) —— 云端调度、认证和计费服务器
- [shareplan-dashboard](https://github.com/willbee-zheng/shareplan-dashboard) —— 管理 Web UI
- [shareplan-extension](https://github.com/willbee-zheng/shareplan-extension) —— Chrome 浏览器扩展
- [cc-switch](https://github.com/farion1231/cc-switch) —— 本地 AI Provider 代理（独立项目）

## 许可证

[MIT](./LICENSE)
