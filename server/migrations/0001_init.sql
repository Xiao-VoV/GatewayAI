-- 0001_init.sql
-- S1 初始表：管理台身份、供应商、设备/Key、预算、最小路由/计量/日志。
-- 约束：UUID 由应用生成；不在客户端 API 暴露密钥/价格字段。
-- 注意：本迁移不含模型目录（catalog）表，v0.1.x 单独迁移。

CREATE TABLE admin_users (
    id            UUID PRIMARY KEY,
    email         TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    display_name  TEXT NOT NULL DEFAULT '',
    status        TEXT NOT NULL DEFAULT 'active',
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE admin_sessions (
    id         UUID PRIMARY KEY,
    user_id    UUID NOT NULL REFERENCES admin_users(id) ON DELETE CASCADE,
    csrf_token TEXT NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_admin_sessions_user ON admin_sessions(user_id);
CREATE INDEX idx_admin_sessions_expires ON admin_sessions(expires_at);

-- 员工/终端用户；MVP 单人也可先建一条。
CREATE TABLE users (
    id           UUID PRIMARY KEY,
    email        TEXT NOT NULL UNIQUE,
    display_name TEXT NOT NULL DEFAULT '',
    status       TEXT NOT NULL DEFAULT 'active',
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE providers (
    id         UUID PRIMARY KEY,
    name       TEXT NOT NULL,
    kind       TEXT NOT NULL,              -- openai_compatible / anthropic / azure_openai ...
    base_url   TEXT NOT NULL,
    status     TEXT NOT NULL DEFAULT 'active',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE provider_accounts (
    id                   UUID PRIMARY KEY,
    provider_id          UUID NOT NULL REFERENCES providers(id) ON DELETE CASCADE,
    name                 TEXT NOT NULL,
    region               TEXT NOT NULL DEFAULT '',
    credential_encrypted BYTEA NOT NULL,
    weight               INTEGER NOT NULL DEFAULT 100 CHECK (weight >= 0 AND weight <= 100),
    priority             INTEGER NOT NULL DEFAULT 1,
    rpm_limit            INTEGER,
    tpm_limit            BIGINT,
    health               TEXT NOT NULL DEFAULT 'unknown',
    status               TEXT NOT NULL DEFAULT 'active',
    created_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at           TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_provider_accounts_provider ON provider_accounts(provider_id);

CREATE TABLE upstream_models (
    id             UUID PRIMARY KEY,
    provider_id    UUID NOT NULL REFERENCES providers(id) ON DELETE CASCADE,
    name           TEXT NOT NULL,          -- 上游模型名，如 claude-sonnet-4-5
    canonical      TEXT NOT NULL,          -- 内部 canonical，仅管理台
    capabilities   JSONB NOT NULL DEFAULT '[]'::jsonb,
    context_window INTEGER,
    max_output     INTEGER,
    price_input    NUMERIC(18, 6),         -- 仅管理台；不返回客户端
    price_output   NUMERIC(18, 6),
    currency       TEXT NOT NULL DEFAULT 'CNY',
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_upstream_models_provider ON upstream_models(provider_id);
CREATE INDEX idx_upstream_models_canonical ON upstream_models(canonical);

CREATE TABLE devices (
    id           UUID PRIMARY KEY,
    user_id      UUID REFERENCES users(id) ON DELETE SET NULL,
    name         TEXT NOT NULL DEFAULT '',
    platform     TEXT NOT NULL DEFAULT '',
    status       TEXT NOT NULL DEFAULT 'pending',
    last_seen_at TIMESTAMPTZ,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_devices_user ON devices(user_id);

CREATE TABLE pairing_codes (
    id         UUID PRIMARY KEY,
    code_hash  TEXT NOT NULL UNIQUE,
    user_id    UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    expires_at TIMESTAMPTZ NOT NULL,
    used_at    TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_pairing_codes_expires ON pairing_codes(expires_at);

CREATE TABLE device_tokens (
    id         UUID PRIMARY KEY,
    device_id  UUID NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
    token_hash TEXT NOT NULL UNIQUE,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_device_tokens_device ON device_tokens(device_id);
CREATE INDEX idx_device_tokens_expires ON device_tokens(expires_at);

-- 一设备一 Key（D4）：device_id 唯一。
CREATE TABLE virtual_keys (
    id          UUID PRIMARY KEY,
    device_id   UUID NOT NULL UNIQUE REFERENCES devices(id) ON DELETE CASCADE,
    user_id     UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    key_prefix  TEXT NOT NULL,
    key_hash    TEXT NOT NULL UNIQUE,
    status      TEXT NOT NULL DEFAULT 'active',
    expires_at  TIMESTAMPTZ,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_virtual_keys_user ON virtual_keys(user_id);

CREATE TABLE budgets (
    id          UUID PRIMARY KEY,
    user_id     UUID NOT NULL UNIQUE REFERENCES users(id) ON DELETE CASCADE,
    daily_limit NUMERIC(18, 6) NOT NULL DEFAULT 0,
    currency    TEXT NOT NULL DEFAULT 'CNY',
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE model_aliases (
    id                UUID PRIMARY KEY,
    alias             TEXT NOT NULL UNIQUE,
    upstream_model_id UUID NOT NULL REFERENCES upstream_models(id) ON DELETE RESTRICT,
    enabled           BOOLEAN NOT NULL DEFAULT true,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE usage_records (
    id            UUID PRIMARY KEY,
    request_id    TEXT NOT NULL,
    user_id       UUID REFERENCES users(id) ON DELETE SET NULL,
    device_id     UUID REFERENCES devices(id) ON DELETE SET NULL,
    key_id        UUID REFERENCES virtual_keys(id) ON DELETE SET NULL,
    model_alias   TEXT NOT NULL,
    deployment_id UUID,
    source        TEXT NOT NULL DEFAULT '',
    input_tokens  INTEGER NOT NULL DEFAULT 0,
    output_tokens INTEGER NOT NULL DEFAULT 0,
    cost          NUMERIC(18, 6) NOT NULL DEFAULT 0,
    currency      TEXT NOT NULL DEFAULT 'CNY',
    latency_ms    INTEGER NOT NULL DEFAULT 0,
    status        TEXT NOT NULL,
    error_code    TEXT,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_usage_records_created ON usage_records(created_at);
CREATE INDEX idx_usage_records_user_created ON usage_records(user_id, created_at);
CREATE INDEX idx_usage_records_request ON usage_records(request_id);

-- 元数据日志：不存 prompt/response/Authorization/API Key。
CREATE TABLE request_logs (
    id             UUID PRIMARY KEY,
    request_id     TEXT NOT NULL,
    user_id        UUID REFERENCES users(id) ON DELETE SET NULL,
    device_id      UUID REFERENCES devices(id) ON DELETE SET NULL,
    method         TEXT NOT NULL,
    path           TEXT NOT NULL,
    model_alias    TEXT,
    upstream_model TEXT,
    provider       TEXT,
    fallback       BOOLEAN NOT NULL DEFAULT false,
    status_code    INTEGER,
    latency_ms     INTEGER NOT NULL DEFAULT 0,
    error_code     TEXT,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_request_logs_created ON request_logs(created_at);
CREATE INDEX idx_request_logs_request ON request_logs(request_id);

CREATE TABLE daily_usage_rollups (
    day           DATE NOT NULL,
    user_id       UUID REFERENCES users(id) ON DELETE CASCADE,
    model_alias   TEXT NOT NULL,
    requests      BIGINT NOT NULL DEFAULT 0,
    input_tokens  BIGINT NOT NULL DEFAULT 0,
    output_tokens BIGINT NOT NULL DEFAULT 0,
    cost          NUMERIC(18, 6) NOT NULL DEFAULT 0,
    PRIMARY KEY (day, user_id, model_alias)
);
