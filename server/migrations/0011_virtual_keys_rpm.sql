-- 0011_virtual_keys_rpm.sql
-- Key RPM 限流（US-011 / S3.1）：虚拟 Key 级每分钟请求上限。
-- NULL = 不限；正整数 = 每分钟请求数上限。
-- 说明：不新增 policy rate_limits 表（MVP 仅 Key 级进程内滑动窗口；TPM/多层级/分布式后置）。

ALTER TABLE virtual_keys
    ADD COLUMN rpm_limit INTEGER;

ALTER TABLE virtual_keys
    ADD CONSTRAINT virtual_keys_rpm_limit_check
        CHECK (rpm_limit IS NULL OR (rpm_limit > 0 AND rpm_limit <= 1000000));
