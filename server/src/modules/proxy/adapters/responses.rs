//! OpenAI Responses 前端 ↔ Chat 上游适配器（`responses_to_chat`，US-014 / Codex CLI）。
//!
//! 范围（proxy Responses 实现设计）：`input` string/数组（message / function_call /
//! function_call_output）、`instructions`、function tools、`tool_choice`、
//! `max_output_tokens`、`temperature/top_p`、`stream`、`parallel_tool_calls`；
//! 未知字段忽略；`previous_response_id` 不存储（缺 input 时 400）。
//!
//! 纯同步：请求转换、非流式响应转换、流式事件状态机；IO/超时/重试/背传在 executor/stream。

use std::collections::BTreeMap;

use bytes::Bytes;
use serde_json::{Map, Value};
use uuid::Uuid;

use crate::platform::error::AppError;

use super::{UsageTokens, estimate_tokens, parse_usage, top_level_error, validate_model};

pub const ADAPTER_ID: &str = "responses_to_chat";
/// 流式工具参数累积上限（2 MiB），防止异常上游撑爆内存。
pub const MAX_TOOL_ARGUMENTS_BYTES: usize = 2 * 1024 * 1024;
/// 最终 response 需要回传文本；累积上限（8 MiB），超出按上游协议错误处理。
pub const MAX_ACCUMULATED_TEXT_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct ResponsesRequest {
    pub alias: String,
    pub is_stream: bool,
    /// 已转换为 Chat Completions 形状；`model` 暂为请求别名，发送前由执行器替换。
    pub chat_body: Value,
    pub prompt_chars: usize,
    pub degraded: Vec<&'static str>,
    pub max_tokens: Option<i64>,
    // 响应回显字段（Codex 可解析；不含敏感信息）。
    pub instructions: Option<String>,
    pub tools_echo: Value,
    pub tool_choice_echo: Value,
    pub parallel_tool_calls: bool,
    pub store: bool,
    pub metadata: Value,
    pub temperature: Value,
    pub top_p: Value,
    pub previous_response_id: Option<String>,
}

impl ResponsesRequest {
    pub fn input_tokens_estimate(&self) -> i64 {
        estimate_tokens(self.prompt_chars)
    }
}

pub fn parse_request(raw: &[u8]) -> Result<ResponsesRequest, AppError> {
    let value = super::parse_json_body(raw)?;
    let object = super::require_object(&value, "body")?;

    let model = object
        .get("model")
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::validation_field("model", "缺少必填字段 model"))?;
    let alias = validate_model(model)?;

    let previous_response_id = object
        .get("previous_response_id")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);
    let input = object.get("input").filter(|value| !value.is_null());
    let Some(input) = input else {
        return if previous_response_id.is_some() {
            Err(AppError::bad_request(
                "unsupported_parameter",
                "previous_response_id 暂不支持服务端存储；请携带完整 input",
            ))
        } else {
            Err(AppError::validation_field("input", "缺少必填字段 input"))
        };
    };

    let is_stream = match object.get("stream") {
        None | Some(Value::Null) => false,
        Some(value) => value
            .as_bool()
            .ok_or_else(|| AppError::validation_field("stream", "stream 必须是布尔值"))?,
    };
    let max_tokens = match object.get("max_output_tokens") {
        None | Some(Value::Null) => None,
        Some(value) => {
            let tokens = value.as_i64().ok_or_else(|| {
                AppError::validation_field("max_output_tokens", "max_output_tokens 必须是整数")
            })?;
            if !(1..=1_000_000).contains(&tokens) {
                return Err(AppError::validation_field(
                    "max_output_tokens",
                    "max_output_tokens 必须在 1–1,000,000 之间",
                ));
            }
            Some(tokens)
        }
    };

    let instructions = match object.get("instructions") {
        None | Some(Value::Null) => None,
        Some(Value::String(text)) => Some(text.clone()),
        Some(_) => {
            return Err(AppError::validation_field(
                "instructions",
                "instructions 必须是字符串",
            ));
        }
    };

    let mut degraded = Vec::new();
    if previous_response_id.is_some() {
        degraded.push("previous_response_id");
    }
    if object.contains_key("reasoning") {
        degraded.push("reasoning");
    }

    let mut messages: Vec<Value> = Vec::new();
    if let Some(instructions) = &instructions
        && !instructions.is_empty()
    {
        messages.push(serde_json::json!({"role": "system", "content": instructions}));
    }
    convert_input(input, &mut messages, &mut degraded)?;
    if messages.is_empty() {
        return Err(AppError::validation_field("input", "input 不能为空"));
    }

    let (tools, tools_echo) = match object.get("tools") {
        Some(tools) => convert_tools(tools, &mut degraded)?,
        None => (None, Value::Array(Vec::new())),
    };
    let (tool_choice, tool_choice_echo) = match object.get("tool_choice") {
        Some(choice) => convert_tool_choice(choice, &mut degraded)?,
        None => (None, Value::String("auto".to_string())),
    };

    let parallel_tool_calls = match object.get("parallel_tool_calls") {
        None | Some(Value::Null) => true,
        Some(value) => value.as_bool().ok_or_else(|| {
            AppError::validation_field("parallel_tool_calls", "parallel_tool_calls 必须是布尔值")
        })?,
    };
    let store = match object.get("store") {
        None | Some(Value::Null) => true,
        Some(value) => value
            .as_bool()
            .ok_or_else(|| AppError::validation_field("store", "store 必须是布尔值"))?,
    };
    let metadata = object
        .get("metadata")
        .filter(|value| value.is_object())
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    let temperature = object.get("temperature").cloned().unwrap_or(Value::Null);
    let top_p = object.get("top_p").cloned().unwrap_or(Value::Null);

    let mut body = Map::new();
    body.insert("model".to_string(), Value::String(alias.clone()));
    body.insert("messages".to_string(), Value::Array(messages));
    body.insert("stream".to_string(), Value::Bool(is_stream));
    if let Some(limit) = max_tokens {
        body.insert("max_tokens".to_string(), Value::Number(limit.into()));
    }
    if let Some(tools) = tools {
        body.insert("tools".to_string(), tools);
    }
    if let Some(choice) = tool_choice {
        body.insert("tool_choice".to_string(), choice);
    }
    if let Some(value) = object.get("temperature").filter(|value| value.is_number()) {
        body.insert("temperature".to_string(), value.clone());
    }
    if let Some(value) = object.get("top_p").filter(|value| value.is_number()) {
        body.insert("top_p".to_string(), value.clone());
    }
    if object.contains_key("parallel_tool_calls") {
        body.insert(
            "parallel_tool_calls".to_string(),
            Value::Bool(parallel_tool_calls),
        );
    }

    let prompt_chars = super::count_text_chars(input)
        + instructions
            .as_deref()
            .map(|value| value.chars().count())
            .unwrap_or(0)
        + object
            .get("tools")
            .map(super::count_text_chars)
            .unwrap_or(0);
    degraded.sort_unstable();
    degraded.dedup();

    Ok(ResponsesRequest {
        alias,
        is_stream,
        chat_body: Value::Object(body),
        prompt_chars,
        degraded,
        max_tokens,
        instructions,
        tools_echo,
        tool_choice_echo,
        parallel_tool_calls,
        store,
        metadata,
        temperature,
        top_p,
        previous_response_id,
    })
}

pub fn prepare_upstream_body(
    request: &ResponsesRequest,
    upstream_model: &str,
    capabilities: &[String],
) -> Result<Bytes, AppError> {
    let mut body = request.chat_body.clone();
    let object = body
        .as_object_mut()
        .ok_or_else(|| AppError::internal("responses chat body must be object"))?;
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
        .map_err(|error| AppError::internal(format!("serialize responses request: {error}")))
}

fn text_from_content(content: &Value, degraded: &mut Vec<&'static str>) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(items) => {
            let mut texts = Vec::new();
            for item in items {
                match item.get("type").and_then(Value::as_str) {
                    Some("input_text" | "output_text" | "text") => {
                        if let Some(text) = item.get("text").and_then(Value::as_str) {
                            texts.push(text.to_string());
                        }
                    }
                    Some("input_image" | "image_url") => degraded.push("input_image"),
                    _ => {}
                }
            }
            texts.join("\n")
        }
        _ => String::new(),
    }
}

fn push_assistant_tool_call(messages: &mut Vec<Value>, call: Value) {
    if let Some(array) = messages
        .last_mut()
        .filter(|last| {
            last.get("role").and_then(Value::as_str) == Some("assistant")
                && last.get("tool_calls").and_then(Value::as_array).is_some()
        })
        .and_then(|last| last.get_mut("tool_calls"))
        .and_then(Value::as_array_mut)
    {
        array.push(call);
        return;
    }
    messages.push(serde_json::json!({
        "role": "assistant",
        "content": Value::Null,
        "tool_calls": [call]
    }));
}

fn convert_input(
    input: &Value,
    messages: &mut Vec<Value>,
    degraded: &mut Vec<&'static str>,
) -> Result<(), AppError> {
    match input {
        Value::String(text) => {
            if text.is_empty() {
                return Err(AppError::validation_field("input", "input 不能为空"));
            }
            messages.push(serde_json::json!({"role": "user", "content": text}));
        }
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                let path = format!("input[{index}]");
                let Some(object) = item.as_object() else {
                    // 兼容数组内直接给字符串的宽泛形态。
                    if let Some(text) = item.as_str() {
                        messages.push(serde_json::json!({"role": "user", "content": text}));
                    }
                    continue;
                };
                match object.get("type").and_then(Value::as_str) {
                    Some("message") => {
                        let role = object.get("role").and_then(Value::as_str).unwrap_or("user");
                        let content = object.get("content").cloned().unwrap_or(Value::Null);
                        let text = text_from_content(&content, degraded);
                        let chat_role = match role {
                            "system" | "developer" => "system",
                            "assistant" => "assistant",
                            _ => "user",
                        };
                        if text.is_empty() && matches!(content, Value::Array(_)) {
                            // 纯非文本内容（图片等）忽略并降级。
                            continue;
                        }
                        messages.push(serde_json::json!({"role": chat_role, "content": text}));
                    }
                    Some("function_call") => {
                        let name = object
                            .get("name")
                            .and_then(Value::as_str)
                            .filter(|name| !name.is_empty())
                            .ok_or_else(|| {
                                super::validation_field(
                                    format!("{path}.name"),
                                    "function_call 缺少 name",
                                )
                            })?;
                        let arguments = match object.get("arguments") {
                            Some(Value::String(arguments)) => arguments.clone(),
                            Some(value) => serde_json::to_string(value).unwrap_or_default(),
                            None => "{}".to_string(),
                        };
                        if arguments.len() > MAX_TOOL_ARGUMENTS_BYTES {
                            return Err(super::validation_field(
                                format!("{path}.arguments"),
                                "function_call arguments 超过 2 MiB",
                            ));
                        }
                        let call_id = object
                            .get("call_id")
                            .and_then(Value::as_str)
                            .filter(|id| !id.is_empty())
                            .map(ToString::to_string)
                            .unwrap_or_else(|| format!("call_{}", Uuid::new_v4().simple()));
                        push_assistant_tool_call(
                            messages,
                            serde_json::json!({
                                "id": call_id,
                                "type": "function",
                                "function": {"name": name, "arguments": arguments}
                            }),
                        );
                    }
                    Some("function_call_output") => {
                        let call_id = object
                            .get("call_id")
                            .and_then(Value::as_str)
                            .filter(|id| !id.is_empty())
                            .ok_or_else(|| {
                                super::validation_field(
                                    format!("{path}.call_id"),
                                    "function_call_output 缺少 call_id",
                                )
                            })?;
                        let output = match object.get("output") {
                            Some(Value::String(output)) => output.clone(),
                            Some(value) => serde_json::to_string(value).unwrap_or_default(),
                            None => String::new(),
                        };
                        messages.push(serde_json::json!({
                            "role": "tool",
                            "tool_call_id": call_id,
                            "content": output
                        }));
                    }
                    Some("reasoning") => degraded.push("reasoning"),
                    _ => {}
                }
            }
        }
        _ => {
            return Err(AppError::validation_field(
                "input",
                "input 必须是字符串或数组",
            ));
        }
    }
    Ok(())
}

fn convert_tools(
    tools: &Value,
    degraded: &mut Vec<&'static str>,
) -> Result<(Option<Value>, Value), AppError> {
    let Some(items) = tools.as_array() else {
        return Err(AppError::validation_field("tools", "tools 必须是数组"));
    };
    let mut converted = Vec::new();
    for (index, tool) in items.iter().enumerate() {
        let Some(object) = tool.as_object() else {
            continue;
        };
        match object.get("type").and_then(Value::as_str) {
            Some("function") => {
                let name = object
                    .get("name")
                    .and_then(Value::as_str)
                    .or_else(|| {
                        object
                            .get("function")
                            .and_then(|function| function.get("name"))
                            .and_then(Value::as_str)
                    })
                    .filter(|name| !name.is_empty())
                    .ok_or_else(|| {
                        super::validation_field(format!("tools[{index}].name"), "缺少 name")
                    })?;
                let description = object
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let parameters = object
                    .get("parameters")
                    .filter(|value| value.is_object())
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({"type": "object"}));
                converted.push(serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": name,
                        "description": description,
                        "parameters": parameters
                    }
                }));
            }
            Some(other) => {
                // 内置工具（web_search/file_search/computer 等）当前不支持，忽略并提示。
                tracing::debug!(tool_type = other, "ignored unsupported responses tool");
                degraded.push("builtin_tools");
            }
            None => {}
        }
    }
    let echo = Value::Array(items.clone());
    Ok((
        (!converted.is_empty()).then_some(Value::Array(converted)),
        echo,
    ))
}

fn convert_tool_choice(
    choice: &Value,
    _degraded: &mut Vec<&'static str>,
) -> Result<(Option<Value>, Value), AppError> {
    if let Some(choice) = choice.as_str() {
        return match choice {
            "auto" | "none" | "required" => Ok((
                Some(Value::String(choice.to_string())),
                choice.to_string().into(),
            )),
            other => Err(AppError::validation_field(
                "tool_choice",
                format!("不支持的 tool_choice: {other}"),
            )),
        };
    }
    let Some(object) = choice.as_object() else {
        return Ok((None, choice.clone()));
    };
    match object.get("type").and_then(Value::as_str) {
        Some("function") => {
            let name = object
                .get("name")
                .and_then(Value::as_str)
                .or_else(|| {
                    object
                        .get("function")
                        .and_then(|function| function.get("name"))
                        .and_then(Value::as_str)
                })
                .ok_or_else(|| {
                    AppError::validation_field("tool_choice.name", "type=function 时缺少 name")
                })?;
            Ok((
                Some(serde_json::json!({"type": "function", "function": {"name": name}})),
                choice.clone(),
            ))
        }
        _ => Ok((None, choice.clone())),
    }
}

// ---------- 非流式响应 ----------

#[derive(Debug, Clone)]
pub struct ResponsesDecoded {
    pub body: Bytes,
    pub usage: UsageTokens,
    pub output_chars: usize,
    pub degraded: Vec<&'static str>,
}

pub fn decode_non_stream(
    raw: &[u8],
    request: &ResponsesRequest,
    request_id: &str,
) -> Result<ResponsesDecoded, AppError> {
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
    let mut output = Vec::new();
    let mut output_chars = 0usize;
    let text = match message.get("content") {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|part| part.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    };
    if !text.is_empty() || message.get("tool_calls").is_none() {
        output_chars += text.chars().count();
        output.push(output_message_item(&text));
    }
    if let Some(tool_calls) = message.get("tool_calls").and_then(Value::as_array) {
        for (index, tool_call) in tool_calls.iter().enumerate() {
            let function = tool_call.get("function");
            let name = function
                .and_then(|function| function.get("name"))
                .and_then(Value::as_str)
                .unwrap_or("unknown_tool")
                .to_string();
            let arguments = function
                .and_then(|function| function.get("arguments"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            if arguments.len() > MAX_TOOL_ARGUMENTS_BYTES {
                return Err(AppError::bad_gateway(
                    "upstream_protocol_error",
                    "上游工具参数超过 2 MiB",
                    None,
                ));
            }
            output_chars += arguments.chars().count();
            let call_id = tool_call
                .get("id")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(|| format!("call_{request_id}_{index}"));
            output.push(serde_json::json!({
                "id": format!("fc_{}", Uuid::new_v4().simple()),
                "type": "function_call",
                "status": "completed",
                "call_id": call_id,
                "name": name,
                "arguments": arguments
            }));
        }
    }

    let (status, incomplete_details) = match finish_reason.as_deref() {
        Some("length") => (
            "incomplete",
            serde_json::json!({"reason": "max_output_tokens"}),
        ),
        Some("content_filter") => {
            degraded.push("content_filter");
            (
                "incomplete",
                serde_json::json!({"reason": "content_filter"}),
            )
        }
        _ => ("completed", Value::Null),
    };

    let mut usage = parse_usage(object.get("usage"));
    if usage.input_tokens.is_none() {
        usage.input_tokens = Some(estimate_tokens(request.prompt_chars));
    }
    if usage.output_tokens.is_none() {
        usage.output_tokens = Some(estimate_tokens(output_chars));
    }
    degraded.sort_unstable();
    degraded.dedup();

    let response = response_object(
        request,
        request_id,
        status,
        incomplete_details,
        Value::Array(output),
        usage,
        None,
    );
    let bytes = serde_json::to_vec(&response)
        .map(Bytes::from)
        .map_err(|error| AppError::internal(format!("serialize responses response: {error}")))?;
    Ok(ResponsesDecoded {
        body: bytes,
        usage,
        output_chars,
        degraded,
    })
}

fn output_message_item(text: &str) -> Value {
    output_message_item_with_id(&format!("msg_{}", Uuid::new_v4().simple()), text)
}

fn output_message_item_with_id(item_id: &str, text: &str) -> Value {
    serde_json::json!({
        "id": item_id,
        "type": "message",
        "status": "completed",
        "role": "assistant",
        "content": [{"type": "output_text", "text": text, "annotations": []}]
    })
}

fn response_object(
    request: &ResponsesRequest,
    request_id: &str,
    status: &str,
    incomplete_details: Value,
    output: Value,
    usage: UsageTokens,
    reasoning: Option<Value>,
) -> Value {
    serde_json::json!({
        "id": format!("resp_{request_id}"),
        "object": "response",
        "created_at": chrono::Utc::now().timestamp(),
        "status": status,
        "error": Value::Null,
        "incomplete_details": incomplete_details,
        "model": request.alias,
        "instructions": request.instructions.clone().map(Value::String).unwrap_or(Value::Null),
        "max_output_tokens": request.max_tokens.map(|value| Value::Number(value.into())).unwrap_or(Value::Null),
        "output": output,
        "parallel_tool_calls": request.parallel_tool_calls,
        "previous_response_id": request.previous_response_id.clone().map(Value::String).unwrap_or(Value::Null),
        "reasoning": reasoning.unwrap_or_else(|| serde_json::json!({"effort": Value::Null, "summary": Value::Null})),
        "store": request.store,
        "temperature": request.temperature,
        "top_p": request.top_p,
        "text": {"format": {"type": "text"}},
        "tool_choice": request.tool_choice_echo,
        "tools": request.tools_echo,
        "truncation": "disabled",
        "usage": {
            "input_tokens": usage.input(),
            "input_tokens_details": {"cached_tokens": 0},
            "output_tokens": usage.output(),
            "output_tokens_details": {"reasoning_tokens": 0},
            "total_tokens": usage.input() + usage.output()
        },
        "user": Value::Null,
        "metadata": request.metadata
    })
}

// ---------- 流式事件 ----------

#[derive(Debug, Default)]
struct TextItem {
    item_id: String,
    output_index: usize,
    text: String,
    opened: bool,
    part_started: bool,
}

#[derive(Debug, Default)]
struct ToolItem {
    item_id: String,
    output_index: usize,
    call_id: String,
    name: String,
    arguments: String,
    added: bool,
}

#[derive(Debug)]
pub struct ResponsesStream {
    request: ResponsesRequest,
    request_id: String,
    seq: u64,
    started: bool,
    finished: bool,
    text: Option<TextItem>,
    tools: BTreeMap<u64, ToolItem>,
    tool_order: Vec<u64>,
    next_output_index: usize,
    finish_reason: Option<String>,
    usage: UsageTokens,
    output_items: Vec<Value>,
    completed_emitted: bool,
    error: Option<String>,
}

impl ResponsesStream {
    pub fn new(request: &ResponsesRequest, request_id: &str) -> Self {
        Self {
            request: request.clone(),
            request_id: request_id.to_string(),
            seq: 0,
            started: false,
            finished: false,
            text: None,
            tools: BTreeMap::new(),
            tool_order: Vec::new(),
            next_output_index: 0,
            finish_reason: None,
            usage: UsageTokens::default(),
            output_items: Vec::new(),
            completed_emitted: false,
            error: None,
        }
    }

    pub fn take_error(&mut self) -> Option<String> {
        self.error.take()
    }

    #[cfg(test)]
    pub fn is_done(&self) -> bool {
        self.finished
    }

    fn next_seq(&mut self) -> u64 {
        self.seq += 1;
        self.seq
    }

    fn frame(&mut self, event: &str, mut value: Value) -> Bytes {
        if let Some(object) = value.as_object_mut() {
            object.insert(
                "sequence_number".to_string(),
                Value::Number(self.next_seq().into()),
            );
        }
        crate::modules::proxy::sse::json_frame(Some(event), &value)
    }

    fn skeleton(&self, status: &str) -> Value {
        serde_json::json!({
            "id": format!("resp_{}", self.request_id),
            "object": "response",
            "created_at": chrono::Utc::now().timestamp(),
            "status": status,
            "error": Value::Null,
            "incomplete_details": Value::Null,
            "model": self.request.alias,
            "output": [],
            "usage": Value::Null
        })
    }

    fn ensure_started(&mut self) -> Vec<Bytes> {
        if self.started {
            return Vec::new();
        }
        self.started = true;
        let created = self.skeleton("in_progress");
        let mut frames = Vec::new();
        frames.push(self.frame(
            "response.created",
            serde_json::json!({"type": "response.created", "response": created}),
        ));
        let in_progress = self.skeleton("in_progress");
        frames.push(self.frame(
            "response.in_progress",
            serde_json::json!({"type": "response.in_progress", "response": in_progress}),
        ));
        frames
    }

    /// 处理一个上游 Chat SSE 事件，返回 Responses SSE 帧。
    pub fn handle_event(&mut self, event: &crate::modules::proxy::sse::SseEvent) -> Vec<Bytes> {
        if self.finished {
            return Vec::new();
        }
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
        let mut frames = self.ensure_started();
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
                    let index = tool_call.get("index").and_then(Value::as_u64).unwrap_or(0);
                    frames.extend(self.tool_delta(index, tool_call));
                }
            }
        }
        if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
            self.finish_reason = Some(reason.to_string());
        }
        frames
    }

    fn text_delta(&mut self, content: &str) -> Vec<Bytes> {
        let mut frames = Vec::new();
        if self.text.is_none() {
            let output_index = self.next_output_index;
            self.next_output_index += 1;
            self.text = Some(TextItem {
                item_id: format!("msg_{}", Uuid::new_v4().simple()),
                output_index,
                text: String::new(),
                opened: false,
                part_started: false,
            });
        }
        let (item_id, output_index, need_open, need_part) = {
            let Some(item) = self.text.as_mut() else {
                return frames;
            };
            let mut need_open = false;
            let mut need_part = false;
            if !item.opened {
                item.opened = true;
                need_open = true;
            }
            if !item.part_started {
                item.part_started = true;
                need_part = true;
            }
            (
                item.item_id.clone(),
                item.output_index,
                need_open,
                need_part,
            )
        };
        if need_open {
            frames.push(self.frame(
                "response.output_item.added",
                serde_json::json!({
                    "type": "response.output_item.added",
                    "output_index": output_index,
                    "item": {
                        "id": item_id,
                        "type": "message",
                        "status": "in_progress",
                        "role": "assistant",
                        "content": []
                    }
                }),
            ));
        }
        if need_part {
            frames.push(self.frame(
                "response.content_part.added",
                serde_json::json!({
                    "type": "response.content_part.added",
                    "item_id": item_id,
                    "output_index": output_index,
                    "content_index": 0,
                    "part": {"type": "output_text", "text": "", "annotations": []}
                }),
            ));
        }
        let too_large = {
            let Some(item) = self.text.as_mut() else {
                return frames;
            };
            if item.text.len() + content.len() > MAX_ACCUMULATED_TEXT_BYTES {
                true
            } else {
                item.text.push_str(content);
                false
            }
        };
        if too_large {
            self.error = Some("响应文本超过网关上限".to_string());
            return frames;
        }
        frames.push(self.frame(
            "response.output_text.delta",
            serde_json::json!({
                "type": "response.output_text.delta",
                "item_id": item_id,
                "output_index": output_index,
                "content_index": 0,
                "delta": content
            }),
        ));
        frames
    }

    fn tool_delta(&mut self, index: u64, tool_call: &Value) -> Vec<Bytes> {
        let mut frames = Vec::new();
        if !self.tools.contains_key(&index) {
            let output_index = self.next_output_index;
            self.next_output_index += 1;
            self.tool_order.push(index);
            self.tools.insert(
                index,
                ToolItem {
                    item_id: format!("fc_{}", Uuid::new_v4().simple()),
                    output_index,
                    call_id: format!("call_{}", Uuid::new_v4().simple()),
                    name: "unknown_tool".to_string(),
                    arguments: String::new(),
                    added: false,
                },
            );
        }
        let (item_id, output_index, call_id, name, need_added, argument_delta, too_large) = {
            let Some(item) = self.tools.get_mut(&index) else {
                return frames;
            };
            if let Some(id) = tool_call
                .get("id")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
            {
                item.call_id = id.to_string();
            }
            let mut need_added = false;
            let mut argument_delta = None;
            let mut too_large = false;
            if let Some(function) = tool_call.get("function") {
                if let Some(name) = function
                    .get("name")
                    .and_then(Value::as_str)
                    .filter(|name| !name.is_empty())
                {
                    item.name = name.to_string();
                }
                if !item.added {
                    item.added = true;
                    need_added = true;
                }
                if let Some(arguments) = function.get("arguments").and_then(Value::as_str)
                    && !arguments.is_empty()
                {
                    if item.arguments.len() + arguments.len() > MAX_TOOL_ARGUMENTS_BYTES {
                        too_large = true;
                    } else {
                        item.arguments.push_str(arguments);
                        argument_delta = Some(arguments.to_string());
                    }
                }
            }
            (
                item.item_id.clone(),
                item.output_index,
                item.call_id.clone(),
                item.name.clone(),
                need_added,
                argument_delta,
                too_large,
            )
        };
        if too_large {
            self.error = Some("工具参数超过网关上限".to_string());
            return frames;
        }
        if need_added {
            frames.push(self.frame(
                "response.output_item.added",
                serde_json::json!({
                    "type": "response.output_item.added",
                    "output_index": output_index,
                    "item": {
                        "id": item_id,
                        "type": "function_call",
                        "status": "in_progress",
                        "call_id": call_id,
                        "name": name,
                        "arguments": ""
                    }
                }),
            ));
        }
        if let Some(delta) = argument_delta {
            frames.push(self.frame(
                "response.function_call_arguments.delta",
                serde_json::json!({
                    "type": "response.function_call_arguments.delta",
                    "item_id": item_id,
                    "output_index": output_index,
                    "delta": delta
                }),
            ));
        }
        frames
    }

    /// 流结束（[DONE]/EOF）：关闭 items 并发送 response.completed（不发 [DONE]）。
    pub fn finish(&mut self) -> Vec<Bytes> {
        if self.finished || self.completed_emitted {
            return Vec::new();
        }
        let mut frames = self.ensure_started();
        let mut completed_items: Vec<(usize, Value)> = Vec::new();
        // 关闭文本 item。
        if let Some(mut item) = self.text.take()
            && item.opened
        {
            let item_id = item.item_id.clone();
            let output_index = item.output_index;
            let text = std::mem::take(&mut item.text);
            frames.push(self.frame(
                "response.output_text.done",
                serde_json::json!({
                    "type": "response.output_text.done",
                    "item_id": item.item_id,
                    "output_index": item.output_index,
                    "content_index": 0,
                    "text": text
                }),
            ));
            frames.push(self.frame(
                "response.content_part.done",
                serde_json::json!({
                    "type": "response.content_part.done",
                    "item_id": item.item_id,
                    "output_index": item.output_index,
                    "content_index": 0,
                    "part": {"type": "output_text", "text": text, "annotations": []}
                }),
            ));
            let output_item = output_message_item_with_id(&item_id, &text);
            frames.push(self.frame(
                "response.output_item.done",
                serde_json::json!({
                    "type": "response.output_item.done",
                    "output_index": output_index,
                    "item": output_item
                }),
            ));
            completed_items.push((output_index, output_item));
        }
        // 关闭工具 items。
        for index in self.tool_order.clone() {
            let Some(item) = self.tools.remove(&index) else {
                continue;
            };
            if !item.added {
                continue;
            }
            frames.push(self.frame(
                "response.function_call_arguments.done",
                serde_json::json!({
                    "type": "response.function_call_arguments.done",
                    "item_id": item.item_id,
                    "output_index": item.output_index,
                    "arguments": item.arguments
                }),
            ));
            let output_item = serde_json::json!({
                "id": item.item_id,
                "type": "function_call",
                "status": "completed",
                "call_id": item.call_id,
                "name": item.name,
                "arguments": item.arguments
            });
            frames.push(self.frame(
                "response.output_item.done",
                serde_json::json!({
                    "type": "response.output_item.done",
                    "output_index": item.output_index,
                    "item": output_item
                }),
            ));
            completed_items.push((item.output_index, output_item));
        }
        completed_items.sort_by_key(|(output_index, _)| *output_index);
        self.output_items = completed_items.into_iter().map(|(_, item)| item).collect();
        let finish_reason = self.finish_reason.clone();
        let (status, incomplete_details) = match finish_reason.as_deref() {
            Some("length") => (
                "incomplete",
                serde_json::json!({"reason": "max_output_tokens"}),
            ),
            Some("content_filter") => (
                "incomplete",
                serde_json::json!({"reason": "content_filter"}),
            ),
            _ => ("completed", Value::Null),
        };
        if self.usage.input_tokens.is_none() {
            self.usage.input_tokens = Some(estimate_tokens(self.request.prompt_chars));
        }
        if self.usage.output_tokens.is_none() {
            let chars: usize = self
                .output_items
                .iter()
                .map(|item| match item.get("type").and_then(Value::as_str) {
                    Some("message") => item
                        .get("content")
                        .and_then(Value::as_array)
                        .and_then(|parts| parts.first())
                        .and_then(|part| part.get("text"))
                        .and_then(Value::as_str)
                        .map(|text| text.chars().count())
                        .unwrap_or(0),
                    Some("function_call") => item
                        .get("arguments")
                        .and_then(Value::as_str)
                        .map(|arguments| arguments.chars().count())
                        .unwrap_or(0),
                    _ => 0,
                })
                .sum();
            self.usage.output_tokens = Some(estimate_tokens(chars));
        }
        let response = response_object(
            &self.request,
            &self.request_id,
            status,
            incomplete_details,
            Value::Array(std::mem::take(&mut self.output_items)),
            self.usage,
            None,
        );
        frames.push(self.frame(
            "response.completed",
            serde_json::json!({"type": "response.completed", "response": response}),
        ));
        self.completed_emitted = true;
        self.finished = true;
        frames
    }

    pub fn usage(&self) -> UsageTokens {
        self.usage
    }

    pub fn output_tokens_estimate(&self) -> i64 {
        self.usage
            .output_tokens
            .unwrap_or_else(|| estimate_tokens(self.output_chars()))
    }

    fn output_chars(&self) -> usize {
        self.text
            .as_ref()
            .map(|item| item.text.chars().count())
            .unwrap_or(0)
            + self
                .tools
                .values()
                .map(|tool| tool.arguments.chars().count())
                .sum::<usize>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(raw: &str) -> ResponsesRequest {
        parse_request(raw.as_bytes()).expect("responses request parses")
    }

    #[test]
    fn parses_string_input_and_instructions() {
        let request = parse(
            r#"{"model":"codex-model","input":"hello","instructions":"be nice","max_output_tokens":512,"stream":false}"#,
        );
        assert_eq!(request.alias, "codex-model");
        let messages = request.chat_body["messages"].as_array().expect("messages");
        assert_eq!(messages[0]["role"], "system");
        assert_eq!(messages[0]["content"], "be nice");
        assert_eq!(messages[1]["role"], "user");
        assert_eq!(messages[1]["content"], "hello");
        assert_eq!(request.max_tokens, Some(512));
    }

    #[test]
    fn parses_messages_tools_and_function_roundtrip() {
        let request = parse(
            r#"{"model":"m","input":[{"type":"message","role":"user","content":[{"type":"input_text","text":"list files"}]},{"type":"function_call","name":"Bash","arguments":"{\"cmd\":\"ls\"}","call_id":"call_1"},{"type":"function_call_output","call_id":"call_1","output":"a.txt"}],"tools":[{"type":"function","name":"Bash","description":"run","parameters":{"type":"object"}}],"tool_choice":"auto"}"#,
        );
        let messages = request.chat_body["messages"].as_array().expect("messages");
        assert_eq!(messages[0]["content"], "list files");
        assert_eq!(messages[1]["role"], "assistant");
        assert_eq!(messages[1]["tool_calls"][0]["id"], "call_1");
        assert_eq!(messages[2]["role"], "tool");
        assert_eq!(messages[2]["tool_call_id"], "call_1");
        assert_eq!(request.chat_body["tools"][0]["function"]["name"], "Bash");
        assert_eq!(request.chat_body["tool_choice"], "auto");
    }

    #[test]
    fn rejects_missing_input_and_bad_previous_response_id() {
        assert!(parse_request(br#"{"model":"m"}"#).is_err());
        assert!(matches!(
            parse_request(br#"{"model":"m","previous_response_id":"resp_1"}"#),
            Err(AppError::BadRequest {
                code: "unsupported_parameter",
                ..
            })
        ));
        assert!(parse_request(br#"{"model":"m","input":""}"#).is_err());
    }

    #[test]
    fn decodes_non_stream_text_and_usage() {
        let request = parse(r#"{"model":"gpt-5-codex","input":"hi"}"#);
        let raw = br#"{"id":"x","choices":[{"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":2}}"#;
        let decoded = decode_non_stream(raw, &request, "req-1").expect("decode");
        let value: Value = serde_json::from_slice(&decoded.body).expect("json");
        assert_eq!(value["object"], "response");
        assert_eq!(value["status"], "completed");
        assert_eq!(value["output"][0]["type"], "message");
        assert_eq!(value["output"][0]["content"][0]["text"], "ok");
        assert_eq!(value["usage"]["input_tokens"], 3);
        assert_eq!(value["usage"]["output_tokens"], 2);
    }

    #[test]
    fn decodes_non_stream_tool_calls() {
        let request = parse(
            r#"{"model":"m","input":"hi","tools":[{"type":"function","name":"Bash","parameters":{"type":"object"}}]}"#,
        );
        let raw = br#"{"choices":[{"message":{"role":"assistant","content":null,"tool_calls":[{"id":"call_1","type":"function","function":{"name":"Bash","arguments":"{\"cmd\":\"ls\"}"}}]},"finish_reason":"tool_calls"}]}"#;
        let decoded = decode_non_stream(raw, &request, "req-2").expect("decode");
        let value: Value = serde_json::from_slice(&decoded.body).expect("json");
        assert_eq!(value["status"], "completed");
        assert_eq!(value["output"][0]["type"], "function_call");
        assert_eq!(value["output"][0]["call_id"], "call_1");
        assert_eq!(value["output"][0]["name"], "Bash");
    }

    #[test]
    fn stream_emits_responses_event_sequence_and_no_done() {
        let request = parse(r#"{"model":"m","input":"hi","stream":true}"#);
        let mut stream = ResponsesStream::new(&request, "req-3");
        let mut frames = Vec::new();
        for data in [
            r#"{"choices":[{"delta":{"role":"assistant","content":"你"},"finish_reason":null}]}"#,
            r#"{"choices":[{"delta":{"content":"好"},"finish_reason":"stop"}]}"#,
            r#"{"choices":[{"delta":{},"finish_reason":null}],"usage":{"prompt_tokens":3,"completion_tokens":2}}"#,
        ] {
            frames.extend(stream.handle_event(&crate::modules::proxy::sse::SseEvent {
                event: None,
                data: data.to_string(),
            }));
        }
        frames.extend(stream.finish());
        let text = String::from_utf8_lossy(&frames.concat()).to_string();
        assert!(text.contains("event: response.created"));
        assert!(text.contains("event: response.in_progress"));
        assert!(text.contains("event: response.output_item.added"));
        assert!(text.contains("event: response.content_part.added"));
        assert!(text.contains("event: response.output_text.delta"));
        assert!(text.contains("event: response.output_text.done"));
        assert!(text.contains("event: response.content_part.done"));
        assert!(text.contains("event: response.output_item.done"));
        assert!(text.contains("event: response.completed"));
        assert!(!text.contains("data: [DONE]"));
        assert!(stream.is_done());
    }

    #[test]
    fn stream_emits_function_call_events() {
        let request = parse(
            r#"{"model":"m","input":"hi","stream":true,"tools":[{"type":"function","name":"Bash","parameters":{"type":"object"}}]}"#,
        );
        let mut stream = ResponsesStream::new(&request, "req-4");
        let mut frames = Vec::new();
        for data in [
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"Bash","arguments":"{\"cmd\":"}}]},"finish_reason":null}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"ls\"}"}}]},"finish_reason":null}]}"#,
            r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
        ] {
            frames.extend(stream.handle_event(&crate::modules::proxy::sse::SseEvent {
                event: None,
                data: data.to_string(),
            }));
        }
        frames.extend(stream.finish());
        let text = String::from_utf8_lossy(&frames.concat()).to_string();
        assert!(text.contains("\"type\":\"function_call\""));
        assert!(text.contains("response.function_call_arguments.delta"));
        assert!(text.contains("response.function_call_arguments.done"));
        assert!(text.contains("response.completed"));
    }
}
