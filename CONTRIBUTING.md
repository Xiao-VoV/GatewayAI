# 参与贡献

感谢参与 GatewayAI！本项目以 **AGPL-3.0** 开源（见 [LICENSE](LICENSE)），当前处于 v0.1 阶段。

## 许可证与贡献授权

- 提交代码或文档即表示你同意以 AGPL-3.0 授权该贡献，并确认你拥有相应权利；
- 项目未来可能为双许可引入 CLA/DCO，届时会在本文件与发布说明中提前公告；
- 请勿提交来源不明的代码、素材或第三方文档。

## 仓库结构

```text
server/       Rust 服务端（Axum + sqlx + PostgreSQL）
apps/admin/   管理台（React 19 + Vite）
apps/client/  桌面客户端（React 19 + Vite + Tauri 2）
contracts/    各模块 OpenAPI 契约
```

## 开发环境

- Rust stable（edition 2024，1.85+）
- Node.js 20+ 与 pnpm 11
- Docker / Docker Compose（本地 PostgreSQL）
- 编译桌面客户端还需要系统 Tauri 依赖（macOS：Xcode Command Line Tools）

## 本地启动

```bash
# 服务端
docker compose up -d postgres
cp server/.env.example server/.env
cd server && cargo run

# 管理台 / 客户端（另开终端）
pnpm install
pnpm dev:admin      # http://localhost:5173
pnpm dev:client     # http://localhost:5175
pnpm --filter @gw/client desktop:dev   # 桌面端
```

## 提交前检查

服务端（在 `server/` 目录执行）：

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
```

涉及数据库的改动还需要跑集成测试：

```bash
GW_TEST_DATABASE_URL=postgres://gw:gw@127.0.0.1:5432/gw_test \
  cargo test --all-targets -- --ignored --test-threads=1
```

前端（仓库根目录）：

```bash
pnpm install
pnpm -r typecheck
pnpm -r build
```

## API 契约

公开 API 契约在 [`contracts/`](contracts/)；`contract_tests` 会校验实现与契约的一致性。修改 API 时必须同步更新对应契约。模块设计文档暂未开源。

## 提交信息

使用 Conventional Commits：`feat:`、`fix:`、`docs:`、`refactor:`、`test:`、`chore:`。

## Pull Request

1. 从 `main` 创建分支；
2. 保持改动聚焦，附上测试或验证方式；
3. 在描述中说明变更动机、影响范围与回滚方式；
4. 新增或修改公开 API 时，附带契约变更说明。
