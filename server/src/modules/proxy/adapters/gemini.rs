//! Gemini 前端（`generateContent` / `streamGenerateContent?alt=sse`）↔ Chat 上游适配器
//! （`gemini_to_chat`，US-015 / Gemini CLI）。
//!
//! 范围（proxy Gemini 实现设计）：contents（user/model、text/functionCall/functionResponse）、
//! systemInstruction、functionDeclarations、toolConfig、generationConfig、safetySettings；
//! 图片/文件 400 `unsupported_content`；不支持字段显式降级（x-gw-degraded）。
//! 纯同步：请求转换、非流式响应转换、SSE 状态机。

use std::collections::BTreeMap;

use bytes::Bytes;
use serde_json::{Map, Value};
use uuid::Uuid;

use crate::platform::error::AppError;

use super::{UsageTokens, estimate_tokens, parse_usage, top_level_error, validate_model};

pub const ADAPTER_ID: &str = "gemini_to_chat";
pub const MAX_TOOL_ARGUMENTS_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_ACCUMULATED_TEXT_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct GeminiRequest {
    pub alias: String,
    pub is_stream: bool,
    pub chat_body: Value,
    pub prompt_chars: usize,
    pub degraded: Vec<&'static str>,
    pub max_tokens: Option<i64>,
}

impl GeminiRequest {
    pub fn input_tokens_estimate(&self) -> i64 {
        estimate_tokens(self.prompt_chars)
    }
}

pub fn parse_request(raw: &[u8], model: &str, is_stream: bool) -> Result<GeminiRequest, AppError> {
    let alias = validate_model(model)?;
    let value = super::parse_json_body(raw)?;
    let object = super::require_object(&value, "body")?;

    let mut degraded = Vec::new();
    let contents = object
        .get("contents")
        .and_then(Value::as_array)
        .ok_or_else(|| super::validation_field("contents", "缺少必填字段 contents"))?;
    if contents.is_empty() {
        return Err(super::validation_field("contents", "contents 不能为空"));
    }

    let mut messages: Vec<Value> = Vec::new();
    let system = object
        .get("systemInstruction")
        .map(|system| system_instruction_text(system, &mut degraded))
        .transpose()?
        .flatten();
    if let Some(system) = system {
        messages.push(serde_json::json!({"role": "system", "content": system}));
    }

    let mut call_ids: BTreeMap<String, String> = BTreeMap::new();
    for (index, content) in contents.iter().enumerate() {
        let path = format!("contents[{index}]");
        let content = content
            .as_object()
            .ok_or_else(|| super::validation_field(&path, "content 必须是 object"))?;
        let role = content
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("user");
        let parts = content
            .get("parts")
            .and_then(Value::as_array)
            .ok_or_else(|| super::validation_field(format!("{path}.parts"), "缺少 parts 数组"))?;
        match role {
            "model" => {
                let mut texts = Vec::new();
                let mut tool_calls = Vec::new();
                for (part_index, part) in parts.iter().enumerate() {
                    let part_path = format!("{path}.parts[{part_index}]");
                    let part = part
                        .as_object()
                        .ok_or_else(|| super::validation_field(&part_path, "part 必须是 object"))?;
                    if let Some(text) = part.get("text").and_then(Value::as_str) {
                        texts.push(text.to_string());
                    } else if let Some(function_call) = part.get("functionCall") {
                        let function_call = function_call.as_object().ok_or_else(|| {
                            super::validation_field(
                                format!("{part_path}.functionCall"),
                                "functionCall 必须是 object",
                            )
                        })?;
                        let name = function_call
                            .get("name")
                            .and_then(Value::as_str)
                            .filter(|name| !name.is_empty())
                            .ok_or_else(|| {
                                super::validation_field(
                                    format!("{part_path}.functionCall.name"),
                                    "functionCall 缺少 name",
                                )
                            })?;
                        let args = function_call
                            .get("args")
                            .cloned()
                            .unwrap_or_else(|| serde_json::json!({}));
                        let arguments = serde_json::to_string(&args).map_err(|error| {
                            AppError::internal(format!("serialize functionCall args: {error}"))
                        })?;
                        if arguments.len() > MAX_TOOL_ARGUMENTS_BYTES {
                            return Err(super::validation_field(
                                format!("{part_path}.functionCall.args"),
                                "functionCall args 超过 2 MiB",
                            ));
                        }
                        let call_id = format!("call_{}", Uuid::new_v4().simple());
                        call_ids.insert(name.to_string(), call_id.clone());
                        tool_calls.push(serde_json::json!({
                            "id": call_id,
                            "type": "function",
                            "function": {"name": name, "arguments": arguments}
                        }));
                    } else if part.get("inlineData").is_some()
                        || part.get("fileData").is_some()
                        || part.get("inline_data").is_some()
                        || part.get("file_data").is_some()
                    {
                        return Err(AppError::bad_request(
                            "unsupported_content",
                            "Gemini 内联图片/文件暂不支持（S4 仅文本与函数调用）",
                        ));
                    } else {
                        degraded.push("unsupported_part");
                    }
                }
                let text = texts.join("\n");
                let mut message = Map::new();
                message.insert("role".to_string(), Value::String("assistant".to_string()));
                message.insert(
                    "content".to_string(),
                    if text.is_empty() {
                        Value::Null
                    } else {
                        Value::String(text)
                    },
                );
                if !tool_calls.is_empty() {
                    message.insert("tool_calls".to_string(), Value::Array(tool_calls));
                }
                if message.get("content") == Some(&Value::Null)
                    && !message.contains_key("tool_calls")
                {
                    continue;
                }
                messages.push(Value::Object(message));
            }
            "user" => {
                let mut tool_messages = Vec::new();
                let mut texts = Vec::new();
                for (part_index, part) in parts.iter().enumerate() {
                    let part_path = format!("{path}.parts[{part_index}]");
                    let part = part
                        .as_object()
                        .ok_or_else(|| super::validation_field(&part_path, "part 必须是 object"))?;
                    if let Some(text) = part.get("text").and_then(Value::as_str) {
                        texts.push(text.to_string());
                    } else if let Some(function_response) = part.get("functionResponse") {
                        let function_response = function_response.as_object().ok_or_else(|| {
                            super::validation_field(
                                format!("{part_path}.functionResponse"),
                                "functionResponse 必须是 object",
                            )
                        })?;
                        let name = function_response
                            .get("name")
                            .and_then(Value::as_str)
                            .filter(|name| !name.is_empty())
                            .unwrap_or("unknown_tool");
                        let call_id = function_response
                            .get("id")
                            .and_then(Value::as_str)
                            .filter(|id| !id.is_empty())
                            .map(ToString::to_string)
                            .or_else(|| call_ids.get(name).cloned())
                            .unwrap_or_else(|| {
                                degraded.push("function_response_without_call");
                                format!("call_{}", Uuid::new_v4().simple())
                            });
                        let output = match function_response.get("response") {
                            Some(Value::String(text)) => text.clone(),
                            Some(response) => serde_json::to_string(response).unwrap_or_default(),
                            None => String::new(),
                        };
                        tool_messages.push(serde_json::json!({
                            "role": "tool",
                            "tool_call_id": call_id,
                            "content": output
                        }));
                    } else if part.get("inlineData").is_some()
                        || part.get("fileData").is_some()
                        || part.get("inline_data").is_some()
                        || part.get("file_data").is_some()
                    {
                        return Err(AppError::bad_request(
                            "unsupported_content",
                            "Gemini 内联图片/文件暂不支持（S4 仅文本与函数调用）",
                        ));
                    } else {
                        degraded.push("unsupported_part");
                    }
                }
                messages.extend(tool_messages);
                if !texts.is_empty() {
                    messages.push(serde_json::json!({"role": "user", "content": texts.join("\n")}));
                }
            }
            other => {
                return Err(super::validation_field(
                    format!("{path}.role"),
                    format!("不支持的 role: {other}（仅 user/model）"),
                ));
            }
        }
    }
    if messages.is_empty() {
        return Err(super::validation_field("contents", "contents 不能为空"));
    }

    let mut body = Map::new();
    body.insert("model".to_string(), Value::String(alias.clone()));
    body.insert("messages".to_string(), Value::Array(messages));
    body.insert("stream".to_string(), Value::Bool(is_stream));

    let max_tokens = parse_generation(object, &mut degraded, &mut body)?;
    let tools = object
        .get("tools")
        .map(|tools| parse_tools(tools, &mut degraded))
        .transpose()?
        .flatten();
    if let Some(tools) = tools {
        body.insert("tools".to_string(), tools);
    }
    if let Some(choice) = parse_tool_config(object.get("toolConfig"), &mut degraded)? {
        body.insert("tool_choice".to_string(), choice);
    }
    if object.contains_key("safetySettings") {
        degraded.push("safety_settings");
    }

    let prompt_chars = super::count_text_chars(&value);
    degraded.sort_unstable();
    degraded.dedup();
    Ok(GeminiRequest {
        alias,
        is_stream,
        chat_body: Value::Object(body),
        prompt_chars,
        degraded,
        max_tokens,
    })
}

pub fn prepare_upstream_body(
    request: &GeminiRequest,
    upstream_model: &str,
    capabilities: &[String],
) -> Result<Bytes, AppError> {
    let mut body = request.chat_body.clone();
    let object = body
        .as_object_mut()
        .ok_or_else(|| AppError::internal("gemini chat body must be object"))?;
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
        .map_err(|error| AppError::internal(format!("serialize gemini request: {error}")))
}

fn system_instruction_text(
    system: &Value,
    degraded: &mut Vec<&'static str>,
) -> Result<Option<String>, AppError> {
    let Some(object) = system.as_object() else {
        return Err(super::validation_field(
            "systemInstruction",
            "systemInstruction 必须是 object",
        ));
    };
    let Some(parts) = object.get("parts").and_then(Value::as_array) else {
        return Ok(None);
    };
    let mut texts = Vec::new();
    for part in parts.iter() {
        let Some(part) = part.as_object() else {
            continue;
        };
        if let Some(text) = part.get("text").and_then(Value::as_str) {
            texts.push(text.to_string());
        } else {
            degraded.push("system_instruction_non_text");
        }
    }
    if texts.is_empty() {
        Ok(None)
    } else {
        Ok(Some(texts.join("\n")))
    }
}

fn parse_generation(
    object: &Map<String, Value>,
    degraded: &mut Vec<&'static str>,
    body: &mut Map<String, Value>,
) -> Result<Option<i64>, AppError> {
    let Some(generation) = object.get("generationConfig") else {
        return Ok(None);
    };
    let generation = generation
        .as_object()
        .ok_or_else(|| super::validation_field("generationConfig", "必须是 object"))?;

    for field in [
        "topK",
        "top_k",
        "responseMimeType",
        "responseSchema",
        "thinkingConfig",
        "presencePenalty",
        "frequencyPenalty",
        "seed",
    ] {
        if generation.contains_key(field) {
            degraded.push(match field {
                "topK" | "top_k" => "top_k",
                "responseMimeType" => "response_mime_type",
                "responseSchema" => "response_schema",
                "thinkingConfig" => "thinking",
                "presencePenalty" => "presence_penalty",
                "frequencyPenalty" => "frequency_penalty",
                _ => "seed",
            });
        }
    }
    if let Some(candidate_count) = generation.get("candidateCount").and_then(Value::as_i64)
        && candidate_count > 1
    {
        return Err(AppError::bad_request(
            "unsupported_parameter",
            "candidateCount > 1 暂不支持",
        ));
    }
    let max_tokens = match generation.get("maxOutputTokens") {
        None | Some(Value::Null) => None,
        Some(value) => {
            let tokens = value.as_i64().ok_or_else(|| {
                super::validation_field("generationConfig.maxOutputTokens", "必须是整数")
            })?;
            if !(1..=1_000_000).contains(&tokens) {
                return Err(super::validation_field(
                    "generationConfig.maxOutputTokens",
                    "必须在 1–1,000,000 之间",
                ));
            }
            Some(tokens)
        }
    };
    if let Some(tokens) = max_tokens {
        body.insert("max_tokens".to_string(), Value::Number(tokens.into()));
    }
    if let Some(temperature) = generation
        .get("temperature")
        .filter(|value| value.is_number())
    {
        body.insert("temperature".to_string(), temperature.clone());
    }
    if let Some(top_p) = generation.get("topP").filter(|value| value.is_number()) {
        body.insert("top_p".to_string(), top_p.clone());
    }
    if let Some(stop) = generation.get("stopSequences").and_then(Value::as_array) {
        let sequences: Vec<Value> = stop
            .iter()
            .filter_map(|value| value.as_str().map(|text| Value::String(text.to_string())))
            .collect();
        if !sequences.is_empty() {
            body.insert("stop".to_string(), Value::Array(sequences));
        }
    }

    Ok(max_tokens)
}

fn parse_tools(tools: &Value, degraded: &mut Vec<&'static str>) -> Result<Option<Value>, AppError> {
    let Some(items) = tools.as_array() else {
        return Err(super::validation_field("tools", "tools 必须是数组"));
    };
    let mut converted = Vec::new();
    for (tool_index, tool) in items.iter().enumerate() {
        let Some(tool) = tool.as_object() else {
            continue;
        };
        let Some(declarations) = tool.get("functionDeclarations").and_then(Value::as_array) else {
            // 内置工具（googleSearch/urlContext/codeExecution 等）暂不支持。
            degraded.push("builtin_tools");
            continue;
        };
        for (index, declaration) in declarations.iter().enumerate() {
            let path = format!("tools[{tool_index}].functionDeclarations[{index}]");
            let Some(declaration) = declaration.as_object() else {
                continue;
            };
            let name = declaration
                .get("name")
                .and_then(Value::as_str)
                .filter(|name| !name.is_empty())
                .ok_or_else(|| super::validation_field(&path, "缺少 name"))?;
            let description = declaration
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let parameters = declaration
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
    }
    Ok((!converted.is_empty()).then_some(Value::Array(converted)))
}

fn parse_tool_config(
    config: Option<&Value>,
    _degraded: &mut Vec<&'static str>,
) -> Result<Option<Value>, AppError> {
    let Some(config) = config else {
        return Ok(None);
    };
    let Some(config) = config.as_object() else {
        return Ok(None);
    };
    let Some(calling_config) = config
        .get("functionCallingConfig")
        .and_then(Value::as_object)
    else {
        return Ok(None);
    };
    let Some(mode) = calling_config.get("mode").and_then(Value::as_str) else {
        return Ok(None);
    };
    match mode {
        "AUTO" => Ok(Some(Value::String("auto".to_string()))),
        "ANY" => Ok(Some(Value::String("required".to_string()))),
        "NONE" => Ok(Some(Value::String("none".to_string()))),
        other => Err(super::validation_field(
            "toolConfig.functionCallingConfig.mode",
            format!("不支持的 mode: {other}"),
        )),
    }
}

// ---------- 非流式响应 ----------

#[derive(Debug, Clone)]
pub struct GeminiDecoded {
    pub body: Bytes,
    pub usage: UsageTokens,
    pub output_chars: usize,
    pub degraded: Vec<&'static str>,
}

pub fn decode_non_stream(
    raw: &[u8],
    request: &GeminiRequest,
    _request_id: &str,
) -> Result<GeminiDecoded, AppError> {
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
    let mut parts = Vec::new();
    let mut output_chars = 0usize;
    if let Some(text) = message.get("content").and_then(Value::as_str)
        && !text.is_empty()
    {
        output_chars += text.chars().count();
        parts.push(serde_json::json!({"text": text}));
    }
    if let Some(tool_calls) = message.get("tool_calls").and_then(Value::as_array) {
        for tool_call in tool_calls.iter() {
            let function = tool_call.get("function");
            let name = function
                .and_then(|function| function.get("name"))
                .and_then(Value::as_str)
                .unwrap_or("unknown_tool");
            let arguments = function
                .and_then(|function| function.get("arguments"))
                .and_then(Value::as_str)
                .unwrap_or("{}");
            if arguments.len() > MAX_TOOL_ARGUMENTS_BYTES {
                return Err(AppError::bad_gateway(
                    "upstream_protocol_error",
                    "上游工具参数超过 2 MiB",
                    None,
                ));
            }
            output_chars += arguments.chars().count();
            let args = serde_json::from_str::<Value>(arguments).unwrap_or_else(|_| {
                degraded.push("tool_arguments_invalid");
                serde_json::json!({})
            });
            parts.push(serde_json::json!({
                "functionCall": {"name": name, "args": args}
            }));
        }
    }
    let finish = map_finish_reason(finish_reason.as_deref(), &mut degraded);
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
        "candidates": [{
            "content": {"role": "model", "parts": parts},
            "finishReason": finish,
            "index": 0
        }],
        "usageMetadata": {
            "promptTokenCount": usage.input(),
            "candidatesTokenCount": usage.output(),
            "totalTokenCount": usage.input() + usage.output()
        },
        "modelVersion": request.alias
    });
    let bytes = serde_json::to_vec(&body)
        .map(Bytes::from)
        .map_err(|error| AppError::internal(format!("serialize gemini response: {error}")))?;
    Ok(GeminiDecoded {
        body: bytes,
        usage,
        output_chars,
        degraded,
    })
}

fn map_finish_reason(
    finish_reason: Option<&str>,
    degraded: &mut Vec<&'static str>,
) -> &'static str {
    match finish_reason {
        Some("length") => "MAX_TOKENS",
        Some("content_filter") => {
            degraded.push("content_filter");
            "SAFETY"
        }
        _ => "STOP",
    }
}

// ---------- 流式 ----------

#[derive(Debug, Default)]
struct ToolState {
    name: String,
    arguments: String,
}

#[derive(Debug)]
pub struct GeminiStream {
    request: GeminiRequest,
    seq: u64,
    finish_reason: Option<String>,
    usage: UsageTokens,
    text_chars: usize,
    tools: BTreeMap<u64, ToolState>,
    tool_order: Vec<u64>,
    finished: bool,
    error: Option<String>,
}

impl GeminiStream {
    pub fn new(request: &GeminiRequest) -> Self {
        Self {
            request: request.clone(),
            seq: 0,
            finish_reason: None,
            usage: UsageTokens::default(),
            text_chars: 0,
            tools: BTreeMap::new(),
            tool_order: Vec::new(),
            finished: false,
            error: None,
        }
    }

    pub fn take_error(&mut self) -> Option<String> {
        self.error.take()
    }

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
        let mut frames = Vec::new();
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
                if self.text_chars + content.chars().count() > MAX_ACCUMULATED_TEXT_BYTES {
                    self.error = Some("响应文本超过网关上限".to_string());
                    return frames;
                }
                self.text_chars += content.chars().count();
                frames.push(self.data_chunk(serde_json::json!({
                    "candidates": [{
                        "content": {"role": "model", "parts": [{"text": content}]},
                        "index": 0
                    }]
                })));
            }
            if let Some(tool_calls) = delta.get("tool_calls").and_then(Value::as_array) {
                for tool_call in tool_calls {
                    let index = tool_call.get("index").and_then(Value::as_u64).unwrap_or(0);
                    if !self.tools.contains_key(&index) {
                        self.tool_order.push(index);
                        self.tools.insert(index, ToolState::default());
                    }
                    if let Some(state) = self.tools.get_mut(&index)
                        && let Some(function) = tool_call.get("function")
                    {
                        if let Some(name) = function
                            .get("name")
                            .and_then(Value::as_str)
                            .filter(|name| !name.is_empty())
                        {
                            state.name = name.to_string();
                        }
                        if let Some(arguments) = function.get("arguments").and_then(Value::as_str)
                            && !arguments.is_empty()
                        {
                            if state.arguments.len() + arguments.len() > MAX_TOOL_ARGUMENTS_BYTES {
                                self.error = Some("工具参数超过网关上限".to_string());
                                return frames;
                            }
                            state.arguments.push_str(arguments);
                        }
                    }
                }
            }
        }
        if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
            self.finish_reason = Some(reason.to_string());
        }
        frames
    }

    fn data_chunk(&mut self, value: Value) -> Bytes {
        self.seq += 1;
        crate::modules::proxy::sse::json_frame(None, &value)
    }

    /// 流结束：先输出最终 functionCall（如有），再输出结束 chunk（finishReason + usageMetadata）。
    pub fn finish(&mut self) -> Vec<Bytes> {
        if self.finished {
            return Vec::new();
        }
        self.finished = true;
        let mut frames = Vec::new();
        if !self.tool_order.is_empty() {
            let mut parts = Vec::new();
            for index in self.tool_order.clone() {
                let Some(tool) = self.tools.get(&index) else {
                    continue;
                };
                if tool.name.is_empty() {
                    continue;
                }
                let args = serde_json::from_str::<Value>(&tool.arguments)
                    .unwrap_or_else(|_| serde_json::json!({}));
                parts.push(serde_json::json!({"functionCall": {"name": tool.name, "args": args}}));
            }
            if !parts.is_empty() {
                frames.push(self.data_chunk(serde_json::json!({
                    "candidates": [{
                        "content": {"role": "model", "parts": parts},
                        "finishReason": "STOP",
                        "index": 0
                    }]
                })));
            }
        }
        if self.usage.input_tokens.is_none() {
            self.usage.input_tokens = Some(estimate_tokens(self.request.prompt_chars));
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
        let finish = map_finish_reason(self.finish_reason.as_deref(), &mut self.request.degraded);
        frames.push(self.data_chunk(serde_json::json!({
            "candidates": [{
                "content": {"role": "model", "parts": []},
                "finishReason": finish,
                "index": 0
            }],
            "usageMetadata": {
                "promptTokenCount": self.usage.input(),
                "candidatesTokenCount": self.usage.output(),
                "totalTokenCount": self.usage.input() + self.usage.output()
            },
            "modelVersion": self.request.alias
        })));
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

    #[cfg(test)]
    pub fn is_done(&self) -> bool {
        self.finished
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(raw: &str, model: &str, stream: bool) -> GeminiRequest {
        parse_request(raw.as_bytes(), model, stream).expect("gemini request parses")
    }

    #[test]
    fn parses_contents_system_and_generation_config() {
        let request = parse(
            r#"{"systemInstruction":{"parts":[{"text":"be nice"}]},"contents":[{"role":"user","parts":[{"text":"hi"}]}],"generationConfig":{"temperature":0.2,"topP":0.9,"maxOutputTokens":128,"topK":40,"candidateCount":1},"safetySettings":[]}"#,
            "gemini-2.5-pro",
            false,
        );
        assert_eq!(request.alias, "gemini-2.5-pro");
        assert_eq!(request.chat_body["messages"][0]["role"], "system");
        assert_eq!(request.chat_body["messages"][1]["content"], "hi");
        assert_eq!(request.chat_body["max_tokens"], 128);
        assert_eq!(request.chat_body["temperature"], 0.2);
        assert!(request.degraded.contains(&"top_k"));
        assert!(request.degraded.contains(&"safety_settings"));
    }

    #[test]
    fn maps_function_calls_and_responses() {
        let request = parse(
            r#"{"contents":[{"role":"user","parts":[{"text":"list files"}]},{"role":"model","parts":[{"functionCall":{"name":"Bash","args":{"cmd":"ls"}}}]},{"role":"user","parts":[{"functionResponse":{"name":"Bash","response":{"output":"a.txt"}}}]}],"tools":[{"functionDeclarations":[{"name":"Bash","description":"run","parameters":{"type":"object"}}]},{"googleSearch":{}}],"toolConfig":{"functionCallingConfig":{"mode":"ANY"}},"generationConfig":{"candidateCount":1}}"#,
            "gemini-pro",
            false,
        );
        let messages = request.chat_body["messages"].as_array().expect("messages");
        assert_eq!(messages[1]["role"], "assistant");
        assert_eq!(messages[1]["tool_calls"][0]["function"]["name"], "Bash");
        let call_id = messages[1]["tool_calls"][0]["id"].as_str().expect("id");
        assert_eq!(messages[2]["role"], "tool");
        assert_eq!(messages[2]["tool_call_id"], call_id);
        assert_eq!(request.chat_body["tools"][0]["function"]["name"], "Bash");
        assert_eq!(request.chat_body["tool_choice"], "required");
        assert!(request.degraded.contains(&"builtin_tools"));
    }

    #[test]
    fn rejects_missing_contents_candidate_count_and_media() {
        assert!(parse_request(br#"{"contents":[]}"#, "m", false).is_err());
        assert!(matches!(
            parse_request(
                br#"{"contents":[{"role":"user","parts":[{"text":"hi"}]}],"generationConfig":{"candidateCount":2}}"#,
                "m",
                false
            ),
            Err(AppError::BadRequest {
                code: "unsupported_parameter",
                ..
            })
        ));
        assert!(matches!(
            parse_request(
                br#"{"contents":[{"role":"user","parts":[{"inlineData":{"mimeType":"image/png","data":"..."}}]}]}"#,
                "m",
                false
            ),
            Err(AppError::BadRequest {
                code: "unsupported_content",
                ..
            })
        ));
    }

    #[test]
    fn decodes_non_stream_text_and_function_call() {
        let request = parse(
            r#"{"contents":[{"role":"user","parts":[{"text":"hi"}]}]}"#,
            "gemini-pro",
            false,
        );
        let raw = br#"{"choices":[{"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":2}}"#;
        let decoded = decode_non_stream(raw, &request, "req-1").expect("decode");
        let value: Value = serde_json::from_slice(&decoded.body).expect("json");
        assert_eq!(value["candidates"][0]["content"]["parts"][0]["text"], "ok");
        assert_eq!(value["candidates"][0]["finishReason"], "STOP");
        assert_eq!(value["usageMetadata"]["promptTokenCount"], 3);
        assert_eq!(value["modelVersion"], "gemini-pro");

        let request = parse(
            r#"{"contents":[{"role":"user","parts":[{"text":"ls"}]}],"tools":[{"functionDeclarations":[{"name":"Bash"}]}]}"#,
            "gemini-pro",
            false,
        );
        let raw = br#"{"choices":[{"message":{"role":"assistant","content":null,"tool_calls":[{"id":"c1","type":"function","function":{"name":"Bash","arguments":"{\"cmd\":\"ls\"}"}}]},"finish_reason":"tool_calls"}]}"#;
        let decoded = decode_non_stream(raw, &request, "req-2").expect("decode");
        let value: Value = serde_json::from_slice(&decoded.body).expect("json");
        assert_eq!(
            value["candidates"][0]["content"]["parts"][0]["functionCall"]["name"],
            "Bash"
        );
    }

    #[test]
    fn stream_text_and_usage_chunks() {
        let request = parse(
            r#"{"contents":[{"role":"user","parts":[{"text":"hi"}]}],"generationConfig":{"maxOutputTokens":10}}"#,
            "gemini-pro",
            true,
        );
        let mut stream = GeminiStream::new(&request);
        let mut frames = Vec::new();
        for data in [
            r#"{"choices":[{"delta":{"content":"你"},"finish_reason":null}]}"#,
            r#"{"choices":[{"delta":{"content":"好"},"finish_reason":"stop"}]}"#,
            r#"{"choices":[],"usage":{"prompt_tokens":3,"completion_tokens":2}}"#,
        ] {
            frames.extend(stream.handle_event(&crate::modules::proxy::sse::SseEvent {
                event: None,
                data: data.to_string(),
            }));
        }
        frames.extend(stream.finish());
        let text = String::from_utf8_lossy(&frames.concat()).to_string();
        assert!(text.contains("\"text\":\"你\""));
        assert!(text.contains("\"finishReason\":\"STOP\""));
        assert!(text.contains("\"usageMetadata\""));
        assert!(!text.contains("[DONE]"));
        assert!(stream.is_done());
    }
}
