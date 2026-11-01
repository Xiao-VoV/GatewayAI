-- 0007_metering.sql
-- metering 模块设计（S4 计量 + S5 用量）：
--   1. usage_records 补齐请求级明细字段（上游/供应商/流式/TTFT/fallback/价格快照/记账口径）；
--   2. 唯一 request_id 幂等键 + 查询索引；
--   3. 重建 daily_usage_rollups（只存“已结账日”的按天/用户/Key/模型聚合）；
--   4. 新增 usage_rollup_days（日结与对账元数据，DB 级保证“明细合计 = 聚合”）。
--
-- 设计依据：设计文档（暂未开源）、技术设计.md。
--
-- 兼容性/前置条件（不满足则迁移中止，需人工处理后重跑）：
--   a) usage_records.request_id 必须无重复行（该表在 S4 前无写入方；若已手工造数需先去重）；
--   b) usage_records.status 现存行必须落在 0007 的 6 值枚举内；
--   c) daily_usage_rollups 必须为空（0001 的旧口径 {day,user,model} 在 S4 前无写入方；
--      本迁移用新口径重建，旧数据无法无损映射到 key 维度）。
--
-- 注意：
--   - 不修改已应用的 0001–0006；
--   - usage_records 不存正文/凭证；cost 恒为 CNY（S4 与 policy 预算币种一致），
--     上游价目为 USD 时由应用按 GW_USD_CNY_RATE 折算并写 fx_rate 快照；
--   - usage_rollups 只含已完成日（Asia/Shanghai 日界），当日数据查询直接走明细。

DO $$
BEGIN
    IF EXISTS (
        SELECT 1
        FROM usage_records
        GROUP BY request_id
        HAVING count(*) > 1
    ) THEN
        RAISE EXCEPTION '0007_metering: usage_records.request_id 存在重复行，需先人工去重';
    END IF;

    IF EXISTS (
        SELECT 1
        FROM usage_records
        WHERE status NOT IN (
            'success', 'upstream_error', 'timeout',
            'gateway_error', 'client_disconnect', 'rejected'
        )
    ) THEN
        RAISE EXCEPTION '0007_metering: usage_records.status 含 0007 枚举之外的历史值，需先映射';
    END IF;

    IF EXISTS (
        SELECT 1
        FROM usage_records
        WHERE source NOT IN ('', 'alias', 'catalog')
    ) THEN
        RAISE EXCEPTION '0007_metering: usage_records.source 含未知值，需先映射为 alias/catalog';
    END IF;

    IF EXISTS (
        SELECT 1
        FROM usage_records
        WHERE currency <> 'CNY'
           OR cost < 0
           OR input_tokens < 0
           OR output_tokens < 0
           OR latency_ms < 0
    ) THEN
        RAISE EXCEPTION '0007_metering: usage_records 现存行含非 CNY 成本/负值 token/cost/latency，需先清洗';
    END IF;

    IF EXISTS (SELECT 1 FROM daily_usage_rollups) THEN
        RAISE EXCEPTION '0007_metering: daily_usage_rollups 已有数据，需先核对口径并清空后再迁移';
    END IF;
END $$;

-- ---------------------------------------------------------------------------
-- 1. usage_records：请求级明细补列
-- ---------------------------------------------------------------------------

ALTER TABLE usage_records
    -- 解析结果快照（请求发生时的事实，不随 providers/routing 后续变更而变）
    ADD COLUMN resolved_upstream_model TEXT NOT NULL DEFAULT '',
    ADD COLUMN provider_id UUID REFERENCES providers(id) ON DELETE SET NULL,
    ADD COLUMN provider_name TEXT NOT NULL DEFAULT '',
    -- deployment 归属：catalog v0.1.x 的 deployments.id，soft reference（不建 FK，避免配置变更阻塞台账）
    -- 0001 已有 deployment_id/source；source 语义见下（'alias' | 'catalog'）。
    ADD COLUMN is_stream BOOLEAN NOT NULL DEFAULT false,
    ADD COLUMN fallback BOOLEAN NOT NULL DEFAULT false,
    ADD COLUMN attempt_count INTEGER NOT NULL DEFAULT 1,
    ADD COLUMN ttft_ms INTEGER,
    ADD COLUMN http_status INTEGER,
    -- 上游 usage 来源：upstream=上游返回；estimated=网关估算；missing=无 usage（如流式未回传）
    ADD COLUMN usage_source TEXT NOT NULL DEFAULT 'missing',
    -- 价目快照（每 1,000,000 tokens 单价）+ 汇率快照，用于成本回溯/对账
    ADD COLUMN unit_price_input NUMERIC(18, 6),
    ADD COLUMN unit_price_output NUMERIC(18, 6),
    ADD COLUMN price_currency TEXT NOT NULL DEFAULT 'CNY',
    ADD COLUMN fx_rate NUMERIC(18, 8);

-- source 旧默认是空串；S4 明确为解析来源。
ALTER TABLE usage_records ALTER COLUMN source SET DEFAULT 'alias';
-- 0001 时代 source 默认 ''（无写入方）；回填后再加 CHECK，避免老数据阻塞迁移。
UPDATE usage_records SET source = 'alias' WHERE source = '';

ALTER TABLE usage_records
    ADD CONSTRAINT usage_records_status_check
        CHECK (status IN (
            'success', 'upstream_error', 'timeout',
            'gateway_error', 'client_disconnect', 'rejected'
        )),
    ADD CONSTRAINT usage_records_usage_source_check
        CHECK (usage_source IN ('upstream', 'estimated', 'missing')),
    -- 解析来源：S4=alias；catalog v0.1.x=catalog
    ADD CONSTRAINT usage_records_source_check
        CHECK (source IN ('alias', 'catalog')),
    ADD CONSTRAINT usage_records_token_check
        CHECK (input_tokens >= 0 AND output_tokens >= 0),
    ADD CONSTRAINT usage_records_cost_check
        CHECK (cost >= 0),
    -- S4 成本统一 CNY（policy 预算固定 CNY）；USD 价目由应用折算后写此列
    ADD CONSTRAINT usage_records_currency_check
        CHECK (currency = 'CNY'),
    ADD CONSTRAINT usage_records_price_currency_check
        CHECK (price_currency IN ('CNY', 'USD')),
    ADD CONSTRAINT usage_records_price_check
        CHECK (
            (unit_price_input IS NULL OR unit_price_input >= 0)
            AND (unit_price_output IS NULL OR unit_price_output >= 0)
        ),
    ADD CONSTRAINT usage_records_fx_check
        CHECK (fx_rate IS NULL OR fx_rate > 0),
    ADD CONSTRAINT usage_records_latency_check
        CHECK (latency_ms >= 0),
    ADD CONSTRAINT usage_records_ttft_check
        CHECK (ttft_ms IS NULL OR ttft_ms >= 0),
    -- 只有流式请求记录 TTFT
    ADD CONSTRAINT usage_records_ttft_stream_check
        CHECK (ttft_ms IS NULL OR is_stream),
    ADD CONSTRAINT usage_records_http_status_check
        CHECK (http_status IS NULL OR http_status BETWEEN 100 AND 599),
    -- S4 proxy 最多 2 次尝试；上限 5 为 v0.1.x 多候选降级预留
    ADD CONSTRAINT usage_records_attempt_count_check
        CHECK (attempt_count BETWEEN 1 AND 5);

-- 幂等键：同一网关请求仅一条明细；proxy/重试/断连重放均 ON CONFLICT DO NOTHING。
-- 0001 的 idx_usage_records_request 由唯一索引取代，删除避免重复维护。
DROP INDEX idx_usage_records_request;
CREATE UNIQUE INDEX uq_usage_records_request_id
    ON usage_records (request_id);

CREATE INDEX idx_usage_records_key_created
    ON usage_records (key_id, created_at DESC);
CREATE INDEX idx_usage_records_model_created
    ON usage_records (model_alias, created_at DESC);
CREATE INDEX idx_usage_records_provider_created
    ON usage_records (provider_id, created_at DESC);

-- ---------------------------------------------------------------------------
-- 2. daily_usage_rollups：按天（Asia/Shanghai）/用户/Key/模型聚合
--    只写入“已完成日”；当日查询直接聚合明细，避免聚合滞后导致
--    总览与用量接口口径不一致。日结与“明细合计 = 聚合”校验记录在 usage_rollup_days。
-- ---------------------------------------------------------------------------

DROP TABLE daily_usage_rollups;

CREATE TABLE daily_usage_rollups (
    day               DATE NOT NULL,
    user_id           UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    key_id            UUID NOT NULL REFERENCES virtual_keys(id) ON DELETE CASCADE,
    model_alias       TEXT NOT NULL,
    requests          BIGINT NOT NULL DEFAULT 0,
    success_requests  BIGINT NOT NULL DEFAULT 0,
    error_requests    BIGINT NOT NULL DEFAULT 0,
    rejected_requests BIGINT NOT NULL DEFAULT 0,
    client_disconnect_requests BIGINT NOT NULL DEFAULT 0,
    input_tokens      BIGINT NOT NULL DEFAULT 0,
    output_tokens     BIGINT NOT NULL DEFAULT 0,
    cost              NUMERIC(18, 6) NOT NULL DEFAULT 0,
    computed_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (day, user_id, key_id, model_alias),
    CONSTRAINT daily_usage_rollups_count_check CHECK (
        requests = success_requests + error_requests + rejected_requests + client_disconnect_requests
    ),
    CONSTRAINT daily_usage_rollups_non_negative_check CHECK (
        requests >= 0
        AND success_requests >= 0
        AND error_requests >= 0
        AND rejected_requests >= 0
        AND client_disconnect_requests >= 0
        AND input_tokens >= 0
        AND output_tokens >= 0
        AND cost >= 0
    )
);

-- PK 前缀覆盖“日期范围”查询；下面索引覆盖按用户/Key/模型跨天筛选。
CREATE INDEX idx_daily_usage_rollups_user_day
    ON daily_usage_rollups (user_id, day);
CREATE INDEX idx_daily_usage_rollups_key_day
    ON daily_usage_rollups (key_id, day);
CREATE INDEX idx_daily_usage_rollups_model_day
    ON daily_usage_rollups (model_alias, day);

-- ---------------------------------------------------------------------------
-- 3. usage_rollup_days：日结元数据 + 对账证据
--    finalized=true 时要求明细源合计与聚合合计完全相等（误差 0），由 DB 约束兜底；
--    source_* 记录日结时从 usage_records 聚合出的权威值，rollup_* 记录写入 rollups 的值。
-- ---------------------------------------------------------------------------

CREATE TABLE usage_rollup_days (
    day             DATE PRIMARY KEY,
    finalized       BOOLEAN NOT NULL DEFAULT false,
    computed_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    source_records  BIGINT NOT NULL DEFAULT 0,
    source_cost     NUMERIC(18, 6) NOT NULL DEFAULT 0,
    rollup_records  BIGINT NOT NULL DEFAULT 0,
    rollup_cost     NUMERIC(18, 6) NOT NULL DEFAULT 0,
    CONSTRAINT usage_rollup_days_non_negative_check CHECK (
        source_records >= 0 AND source_cost >= 0
        AND rollup_records >= 0 AND rollup_cost >= 0
    ),
    CONSTRAINT usage_rollup_days_balanced_check CHECK (
        finalized = false
        OR (source_records = rollup_records AND source_cost = rollup_cost)
    )
);

CREATE INDEX idx_usage_rollup_days_finalized
    ON usage_rollup_days (finalized, day);

-- 回滚（人工，仅无生产数据时）：
--   DROP TABLE usage_rollup_days;
--   DROP TABLE daily_usage_rollups;
--   CREATE TABLE daily_usage_rollups (... 0001 原定义 ...);
--   ALTER TABLE usage_records DROP COLUMN ...（逐列）;
--   DROP INDEX uq_usage_records_request_id; CREATE INDEX idx_usage_records_request ...
--   最后将 migrations 版本回退到 0006（sqlx 无自动回滚，需重建库）。
