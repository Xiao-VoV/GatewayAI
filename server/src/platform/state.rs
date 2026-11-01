//! 进程内共享状态。业务模块通过 `State<AppState>` 取得数据库与配置。

use crate::platform::{config::Config, crypto::Crypto, http};
use sqlx::PgPool;
use std::{sync::Arc, time::Instant};

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub db: Option<PgPool>,
    /// 主密钥封装器；providers 用它加解密上游凭证。
    pub crypto: Arc<Crypto>,
    /// 共享上游 HTTP 客户端（连接池/超时/重定向策略）；proxy 使用，providers 测试可复用。
    pub http: Arc<reqwest::Client>,
    pub started_at: Instant,
}

impl AppState {
    pub fn new(config: Arc<Config>, db: Option<PgPool>, crypto: Arc<Crypto>) -> Self {
        Self {
            config,
            db,
            crypto,
            http: http::shared_client(),
            started_at: Instant::now(),
        }
    }
}
