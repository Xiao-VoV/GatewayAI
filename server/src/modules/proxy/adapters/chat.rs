//! OpenAI Chat Completions 透传适配器（`chat_passthrough`）。
//!
//! - 请求：只校验 `model`/`messages`/`stream` 基本结构；`model` 替换为上游模型名，
//!   其余字段原样透传（`serde_json` 开启 `preserve_order` 以降低 prompt cache 重排风险）。
//! - 非流式响应：回写请求的对外模型别名，提取 usage。
//! - 流式响应：字节直通；`inspect_stream_data` 旁路提取 usage/finish/error。

use bytes::Bytes;
use serde_json::{Map, Value};

use crate::platform::error::AppError;

use super::{UsageTokens, parse_usage, require_object, top_level_error, validate_model};

pub const ADAPTER_ID: &str = "chat_passthrough";

#[derive(Debug, Clone)]
pub struct ChatRequest {
    pub alias: String,
    pub is_stream: bool,
    pub prompt_chars: usize,
    pub max_tokens: Option<i64>,
    pub value: Value,
}

/// 解析并校验 Chat 请求（不依赖 routing 结果）。
pub fn parse_request(raw: &[u8]) -> Result<ChatRequest, AppError> {
    let value = super::parse_json_body(raw)?;
    let object = require_object(&value, "body")?;
    let model = object
        .get("model")
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::validation_field("model", "缺少必填字段 model"))?;
    let alias = validate_model(model)?;
    let messages = object
        .get("messages")
        .and_then(Value::as_array)
        .ok_or_else(|| AppError::validation_field("messages", "messages 必须是数组"))?;
    if messages.is_empty() {
        return Err(AppError::validation_field(
            "messages",
            "messages 不能为空数组",
        ));
    }
    let is_stream = match object.get("stream") {
        None | Some(Value::Null) => false,
        Some(value) => value
            .as_bool()
            .ok_or_else(|| AppError::validation_field("stream", "stream 必须是布尔值"))?,
    };
    let prompt_chars = object
        .get("messages")
        .map(super::count_text_chars)
        .unwrap_or(0)
        + object
            .get("tools")
            .map(super::count_text_chars)
            .unwrap_or(0)
        + object
            .get("system")
            .map(super::count_text_chars)
            .unwrap_or(0);
    let max_tokens = object
        .get("max_tokens")
        .or_else(|| object.get("max_completion_tokens"))
        .and_then(Value::as_i64)
        .filter(|value| *value > 0);
    Ok(ChatRequest {
        alias,
        is_stream,
        prompt_chars,
        max_tokens,
        value,
    })
}

/// 构造上游请求体：替换 model；流式且能力允许时补 `stream_options.include_usage`。
pub fn prepare_upstream_body(
    request: &ChatRequest,
    upstream_model: &str,
    capabilities: &[String],
) -> Result<Bytes, AppError> {
    let object = request
        .value
        .as_object()
        .ok_or_else(|| AppError::validation_field("body", "请求体必须是 JSON object"))?;
    let mut object = object.clone();
    object.insert(
        "model".to_string(),
        Value::String(upstream_model.to_string()),
    );
    if request.is_stream
        && capabilities
            .iter()
            .any(|capability| capability == "stream_usage")
        && !object.contains_key("stream_options")
    {
        object.insert(
            "stream_options".to_string(),
            Value::Object(Map::from_iter([(
                "include_usage".to_string(),
                Value::Bool(true),
            )])),
        );
    }
    serde_json::to_vec(&Value::Object(object))
        .map(Bytes::from)
        .map_err(|error| AppError::internal(format!("serialize chat request: {error}")))
}

/// 非流式响应解码：校验结构、回写别名、提取 usage，并返回转发字节。
pub fn decode_non_stream(raw: &[u8], alias: &str) -> Result<(Bytes, UsageTokens, usize), AppError> {
    let mut value: Value = serde_json::from_slice(raw).map_err(|_| {
        AppError::bad_gateway("upstream_protocol_error", "上游响应不是合法 JSON", None)
    })?;
    let object = require_object(&value, "body").map_err(|_| {
        AppError::bad_gateway(
            "upstream_protocol_error",
            "上游响应必须是 JSON object",
            None,
        )
    })?;
    if object.get("choices").and_then(Value::as_array).is_none() {
        return Err(AppError::bad_gateway(
            "upstream_protocol_error",
            "上游响应缺少 choices 数组",
            None,
        ));
    }
    let usage = parse_usage(object.get("usage"));
    let output_chars = object
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
        .map(|message| match message.get("content") {
            Some(Value::String(text)) => text.chars().count(),
            Some(Value::Array(parts)) => parts
                .iter()
                .filter_map(|part| part.get("text").and_then(Value::as_str))
                .map(|text| text.chars().count())
                .sum(),
            _ => 0,
        })
        .unwrap_or(0);
    let object = value.as_object_mut().expect("checked object above");
    object.insert("model".to_string(), Value::String(alias.to_string()));
    let bytes = serde_json::to_vec(&value)
        .map(Bytes::from)
        .map_err(|error| AppError::internal(format!("serialize chat response: {error}")))?;
    Ok((bytes, usage, output_chars))
}

#[derive(Debug, Clone, Default)]
pub struct ChatStreamInfo {
    pub usage: UsageTokens,
    pub finish_reason: Option<String>,
    pub upstream_error: Option<String>,
}

/// 解析 Chat SSE `data`（单条事件）；返回旁路信息。
pub fn inspect_stream_data(data: &str) -> ChatStreamInfo {
    let mut info = ChatStreamInfo::default();
    let Ok(value) = serde_json::from_str::<Value>(data) else {
        return info;
    };
    if let Some(error) = top_level_error(&value) {
        info.upstream_error = Some(error.to_string());
        return info;
    }
    ObjectAccess::usage(&value, &mut info);
    if let Some(reason) = value
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("finish_reason"))
        .and_then(Value::as_str)
    {
        info.finish_reason = Some(reason.to_string());
    }
    info
}

struct ObjectAccess;

impl ObjectAccess {
    fn usage(value: &Value, info: &mut ChatStreamInfo) {
        if let Some(usage) = value.get("usage").filter(|usage| !usage.is_null()) {
            info.usage = parse_usage(Some(usage));
        }
    }
}

/// 流式增量文本（用于 usage 估算）。
pub fn stream_content_delta(data: &str) -> String {
    let Ok(value) = serde_json::from_str::<Value>(data) else {
        return String::new();
    };
    value
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("delta"))
        .and_then(|delta| delta.get("content"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(raw: &str) -> ChatRequest {
        parse_request(raw.as_bytes()).expect("request parses")
    }

    #[test]
    fn validates_chat_request() {
        let request = parse(r#"{"model":"fast","messages":[{"role":"user","content":"hi"}]}"#);
        assert_eq!(request.alias, "fast");
        assert!(!request.is_stream);
        assert!(request.prompt_chars >= 2);
        assert!(parse_request(br#"{"messages":[]}"#).is_err());
        assert!(parse_request(br#"{"model":"fast","messages":[],"stream":"yes"}"#).is_err());
    }

    #[test]
    fn replaces_model_and_keeps_unknown_fields() {
        let request = parse(
            r#"{"model":"fast","messages":[{"role":"user","content":"hi"}],"temperature":0.2,"x_ext":1}"#,
        );
        let body = prepare_upstream_body(&request, "deepseek-chat", &[]).expect("body");
        let value: Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(value["model"], "deepseek-chat");
        assert_eq!(value["temperature"], 0.2);
        assert_eq!(value["x_ext"], 1);
        assert!(value.get("stream_options").is_none());
    }

    #[test]
    fn injects_stream_usage_only_when_capability_present() {
        let request =
            parse(r#"{"model":"fast","stream":true,"messages":[{"role":"user","content":"hi"}]}"#);
        let body = prepare_upstream_body(
            &request,
            "deepseek-chat",
            &["stream".to_string(), "stream_usage".to_string()],
        )
        .expect("body");
        let value: Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(value["stream_options"]["include_usage"], true);
    }

    #[test]
    fn decode_non_stream_rewrites_model_and_reads_usage() {
        let raw = br#"{"id":"x","object":"chat.completion","model":"deepseek-chat","choices":[{"index":0,"message":{"role":"assistant","content":"hi"},"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}}"#;
        let (bytes, usage, _output_chars) = decode_non_stream(raw, "fast").expect("decode");
        let value: Value = serde_json::from_slice(&bytes).expect("json");
        assert_eq!(value["model"], "fast");
        assert_eq!(usage.input(), 3);
        assert_eq!(usage.output(), 2);
        assert!(decode_non_stream(b"not-json", "fast").is_err());
        assert!(decode_non_stream(br#"{"id":"x"}"#, "fast").is_err());
    }

    #[test]
    fn inspects_stream_chunks() {
        let first = inspect_stream_data(
            r#"{"choices":[{"index":0,"delta":{"content":"hi"},"finish_reason":null}]}"#,
        );
        assert_eq!(first.finish_reason, None);
        let last = inspect_stream_data(
            r#"{"choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":4,"completion_tokens":6}}"#,
        );
        assert_eq!(last.finish_reason.as_deref(), Some("stop"));
        assert_eq!(last.usage.input(), 4);
        assert_eq!(last.usage.output(), 6);
        assert!(
            inspect_stream_data(r#"{"error":{"message":"boom"}}"#)
                .upstream_error
                .is_some()
        );
        assert_eq!(
            stream_content_delta(r#"{"choices":[{"delta":{"content":"ab"}}]}"#),
            "ab"
        );
    }
}
