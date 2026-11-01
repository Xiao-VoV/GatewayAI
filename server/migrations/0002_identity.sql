-- 0002_identity.sql
-- identity 模块设计：管理员登录/会话/CSRF 所需字段。
-- 说明：0001 已建 admin_users/admin_sessions；本迁移补齐 0002 所需字段。
-- 假设当前尚无已登录会话数据（identity handler 还没实现）；
-- 若目标环境已有 admin_sessions 行，需先回填 token_hash 再执行 SET NOT NULL。

ALTER TABLE admin_users
    ADD COLUMN last_login_at      TIMESTAMPTZ,
    ADD COLUMN password_changed_at TIMESTAMPTZ;

ALTER TABLE admin_users
    ADD CONSTRAINT admin_users_status_check
        CHECK (status IN ('active', 'disabled'));

ALTER TABLE admin_sessions
    ADD COLUMN token_hash   TEXT,
    ADD COLUMN user_agent   TEXT NOT NULL DEFAULT '',
    ADD COLUMN ip_address   TEXT NOT NULL DEFAULT '',
    ADD COLUMN last_seen_at TIMESTAMPTZ NOT NULL DEFAULT now();

ALTER TABLE admin_sessions
    ALTER COLUMN token_hash SET NOT NULL;

CREATE UNIQUE INDEX idx_admin_sessions_token_hash ON admin_sessions(token_hash);
