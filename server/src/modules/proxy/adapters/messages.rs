//! Anthropic Messages 前端 ↔ OpenAI Chat 上游适配器（`messages_to_chat`）。
//!
//! S4 支持：文本、system、tools、tool_choice、tool_use/tool_result、stop/temperature/top_p、
//! 流式事件序列、usage/finish。不支持内容块（image/document/audio/video/thinking）→ 400
//! `unsupported_content`；可近似字段（top_k/thinking/cache_control/stop>4/is_error）→ `x-gw-degraded`。
//!
//! 纯同步实现：请求转换、非流式响应转换、流式事件状态机；IO/超时/重试在 `upstream`/`stream`。

use std::collections::BTreeMap;

use bytes::Bytes;
use serde_json::{Map, Value};
use uuid::Uuid;

use crate::platform::error::AppError;

use super::{UsageTokens, estimate_tokens, parse_usage, top_level_error, validate_model};

pub const ADAPTER_ID: &str = "messages_to_chat";

const ROOT_FIELDS: &[&str] = &[
    "model",
    "max_tokens",
    "messages",
    "system",
    "stream",
    "tools",
    "tool_choice",
    "temperature",
    "top_p",
    "top_k",
    "stop_sequences",
    "metadata",
    "thinking",
    "service_tier",
    // Claude Code 2.1.x 会发送 context_management；S4 不支持该能力，静默丢弃。
    "context_management",
    // Claude Code 2.1.x 的 output_config（如 effort）；S4 不支持，静默丢弃。
    "output_config",
];

const MESSAGE_FIELDS: &[&str] = &["role", "content"];
const TOOL_FIELDS: &[&str] = &["name", "description", "input_schema", "cache_control"];
const TOOL_CHOICE_FIELDS: &[&str] = &["type", "name"];

#[derive(Debug, Clone)]
pub struct MessagesRequest {
    pub alias: String,
    pub is_stream: bool,
    /// 已转换为 Chat Completions 形状的请求体；`model` 暂为请求别名，发送前由执行器替换。
    pub chat_body: Value,
    pub prompt_chars: usize,
    pub degraded: Vec<&'static str>,
    pub max_tokens: i64,
}

impl MessagesRequest {
    pub fn input_tokens_estimate(&self) -> i64 {
        estimate_tokens(self.prompt_chars)
    }
}

pub fn parse_request(raw: &[u8]) -> Result<MessagesRequest, AppError> {
    let value = super::parse_json_body(raw)?;
    let object = super::require_object(&value, "body")?;
    reject_unknown(object, ROOT_FIELDS, "")?;

    let model = object
        .get("model")
        .and_then(Value::as_str)
        .ok_or_else(|| super::validation_field("model", "缺少必填字段 model"))?;
    let alias = validate_model(model)?;

    let max_tokens = object
        .get("max_tokens")
        .ok_or_else(|| super::validation_field("max_tokens", "缺少必填字段 max_tokens"))
        .and_then(|value| {
            value
                .as_i64()
                .ok_or_else(|| super::validation_field("max_tokens", "max_tokens 必须是整数"))
        })?;
    if !(1..=1_000_000).contains(&max_tokens) {
        return Err(super::validation_field(
            "max_tokens",
            "max_tokens 必须在 1–1,000,000 之间",
        ));
    }

    let is_stream = match object.get("stream") {
        None | Some(Value::Null) => false,
        Some(value) => value
            .as_bool()
            .ok_or_else(|| super::validation_field("stream", "stream 必须是布尔值"))?,
    };

    let mut degraded = Vec::new();
    let system = convert_system(object.get("system"), &mut degraded)?;
    let messages = object
        .get("messages")
        .and_then(Value::as_array)
        .ok_or_else(|| super::validation_field("messages", "messages 必须是数组"))?;
    if messages.is_empty() {
        return Err(super::validation_field("messages", "messages 不能为空数组"));
    }
    let mut chat_messages: Vec<Value> = Vec::new();
    if let Some(system) = system {
        chat_messages.push(serde_json::json!({"role": "system", "content": system}));
    }
    for (index, message) in messages.iter().enumerate() {
        convert_message(message, index, &mut chat_messages, &mut degraded)?;
    }

    let mut body = Map::new();
    body.insert("model".to_string(), Value::String(alias.clone()));
    body.insert("messages".to_string(), Value::Array(chat_messages));
    body.insert("max_tokens".to_string(), Value::Number(max_tokens.into()));
    body.insert("stream".to_string(), Value::Bool(is_stream));

    if let Some(tools) = object.get("tools") {
        body.insert("tools".to_string(), convert_tools(tools, &mut degraded)?);
    }
    if let Some(tool_choice) = object.get("tool_choice") {
        body.insert(
            "tool_choice".to_string(),
            convert_tool_choice(tool_choice, &mut degraded)?,
        );
    }
    if let Some(temperature) = object.get("temperature") {
        if !temperature.is_number() {
            return Err(super::validation_field(
                "temperature",
                "temperature 必须是数字",
            ));
        }
        body.insert("temperature".to_string(), temperature.clone());
    }
    if let Some(top_p) = object.get("top_p") {
        if !top_p.is_number() {
            return Err(super::validation_field("top_p", "top_p 必须是数字"));
        }
        body.insert("top_p".to_string(), top_p.clone());
    }
    let mut stop_sequences = Vec::new();
    if let Some(stop) = object.get("stop_sequences") {
        let Some(items) = stop.as_array() else {
            return Err(super::validation_field(
                "stop_sequences",
                "stop_sequences 必须是数组",
            ));
        };
        for (index, item) in items.iter().enumerate() {
            let Some(sequence) = item.as_str() else {
                return Err(super::validation_field(
                    format!("stop_sequences[{index}]"),
                    "stop_sequence 必须是字符串",
                ));
            };
            stop_sequences.push(sequence.to_string());
        }
        if stop_sequences.len() > 4 {
            degraded.push("stop_sequences");
            stop_sequences.truncate(4);
        }
        if !stop_sequences.is_empty() {
            body.insert(
                "stop".to_string(),
                Value::Array(
                    stop_sequences
                        .iter()
                        .map(|sequence| Value::String(sequence.clone()))
                        .collect(),
                ),
            );
        }
    }
    if object.contains_key("top_k") {
        degraded.push("top_k");
    }
    if object.contains_key("thinking") {
        degraded.push("thinking");
    }
    // metadata / service_tier：丢弃（metadata 可能含用户标识，不写日志）。

    let prompt_chars = super::count_text_chars(&value);
    degraded.sort_unstable();
    degraded.dedup();

    Ok(MessagesRequest {
        alias,
        is_stream,
        chat_body: Value::Object(body),
        prompt_chars,
        degraded,
        max_tokens,
    })
}

/// 发送前替换 model 并按能力注入 stream usage 选项。
pub fn prepare_upstream_body(
    request: &MessagesRequest,
    upstream_model: &str,
    capabilities: &[String],
) -> Result<Bytes, AppError> {
    let mut body = request.chat_body.clone();
    let object = body
        .as_object_mut()
        .ok_or_else(|| AppError::internal("messages chat body must be object"))?;
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
            serde_json::json!({"include_usage": true}),
        );
    }
    serde_json::to_vec(&body)
        .map(Bytes::from)
        .map_err(|error| AppError::internal(format!("serialize messages request: {error}")))
}

fn convert_system(
    system: Option<&Value>,
    degraded: &mut Vec<&'static str>,
) -> Result<Option<String>, AppError> {
    let Some(system) = system else {
        return Ok(None);
    };
    match system {
        Value::String(text) => Ok(Some(text.clone())),
        Value::Array(items) => {
            let mut texts = Vec::new();
            for (index, item) in items.iter().enumerate() {
                let object = item.as_object().ok_or_else(|| {
                    super::validation_field(
                        format!("system[{index}]"),
                        "system block 必须是 object",
                    )
                })?;
                if object.get("type").and_then(Value::as_str) != Some("text") {
                    return Err(AppError::bad_request(
                        "unsupported_content",
                        "system 仅支持 text block（S4）",
                    ));
                }
                let text = object.get("text").and_then(Value::as_str).ok_or_else(|| {
                    super::validation_field(format!("system[{index}].text"), "text block 缺少 text")
                })?;
                if object.contains_key("cache_control") {
                    degraded.push("cache_control");
                }
                texts.push(text.to_string());
            }
            if texts.is_empty() {
                Ok(None)
            } else {
                Ok(Some(texts.join("\n\n")))
            }
        }
        _ => Err(super::validation_field(
            "system",
            "system 必须是字符串或 text block 数组",
        )),
    }
}

fn convert_message(
    message: &Value,
    index: usize,
    chat_messages: &mut Vec<Value>,
    degraded: &mut Vec<&'static str>,
) -> Result<(), AppError> {
    let object = message.as_object().ok_or_else(|| {
        super::validation_field(format!("messages[{index}]"), "message 必须是 object")
    })?;
    reject_unknown(object, MESSAGE_FIELDS, &format!("messages[{index}]"))?;
    let role = object
        .get("role")
        .and_then(Value::as_str)
        .ok_or_else(|| super::validation_field(format!("messages[{index}].role"), "缺少 role"))?;
    if role != "user" && role != "assistant" && role != "system" {
        return Err(super::validation_field(
            format!("messages[{index}].role"),
            "role 必须是 user/assistant/system",
        ));
    }
    let content = object.get("content").ok_or_else(|| {
        super::validation_field(format!("messages[{index}].content"), "缺少 content")
    })?;

    if role == "system" {
        // Claude Code 2.1.x 会在 messages 数组中发送 role=system（环境提示）；
        // Anthropic 官方请求通常用顶层 system，但为真实客户端兼容，转成 Chat system 消息。
        if let Some(text) = convert_system(Some(content), degraded)? {
            chat_messages.push(serde_json::json!({"role": "system", "content": text}));
        }
        return Ok(());
    }
    if role == "assistant" {
        convert_assistant_content(content, index, chat_messages, degraded)?;
    } else {
        convert_user_content(content, index, chat_messages, degraded)?;
    }
    Ok(())
}

fn convert_assistant_content(
    content: &Value,
    index: usize,
    chat_messages: &mut Vec<Value>,
    degraded: &mut Vec<&'static str>,
) -> Result<(), AppError> {
    if let Some(text) = content.as_str() {
        chat_messages.push(serde_json::json!({"role": "assistant", "content": text}));
        return Ok(());
    }
    let Some(blocks) = content.as_array() else {
        return Err(super::validation_field(
            format!("messages[{index}].content"),
            "content 必须是字符串或 block 数组",
        ));
    };
    let mut texts = Vec::new();
    let mut tool_calls = Vec::new();
    for (block_index, block) in blocks.iter().enumerate() {
        let path = format!("messages[{index}].content[{block_index}]");
        let object = block
            .as_object()
            .ok_or_else(|| super::validation_field(path.as_str(), "content block 必须是 object"))?;
        collect_cache_control(object, degraded);
        match object.get("type").and_then(Value::as_str) {
            Some("text") => {
                let text = object.get("text").and_then(Value::as_str).ok_or_else(|| {
                    super::validation_field(format!("{path}.text"), "text block 缺少 text")
                })?;
                texts.push(text.to_string());
            }
            Some("tool_use") => {
                let id = object
                    .get("id")
                    .and_then(Value::as_str)
                    .filter(|id| !id.is_empty())
                    .ok_or_else(|| {
                        super::validation_field(format!("{path}.id"), "tool_use 缺少 id")
                    })?;
                let name = object
                    .get("name")
                    .and_then(Value::as_str)
                    .filter(|name| !name.is_empty())
                    .ok_or_else(|| {
                        super::validation_field(format!("{path}.name"), "tool_use 缺少 name")
                    })?;
                let input = object
                    .get("input")
                    .cloned()
                    .unwrap_or_else(|| Value::Object(Map::new()));
                if !input.is_object() {
                    return Err(super::validation_field(
                        format!("{path}.input"),
                        "tool_use.input 必须是 object",
                    ));
                }
                let arguments = serde_json::to_string(&input).map_err(|error| {
                    AppError::internal(format!("serialize tool input: {error}"))
                })?;
                tool_calls.push(serde_json::json!({
                    "id": id,
                    "type": "function",
                    "function": {"name": name, "arguments": arguments}
                }));
            }
            other => {
                return Err(AppError::bad_request(
                    "unsupported_content",
                    format!(
                        "不支持的 assistant content block 类型: {}",
                        other.unwrap_or("<missing type>")
                    ),
                ));
            }
        }
    }
    let mut message = Map::new();
    message.insert("role".to_string(), Value::String("assistant".to_string()));
    if texts.is_empty() {
        message.insert("content".to_string(), Value::Null);
    } else {
        message.insert("content".to_string(), Value::String(texts.join("\n")));
    }
    if !tool_calls.is_empty() {
        message.insert("tool_calls".to_string(), Value::Array(tool_calls));
    }
    chat_messages.push(Value::Object(message));
    Ok(())
}

fn convert_user_content(
    content: &Value,
    index: usize,
    chat_messages: &mut Vec<Value>,
    degraded: &mut Vec<&'static str>,
) -> Result<(), AppError> {
    if let Some(text) = content.as_str() {
        chat_messages.push(serde_json::json!({"role": "user", "content": text}));
        return Ok(());
    }
    let Some(blocks) = content.as_array() else {
        return Err(super::validation_field(
            format!("messages[{index}].content"),
            "content 必须是字符串或 block 数组",
        ));
    };
    let mut tool_messages = Vec::new();
    let mut texts = Vec::new();
    for (block_index, block) in blocks.iter().enumerate() {
        let path = format!("messages[{index}].content[{block_index}]");
        let object = block
            .as_object()
            .ok_or_else(|| super::validation_field(path.as_str(), "content block 必须是 object"))?;
        collect_cache_control(object, degraded);
        match object.get("type").and_then(Value::as_str) {
            Some("text") => {
                let text = object.get("text").and_then(Value::as_str).ok_or_else(|| {
                    super::validation_field(format!("{path}.text"), "text block 缺少 text")
                })?;
                texts.push(text.to_string());
            }
            Some("tool_result") => {
                let tool_use_id = object
                    .get("tool_use_id")
                    .and_then(Value::as_str)
                    .filter(|id| !id.is_empty())
                    .ok_or_else(|| {
                        super::validation_field(
                            format!("{path}.tool_use_id"),
                            "tool_result 缺少 tool_use_id",
                        )
                    })?;
                if object.get("is_error").and_then(Value::as_bool) == Some(true) {
                    degraded.push("tool_result.is_error");
                }
                let result_text = convert_tool_result_content(object.get("content"), &path)?;
                tool_messages.push(serde_json::json!({
                    "role": "tool",
                    "tool_call_id": tool_use_id,
                    "content": result_text
                }));
            }
            other => {
                return Err(AppError::bad_request(
                    "unsupported_content",
                    format!(
                        "不支持的 user content block 类型: {}",
                        other.unwrap_or("<missing type>")
                    ),
                ));
            }
        }
    }
    // tool 消息必须紧跟 assistant tool_calls，先于同条 user 文本输出。
    chat_messages.extend(tool_messages);
    if !texts.is_empty() {
        chat_messages.push(serde_json::json!({"role": "user", "content": texts.join("\n")}));
    }
    Ok(())
}

fn convert_tool_result_content(content: Option<&Value>, path: &str) -> Result<String, AppError> {
    let Some(content) = content else {
        return Ok(String::new());
    };
    if let Some(text) = content.as_str() {
        return Ok(text.to_string());
    }
    let Some(blocks) = content.as_array() else {
        return Err(super::validation_field(
            format!("{path}.content"),
            "tool_result.content 必须是字符串或 text block 数组",
        ));
    };
    let mut texts = Vec::new();
    for (index, block) in blocks.iter().enumerate() {
        let object = block.as_object().ok_or_else(|| {
            super::validation_field(
                format!("{path}.content[{index}]"),
                "tool_result block 必须是 object",
            )
        })?;
        if object.get("type").and_then(Value::as_str) != Some("text") {
            return Err(AppError::bad_request(
                "unsupported_content",
                "tool_result.content 仅支持 text block（S4）",
            ));
        }
        let text = object.get("text").and_then(Value::as_str).ok_or_else(|| {
            super::validation_field(
                format!("{path}.content[{index}].text"),
                "text block 缺少 text",
            )
        })?;
        texts.push(text.to_string());
    }
    Ok(texts.join("\n"))
}

fn convert_tools(tools: &Value, degraded: &mut Vec<&'static str>) -> Result<Value, AppError> {
    let Some(items) = tools.as_array() else {
        return Err(super::validation_field("tools", "tools 必须是数组"));
    };
    if items.len() > 128 {
        return Err(super::validation_field("tools", "tools 最多 128 个"));
    }
    let mut converted = Vec::with_capacity(items.len());
    for (index, tool) in items.iter().enumerate() {
        let path = format!("tools[{index}]");
        let object = tool
            .as_object()
            .ok_or_else(|| super::validation_field(path.as_str(), "tool 必须是 object"))?;
        reject_unknown(object, TOOL_FIELDS, &path)?;
        if object.contains_key("cache_control") {
            degraded.push("cache_control");
        }
        let name = object
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| {
                !name.is_empty()
                    && name.chars().count() <= super::MAX_TOOL_NAME_LEN
                    && name
                        .chars()
                        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))
            })
            .ok_or_else(|| {
                super::validation_field(
                    format!("{path}.name"),
                    "tool name 必须是 1–64 位字母/数字/_/-",
                )
            })?;
        let input_schema = object
            .get("input_schema")
            .cloned()
            .filter(Value::is_object)
            .ok_or_else(|| {
                super::validation_field(
                    format!("{path}.input_schema"),
                    "tool.input_schema 必须是 object",
                )
            })?;
        let description = object
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or_default();
        converted.push(serde_json::json!({
            "type": "function",
            "function": {
                "name": name,
                "description": description,
                "parameters": input_schema
            }
        }));
    }
    Ok(Value::Array(converted))
}

fn convert_tool_choice(
    choice: &Value,
    _degraded: &mut Vec<&'static str>,
) -> Result<Value, AppError> {
    let object = choice
        .as_object()
        .ok_or_else(|| super::validation_field("tool_choice", "tool_choice 必须是 object"))?;
    reject_unknown(object, TOOL_CHOICE_FIELDS, "tool_choice")?;
    let choice_type = object
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| super::validation_field("tool_choice.type", "缺少 type"))?;
    match choice_type {
        "auto" => Ok(Value::String("auto".to_string())),
        "any" => Ok(Value::String("required".to_string())),
        "none" => Ok(Value::String("none".to_string())),
        "tool" => {
            let name = object.get("name").and_then(Value::as_str).ok_or_else(|| {
                super::validation_field("tool_choice.name", "type=tool 时缺少 name")
            })?;
            Ok(serde_json::json!({"type": "function", "function": {"name": name}}))
        }
        other => Err(super::validation_field(
            "tool_choice.type",
            format!("不支持的 tool_choice.type: {other}"),
        )),
    }
}

fn collect_cache_control(object: &Map<String, Value>, degraded: &mut Vec<&'static str>) {
    if object.contains_key("cache_control") {
        degraded.push("cache_control");
    }
}

fn reject_unknown(
    object: &Map<String, Value>,
    allowed: &[&str],
    prefix: &str,
) -> Result<(), AppError> {
    for key in object.keys() {
        if !allowed.contains(&key.as_str()) {
            let field = if prefix.is_empty() {
                key.clone()
            } else {
                format!("{prefix}.{key}")
            };
            return Err(super::validation_field(field, "未知字段（S4 不支持）"));
        }
    }
    Ok(())
}

// ---------- 非流式响应 ----------

#[derive(Debug, Clone)]
pub struct MessagesDecoded {
    pub body: Bytes,
    pub usage: UsageTokens,
    pub degraded: Vec<&'static str>,
    pub output_chars: usize,
}

pub fn decode_non_stream(
    raw: &[u8],
    request: &MessagesRequest,
    request_id: &str,
) -> Result<MessagesDecoded, AppError> {
    let value: Value = serde_json::from_slice(raw).map_err(|_| {
        AppError::bad_gateway("upstream_protocol_error", "上游响应不是合法 JSON", None)
    })?;
    let object = value.as_object().ok_or_else(|| {
        AppError::bad_gateway(
            "upstream_protocol_error",
            "上游响应必须是 JSON object",
            None,
        )
    })?;
    let choice = object
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .ok_or_else(|| {
            AppError::bad_gateway("upstream_protocol_error", "上游响应缺少 choices[0]", None)
        })?;
    let message = choice
        .get("message")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            AppError::bad_gateway(
                "upstream_protocol_error",
                "上游响应缺少 choices[0].message",
                None,
            )
        })?;
    let finish_reason = choice
        .get("finish_reason")
        .and_then(Value::as_str)
        .map(ToString::to_string);

    let mut degraded = request.degraded.clone();
    let mut content = Vec::new();
    let mut output_chars = 0usize;
    match message.get("content") {
        Some(Value::String(text)) if !text.is_empty() => {
            output_chars += text.chars().count();
            content.push(serde_json::json!({"type": "text", "text": text}));
        }
        Some(Value::Array(parts)) => {
            let text: String = parts
                .iter()
                .filter_map(|part| part.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("");
            if !text.is_empty() {
                output_chars += text.chars().count();
                content.push(serde_json::json!({"type": "text", "text": text}));
            }
        }
        _ => {}
    }
    if let Some(tool_calls) = message.get("tool_calls").and_then(Value::as_array) {
        for (index, tool_call) in tool_calls.iter().enumerate() {
            let function = tool_call.get("function");
            let name = function
                .and_then(|function| function.get("name"))
                .and_then(Value::as_str)
                .unwrap_or("unknown_tool");
            let arguments = function
                .and_then(|function| function.get("arguments"))
                .and_then(Value::as_str)
                .unwrap_or("");
            output_chars += arguments.chars().count();
            let input = match serde_json::from_str::<Value>(arguments) {
                Ok(value) if value.is_object() => value,
                _ => {
                    degraded.push("tool_arguments_invalid");
                    Value::Object(Map::new())
                }
            };
            let id = tool_call
                .get("id")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(|| format!("toolu_{request_id}_{index}"));
            content.push(serde_json::json!({
                "type": "tool_use",
                "id": id,
                "name": name,
                "input": input
            }));
        }
    }
    if content.is_empty() {
        content.push(serde_json::json!({"type": "text", "text": ""}));
    }

    let stop_reason = map_stop_reason(finish_reason.as_deref(), !degraded.is_empty());
    if finish_reason.as_deref() == Some("content_filter") {
        degraded.push("content_filter");
    }
    let mut usage = parse_usage(object.get("usage"));
    if usage.input_tokens.is_none() {
        usage.input_tokens = Some(estimate_tokens(request.prompt_chars));
    }
    if usage.output_tokens.is_none() {
        usage.output_tokens = Some(estimate_tokens(output_chars));
    }
    degraded.sort_unstable();
    degraded.dedup();

    let body = serde_json::json!({
        "id": message_id(request_id),
        "type": "message",
        "role": "assistant",
        "model": request.alias,
        "content": content,
        "stop_reason": stop_reason,
        "stop_sequence": Value::Null,
        "usage": {
            "input_tokens": usage.input(),
            "output_tokens": usage.output()
        }
    });
    let bytes = serde_json::to_vec(&body)
        .map(Bytes::from)
        .map_err(|error| AppError::internal(format!("serialize messages response: {error}")))?;
    Ok(MessagesDecoded {
        body: bytes,
        usage,
        degraded,
        output_chars,
    })
}

fn map_stop_reason(finish_reason: Option<&str>, _degraded: bool) -> &'static str {
    match finish_reason {
        Some("length") => "max_tokens",
        Some("tool_calls") | Some("function_call") => "tool_use",
        Some("content_filter") => "end_turn",
        _ => "end_turn",
    }
}

fn message_id(request_id: &str) -> String {
    let cleaned: String = request_id
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .take(48)
        .collect();
    if cleaned.is_empty() {
        format!("msg_{}", Uuid::new_v4().simple())
    } else {
        format!("msg_{cleaned}")
    }
}

// ---------- 流式转换状态机 ----------

#[derive(Debug, Clone, Default)]
struct ToolState {
    id: String,
    name: String,
    arguments: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OpenBlock {
    Text(usize),
}

#[derive(Debug)]
pub struct MessagesStream {
    request_id: String,
    model: String,
    input_tokens_est: i64,
    degraded: Vec<&'static str>,
    started: bool,
    current_block: Option<OpenBlock>,
    next_block: usize,
    text_chars: usize,
    tools: BTreeMap<u64, ToolState>,
    tool_order: Vec<u64>,
    finish_reason: Option<String>,
    usage: UsageTokens,
    error: Option<String>,
    done: bool,
}

impl MessagesStream {
    pub fn new(request: &MessagesRequest, request_id: &str) -> Self {
        Self {
            request_id: message_id(request_id),
            model: request.alias.clone(),
            input_tokens_est: request.input_tokens_estimate(),
            degraded: request.degraded.clone(),
            started: false,
            current_block: None,
            next_block: 0,
            text_chars: 0,
            tools: BTreeMap::new(),
            tool_order: Vec::new(),
            finish_reason: None,
            usage: UsageTokens::default(),
            error: None,
            done: false,
        }
    }

    pub fn take_error(&mut self) -> Option<String> {
        self.error.take()
    }

    pub fn degrade(&mut self, item: &'static str) {
        self.degraded.push(item);
        self.degraded.sort_unstable();
        self.degraded.dedup();
    }

    /// 处理一个 SSE 事件（Chat 上游帧），返回需要写给客户端的 Anthropic SSE 帧。
    pub fn handle_event(&mut self, event: &super::super::sse::SseEvent) -> Vec<Bytes> {
        if event.is_done() {
            return self.finish();
        }
        let data = event.data.trim();
        if data.is_empty() {
            return Vec::new();
        }
        let Ok(value) = serde_json::from_str::<Value>(data) else {
            return Vec::new();
        };
        if let Some(error) = top_level_error(&value) {
            self.error = Some(error.to_string());
            return Vec::new();
        }
        self.handle_chunk(&value)
    }

    fn handle_chunk(&mut self, value: &Value) -> Vec<Bytes> {
        let mut frames = Vec::new();
        if !self.started {
            frames.push(self.message_start_frame());
            self.started = true;
        }
        if let Some(usage) = value.get("usage").filter(|usage| !usage.is_null()) {
            self.usage = parse_usage(Some(usage));
        }
        let Some(choice) = value
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
        else {
            return frames;
        };
        if let Some(delta) = choice.get("delta") {
            if let Some(content) = delta.get("content").and_then(Value::as_str)
                && !content.is_empty()
            {
                frames.extend(self.text_delta(content));
            }
            if let Some(tool_calls) = delta.get("tool_calls").and_then(Value::as_array) {
                for tool_call in tool_calls {
                    let index = tool_call
                        .get("index")
                        .and_then(Value::as_u64)
                        .or_else(|| {
                            tool_call
                                .get("index")
                                .and_then(Value::as_i64)
                                .map(|v| v as u64)
                        })
                        .unwrap_or(0);
                    let entry = self.tools.entry(index).or_insert_with(|| {
                        if !self.tool_order.contains(&index) {
                            self.tool_order.push(index);
                        }
                        ToolState::default()
                    });
                    if let Some(id) = tool_call.get("id").and_then(Value::as_str)
                        && !id.is_empty()
                    {
                        entry.id = id.to_string();
                    }
                    if let Some(function) = tool_call.get("function") {
                        if let Some(name) = function.get("name").and_then(Value::as_str)
                            && !name.is_empty()
                        {
                            entry.name = name.to_string();
                        }
                        if let Some(arguments) = function.get("arguments").and_then(Value::as_str) {
                            entry.arguments.push_str(arguments);
                        }
                    }
                }
            }
        }
        if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
            self.finish_reason = Some(reason.to_string());
            frames.extend(self.close_current_block());
        }
        frames
    }

    fn text_delta(&mut self, content: &str) -> Vec<Bytes> {
        let mut frames = Vec::new();
        let index = match self.current_block {
            Some(OpenBlock::Text(index)) => index,
            None => {
                let index = self.next_block;
                self.next_block += 1;
                self.current_block = Some(OpenBlock::Text(index));
                frames.push(block_start(
                    index,
                    serde_json::json!({"type": "text", "text": ""}),
                ));
                index
            }
        };
        self.text_chars += content.chars().count();
        frames.push(block_delta(
            index,
            serde_json::json!({"type": "text_delta", "text": content}),
        ));
        frames
    }

    fn close_current_block(&mut self) -> Vec<Bytes> {
        match self.current_block.take() {
            Some(OpenBlock::Text(index)) => vec![block_stop(index)],
            None => Vec::new(),
        }
    }

    fn message_start_frame(&mut self) -> Bytes {
        let value = serde_json::json!({
            "type": "message_start",
            "message": {
                "id": self.request_id,
                "type": "message",
                "role": "assistant",
                "model": self.model,
                "content": [],
                "stop_reason": Value::Null,
                "stop_sequence": Value::Null,
                "usage": {
                    "input_tokens": self.input_tokens_est,
                    "output_tokens": 0
                }
            }
        });
        crate::modules::proxy::sse::json_frame(Some("message_start"), &value)
    }

    /// 流结束（[DONE]/EOF/错误前）冲刷：关闭 block、tool blocks、message_delta/message_stop。
    pub fn finish(&mut self) -> Vec<Bytes> {
        if self.done {
            return Vec::new();
        }
        self.done = true;
        let mut frames = Vec::new();
        if !self.started {
            frames.push(self.message_start_frame());
            self.started = true;
        }
        if self.finish_reason.is_none() {
            // 上游未给 finish_reason 时按正常结束处理。
            self.finish_reason = Some("stop".to_string());
        }
        frames.extend(self.close_current_block());
        // 工具调用在结束时一次性输出完整 input（协议合法且支持并行工具；S4 取舍见技术设计 §6.3.3）。
        for index in self.tool_order.clone() {
            let Some(tool) = self.tools.get(&index).cloned() else {
                continue;
            };
            let block_index = self.next_block;
            self.next_block += 1;
            frames.push(block_start(
                block_index,
                serde_json::json!({
                    "type": "tool_use",
                    "id": tool.id,
                    "name": tool.name,
                    "input": {}
                }),
            ));
            frames.push(block_delta(
                block_index,
                serde_json::json!({
                    "type": "input_json_delta",
                    "partial_json": tool.arguments
                }),
            ));
            frames.push(block_stop(block_index));
        }
        let stop_reason = if !self.tool_order.is_empty() {
            "tool_use"
        } else {
            map_stop_reason(self.finish_reason.as_deref(), false)
        };
        if self.finish_reason.as_deref() == Some("content_filter") {
            self.degrade("content_filter");
        }
        if self.usage.output_tokens.is_none() {
            let chars = self.text_chars
                + self
                    .tools
                    .values()
                    .map(|tool| tool.arguments.chars().count())
                    .sum::<usize>();
            self.usage.output_tokens = Some(estimate_tokens(chars));
        }
        let value = serde_json::json!({
            "type": "message_delta",
            "delta": {
                "stop_reason": stop_reason,
                "stop_sequence": Value::Null
            },
            "usage": {
                "output_tokens": self.usage.output()
            }
        });
        frames.push(crate::modules::proxy::sse::json_frame(
            Some("message_delta"),
            &value,
        ));
        frames.push(crate::modules::proxy::sse::json_frame(
            Some("message_stop"),
            &serde_json::json!({"type": "message_stop"}),
        ));
        frames
    }

    pub fn usage(&self) -> UsageTokens {
        self.usage
    }

    pub fn output_tokens_estimate(&self) -> i64 {
        self.usage
            .output_tokens
            .unwrap_or_else(|| estimate_tokens(self.text_chars))
    }
}

fn block_start(index: usize, block: Value) -> Bytes {
    crate::modules::proxy::sse::json_frame(
        Some("content_block_start"),
        &serde_json::json!({"type": "content_block_start", "index": index, "content_block": block}),
    )
}

fn block_delta(index: usize, delta: Value) -> Bytes {
    crate::modules::proxy::sse::json_frame(
        Some("content_block_delta"),
        &serde_json::json!({"type": "content_block_delta", "index": index, "delta": delta}),
    )
}

fn block_stop(index: usize) -> Bytes {
    crate::modules::proxy::sse::json_frame(
        Some("content_block_stop"),
        &serde_json::json!({"type": "content_block_stop", "index": index}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(raw: &str) -> MessagesRequest {
        parse_request(raw.as_bytes()).expect("messages request parses")
    }

    #[test]
    fn converts_simple_text_request() {
        let request = parse(
            r#"{"model":"fast","max_tokens":256,"messages":[{"role":"user","content":"hi"}]}"#,
        );
        assert_eq!(request.alias, "fast");
        assert!(!request.is_stream);
        assert_eq!(request.chat_body["messages"][0]["content"], "hi");
        assert_eq!(request.chat_body["max_tokens"], 256);
    }

    #[test]
    fn tolerates_claude_code_context_management() {
        let request = parse(
            r#"{"model":"fast","max_tokens":256,"context_management":{"edits":[]},"output_config":{"effort":"high"},"messages":[{"role":"user","content":"hi"}]}"#,
        );
        assert_eq!(request.alias, "fast");
        assert!(request.chat_body.get("context_management").is_none());
        assert!(request.chat_body.get("output_config").is_none());
    }

    #[test]
    fn tolerates_claude_code_system_role_in_messages() {
        let request = parse(
            r#"{"model":"fast","max_tokens":256,"messages":[{"role":"user","content":"hi"},{"role":"system","content":[{"type":"text","text":"env"}]}]}"#,
        );
        let messages = request.chat_body["messages"].as_array().expect("messages");
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[1]["role"], "system");
        assert_eq!(messages[1]["content"], "env");
    }

    #[test]
    fn converts_system_and_text_blocks() {
        let request = parse(
            r#"{"model":"fast","max_tokens":256,"system":[{"type":"text","text":"a","cache_control":{"type":"ephemeral"}},{"type":"text","text":"b"}],"messages":[{"role":"user","content":[{"type":"text","text":"hi"},{"type":"text","text":"there"}]}]}"#,
        );
        assert_eq!(request.chat_body["messages"][0]["role"], "system");
        assert_eq!(request.chat_body["messages"][0]["content"], "a\n\nb");
        assert_eq!(request.chat_body["messages"][1]["content"], "hi\nthere");
        assert!(request.degraded.contains(&"cache_control"));
    }

    #[test]
    fn converts_tool_use_and_tool_result() {
        let request = parse(
            r#"{"model":"fast","max_tokens":256,"tools":[{"name":"Read","description":"read","input_schema":{"type":"object"}}],"tool_choice":{"type":"auto"},"messages":[{"role":"user","content":"read"},{"role":"assistant","content":[{"type":"tool_use","id":"t1","name":"Read","input":{"path":"a"}}]},{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"ok"},{"type":"text","text":"thanks"}]}]}"#,
        );
        let messages = request.chat_body["messages"].as_array().expect("messages");
        assert_eq!(messages[1]["tool_calls"][0]["id"], "t1");
        assert_eq!(messages[2]["role"], "tool");
        assert_eq!(messages[2]["tool_call_id"], "t1");
        assert_eq!(messages[3]["role"], "user");
        assert_eq!(request.chat_body["tools"][0]["function"]["name"], "Read");
        assert_eq!(request.chat_body["tool_choice"], "auto");
    }

    #[test]
    fn rejects_unsupported_and_invalid() {
        assert!(parse_request(
            br#"{"model":"fast","max_tokens":1,"messages":[{"role":"user","content":[{"type":"image","source":{}}]}]}"#
        )
        .is_err());
        assert!(parse_request(br#"{"model":"fast","messages":[]}"#).is_err());
        assert!(parse_request(br#"{"model":"fast","max_tokens":0,"messages":[]}"#).is_err());
        assert!(
            parse_request(
                br#"{"model":"fast","max_tokens":1,"messages":[{"role":"tool","content":"x"}]}"#
            )
            .is_err()
        );
        assert!(
            parse_request(br#"{"model":"fast","max_tokens":1,"messages":[],"unknown":1}"#).is_err()
        );
    }

    #[test]
    fn decodes_non_stream_with_tools() {
        let request = parse(
            r#"{"model":"fast","max_tokens":256,"messages":[{"role":"user","content":"hi"}]}"#,
        );
        let raw = br#"{"id":"x","choices":[{"message":{"role":"assistant","content":"ok","tool_calls":[{"id":"t1","type":"function","function":{"name":"Read","arguments":"{\"path\":\"a\"}"}}]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":3,"completion_tokens":4}}"#;
        let decoded = decode_non_stream(raw, &request, "req-1").expect("decode");
        let value: Value = serde_json::from_slice(&decoded.body).expect("json");
        assert_eq!(value["type"], "message");
        assert_eq!(value["stop_reason"], "tool_use");
        assert_eq!(value["content"][0]["type"], "text");
        assert_eq!(value["content"][1]["type"], "tool_use");
        assert_eq!(value["content"][1]["input"]["path"], "a");
        assert_eq!(value["model"], "fast");
        assert_eq!(decoded.usage.output(), 4);
    }

    #[test]
    fn stream_emits_anthropic_event_sequence() {
        let request = parse(
            r#"{"model":"fast","max_tokens":256,"stream":true,"messages":[{"role":"user","content":"hi"}]}"#,
        );
        let mut stream = MessagesStream::new(&request, "req-1");
        let mut all = Vec::new();
        for data in [
            r#"{"choices":[{"delta":{"role":"assistant","content":"你"},"finish_reason":null}]}"#,
            r#"{"choices":[{"delta":{"content":"好"},"finish_reason":null}]}"#,
            r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#,
            r#"{"choices":[],"usage":{"prompt_tokens":3,"completion_tokens":2}}"#,
        ] {
            all.extend(stream.handle_event(&crate::modules::proxy::sse::SseEvent {
                event: None,
                data: data.to_string(),
            }));
        }
        all.extend(stream.finish());
        let text = String::from_utf8_lossy(&all.concat()).to_string();
        assert!(text.contains("event: message_start"));
        assert!(text.contains("event: content_block_start"));
        assert!(text.contains("\"text\":\"你\""));
        assert!(text.contains("event: message_delta"));
        assert!(text.contains("\"stop_reason\":\"end_turn\""));
        assert!(text.contains("event: message_stop"));
    }

    #[test]
    fn stream_buffers_tool_calls_into_tool_use_blocks() {
        let request = parse(
            r#"{"model":"fast","max_tokens":256,"stream":true,"tools":[{"name":"Read","input_schema":{"type":"object"}}],"messages":[{"role":"user","content":"read"}]}"#,
        );
        let mut stream = MessagesStream::new(&request, "req-2");
        let mut all = Vec::new();
        for data in [
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"t1","function":{"name":"Read","arguments":"{\"path\":"}}]},"finish_reason":null}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"a\"}"}}]},"finish_reason":null}]}"#,
            r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
        ] {
            all.extend(stream.handle_event(&crate::modules::proxy::sse::SseEvent {
                event: None,
                data: data.to_string(),
            }));
        }
        all.extend(stream.finish());
        let text = String::from_utf8_lossy(&all.concat()).to_string();
        assert!(text.contains("\"type\":\"tool_use\""));
        assert!(text.contains("\"name\":\"Read\""));
        assert!(text.contains("input_json_delta"));
        assert!(text.contains("\"stop_reason\":\"tool_use\""));
    }

    #[test]
    fn stream_reports_upstream_error() {
        let request = parse(
            r#"{"model":"fast","max_tokens":256,"stream":true,"messages":[{"role":"user","content":"hi"}]}"#,
        );
        let mut stream = MessagesStream::new(&request, "req-3");
        stream.handle_event(&crate::modules::proxy::sse::SseEvent {
            event: None,
            data: r#"{"error":{"message":"boom"}}"#.to_string(),
        });
        assert!(stream.take_error().is_some());
    }
}
