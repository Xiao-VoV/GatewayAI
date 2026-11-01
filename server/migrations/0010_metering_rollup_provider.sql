-- 0010_metering_rollup_provider.sql
-- metering S5.1：daily_usage_rollups 增加供应商（provider）维度。
--
-- 背景：0007 重建的 rollup 按 (day, user_id, key_id, model_alias) 聚合，`/usage` 支持
-- provider_id 过滤；明细被保留期清理后仍要能按供应商筛选，因此 rollup 需要 provider 维度。
--
-- 约定：
--   - provider_id 为 00000000-0000-0000-0000-000000000000（zero UUID 哨兵）表示“无供应商”
--     （rejected / 未解析 / provider 已删除快照为空）；
--   - 非 FK：rollup 是历史台账快照，供应商物理删除不应丢失/级联历史聚合；
--   - 设计依据：设计文档（暂未开源） §1、数据库设计 §2.3/§3.2。
--   - 不改 0007；表在 S5.1 前无写入方，重建 PK 安全（若已有数据，默认 provider=zero 后重建）。

ALTER TABLE daily_usage_rollups DROP CONSTRAINT daily_usage_rollups_pkey;

ALTER TABLE daily_usage_rollups
    ADD COLUMN provider_id UUID NOT NULL DEFAULT '00000000-0000-0000-0000-000000000000';

ALTER TABLE daily_usage_rollups
    ADD PRIMARY KEY (day, user_id, key_id, model_alias, provider_id);

-- 跨天按供应商筛选（`/usage?provider_id=`）。
CREATE INDEX idx_daily_usage_rollups_provider_day
    ON daily_usage_rollups (provider_id, day);

COMMENT ON COLUMN daily_usage_rollups.provider_id IS
    '供应商维度快照；00000000-0000-0000-0000-000000000000 = 无供应商（rejected/未解析）；非 FK';

-- 回滚（人工，仅无生产 rollup 数据时）：
--   ALTER TABLE daily_usage_rollups DROP CONSTRAINT daily_usage_rollups_pkey;
--   DROP INDEX idx_daily_usage_rollups_provider_day;
--   ALTER TABLE daily_usage_rollups DROP COLUMN provider_id;
--   ALTER TABLE daily_usage_rollups ADD PRIMARY KEY (day, user_id, key_id, model_alias);
