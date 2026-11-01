//! 协议适配器：Chat 透传、Messages↔Chat、Responses↔Chat 转换（纯同步，无 IO）。
//!
//! 设计：`设计文档（暂未开源）` §6。适配器负责请求/响应/SSE 事件的
//! 形状转换与 usage 提取；连接、超时、重试、背压由 `upstream`/`stream` 模块负责。

pub mod chat;
pub mod gemini;
pub mod messages;
pub mod responses;

use serde_json::Value;

use crate::platform::error::AppError;

pub const MAX_MODEL_LEN: usize = 128;
pub const MAX_TOOL_NAME_LEN: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrontendProtocol {
    ChatCompletions,
    Messages,
    Responses,
    Gemini,
}

/// 通用 token 用量（上游缺失时由估算填充）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UsageTokens {
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
}

impl UsageTokens {
    pub fn input(&self) -> i64 {
        self.input_tokens.unwrap_or(0).max(0)
    }

    pub fn output(&self) -> i64 {
        self.output_tokens.unwrap_or(0).max(0)
    }
}

/// 粗估 token（ceil(chars/3)）；S4 与 metering 的估算公式保持一致。
/// 集成后由 `metering::cost::estimate_tokens` 统一提供（见 proxy 技术设计 §4.4/§7.5）。
pub fn estimate_tokens(chars: usize) -> i64 {
    if chars == 0 {
        return 0;
    }
    let tokens = (chars as f64 / 3.0).ceil() as i64;
    tokens.max(1)
}

/// 统计 JSON 中所有字符串字符数（用于 prompt 估算；不含 key 名）。
pub fn count_text_chars(value: &Value) -> usize {
    match value {
        Value::String(text) => text.chars().count(),
        Value::Array(items) => items.iter().map(count_text_chars).sum(),
        Value::Object(map) => map.values().map(count_text_chars).sum(),
        _ => 0,
    }
}

/// 构造 400 `validation_failed`（field 可为动态路径，如 `messages[2].content[0].type`）。
pub fn validation_field(field: impl Into<String>, message: impl Into<String>) -> AppError {
    let mut errors = std::collections::BTreeMap::new();
    errors.insert(field.into(), message.into());
    AppError::Validation { errors }
}

pub fn validate_model(raw: &str) -> Result<String, AppError> {
    let model = raw.trim();
    if model.is_empty() {
        return Err(AppError::validation_field("model", "model 不能为空"));
    }
    if model.chars().count() > MAX_MODEL_LEN {
        return Err(AppError::validation_field(
            "model",
            format!("model 不能超过 {MAX_MODEL_LEN} 个字符"),
        ));
    }
    if model
        .chars()
        .any(|ch| ch.is_control() || ch.is_whitespace())
    {
        return Err(AppError::validation_field(
            "model",
            "model 不能包含空白或控制字符",
        ));
    }
    Ok(model.to_string())
}

pub fn parse_json_body(raw: &[u8]) -> Result<Value, AppError> {
    serde_json::from_slice(raw)
        .map_err(|_| AppError::validation_field("body", "请求体不是合法的 JSON"))
}

pub fn require_object<'a>(
    value: &'a Value,
    field: &str,
) -> Result<&'a serde_json::Map<String, Value>, AppError> {
    value
        .as_object()
        .ok_or_else(|| AppError::validation_field(field, "必须是 JSON object"))
}

pub fn parse_usage(usage: Option<&Value>) -> UsageTokens {
    let Some(usage) = usage else {
        return UsageTokens::default();
    };
    UsageTokens {
        input_tokens: usage
            .get("prompt_tokens")
            .and_then(Value::as_i64)
            .or_else(|| usage.get("input_tokens").and_then(Value::as_i64)),
        output_tokens: usage
            .get("completion_tokens")
            .and_then(Value::as_i64)
            .or_else(|| usage.get("output_tokens").and_then(Value::as_i64)),
    }
}

/// 顶层 `error` 对象提取（上游 SSE/JSON 可能以 2xx + error 返回）。
pub fn top_level_error(value: &Value) -> Option<&Value> {
    value.get("error")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_estimation_is_monotonic_and_zero_safe() {
        assert_eq!(estimate_tokens(0), 0);
        assert_eq!(estimate_tokens(1), 1);
        assert_eq!(estimate_tokens(3), 1);
        assert_eq!(estimate_tokens(4), 2);
        assert!(estimate_tokens(100) > estimate_tokens(99));
    }

    #[test]
    fn model_validation_rejects_bad_input() {
        assert_eq!(validate_model(" fast ").expect("valid"), "fast");
        assert!(validate_model("").is_err());
        assert!(validate_model("a b").is_err());
        assert!(validate_model(&"x".repeat(MAX_MODEL_LEN + 1)).is_err());
    }

    #[test]
    fn usage_parsing_supports_openai_and_anthropic_names() {
        let openai = parse_usage(Some(
            &serde_json::json!({"prompt_tokens": 3, "completion_tokens": 4}),
        ));
        assert_eq!(openai.input(), 3);
        assert_eq!(openai.output(), 4);
        let anthropic = parse_usage(Some(
            &serde_json::json!({"input_tokens": 5, "output_tokens": 6}),
        ));
        assert_eq!(anthropic.input(), 5);
        assert_eq!(anthropic.output(), 6);
    }

    #[test]
    fn counts_text_chars_only_from_strings() {
        let value = serde_json::json!({"messages": [{"role": "user", "content": "abc"}], "n": 12});
        assert_eq!(count_text_chars(&value), 7); // "user" 4 + "abc" 3
    }
}
