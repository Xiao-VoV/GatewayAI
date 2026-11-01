# @gw/admin — AI Gateway 管理台

独立管理端应用（React 19 + Vite + TypeScript），只通过 HTTP 调用 `server/`，**不导入服务端代码**。

## 开发

```sh
# 1. 启动服务端（8080）
cargo run --manifest-path server/Cargo.toml

# 2. 启动管理台（5173，/api 代理到 8080）
pnpm --filter @gw/admin dev
```

首次打开空库会进入 `/setup` 创建管理员；之后走 `/login`。

## 环境变量

| 变量 | 默认 | 说明 |
|---|---|---|
| `VITE_API_BASE` | 空（同源 `/api`） | 生产同源部署一般留空；前后端分域时填写网关 origin |

## 当前切片

- setup / login / me / logout、Session + CSRF、401 统一登出、启动状态机。
- AdminShell（运营/资源/系统导航）、总览真实数据（`GET /api/admin/overview?days=7`）。
- **供应商**：列表（状态筛选/分页总量）、创建（含首个账号）、详情（账号池/上游模型/测试连通性/启用禁用/删除）。
- **模型目录**：模型组列表/搜索/状态筛选、创建草稿、详情（编辑/状态、部署添加/启停/移除、路由预览、归档）。
- **发布**：模型组详情内创建/编辑（all_users/user/user_group targets、visible/allowed、quota/数据等级配置）、暂停/恢复、删除。
- **工具默认**：global/user_group/user 作用域、默认模型组、allow_switch、可切换白名单；增删改查。
- **员工目录预览**：按员工/设备/Key 调 `/api/admin/catalog/preview`，展示实际可见/可用模型、matched_by、工具默认与告警。
- **用户与用户组**：员工列表/编辑/禁用/生成配对码；用户组 CRUD + 成员全量替换。
- **虚拟 Key**：列表/状态筛选、白名单/RPM/过期编辑、重置（新 key 只显示一次）、吊销。
- **设备与配对**：设备列表/在线状态/Key 前缀、编辑/解绑；配对码创建/列表/作废（明文仅创建时显示）。
- **请求日志 / 审计**：筛选、详情弹窗、CSV 导出（同筛选，带 Cookie/CSRF 语义）。
- **设置**：默认供应商、日志/审计保留天数、上游超时；范围校验与保存反馈。
- **账号与会话**：当前管理员、修改密码（吊销其他会话）、退出登录。
- 全文真实 API；加载/空/错误/重试齐全。

## 门禁

```sh
pnpm --filter @gw/admin typecheck
pnpm --filter @gw/admin build
```

构建产物在 `apps/admin/dist/`（gitignore）。服务端 API 契约见 [`contracts/`](../../contracts/)；管理台产品设计文档暂未开源。
