-- 0003_providers.sql
-- providers 模块设计：供应商/渠道账号/上游模型所需字段、约束与索引。
-- 说明：0001 已建 providers/provider_accounts/upstream_models；本迁移补齐 S2 所需约束。
-- 上线前预检（任一非 0 行需先清洗，否则本迁移会失败）：
--   SELECT name FROM providers GROUP BY name HAVING count(*) > 1;
--   SELECT provider_id, name FROM provider_accounts GROUP BY 1,2 HAVING count(*) > 1;
--   SELECT provider_id, name FROM upstream_models GROUP BY 1,2 HAVING count(*) > 1;
--   SELECT count(*) FROM providers
--     WHERE status NOT IN ('active','disabled') OR kind NOT IN ('openai_compatible','anthropic');
--   SELECT count(*) FROM provider_accounts
--     WHERE status NOT IN ('active','disabled') OR health NOT IN ('unknown','healthy','unhealthy')
--        OR priority <= 0 OR rpm_limit <= 0 OR tpm_limit <= 0;
--   SELECT count(*) FROM upstream_models
--     WHERE (context_window IS NOT NULL AND context_window <= 0)
--        OR (max_output IS NOT NULL AND max_output <= 0)
--        OR price_input < 0 OR price_output < 0
--        OR jsonb_typeof(capabilities) <> 'array'
--        OR currency NOT IN ('CNY','USD');

-- providers
ALTER TABLE providers ADD COLUMN description TEXT NOT NULL DEFAULT '';

ALTER TABLE providers ADD CONSTRAINT providers_status_check
    CHECK (status IN ('active', 'disabled'));
ALTER TABLE providers ADD CONSTRAINT providers_kind_check
    CHECK (kind IN ('openai_compatible', 'anthropic'));

CREATE UNIQUE INDEX providers_name_key ON providers (name);
CREATE INDEX idx_providers_status ON providers (status);

-- provider_accounts
ALTER TABLE provider_accounts ADD COLUMN credential_hint TEXT NOT NULL DEFAULT '';
ALTER TABLE provider_accounts ADD COLUMN last_checked_at TIMESTAMPTZ;
ALTER TABLE provider_accounts ADD COLUMN last_error TEXT;

ALTER TABLE provider_accounts ADD CONSTRAINT provider_accounts_status_check
    CHECK (status IN ('active', 'disabled'));
ALTER TABLE provider_accounts ADD CONSTRAINT provider_accounts_health_check
    CHECK (health IN ('unknown', 'healthy', 'unhealthy'));
ALTER TABLE provider_accounts ADD CONSTRAINT provider_accounts_priority_check
    CHECK (priority > 0);
ALTER TABLE provider_accounts ADD CONSTRAINT provider_accounts_rpm_check
    CHECK (rpm_limit IS NULL OR rpm_limit > 0);
ALTER TABLE provider_accounts ADD CONSTRAINT provider_accounts_tpm_check
    CHECK (tpm_limit IS NULL OR tpm_limit > 0);

CREATE UNIQUE INDEX provider_accounts_provider_name_key
    ON provider_accounts (provider_id, name);

-- upstream_models
ALTER TABLE upstream_models ADD CONSTRAINT upstream_models_positive_check
    CHECK (
        (context_window IS NULL OR context_window > 0)
        AND (max_output IS NULL OR max_output > 0)
        AND (price_input IS NULL OR price_input >= 0)
        AND (price_output IS NULL OR price_output >= 0)
        AND jsonb_typeof(capabilities) = 'array'
    );
ALTER TABLE upstream_models ADD CONSTRAINT upstream_models_currency_check
    CHECK (currency IN ('CNY', 'USD'));

CREATE UNIQUE INDEX upstream_models_provider_name_key
    ON upstream_models (provider_id, name);
