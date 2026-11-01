-- 0009_catalog.sql
-- catalog 模块（v0.1.x）：模型组（model group）/ 部署（deployment）/ 发布（publication）
-- / 工具默认模型（tool model default）/ 聚合决策 / 目录版本状态。
--
-- 0001 核对结论：
--   * 0001 没有 catalog 相关表；本迁移全部为新建表。
--   * `model_aliases` 归 routing（0006 已改为 aliases + targets）；本迁移不修改、不迁移、不双写。
--   * `usage_records.deployment_id UUID` / `source TEXT` 是 metering 在 0001 预留的软引用列，
--     本迁移不为它们建外键（metering 不阻塞 catalog 配置变更，历史台账不因配置变更失效）。
--
-- 表归属（只写自己的表）：
--   catalog 独占：model_groups / deployments / publications / publication_targets /
--                tool_model_defaults / tool_model_default_models /
--                model_group_merge_decisions / catalog_state
--   只读引用（仅建 FK，不写对方表）：providers.upstream_models、providers.provider_accounts、
--   access.users（经发布对象语义，见下）、identity.admin_users。
--
-- 跨模块前置（实现时同步，不在本迁移内完成）：
--   1) providers 删除 provider / provider_account / upstream_model 之前必须统计 deployments 引用并
--      返回 409；否则 FK RESTRICT 会让删除以 500 失败。见 catalog/数据库设计.md §6.3。
--   2) access 增加用户组/应用能力后，publications.subject_type 的 user_group/application 才可启用；
--      v0.1.x 写 API 只接受 all_users/user（数据库允许全部枚举，解析逻辑已完整实现并 fail-closed）。
--
-- 本迁移为纯增量：不修改任何已有表/列/索引，可在已迁移库安全执行（sqlx 版本表保证只执行一次）。
-- 回滚（手工，无 down migration；需先停写）：
--   DROP TABLE catalog_state;
--   DROP TABLE model_group_merge_decisions;
--   DROP TABLE tool_model_default_models;
--   DROP TABLE tool_model_defaults;
--   DROP TABLE publication_targets;
--   DROP TABLE publications;
--   DROP TABLE deployments;
--   DROP TABLE model_groups;
-- 生产回滚前先备份；删除顺序与建表顺序相反即可（publications/model_groups 之间只剩 FK，级联清理）。

-- ---------------------------------------------------------------------------
-- 0. 目录版本状态（catalog_version 的唯一事实源）
-- ---------------------------------------------------------------------------

-- 单行版本表：每次 catalog 配置写事务内 `version = version + 1`，读路径 O(1)。
-- 客户端 `catalog_version = 'cat:' || version`；≤5s 生效由进程内快照 TTL + 写后失效保证。
CREATE TABLE catalog_state (
    id         SMALLINT PRIMARY KEY,
    version    BIGINT NOT NULL DEFAULT 0,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT catalog_state_singleton_check CHECK (id = 1)
);

INSERT INTO catalog_state (id, version) VALUES (1, 0);

-- ---------------------------------------------------------------------------
-- 1. 模型组（对外模型单位；display_name 为具体模型名 + 版本）
-- ---------------------------------------------------------------------------

CREATE TABLE model_groups (
    id                UUID PRIMARY KEY,
    client_model_name TEXT NOT NULL,                      -- 对外模型名/调用 id，大小写敏感
    display_name      TEXT NOT NULL DEFAULT '',            -- 客户端展示名（如 Claude Sonnet 4.5）
    canonical         TEXT NOT NULL,                       -- 内部 canonical（聚合/一致性校验）
    description       TEXT NOT NULL DEFAULT '',
    capabilities      JSONB NOT NULL DEFAULT '[]'::jsonb, -- 能力标签数组（组级，客户端可见）
    context_window    INTEGER,
    max_output        INTEGER,
    protocols         JSONB NOT NULL DEFAULT '[]'::jsonb, -- 入口协议数组（客户端可见）
    parameters        JSONB NOT NULL DEFAULT '{}'::jsonb, -- 参数/协议兼容提示（客户端可见）
    price_input       NUMERIC(18, 6),                      -- 内部价：每 1,000,000 tokens；仅管理台
    price_output      NUMERIC(18, 6),                      -- 内部价：每 1,000,000 tokens；仅管理台
    currency          TEXT NOT NULL DEFAULT 'CNY',
    status            TEXT NOT NULL DEFAULT 'draft',       -- draft / active / archived
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT model_groups_client_model_name_check
        CHECK (client_model_name ~ '^[A-Za-z0-9][A-Za-z0-9._:-]{0,63}$'),
    CONSTRAINT model_groups_canonical_check
        CHECK (char_length(canonical) BETWEEN 1 AND 100 AND canonical !~ '[[:space:]]'),
    CONSTRAINT model_groups_display_name_check
        CHECK (char_length(display_name) <= 128),
    CONSTRAINT model_groups_description_check
        CHECK (char_length(description) <= 500),
    CONSTRAINT model_groups_capabilities_check
        CHECK (jsonb_typeof(capabilities) = 'array'),
    CONSTRAINT model_groups_protocols_check
        CHECK (jsonb_typeof(protocols) = 'array'),
    CONSTRAINT model_groups_parameters_check
        CHECK (jsonb_typeof(parameters) = 'object'),
    CONSTRAINT model_groups_context_check
        CHECK ((context_window IS NULL OR context_window > 0)
           AND (max_output IS NULL OR max_output > 0)
           AND (max_output IS NULL OR context_window IS NULL OR max_output <= context_window)),
    CONSTRAINT model_groups_price_check
        CHECK ((price_input IS NULL OR price_input >= 0)
           AND (price_output IS NULL OR price_output >= 0)),
    CONSTRAINT model_groups_currency_check
        CHECK (currency IN ('CNY', 'USD')),
    CONSTRAINT model_groups_status_check
        CHECK (status IN ('draft', 'active', 'archived'))
);

CREATE UNIQUE INDEX model_groups_client_model_name_key
    ON model_groups (client_model_name);
CREATE INDEX idx_model_groups_canonical ON model_groups (canonical);
CREATE INDEX idx_model_groups_status ON model_groups (status);
CREATE INDEX idx_model_groups_created ON model_groups (created_at DESC, id DESC);

-- ---------------------------------------------------------------------------
-- 2. 部署（部署 = 上游模型 × 渠道账号 + 优先级/权重/启停；可暂不归属模型组）
-- ---------------------------------------------------------------------------

CREATE TABLE deployments (
    id                  UUID PRIMARY KEY,
    model_group_id      UUID REFERENCES model_groups (id) ON DELETE SET NULL,
    upstream_model_id   UUID NOT NULL REFERENCES upstream_models (id) ON DELETE RESTRICT,
    provider_account_id UUID NOT NULL REFERENCES provider_accounts (id) ON DELETE RESTRICT,
    priority            INTEGER NOT NULL DEFAULT 1,   -- 1–1000，小者优先
    weight              INTEGER NOT NULL DEFAULT 100, -- 1–100，同组加权
    enabled             BOOLEAN NOT NULL DEFAULT true,
    notes               TEXT NOT NULL DEFAULT '',
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT deployments_priority_check CHECK (priority BETWEEN 1 AND 1000),
    CONSTRAINT deployments_weight_check CHECK (weight BETWEEN 1 AND 100),
    CONSTRAINT deployments_notes_check CHECK (char_length(notes) <= 500),
    -- 一个 (上游模型, 渠道账号) 只能有一条部署：部署是物理配对，模型组归属是属性。
    CONSTRAINT deployments_upstream_account_key
        UNIQUE (upstream_model_id, provider_account_id)
);

CREATE INDEX idx_deployments_group ON deployments (model_group_id, priority, id);
CREATE INDEX idx_deployments_upstream ON deployments (upstream_model_id);
CREATE INDEX idx_deployments_account ON deployments (provider_account_id);
-- 聚合建议只扫描未入组部署；canonical/name 从 upstream_models 侧过滤（有 idx_upstream_models_canonical）。
CREATE INDEX idx_deployments_ungrouped
    ON deployments (upstream_model_id) WHERE model_group_id IS NULL;

-- ---------------------------------------------------------------------------
-- 3. 发布（模型组 × 对象；四维：visible / allowed / quota / 数据等级上限）
-- ---------------------------------------------------------------------------

-- 一个模型组一条发布记录（1:1）；对象在 publication_targets。删除发布 = 回收（物理删除）；
-- 暂停 = status 'paused'（保留对象配置，立即不可见/不可用）。
CREATE TABLE publications (
    id                   UUID PRIMARY KEY,
    model_group_id       UUID NOT NULL REFERENCES model_groups (id) ON DELETE CASCADE,
    status               TEXT NOT NULL DEFAULT 'active',
    data_level_max       SMALLINT,  -- L1–L4；可发给该模型组的最高数据等级（NULL=未限制）
    quota_rpm            INTEGER,   -- 配置面；执行面归 policy（v0.1.x 不强制）
    quota_tpm            BIGINT,    -- 配置面；执行面归 policy（v0.1.x 不强制）
    quota_daily_requests BIGINT,    -- 配置面；执行面归 policy（v0.1.x 不强制）
    note                 TEXT NOT NULL DEFAULT '',
    created_by           UUID REFERENCES admin_users (id) ON DELETE SET NULL,
    created_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT publications_status_check CHECK (status IN ('active', 'paused')),
    CONSTRAINT publications_data_level_check
        CHECK (data_level_max IS NULL OR data_level_max BETWEEN 1 AND 4),
    CONSTRAINT publications_quota_check
        CHECK ((quota_rpm IS NULL OR quota_rpm > 0)
           AND (quota_tpm IS NULL OR quota_tpm > 0)
           AND (quota_daily_requests IS NULL OR quota_daily_requests > 0)),
    CONSTRAINT publications_note_check CHECK (char_length(note) <= 500),
    CONSTRAINT publications_model_group_key UNIQUE (model_group_id)
);

CREATE INDEX idx_publications_status ON publications (status);
CREATE INDEX idx_publications_updated ON publications (updated_at DESC);

-- 发布对象：多态 subject。v0.1.x 写 API 只接受 all_users / user；
-- user_group / application / device 为预留枚举（解析逻辑完整、API 拒绝写入，见 catalog/技术设计.md §6）。
CREATE TABLE publication_targets (
    id             UUID PRIMARY KEY,
    publication_id UUID NOT NULL REFERENCES publications (id) ON DELETE CASCADE,
    subject_type   TEXT NOT NULL,
    subject_id     UUID,  -- all_users 为 NULL；其余为对端对象 ID（不建 FK：多态，见数据库设计 §6.4）
    visible        BOOLEAN NOT NULL DEFAULT true,
    allowed        BOOLEAN NOT NULL DEFAULT true,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT publication_targets_subject_type_check
        CHECK (subject_type IN ('all_users', 'user', 'user_group', 'application', 'device')),
    CONSTRAINT publication_targets_subject_check
        CHECK ((subject_type = 'all_users' AND subject_id IS NULL)
            OR (subject_type <> 'all_users' AND subject_id IS NOT NULL)),
    -- 可用必须可见：allowed=true 而 visible=false 是自相矛盾的对象。
    CONSTRAINT publication_targets_visible_allowed_check CHECK (NOT allowed OR visible),
    CONSTRAINT publication_targets_subject_key
        UNIQUE (publication_id, subject_type, subject_id)
);

-- UNIQUE 对 NULL 不生效（all_users 的 subject_id 为 NULL），补一个部分唯一索引。
CREATE UNIQUE INDEX publication_targets_all_users_key
    ON publication_targets (publication_id) WHERE subject_type = 'all_users';
CREATE INDEX idx_publication_targets_publication ON publication_targets (publication_id);
CREATE INDEX idx_publication_targets_subject ON publication_targets (subject_type, subject_id);

-- ---------------------------------------------------------------------------
-- 4. 工具默认模型（每工具默认模型 + 白名单内切换；按 global / user_group / user 覆盖）
-- ---------------------------------------------------------------------------

CREATE TABLE tool_model_defaults (
    id                     UUID PRIMARY KEY,
    tool_key               TEXT NOT NULL,  -- claude_code / codex / gemini_cli / cursor / ...（自由标识，客户端约定）
    scope_type             TEXT NOT NULL,
    scope_id               UUID,           -- global 为 NULL；user_group/user 为对端对象 ID
    default_model_group_id UUID NOT NULL REFERENCES model_groups (id) ON DELETE RESTRICT,
    allow_switch           BOOLEAN NOT NULL DEFAULT false,  -- 是否允许在白名单内切换
    created_at             TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at             TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT tool_model_defaults_tool_key_check
        CHECK (tool_key ~ '^[a-z][a-z0-9_]{0,63}$'),
    CONSTRAINT tool_model_defaults_scope_type_check
        CHECK (scope_type IN ('global', 'user_group', 'user')),
    CONSTRAINT tool_model_defaults_scope_check
        CHECK ((scope_type = 'global' AND scope_id IS NULL)
            OR (scope_type <> 'global' AND scope_id IS NOT NULL)),
    CONSTRAINT tool_model_defaults_scope_key
        UNIQUE (tool_key, scope_type, scope_id)
);

-- 同上：global 作用域的 scope_id 为 NULL，UNIQUE 不生效，补部分唯一索引。
CREATE UNIQUE INDEX tool_model_defaults_global_key
    ON tool_model_defaults (tool_key) WHERE scope_type = 'global';
CREATE INDEX idx_tool_model_defaults_group ON tool_model_defaults (default_model_group_id);
CREATE INDEX idx_tool_model_defaults_scope ON tool_model_defaults (scope_type, scope_id);

-- 可切换白名单：只收窄、不放宽（必须同时被发布允许，见解析规则）；空 = 只能使用默认模型。
CREATE TABLE tool_model_default_models (
    id              UUID PRIMARY KEY,
    tool_default_id UUID NOT NULL REFERENCES tool_model_defaults (id) ON DELETE CASCADE,
    model_group_id  UUID NOT NULL REFERENCES model_groups (id) ON DELETE CASCADE,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT tool_model_default_models_key UNIQUE (tool_default_id, model_group_id)
);

CREATE INDEX idx_tool_model_default_models_group
    ON tool_model_default_models (model_group_id);

-- ---------------------------------------------------------------------------
-- 5. 聚合建议决策（建议本身实时计算；本表持久化「确认/拒绝」以做抑制与审计）
-- ---------------------------------------------------------------------------

-- suggestion_key = 建议指纹（canonical + 上游模型名 + 排序后的部署 ID 的 sha256 前 32 hex）。
-- 拒绝后同指纹不再重复提示；部署集合变化会产生新指纹 → 可再次建议。
CREATE TABLE model_group_merge_decisions (
    id                  UUID PRIMARY KEY,
    suggestion_key      TEXT NOT NULL,
    canonical           TEXT NOT NULL,
    upstream_model_name TEXT NOT NULL,
    decision            TEXT NOT NULL,  -- merged / rejected
    model_group_id      UUID REFERENCES model_groups (id) ON DELETE SET NULL, -- merged 时记录目标组
    deployment_ids      UUID[] NOT NULL, -- 决策时的部署清单快照（审计；不建 FK）
    diff                JSONB NOT NULL DEFAULT '[]'::jsonb,
    decided_by          UUID REFERENCES admin_users (id) ON DELETE SET NULL,
    note                TEXT NOT NULL DEFAULT '',
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT model_group_merge_decisions_suggestion_key_check
        CHECK (suggestion_key ~ '^[a-f0-9]{32}$'),
    CONSTRAINT model_group_merge_decisions_decision_check
        CHECK (decision IN ('merged', 'rejected')),
    CONSTRAINT model_group_merge_decisions_diff_check
        CHECK (jsonb_typeof(diff) = 'array'),
    CONSTRAINT model_group_merge_decisions_note_check
        CHECK (char_length(note) <= 500),
    CONSTRAINT model_group_merge_decisions_key UNIQUE (suggestion_key)
);

CREATE INDEX idx_merge_decisions_canonical ON model_group_merge_decisions (canonical);
CREATE INDEX idx_merge_decisions_created ON model_group_merge_decisions (created_at DESC);
