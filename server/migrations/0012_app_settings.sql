-- 0012_app_settings.sql
-- 平台设置持久化（US-024 / S5.3）：单例运行期设置。
--
-- 权威约定（设计文档暂未开源）：
--   - app_settings 单行（id=1）为运行期权威；env GW_* 仅 DB 不可用兜底；
--   - request_logs/audit_logs 保留天数、上游超时、默认供应商由此持久化；
--   - PATCH 后进程内 runtime 立即生效（清理任务下一 tick 生效）。

CREATE TABLE app_settings (
    id SMALLINT PRIMARY KEY CHECK (id = 1),
    -- 默认供应商（管理默认值；路由兜底语义 v0.1.x 另行接入）
    default_provider_id UUID REFERENCES providers(id) ON DELETE SET NULL,
    -- 请求日志/审计保留天数（与 observability 清理任务读取的 runtime 一致）
    request_log_retention_days INTEGER NOT NULL DEFAULT 30
        CHECK (request_log_retention_days BETWEEN 1 AND 365),
    audit_retention_days INTEGER NOT NULL DEFAULT 180
        CHECK (audit_retention_days BETWEEN 180 AND 3650),
    -- 上游 HTTP 超时（秒）：proxy 的 headers/stream-idle 超时
    upstream_timeout_secs INTEGER NOT NULL DEFAULT 60
        CHECK (upstream_timeout_secs BETWEEN 5 AND 600),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

INSERT INTO app_settings (id) VALUES (1);

COMMENT ON TABLE app_settings IS
    '平台设置单例（id=1）：运行期权威；env 仅 DB 不可用兜底；PATCH 后 runtime 生效';

-- 回滚（人工）：
--   DROP TABLE app_settings;
