//! policy 领域模型与请求/响应 DTO（对齐 `接口设计.md`、`接口契约.yaml` v0.1.0）。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const MAX_PAGE_LIMIT: i64 = 200;
pub const DEFAULT_PAGE_LIMIT: i64 = 50;
pub const MAX_BODY_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Serialize)]
pub struct Budget {
    pub id: Uuid,
    pub user_id: Uuid,
    /// 6 位小数字符串；0=不限额。
    pub daily_limit: String,
    pub currency: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub total: i64,
    pub limit: i64,
    pub offset: i64,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PageQuery {
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub offset: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpsertBudgetRequest {
    pub daily_limit: String,
    #[serde(default)]
    pub currency: Option<String>,
}
