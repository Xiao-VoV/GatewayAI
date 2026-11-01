-- 0013_user_groups.sql
-- access 用户组与成员（v0.1 / US-041 发布对象依赖）所需表、约束与索引。
-- 说明：additive 新表（无存量数据需回填）；users 表已存在（0001）。
--       名称大小写不敏感唯一；成员 ≤500 由服务端校验（DB 不约束行数）。
--       组删除级联删除成员；组历史发布对象（catalog 多态无 FK）由 catalog 解析 fail-closed。

CREATE TABLE user_groups (
    id          UUID PRIMARY KEY,
    name        TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT user_groups_name_check CHECK (char_length(name) BETWEEN 1 AND 64),
    CONSTRAINT user_groups_description_check CHECK (char_length(description) <= 200)
);
CREATE UNIQUE INDEX user_groups_name_key ON user_groups (lower(name));
CREATE INDEX idx_user_groups_updated ON user_groups (updated_at DESC);

CREATE TABLE user_group_members (
    group_id   UUID NOT NULL REFERENCES user_groups(id) ON DELETE CASCADE,
    user_id    UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (group_id, user_id)
);
CREATE INDEX idx_user_group_members_user ON user_group_members (user_id);
