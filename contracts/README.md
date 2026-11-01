# API Contracts

各模块的 OpenAPI 契约（YAML）。服务端 `cargo test` 中的 `contract_tests` 会校验：

- YAML 结构合法、`$ref` 可解析；
- 操作集合与实现一致；
- 成功响应头、错误码覆盖与敏感字段约束。

修改 API 时请同步更新对应契约；完整设计文档暂未开源。

| 文件 | 模块 |
|---|---|
| `identity.yaml` | 管理台身份、Session、CSRF、限流 |
| `providers.yaml` | 供应商、渠道账号、上游模型与凭证加密 |
| `access.yaml` | 配对码、设备、设备 token、虚拟 Key |
| `policy.yaml` | 用户日预算、限流、超支拒绝 |
| `routing.yaml` | 模型别名、多目标路由、可见模型投影 |
| `proxy.yaml` | OpenAI Chat / Anthropic Messages 代理 |
| `metering.yaml` | 用量、成本、日聚合 |
| `observability.yaml` | 请求日志、审计、CSV 导出 |
| `catalog.yaml` | 模型目录与发布 |
| `settings.yaml` | 平台设置 |
