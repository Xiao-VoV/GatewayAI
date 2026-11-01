# @gw/client — AI Gateway 客户端

独立客户端应用（React 19 + Vite + TypeScript），只通过 HTTP 调用 `server/`，**不导入服务端代码**。

当前切片实现真实设备配对与目录同步：

- `/welcome` → `/connect`：网关地址 + 一次性配对码，调 `POST /api/client/pair`；设备密钥/虚拟 Key 存当前 `sessionStorage`（标签页生命周期，浏览器预览用）。
- 首页：`GET /api/client/device`、`GET /v1/models`、`GET /v1/model-defaults`、`GET /api/client/usage`，展示设备/Key/已发布模型/工具默认/今日与 7 天用量（仅本人）。
- 模型：本组发布投影（不含价格/供应商/canonical）。
- 我的设备 / 设置：设备与 Key 状态、同步、清除本机配对。
- token 15 分钟 TTL，访问前自动用 device_secret 交换；401 时强制刷新一次。
- AI 工具（桌面版）：Rust `scan_agents` 只读扫描 9 个 Agent；`preview_agent_config` / `apply_agent_config` / `rollback_agent_config` 为通用 agent_id 命令。新增 Agent 只需实现 `AgentAdapter` 并在 registry 注册，UI/命令不改。
- 本地路由（桌面版）：127.0.0.1 随机端口接收 `/v1/*`，注入虚拟 Key 与短期设备 token（自动刷新）后转发网关，SSE 流式透传；工具配置指向本地路由。
- 桌面 WebView 的 API/健康检查走 Rust `native_http_request`，避免 CORS；浏览器预览走 fetch/Vite 代理。
- 诊断与恢复：网关 healthz/readyz、本地路由、目录同步检查，重启本地路由，复制脱敏诊断信息。
- 托盘驻留：关闭主窗口隐藏到系统托盘，本地路由/工具配置继续运行；托盘菜单显示窗口或退出。
- 桌面集成：单实例（再次启动聚焦已有窗口）、窗口大小/位置记忆、开机自启开关（设置页）。
- Claude Code / Codex / Gemini / Aider 配置事务：写入用户级配置前备份，只写本地路由地址 + 占位 token（真实 Key 不落配置），支持一键回滚；Codex 用 toml_edit 保留格式/注释，Gemini 双文件（`.env`+settings）事务，Aider 行级 upsert。
- 真实工具 E2E：Claude Code 2.1.316、Codex 0.166、Gemini CLI 0.66、Aider 0.86.2 均经本地路由→网关→mock 上游返回 `pong`。
- Rust 后端模块化：`commands/`（薄命令）、`agents/`（trait+registry+适配器）、`config_tx/`（通用事务）、`platform/`、`router/`、`desktop/`。
- 本地路由代理 `/v1/*` 与 `/v1beta/*`（Gemini 原生协议）；`/routing` 页面展示本地路由状态、工具默认映射、模型目录与链路测试（只读，模型分配由服务端管理）。
- 真实 Claude Code 2.1.316 已通过本地路由→网关→mock 上游 E2E；网关同步兼容 `context_management`/`output_config`/messages `system` role。

## 开发

```sh
cargo run --manifest-path server/Cargo.toml   # 8080
pnpm --filter @gw/client dev                   # 浏览器预览 5175（/api、/v1 代理到 8080）

# Tauri 桌面版（需要 Rust / Xcode Command Line Tools）
pnpm --filter @gw/client desktop:dev           # 打开原生窗口
pnpm --filter @gw/client desktop:build         # 构建 .app/.dmg
```

## 设计文档

详细设计文档暂未开源；公开 API 契约见 [`contracts/`](../../contracts/)。实现分层与安全边界见本文件后续小节。

## 安全边界

- API Key 与 device_secret 不展示明文、不写 localStorage。
- Tauri 桌面版通过 Rust 命令写入系统钥匙串（macOS Keychain）；浏览器预览退化为 `sessionStorage`（标签页生命周期），关闭即失。
- 其他工具的配置写入、托盘/更新/用量、签名公证等后续切片。

## 门禁

```sh
pnpm --filter @gw/client typecheck
pnpm --filter @gw/client build
cargo test --manifest-path apps/client/src-tauri/Cargo.toml --lib
cargo check --manifest-path apps/client/src-tauri/Cargo.toml
pnpm --filter @gw/client desktop:build   # 发布前
```
