## 变更说明

<!-- 这个 PR 做了什么？为什么需要？ -->

## 改动范围

<!-- 涉及的模块 / 文件 / API；是否影响数据库迁移、契约或配置 -->

## 验证方式

<!-- 跑了哪些命令 / 测试；关键路径的手工验证步骤 -->

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --all-targets -- -D warnings`
- [ ] `cargo test --all-targets`
- [ ] 涉及数据库时跑了 `--ignored` 集成测试
- [ ] `pnpm -r typecheck`
- [ ] `pnpm -r build`

## 回滚方式

<!-- 出问题时如何回滚：是否可 revert、迁移是否需要回退 -->

## 关联 issue

<!-- Closes #123 -->
