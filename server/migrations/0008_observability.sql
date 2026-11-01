-- 0008_observability.sql
-- observability 模块设计（S4 请求日志 + v0.1.x 审计）：
--   1. request_logs 补齐元数据字段（Key/设备/上游/流式/TTFT/响应大小/客户端网络信息/协议）；
--   2. 唯一 request_id 幂等键 + 管理台查询索引（时间/Key/模型/状态/供应商/trace）；
--   3. 新建 audit_logs（管理操作审计表，S4 只建表预留，写入/查询 v0.1.x 启用）。
--
-- 设计依据：设计文档（暂未开源）、技术设计.md。
--
-- 隐私红线（US-035 / D12 / FR-27）：
--   request_logs / audit_logs 只存元数据；不存请求/响应正文、Authorization、
--   x-api-key、设备 token、虚拟 Key 明文、Cookie、CSRF token。
--
-- 兼容性/前置条件：
--   a) request_id 必须无重复行（表在 S4 前无写入方；若有手工造数需先去重）；
--   b) request_logs.created_at 语义调整为“请求开始时间”：应用显式写入，
--      0001 的 DEFAULT now() 仅作兜底；历史数据（若有）按插入时间理解；
--   c) method/path 由 proxy 写入路由模板（不含 query string）。

DO $$
BEGIN
    IF EXISTS (
        SELECT 1
        FROM request_logs
        GROUP BY request_id
        HAVING count(*) > 1
    ) THEN
        RAISE EXCEPTION '0008_observability: request_logs.request_id 存在重复行，需先人工去重';
    END IF;
END $$;

-- ---------------------------------------------------------------------------
-- 1. request_logs：补齐元数据字段
-- ---------------------------------------------------------------------------

ALTER TABLE request_logs
    ADD COLUMN key_id UUID REFERENCES virtual_keys(id) ON DELETE SET NULL,
    -- Key 前缀快照（sk-gw-xxxx）：供 US-018 直接展示；重置 Key 后历史记录保留旧前缀
    ADD COLUMN key_prefix TEXT,
    ADD COLUMN provider_id UUID REFERENCES providers(id) ON DELETE SET NULL,
    -- trace 关联：S4 = request_id（同值）；v0.1.x 解析 W3C traceparent 后可不同
    ADD COLUMN trace_id TEXT,
    ADD COLUMN is_stream BOOLEAN NOT NULL DEFAULT false,
    ADD COLUMN ttft_ms INTEGER,
    -- 实际发送给客户端的响应字节数；流式为整条流累计字节；未知为 NULL
    ADD COLUMN response_size BIGINT,
    -- 仅存 IP，不存端口；X-Forwarded-For 仅在 GW_TRUST_PROXY_HEADERS=true 时采信
    ADD COLUMN client_ip TEXT,
    -- 截断到 512 字符并去除控制字符后的 UA；绝不存 Cookie/Authorization
    ADD COLUMN user_agent TEXT NOT NULL DEFAULT '',
    -- 协议维度：S4 仅 chat_completions；v0.1.x messages/responses/gemini；models=目录查询
    ADD COLUMN protocol TEXT NOT NULL DEFAULT 'chat_completions',
    ADD COLUMN upstream_protocol TEXT,
    -- 适配器标识（proxy 定义注册表：chat_passthrough / messages_to_chat / …；不建 CHECK 便于扩展）
    ADD COLUMN adapter TEXT,
    ADD COLUMN attempt_count INTEGER NOT NULL DEFAULT 1;

ALTER TABLE request_logs
    ADD CONSTRAINT request_logs_status_code_check
        CHECK (status_code IS NULL OR status_code BETWEEN 100 AND 599),
    ADD CONSTRAINT request_logs_latency_check
        CHECK (latency_ms >= 0),
    ADD CONSTRAINT request_logs_ttft_check
        CHECK (ttft_ms IS NULL OR ttft_ms >= 0),
    ADD CONSTRAINT request_logs_ttft_stream_check
        CHECK (ttft_ms IS NULL OR is_stream),
    ADD CONSTRAINT request_logs_response_size_check
        CHECK (response_size IS NULL OR response_size >= 0),
    ADD CONSTRAINT request_logs_attempt_count_check
        CHECK (attempt_count BETWEEN 1 AND 5),
    ADD CONSTRAINT request_logs_protocol_check
        CHECK (protocol IN ('chat_completions', 'messages', 'responses', 'gemini', 'models', 'other'));

COMMENT ON COLUMN request_logs.created_at IS
    '请求开始时间（UTC）；应用显式写入，DEFAULT now() 仅兜底';
COMMENT ON COLUMN request_logs.path IS
    '路由模板（如 /v1/chat/completions），不含 query string';
COMMENT ON COLUMN request_logs.status_code IS
    '返回给客户端的 HTTP 状态码；proxy 内部未映射时为 NULL';
COMMENT ON COLUMN request_logs.adapter IS
    '适配器标识注册表（proxy 定义，如 chat_passthrough/messages_to_chat）；TEXT 不建 CHECK';

-- 幂等键：同一网关请求仅一条日志；重复写入 ON CONFLICT DO NOTHING。
DROP INDEX idx_request_logs_request;
CREATE UNIQUE INDEX uq_request_logs_request_id
    ON request_logs (request_id);

-- 0001 的 idx_request_logs_created 由 (created_at DESC, id DESC) 覆盖，删除避免重复维护
DROP INDEX idx_request_logs_created;

CREATE INDEX idx_request_logs_created_id
    ON request_logs (created_at DESC, id DESC);
CREATE INDEX idx_request_logs_user_created
    ON request_logs (user_id, created_at DESC);
CREATE INDEX idx_request_logs_key_created
    ON request_logs (key_id, created_at DESC);
CREATE INDEX idx_request_logs_model_created
    ON request_logs (model_alias, created_at DESC);
CREATE INDEX idx_request_logs_provider_created
    ON request_logs (provider, created_at DESC);
CREATE INDEX idx_request_logs_status_created
    ON request_logs (status_code, created_at DESC);
CREATE INDEX idx_request_logs_trace
    ON request_logs (trace_id)
    WHERE trace_id IS NOT NULL;
-- 错误视图：列表默认只看失败时避免全表扫
CREATE INDEX idx_request_logs_errors_created
    ON request_logs (created_at DESC)
    WHERE status_code >= 400 OR error_code IS NOT NULL;

-- ---------------------------------------------------------------------------
-- 2. audit_logs：管理操作审计（v0.1.x 启用写入与查询）
--    只追加，不提供 UPDATE/DELETE 业务路径；保留期清理是唯一合法删除。
--    detail 为脱敏后的前后值摘要（JSONB），绝不存密钥/凭证/正文。
-- ---------------------------------------------------------------------------

CREATE TABLE audit_logs (
    id            UUID PRIMARY KEY,
    -- 事件发生时间（UTC，应用显式写入）
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    actor_type    TEXT NOT NULL,
    -- 管理员操作：actor_id=admin_users.id；登录失败等匿名事件为 NULL
    actor_id      UUID REFERENCES admin_users(id) ON DELETE SET NULL,
    -- 操作者邮箱快照（登录失败记录尝试邮箱；成功后记录管理员邮箱）
    actor_email   TEXT,
    -- 动作注册表见 observability/数据库设计.md §2.3；TEXT 避免每加动作都迁移
    action        TEXT NOT NULL,
    resource_type TEXT NOT NULL DEFAULT '',
    resource_id   TEXT,
    outcome       TEXT NOT NULL,
    client_ip     TEXT,
    user_agent    TEXT NOT NULL DEFAULT '',
    request_id    TEXT,
    detail        JSONB NOT NULL DEFAULT '{}'::jsonb,
    CONSTRAINT audit_logs_actor_type_check
        CHECK (actor_type IN ('admin', 'system', 'anonymous')),
    CONSTRAINT audit_logs_outcome_check
        CHECK (outcome IN ('success', 'failure', 'denied'))
);

COMMENT ON TABLE audit_logs IS
    '管理操作审计：只追加；保留期清理是唯一删除路径；detail 必须已脱敏';

CREATE INDEX idx_audit_logs_created_id
    ON audit_logs (created_at DESC, id DESC);
CREATE INDEX idx_audit_logs_actor_created
    ON audit_logs (actor_id, created_at DESC);
CREATE INDEX idx_audit_logs_action_created
    ON audit_logs (action, created_at DESC);
CREATE INDEX idx_audit_logs_resource_created
    ON audit_logs (resource_type, resource_id, created_at DESC);
CREATE INDEX idx_audit_logs_outcome_created
    ON audit_logs (outcome, created_at DESC);

-- 回滚（人工，仅无生产数据时）：
--   DROP TABLE audit_logs;
--   DROP INDEX uq_request_logs_request_id / idx_request_logs_*;
--   ALTER TABLE request_logs DROP COLUMN ...（逐列）；
--   重建 0001 的 idx_request_logs_request / idx_request_logs_created。
