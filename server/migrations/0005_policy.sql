-- 0005_policy.sql
-- policy 预算模块：budgets 约束与查询索引（S3）。
-- 0001 已建 budgets（id/user_id/daily_limit/currency/created_at/updated_at）；S3 不新增表/列。
-- 前置检查：已有 budgets 行必须满足 S3 不变量（daily_limit>=0、currency='CNY'）。
-- 若不满足，先清洗数据再执行本迁移。

DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM budgets WHERE daily_limit < 0 OR currency <> 'CNY') THEN
        RAISE EXCEPTION '0005_policy requires budgets rows with daily_limit >= 0 and currency = CNY';
    END IF;
END $$;

ALTER TABLE budgets ADD CONSTRAINT budgets_daily_limit_check
    CHECK (daily_limit >= 0);
-- S3 只支持 CNY；v0.1.x 多币种需后续迁移替换本约束。
ALTER TABLE budgets ADD CONSTRAINT budgets_currency_check
    CHECK (currency = 'CNY');

CREATE INDEX idx_budgets_updated ON budgets (updated_at DESC);
