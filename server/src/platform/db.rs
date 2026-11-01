//! PostgreSQL 连接与迁移。
//!
//! - 连接池使用 `connect_lazy`：没有数据库时进程也能启动，`/readyz` 会报告未就绪。
//! - 迁移使用运行时 `sqlx::migrate!`，SQL 在 `server/migrations/`。

use anyhow::{Context, Result};
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::time::Duration;

pub async fn connect(database_url: Option<&str>) -> Result<Option<PgPool>> {
    let Some(url) = database_url else {
        tracing::warn!(
            "GW_DATABASE_URL / DATABASE_URL 未配置：数据库相关功能不可用，/readyz 将返回 503"
        );
        return Ok(None);
    };

    let pool = PgPoolOptions::new()
        .max_connections(10)
        .acquire_timeout(Duration::from_secs(5))
        .connect_lazy(url)
        .with_context(|| "解析 PostgreSQL 连接串失败")?;

    Ok(Some(pool))
}

pub async fn migrate(pool: &PgPool) -> Result<()> {
    sqlx::migrate!("./migrations")
        .run(pool)
        .await
        .context("执行数据库迁移失败")?;
    Ok(())
}
