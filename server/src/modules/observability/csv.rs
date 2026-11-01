//! CSV 导出格式化（RFC 4180 转义 + 电子表格公式注入防护）。
//!
//! 设计见 `设计文档（暂未开源）` §4：
//! - 列与 `/api/admin/requests`、`/api/admin/audit` 列表一致；
//! - `text/csv; charset=utf-8` + `Content-Disposition: attachment`；
//! - 逗号/引号/换行按 RFC 4180 加引号；以 `= + - @ \t \r` 开头的字段前置 `'`
//!   （OWASP CSV injection 防护，避免 Excel/Sheets 执行公式）；
//! - 只导出元数据，无正文/Authorization/Key/token/Cookie。

use chrono::{DateTime, SecondsFormat, Utc};
use uuid::Uuid;

use super::repo::{AuditLogRow, RequestLogRow};

/// 请求日志导出列（与 `/api/admin/requests` 列表行一致，27 列）。
const REQUEST_LOG_COLUMNS: [&str; 27] = [
    "id",
    "request_id",
    "trace_id",
    "created_at",
    "method",
    "path",
    "protocol",
    "upstream_protocol",
    "adapter",
    "user_id",
    "device_id",
    "key_id",
    "key_prefix",
    "model_alias",
    "upstream_model",
    "provider",
    "provider_id",
    "is_stream",
    "fallback",
    "attempt_count",
    "status_code",
    "latency_ms",
    "ttft_ms",
    "response_size",
    "error_code",
    "client_ip",
    "user_agent",
];

/// 审计导出列（与 `/api/admin/audit` 列表行一致，13 列）。
const AUDIT_COLUMNS: [&str; 13] = [
    "id",
    "created_at",
    "actor_type",
    "actor_id",
    "actor_email",
    "action",
    "resource_type",
    "resource_id",
    "outcome",
    "client_ip",
    "user_agent",
    "request_id",
    "detail",
];

/// 请求日志 CSV 文件名：`requests-<from>-<to>.csv`（时间压缩为安全 ASCII）。
pub fn requests_filename(from: DateTime<Utc>, to: DateTime<Utc>) -> String {
    format!(
        "requests-{}-{}.csv",
        compact_timestamp(from),
        compact_timestamp(to)
    )
}

/// 审计 CSV 文件名：`audit-<from>-<to>.csv`。
pub fn audit_filename(from: DateTime<Utc>, to: DateTime<Utc>) -> String {
    format!(
        "audit-{}-{}.csv",
        compact_timestamp(from),
        compact_timestamp(to)
    )
}

/// 渲染请求日志 CSV（含表头；CRLF 行尾；末尾带终止换行）。
pub fn render_request_logs(rows: &[RequestLogRow]) -> String {
    let mut lines = Vec::with_capacity(rows.len() + 1);
    lines.push(join_fields(REQUEST_LOG_COLUMNS.iter().copied()));
    for row in rows {
        lines.push(join_fields([
            row.id.to_string(),
            row.request_id.clone(),
            opt_str(row.trace_id.as_deref()),
            format_timestamp(row.created_at),
            row.method.clone(),
            row.path.clone(),
            row.protocol.clone(),
            opt_str(row.upstream_protocol.as_deref()),
            opt_str(row.adapter.as_deref()),
            opt_uuid(row.user_id),
            opt_uuid(row.device_id),
            opt_uuid(row.key_id),
            opt_str(row.key_prefix.as_deref()),
            opt_str(row.model_alias.as_deref()),
            opt_str(row.upstream_model.as_deref()),
            opt_str(row.provider.as_deref()),
            opt_uuid(row.provider_id),
            bool_text(row.is_stream),
            bool_text(row.fallback),
            row.attempt_count.to_string(),
            opt_i32(row.status_code),
            row.latency_ms.to_string(),
            opt_i32(row.ttft_ms),
            opt_i64(row.response_size),
            opt_str(row.error_code.as_deref()),
            opt_str(row.client_ip.as_deref()),
            row.user_agent.clone(),
        ]));
    }
    lines.join("\r\n") + "\r\n"
}

/// 渲染审计 CSV（含表头；`detail` 为脱敏后的紧凑 JSON 字符串）。
pub fn render_audit_logs(rows: &[AuditLogRow]) -> String {
    let mut lines = Vec::with_capacity(rows.len() + 1);
    lines.push(join_fields(AUDIT_COLUMNS.iter().copied()));
    for row in rows {
        lines.push(join_fields([
            row.id.to_string(),
            format_timestamp(row.created_at),
            row.actor_type.clone(),
            opt_uuid(row.actor_id),
            opt_str(row.actor_email.as_deref()),
            row.action.clone(),
            row.resource_type.clone(),
            opt_str(row.resource_id.as_deref()),
            row.outcome.clone(),
            opt_str(row.client_ip.as_deref()),
            row.user_agent.clone(),
            opt_str(row.request_id.as_deref()),
            row.detail.to_string(),
        ]));
    }
    lines.join("\r\n") + "\r\n"
}

/// 单字段 CSV 转义：先做公式注入防护（前置 `'`），再按 RFC 4180 加引号。
pub fn escape_field(raw: &str) -> String {
    let protected = if starts_with_formula_trigger(raw) {
        format!("'{raw}")
    } else {
        raw.to_string()
    };
    if protected.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", protected.replace('"', "\"\""))
    } else {
        protected
    }
}

fn join_fields(fields: impl IntoIterator<Item = impl AsRef<str>>) -> String {
    fields
        .into_iter()
        .map(|field| escape_field(field.as_ref()))
        .collect::<Vec<_>>()
        .join(",")
}

fn starts_with_formula_trigger(raw: &str) -> bool {
    matches!(
        raw.chars().next(),
        Some('=' | '+' | '-' | '@' | '\t' | '\r')
    )
}

fn format_timestamp(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn compact_timestamp(value: DateTime<Utc>) -> String {
    value.format("%Y%m%dT%H%M%SZ").to_string()
}

fn bool_text(value: bool) -> String {
    if value {
        "true".to_string()
    } else {
        "false".to_string()
    }
}

fn opt_str(value: Option<&str>) -> String {
    value.unwrap_or_default().to_string()
}

fn opt_uuid(value: Option<Uuid>) -> String {
    value.map(|id| id.to_string()).unwrap_or_default()
}

fn opt_i32(value: Option<i32>) -> String {
    value.map(|value| value.to_string()).unwrap_or_default()
}

fn opt_i64(value: Option<i64>) -> String {
    value.map(|value| value.to_string()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_request_log() -> RequestLogRow {
        RequestLogRow {
            id: Uuid::new_v4(),
            request_id: "req-csv-1".to_string(),
            trace_id: Some("req-csv-1".to_string()),
            created_at: Utc::now(),
            method: "POST".to_string(),
            path: "/v1/chat/completions".to_string(),
            protocol: "chat_completions".to_string(),
            upstream_protocol: Some("openai_chat".to_string()),
            adapter: Some("chat_passthrough".to_string()),
            user_id: None,
            device_id: None,
            key_id: None,
            key_prefix: Some("sk-gw-abcd".to_string()),
            model_alias: Some("fast,smart".to_string()),
            upstream_model: Some("deepseek-chat".to_string()),
            provider: Some("Test,Provider".to_string()),
            provider_id: None,
            is_stream: true,
            fallback: false,
            attempt_count: 1,
            status_code: Some(200),
            latency_ms: 12,
            ttft_ms: Some(3),
            response_size: Some(100),
            error_code: None,
            client_ip: Some("10.0.3.7".to_string()),
            user_agent: "=HYPERLINK(\"http://evil\")".to_string(),
        }
    }

    fn sample_audit_log() -> AuditLogRow {
        AuditLogRow {
            id: Uuid::new_v4(),
            created_at: Utc::now(),
            actor_type: "admin".to_string(),
            actor_id: None,
            actor_email: Some("admin@corp.example.com".to_string()),
            action: "observability.request.exported".to_string(),
            resource_type: "request_log".to_string(),
            resource_id: None,
            outcome: "success".to_string(),
            client_ip: Some("10.0.3.7".to_string()),
            user_agent: "curl/8.0".to_string(),
            request_id: Some("req-csv-1".to_string()),
            detail: json!({"rows": 1, "filters": {"from": "2026-10-01T00:00:00Z"}}),
        }
    }

    #[test]
    fn escape_field_quotes_and_protects_formulas() {
        assert_eq!(escape_field("plain"), "plain");
        assert_eq!(escape_field("a,b"), "\"a,b\"");
        assert_eq!(escape_field("he said \"hi\""), "\"he said \"\"hi\"\"\"");
        assert_eq!(escape_field("line1\nline2"), "\"line1\nline2\"");
        // 公式注入防护：前置单引号；含逗号时再加引号。
        assert_eq!(escape_field("=SUM(A1)"), "'=SUM(A1)");
        assert_eq!(escape_field("+1"), "'+1");
        assert_eq!(escape_field("-2"), "'-2");
        assert_eq!(escape_field("@cmd"), "'@cmd");
        assert_eq!(escape_field("=a,b"), "\"'=a,b\"");
        // 已有前缀单引号不重复处理（首字符为 '）。
        assert_eq!(escape_field("'=SUM(A1)"), "'=SUM(A1)");
    }

    #[test]
    fn request_csv_header_escaping_and_formula_protection() {
        let request = sample_request_log();
        let csv = render_request_logs(std::slice::from_ref(&request));
        let mut lines = csv.split("\r\n");
        let header = lines.next().expect("header");
        assert_eq!(header.split(',').count(), REQUEST_LOG_COLUMNS.len());
        assert!(header.starts_with("id,request_id,trace_id,created_at,method,path,protocol,"));
        assert!(header.contains("user_agent") && !header.contains("authorization"));
        assert!(csv.ends_with("\r\n"));
        // 逗号字段被引号包裹；公式注入字段前置 `'`。
        assert!(csv.contains("\"fast,smart\""));
        assert!(csv.contains("\"Test,Provider\""));
        assert!(csv.contains("'=HYPERLINK"));
        // 时间格式为 RFC3339 UTC（毫秒）。
        assert!(csv.contains(&format_timestamp(request.created_at)));
    }

    #[test]
    fn audit_csv_has_expected_columns_and_detail_json() {
        let audit = sample_audit_log();
        let csv = render_audit_logs(std::slice::from_ref(&audit));
        let header = csv.lines().next().expect("header");
        assert_eq!(
            header,
            "id,created_at,actor_type,actor_id,actor_email,action,resource_type,resource_id,\
             outcome,client_ip,user_agent,request_id,detail"
        );
        // detail 为 JSON 字符串（含逗号 → 引号包裹，内部双引号转义）。
        assert!(csv.contains("\"{\"\"rows\"\":1,\"\"filters\"\""));
        assert!(csv.contains("observability.request.exported"));
    }

    #[test]
    fn filenames_are_safe_ascii() {
        let from = "2026-10-24T00:00:00Z"
            .parse::<DateTime<Utc>>()
            .expect("from");
        let to = "2026-10-31T00:00:00Z".parse::<DateTime<Utc>>().expect("to");
        let request_name = requests_filename(from, to);
        assert_eq!(
            request_name,
            "requests-20261024T000000Z-20261031T000000Z.csv"
        );
        assert!(
            request_name
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '.'))
        );
        assert_eq!(
            audit_filename(from, to),
            "audit-20261024T000000Z-20261031T000000Z.csv"
        );
    }
}
