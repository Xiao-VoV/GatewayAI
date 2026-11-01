//! S4 proxy 运行参数（超时/上限/汇率等）。
//!
//! 设计：`设计文档（暂未开源）` §2.3。默认值集中在此，
//! 测试可构造更小的超时/上限；平台 Config 提供 body 上限/汇率/信任代理/供应商头开关。

use std::time::Duration;

use rust_decimal::Decimal;

use crate::modules::settings::runtime;
use crate::platform::config::Config;

/// 非流式上游响应体上限（16 MiB）。
pub const MAX_UPSTREAM_JSON_BYTES: usize = 16 * 1024 * 1024;
/// 上游错误体读取上限（64 KiB）。
pub const MAX_UPSTREAM_ERROR_BYTES: usize = 64 * 1024;
/// SSE 单事件行缓冲上限（1 MiB）：超过后停止解析（仍继续转发）。
pub const SSE_LINE_BUFFER_MAX: usize = 1024 * 1024;
/// 最多上游 HTTP 尝试次数（首次 + 1 次重试）。
pub const MAX_ATTEMPTS: u8 = 2;
/// 未显式给出 max_tokens 时的预算估算输出上限。
pub const DEFAULT_OUTPUT_TOKENS_ESTIMATE: i64 = 1024;
/// 输出 token 估算硬上限（防止恶意 max_tokens 打爆预算估算）。
pub const MAX_OUTPUT_TOKENS_ESTIMATE: i64 = 1_000_000;

#[derive(Debug, Clone)]
pub struct ProxySettings {
    pub max_body_bytes: usize,
    pub max_upstream_json_bytes: usize,
    pub headers_timeout: Duration,
    pub non_stream_total_timeout: Duration,
    pub stream_idle_timeout: Duration,
    pub stream_max_duration: Duration,
    pub max_attempts: u8,
    pub expose_provider_header: bool,
    pub usd_cny_rate: Decimal,
    pub trust_proxy_headers: bool,
}

impl ProxySettings {
    pub fn from_config(config: &Config) -> Self {
        // 运行期权威（app_settings.upstream_timeout_secs）：headers 与 stream-idle 共用；
        // PATCH 后下一请求生效；总时长/最大时长维持既有常量。
        let upstream_timeout = Duration::from_secs(
            u64::try_from(runtime::current().upstream_timeout_secs).unwrap_or(60),
        );
        Self {
            max_body_bytes: config.proxy_max_body_bytes,
            max_upstream_json_bytes: MAX_UPSTREAM_JSON_BYTES,
            headers_timeout: upstream_timeout,
            non_stream_total_timeout: Duration::from_secs(300),
            stream_idle_timeout: upstream_timeout,
            stream_max_duration: Duration::from_secs(900),
            max_attempts: MAX_ATTEMPTS,
            expose_provider_header: config.proxy_expose_provider_header,
            usd_cny_rate: config.usd_cny_rate,
            trust_proxy_headers: config.trust_proxy_headers,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_reasonable() {
        assert_eq!(MAX_ATTEMPTS, 2);
        assert_eq!(SSE_LINE_BUFFER_MAX, 1024 * 1024);
    }
}
