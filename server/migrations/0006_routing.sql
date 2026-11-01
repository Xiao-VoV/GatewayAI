-- 0006_routing.sql
-- routing 模块：模型别名 → 多上游目标（S4 完整多目标/权重/优先级/降级）。
-- 0001 已建 model_aliases（单 upstream_model_id）；本迁移创建 targets、回填、删除旧列。
-- 注意：删除旧列会破坏仍查询 model_aliases.upstream_model_id 的代码（providers 引用检查/测试）；
--       实现 routing 时必须同步更新 providers，并在同一发布中迁移。

CREATE TABLE model_alias_targets (
    id                UUID PRIMARY KEY,
    alias_id          UUID NOT NULL REFERENCES model_aliases(id) ON DELETE CASCADE,
    upstream_model_id UUID NOT NULL REFERENCES upstream_models(id) ON DELETE RESTRICT,
    priority          INTEGER NOT NULL DEFAULT 1 CHECK (priority BETWEEN 1 AND 1000),
    weight            INTEGER NOT NULL DEFAULT 100 CHECK (weight BETWEEN 1 AND 100),
    enabled           BOOLEAN NOT NULL DEFAULT true,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (alias_id, upstream_model_id)
);

-- 回填旧单目标：每个 alias 一行 target，priority=1/weight=100/enabled=true（别名启用状态保留在 model_aliases.enabled）。
INSERT INTO model_alias_targets (
    id, alias_id, upstream_model_id, priority, weight, enabled, created_at, updated_at
)
SELECT gen_random_uuid(), id, upstream_model_id, 1, 100, true, created_at, updated_at
FROM model_aliases;

ALTER TABLE model_aliases DROP COLUMN upstream_model_id;

CREATE INDEX idx_model_alias_targets_alias
    ON model_alias_targets (alias_id, priority, id);
CREATE INDEX idx_model_alias_targets_upstream
    ON model_alias_targets (upstream_model_id);
