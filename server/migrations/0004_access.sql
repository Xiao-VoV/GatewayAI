-- 0004_access.sql
-- access 模块设计：员工用户/设备/配对码/设备 token/虚拟 Key 所需字段、约束与索引。
-- 说明：0001 已建 users/devices/pairing_codes/device_tokens/virtual_keys。
-- 本迁移假定 access 表尚未产生数据（S3 前未使用，devices 行数应为 0）；上线前检查：
--   SELECT count(*) FROM devices;                         -- 应为 0
--   SELECT count(*) FROM devices WHERE status NOT IN ('active','disabled');
--   SELECT count(*) FROM pairing_codes;                   -- 可为 0
--   SELECT count(*) FROM virtual_keys;                    -- 可为 0
-- 若 devices 非空，需先补 device_secret_hash 再执行 SET NOT NULL。

-- 前置检查：0004 假定 access 表为空（S3 前未使用）。
-- 若 devices 非空，说明已有设备行缺少 device_secret_hash，必须先回填再执行本迁移。
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM devices) THEN
        RAISE EXCEPTION '0004_access requires an empty devices table; backfill device_secret_hash before applying';
    END IF;
END $$;

-- users：状态约束（员工身份归 access）
ALTER TABLE users ADD CONSTRAINT users_status_check
    CHECK (status IN ('active', 'disabled'));

-- devices：设备密钥哈希 / 客户端版本 / 最近 IP / 状态
ALTER TABLE devices ADD COLUMN device_secret_hash TEXT;
ALTER TABLE devices ADD COLUMN client_version TEXT NOT NULL DEFAULT '';
ALTER TABLE devices ADD COLUMN last_ip TEXT NOT NULL DEFAULT '';

ALTER TABLE devices ALTER COLUMN status SET DEFAULT 'active';
ALTER TABLE devices ADD CONSTRAINT devices_status_check
    CHECK (status IN ('active', 'disabled'));
ALTER TABLE devices ALTER COLUMN device_secret_hash SET NOT NULL;

CREATE UNIQUE INDEX devices_device_secret_hash_key ON devices (device_secret_hash);
CREATE INDEX idx_devices_status ON devices (status);

-- pairing_codes：生成人 / 主动失效时间
ALTER TABLE pairing_codes ADD COLUMN created_by UUID REFERENCES admin_users(id) ON DELETE SET NULL;
ALTER TABLE pairing_codes ADD COLUMN invalidated_at TIMESTAMPTZ;
CREATE INDEX idx_pairing_codes_user ON pairing_codes (user_id, expires_at);

-- device_tokens：最近使用时间
ALTER TABLE device_tokens ADD COLUMN last_used_at TIMESTAMPTZ;

-- virtual_keys：模型白名单 / 最近使用 / 吊销时间 / 状态约束
ALTER TABLE virtual_keys ADD COLUMN allowed_models TEXT[] NOT NULL DEFAULT '{}';
ALTER TABLE virtual_keys ADD COLUMN last_used_at TIMESTAMPTZ;
ALTER TABLE virtual_keys ADD COLUMN revoked_at TIMESTAMPTZ;
ALTER TABLE virtual_keys ADD CONSTRAINT virtual_keys_status_check
    CHECK (status IN ('active', 'paused', 'revoked'));
CREATE INDEX idx_virtual_keys_status ON virtual_keys (status);
