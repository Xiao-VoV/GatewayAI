//! 上游 SSE → 客户端 SSE 的流式转发：Chat 直通与 Messages 转换。
//!
//! 设计：proxy 技术设计 §6.4。要点：
//! - 不缓冲全量 body：边转发边解析（`SseParser` 旁路），背压由 `Body::from_stream` 传递；
//! - 首字节已转发后失败不重试：按前端协议发错误帧后关闭；
//! - 客户端断开（Body 被 drop）由 `Drop` 终结为 `client_disconnect`，并释放上游连接；
//! - 空闲/总时长超时；usage 在流结束/断开时写入（detached，不阻塞）。

use std::{
    convert::Infallible,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
    time::{Duration, Instant},
};

use axum::body::Body;
use bytes::Bytes;
use futures_util::{Stream, StreamExt, stream};

use crate::modules::metering::model::{UsageSource, UsageStatus};

use super::adapters::{
    UsageTokens, chat, estimate_tokens,
    gemini::{GeminiRequest, GeminiStream},
    messages::{MessagesRequest, MessagesStream},
    responses::{ResponsesRequest, ResponsesStream},
};
use super::error::{
    chat_stream_error_frame, gemini_stream_error_frame, messages_stream_error_frame,
    responses_stream_error_frame,
};
use super::settings::ProxySettings;
use super::sse::{SseEvent, SseParser};
use super::telemetry::{Outcome, Telemetry};
use super::upstream::StreamUpstream;

type BoxStream = Pin<Box<dyn Stream<Item = Result<Bytes, reqwest::Error>> + Send + 'static>>;

struct SseRuntime {
    telemetry: Option<Telemetry>,
    input_tokens_estimate: i64,
    ttft_ms: Option<i64>,
    output_chars: usize,
    usage: UsageTokens,
    estimated_output_tokens: Option<i64>,
    bytes_sent: i64,
    started: Instant,
}

impl SseRuntime {
    fn new(telemetry: Telemetry, input_tokens_estimate: i64) -> Self {
        Self {
            telemetry: Some(telemetry),
            input_tokens_estimate,
            ttft_ms: None,
            output_chars: 0,
            usage: UsageTokens::default(),
            estimated_output_tokens: None,
            bytes_sent: 0,
            started: Instant::now(),
        }
    }

    fn mark_ttft(&mut self) {
        if self.ttft_ms.is_none() {
            self.ttft_ms =
                Some(i64::try_from(self.started.elapsed().as_millis()).unwrap_or(i64::MAX));
        }
    }

    fn add_usage(&mut self, usage: UsageTokens) {
        if usage.input_tokens.is_some() {
            self.usage.input_tokens = usage.input_tokens;
        }
        if usage.output_tokens.is_some() {
            self.usage.output_tokens = usage.output_tokens;
        }
    }
}

/// 终结流：成功/超时/上游错误/客户端断开；幂等（runtime 只取一次）。
fn finalize(
    runtime: &Arc<Mutex<Option<SseRuntime>>>,
    status: UsageStatus,
    error_code: Option<String>,
) {
    let Some(mut runtime) = runtime.lock().ok().and_then(|mut guard| guard.take()) else {
        return;
    };
    let Some(telemetry) = runtime.telemetry.take() else {
        return;
    };
    let input_tokens = runtime
        .usage
        .input_tokens
        .or_else(|| (runtime.input_tokens_estimate > 0).then_some(runtime.input_tokens_estimate));
    let output_tokens = runtime
        .usage
        .output_tokens
        .or(runtime.estimated_output_tokens)
        .or_else(|| Some(estimate_tokens(runtime.output_chars)));
    let usage_source =
        if runtime.usage.input_tokens.is_some() || runtime.usage.output_tokens.is_some() {
            UsageSource::Upstream
        } else if input_tokens.is_some() || output_tokens.is_some() {
            UsageSource::Estimated
        } else {
            UsageSource::Missing
        };
    telemetry.finalize(Outcome {
        status,
        http_status: 200,
        error_code,
        input_tokens,
        output_tokens,
        usage_source,
        ttft_ms: runtime.ttft_ms,
        response_size: Some(runtime.bytes_sent),
    });
}

/// `Body::from_stream` 包装：客户端断开时 Body 被 drop，终结为 `client_disconnect`。
struct GuardedBody {
    inner: Pin<Box<dyn Stream<Item = Result<Bytes, Infallible>> + Send + 'static>>,
    runtime: Arc<Mutex<Option<SseRuntime>>>,
}

impl Stream for GuardedBody {
    type Item = Result<Bytes, Infallible>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.inner.as_mut().poll_next(cx)
    }
}

impl Drop for GuardedBody {
    fn drop(&mut self) {
        finalize(&self.runtime, UsageStatus::ClientDisconnect, None);
    }
}

/// Chat 直通 SSE：字节原样转发；旁路解析 usage/finish/error，`[DONE]` 后正常结束。
pub fn chat_sse_body(
    upstream: StreamUpstream,
    telemetry: Telemetry,
    input_tokens_estimate: i64,
    settings: ProxySettings,
) -> Body {
    let runtime = Arc::new(Mutex::new(Some(SseRuntime::new(
        telemetry,
        input_tokens_estimate,
    ))));
    let inner: BoxStream = Box::pin(upstream.response.bytes_stream());
    let state = ChatRelayState {
        upstream: inner,
        first_chunk: upstream.first_chunk,
        parser: SseParser::new(),
        runtime: runtime.clone(),
        idle_timeout: settings.stream_idle_timeout,
        deadline: Instant::now() + settings.stream_max_duration,
        finished: false,
    };
    let stream = stream::unfold(state, |mut state| async move {
        if state.finished {
            return None;
        }
        let chunk = match state.first_chunk.take() {
            Some(chunk) => Some(Ok::<Bytes, reqwest::Error>(chunk)),
            None => {
                if Instant::now() >= state.deadline {
                    let frame = chat_stream_error_frame(
                        "上游响应超时，已中断",
                        "timeout",
                        "upstream_timeout",
                    );
                    state.finished = true;
                    finalize(
                        &state.runtime,
                        UsageStatus::Timeout,
                        Some("upstream_timeout".to_string()),
                    );
                    return Some((Ok(frame), state));
                }
                match tokio::time::timeout(state.idle_timeout, state.upstream.next()).await {
                    Err(_) => {
                        let frame = chat_stream_error_frame(
                            "上游响应超时，已中断",
                            "timeout",
                            "upstream_timeout",
                        );
                        state.finished = true;
                        finalize(
                            &state.runtime,
                            UsageStatus::Timeout,
                            Some("upstream_timeout".to_string()),
                        );
                        return Some((Ok(frame), state));
                    }
                    Ok(None) => {
                        state.finished = true;
                        finalize(&state.runtime, UsageStatus::Success, None);
                        return None;
                    }
                    Ok(Some(Err(_))) => {
                        let frame = chat_stream_error_frame(
                            "上游流读取失败，已中断",
                            "upstream_error",
                            "upstream_error",
                        );
                        state.finished = true;
                        finalize(
                            &state.runtime,
                            UsageStatus::UpstreamError,
                            Some("upstream_error".to_string()),
                        );
                        return Some((Ok(frame), state));
                    }
                    Ok(Some(Ok(bytes))) => Some(Ok(bytes)),
                }
            }
        };
        let Some(Ok(bytes)) = chunk else {
            state.finished = true;
            finalize(
                &state.runtime,
                UsageStatus::UpstreamError,
                Some("upstream_error".to_string()),
            );
            return None;
        };
        let events = state.parser.push(&bytes);
        let inspection = inspect_chat_chunk(&events);
        if let Ok(mut guard) = state.runtime.lock()
            && let Some(rt) = guard.as_mut()
        {
            rt.add_usage(inspection.usage);
            rt.output_chars += inspection.output_chars;
            rt.bytes_sent += bytes.len() as i64;
            if inspection.output_chars > 0 || inspection.usage.output_tokens.is_some() {
                rt.mark_ttft();
            }
        }
        if inspection.upstream_error {
            let frame =
                chat_stream_error_frame("上游返回错误，已中断", "upstream_error", "upstream_error");
            state.finished = true;
            finalize(
                &state.runtime,
                UsageStatus::UpstreamError,
                Some("upstream_error".to_string()),
            );
            return Some((Ok(frame), state));
        }
        if inspection.done {
            state.finished = true;
            finalize(&state.runtime, UsageStatus::Success, None);
            return Some((Ok(bytes), state));
        }
        Some((Ok(bytes), state))
    });
    Body::from_stream(GuardedBody {
        inner: Box::pin(stream),
        runtime,
    })
}

struct ChatRelayState {
    upstream: BoxStream,
    first_chunk: Option<Bytes>,
    parser: SseParser,
    runtime: Arc<Mutex<Option<SseRuntime>>>,
    idle_timeout: Duration,
    deadline: Instant,
    finished: bool,
}

struct ChatInspection {
    usage: UsageTokens,
    output_chars: usize,
    done: bool,
    upstream_error: bool,
}

fn inspect_chat_chunk(events: &[SseEvent]) -> ChatInspection {
    let mut usage = UsageTokens::default();
    let mut output_chars = 0usize;
    let mut done = false;
    let mut upstream_error = false;
    for event in events {
        if event.is_done() {
            done = true;
            continue;
        }
        let info = chat::inspect_stream_data(&event.data);
        if info.upstream_error.is_some() {
            upstream_error = true;
        }
        if info.usage != UsageTokens::default() {
            usage = info.usage;
        }
        output_chars += chat::stream_content_delta(&event.data).chars().count();
    }
    ChatInspection {
        usage,
        output_chars,
        done,
        upstream_error,
    }
}

/// Messages 转换 SSE：上游 Chat chunks → Anthropic 事件序列。
pub fn messages_sse_body(
    upstream: StreamUpstream,
    telemetry: Telemetry,
    request: &MessagesRequest,
    request_id: &str,
    settings: ProxySettings,
) -> Body {
    let input_estimate = request.input_tokens_estimate();
    let runtime = Arc::new(Mutex::new(Some(SseRuntime::new(telemetry, input_estimate))));
    let inner: BoxStream = Box::pin(upstream.response.bytes_stream());
    let state = MessagesRelayState {
        upstream: inner,
        first_chunk: upstream.first_chunk,
        parser: SseParser::new(),
        runtime: runtime.clone(),
        idle_timeout: settings.stream_idle_timeout,
        deadline: Instant::now() + settings.stream_max_duration,
        finished: false,
        transform: MessagesStream::new(request, request_id),
    };
    let stream = stream::unfold(state, |mut state| async move {
        loop {
            if state.finished {
                return None;
            }
            let chunk = match state.first_chunk.take() {
                Some(chunk) => Some(Ok::<Bytes, reqwest::Error>(chunk)),
                None => {
                    if Instant::now() >= state.deadline {
                        let frame =
                            messages_stream_error_frame("overloaded_error", "上游响应超时，已中断");
                        state.finished = true;
                        finalize(
                            &state.runtime,
                            UsageStatus::Timeout,
                            Some("upstream_timeout".to_string()),
                        );
                        return Some((Ok(frame), state));
                    }
                    match tokio::time::timeout(state.idle_timeout, state.upstream.next()).await {
                        Err(_) => {
                            let frame = messages_stream_error_frame(
                                "overloaded_error",
                                "上游响应超时，已中断",
                            );
                            state.finished = true;
                            finalize(
                                &state.runtime,
                                UsageStatus::Timeout,
                                Some("upstream_timeout".to_string()),
                            );
                            return Some((Ok(frame), state));
                        }
                        Ok(None) => {
                            let frames = state.transform.finish();
                            let emit = concat_frames(&frames);
                            let usage = state.transform.usage();
                            record_transform_usage(
                                &state.runtime,
                                usage,
                                state.transform.output_tokens_estimate(),
                            );
                            add_sent(&state.runtime, &emit);
                            state.finished = true;
                            finalize(&state.runtime, UsageStatus::Success, None);
                            return if emit.is_empty() {
                                None
                            } else {
                                Some((Ok(emit), state))
                            };
                        }
                        Ok(Some(Err(_))) => {
                            let frame =
                                messages_stream_error_frame("api_error", "上游流读取失败，已中断");
                            state.finished = true;
                            finalize(
                                &state.runtime,
                                UsageStatus::UpstreamError,
                                Some("upstream_error".to_string()),
                            );
                            return Some((Ok(frame), state));
                        }
                        Ok(Some(Ok(bytes))) => Some(Ok(bytes)),
                    }
                }
            };
            let Some(Ok(bytes)) = chunk else {
                state.finished = true;
                finalize(
                    &state.runtime,
                    UsageStatus::UpstreamError,
                    Some("upstream_error".to_string()),
                );
                return None;
            };
            let events = state.parser.push(&bytes);
            let mut frames: Vec<Bytes> = Vec::new();
            let mut done = false;
            for event in &events {
                if event.is_done() {
                    done = true;
                } else {
                    frames.extend(state.transform.handle_event(event));
                }
            }
            if state.transform.take_error().is_some() {
                tracing::warn!("upstream messages stream returned an error object");
                let frame = messages_stream_error_frame("api_error", "上游返回错误，已中断");
                state.finished = true;
                finalize(
                    &state.runtime,
                    UsageStatus::UpstreamError,
                    Some("upstream_error".to_string()),
                );
                return Some((Ok(frame), state));
            }
            if done {
                frames.extend(state.transform.finish());
                let usage = state.transform.usage();
                record_transform_usage(
                    &state.runtime,
                    usage,
                    state.transform.output_tokens_estimate(),
                );
                let emit = concat_frames(&frames);
                add_sent(&state.runtime, &emit);
                state.finished = true;
                finalize(&state.runtime, UsageStatus::Success, None);
                return if emit.is_empty() {
                    None
                } else {
                    Some((Ok(emit), state))
                };
            }
            let emit = concat_frames(&frames);
            if !emit.is_empty() {
                add_sent(&state.runtime, &emit);
                return Some((Ok(emit), state));
            }
        }
    });
    Body::from_stream(GuardedBody {
        inner: Box::pin(stream),
        runtime,
    })
}

/// Responses 转换 SSE：上游 Chat chunks → OpenAI Responses 事件序列（不发 [DONE]）。
pub fn responses_sse_body(
    upstream: StreamUpstream,
    telemetry: Telemetry,
    request: &ResponsesRequest,
    request_id: &str,
    settings: ProxySettings,
) -> Body {
    let input_estimate = request.input_tokens_estimate();
    let runtime = Arc::new(Mutex::new(Some(SseRuntime::new(telemetry, input_estimate))));
    let inner: BoxStream = Box::pin(upstream.response.bytes_stream());
    let state = ResponsesRelayState {
        upstream: inner,
        first_chunk: upstream.first_chunk,
        parser: SseParser::new(),
        runtime: runtime.clone(),
        idle_timeout: settings.stream_idle_timeout,
        deadline: Instant::now() + settings.stream_max_duration,
        finished: false,
        transform: ResponsesStream::new(request, request_id),
    };
    let stream = stream::unfold(state, |mut state| async move {
        loop {
            if state.finished {
                return None;
            }
            let chunk = match state.first_chunk.take() {
                Some(chunk) => Some(Ok::<Bytes, reqwest::Error>(chunk)),
                None => {
                    if Instant::now() >= state.deadline {
                        let frame = responses_stream_error_frame("上游响应超时，已中断");
                        state.finished = true;
                        finalize(
                            &state.runtime,
                            UsageStatus::Timeout,
                            Some("upstream_timeout".to_string()),
                        );
                        return Some((Ok(frame), state));
                    }
                    match tokio::time::timeout(state.idle_timeout, state.upstream.next()).await {
                        Err(_) => {
                            let frame = responses_stream_error_frame("上游响应超时，已中断");
                            state.finished = true;
                            finalize(
                                &state.runtime,
                                UsageStatus::Timeout,
                                Some("upstream_timeout".to_string()),
                            );
                            return Some((Ok(frame), state));
                        }
                        Ok(None) => {
                            let frames = state.transform.finish();
                            let emit = concat_frames(&frames);
                            let usage = state.transform.usage();
                            record_transform_usage(
                                &state.runtime,
                                usage,
                                state.transform.output_tokens_estimate(),
                            );
                            add_sent(&state.runtime, &emit);
                            state.finished = true;
                            finalize(&state.runtime, UsageStatus::Success, None);
                            return if emit.is_empty() {
                                None
                            } else {
                                Some((Ok(emit), state))
                            };
                        }
                        Ok(Some(Err(_))) => {
                            let frame = responses_stream_error_frame("上游流读取失败，已中断");
                            state.finished = true;
                            finalize(
                                &state.runtime,
                                UsageStatus::UpstreamError,
                                Some("upstream_error".to_string()),
                            );
                            return Some((Ok(frame), state));
                        }
                        Ok(Some(Ok(bytes))) => Some(Ok(bytes)),
                    }
                }
            };
            let Some(Ok(bytes)) = chunk else {
                state.finished = true;
                finalize(
                    &state.runtime,
                    UsageStatus::UpstreamError,
                    Some("upstream_error".to_string()),
                );
                return None;
            };
            let events = state.parser.push(&bytes);
            let mut frames: Vec<Bytes> = Vec::new();
            let mut done = false;
            for event in &events {
                if event.is_done() {
                    done = true;
                } else {
                    frames.extend(state.transform.handle_event(event));
                }
            }
            if state.transform.take_error().is_some() {
                tracing::warn!("upstream responses stream returned an error object");
                let frame = responses_stream_error_frame("上游返回错误，已中断");
                state.finished = true;
                finalize(
                    &state.runtime,
                    UsageStatus::UpstreamError,
                    Some("upstream_error".to_string()),
                );
                return Some((Ok(frame), state));
            }
            if done {
                frames.extend(state.transform.finish());
                let usage = state.transform.usage();
                record_transform_usage(
                    &state.runtime,
                    usage,
                    state.transform.output_tokens_estimate(),
                );
                let emit = concat_frames(&frames);
                add_sent(&state.runtime, &emit);
                state.finished = true;
                finalize(&state.runtime, UsageStatus::Success, None);
                return if emit.is_empty() {
                    None
                } else {
                    Some((Ok(emit), state))
                };
            }
            let emit = concat_frames(&frames);
            if !emit.is_empty() {
                add_sent(&state.runtime, &emit);
                return Some((Ok(emit), state));
            }
        }
    });
    Body::from_stream(GuardedBody {
        inner: Box::pin(stream),
        runtime,
    })
}

/// Gemini 转换 SSE：上游 Chat chunks → Gemini alt=sse chunk（不发 [DONE]）。
pub fn gemini_sse_body(
    upstream: StreamUpstream,
    telemetry: Telemetry,
    request: &GeminiRequest,
    request_id: &str,
    settings: ProxySettings,
) -> Body {
    let input_estimate = request.input_tokens_estimate();
    let runtime = Arc::new(Mutex::new(Some(SseRuntime::new(telemetry, input_estimate))));
    let inner: BoxStream = Box::pin(upstream.response.bytes_stream());
    let state = GeminiRelayState {
        upstream: inner,
        first_chunk: upstream.first_chunk,
        parser: SseParser::new(),
        runtime: runtime.clone(),
        idle_timeout: settings.stream_idle_timeout,
        deadline: Instant::now() + settings.stream_max_duration,
        finished: false,
        transform: GeminiStream::new(request),
        request_id: request_id.to_string(),
    };
    let stream = stream::unfold(state, |mut state| async move {
        loop {
            if state.finished {
                return None;
            }
            let chunk = match state.first_chunk.take() {
                Some(chunk) => Some(Ok::<Bytes, reqwest::Error>(chunk)),
                None => {
                    if Instant::now() >= state.deadline {
                        let frame = gemini_stream_error_frame("上游响应超时，已中断");
                        state.finished = true;
                        finalize(
                            &state.runtime,
                            UsageStatus::Timeout,
                            Some("upstream_timeout".to_string()),
                        );
                        return Some((Ok(frame), state));
                    }
                    match tokio::time::timeout(state.idle_timeout, state.upstream.next()).await {
                        Err(_) => {
                            let frame = gemini_stream_error_frame("上游响应超时，已中断");
                            state.finished = true;
                            finalize(
                                &state.runtime,
                                UsageStatus::Timeout,
                                Some("upstream_timeout".to_string()),
                            );
                            return Some((Ok(frame), state));
                        }
                        Ok(None) => {
                            let frames = state.transform.finish();
                            let emit = concat_frames(&frames);
                            let usage = state.transform.usage();
                            record_transform_usage(
                                &state.runtime,
                                usage,
                                state.transform.output_tokens_estimate(),
                            );
                            add_sent(&state.runtime, &emit);
                            state.finished = true;
                            finalize(&state.runtime, UsageStatus::Success, None);
                            return if emit.is_empty() {
                                None
                            } else {
                                Some((Ok(emit), state))
                            };
                        }
                        Ok(Some(Err(_))) => {
                            let frame = gemini_stream_error_frame("上游流读取失败，已中断");
                            state.finished = true;
                            finalize(
                                &state.runtime,
                                UsageStatus::UpstreamError,
                                Some("upstream_error".to_string()),
                            );
                            return Some((Ok(frame), state));
                        }
                        Ok(Some(Ok(bytes))) => Some(Ok(bytes)),
                    }
                }
            };
            let Some(Ok(bytes)) = chunk else {
                state.finished = true;
                finalize(
                    &state.runtime,
                    UsageStatus::UpstreamError,
                    Some("upstream_error".to_string()),
                );
                return None;
            };
            let events = state.parser.push(&bytes);
            let mut frames: Vec<Bytes> = Vec::new();
            let mut done = false;
            for event in &events {
                if event.is_done() {
                    done = true;
                } else {
                    frames.extend(state.transform.handle_event(event));
                }
            }
            if state.transform.take_error().is_some() {
                tracing::warn!(request_id = %state.request_id, "upstream gemini stream returned an error object");
                let frame = gemini_stream_error_frame("上游返回错误，已中断");
                state.finished = true;
                finalize(
                    &state.runtime,
                    UsageStatus::UpstreamError,
                    Some("upstream_error".to_string()),
                );
                return Some((Ok(frame), state));
            }
            if done {
                frames.extend(state.transform.finish());
                let usage = state.transform.usage();
                record_transform_usage(
                    &state.runtime,
                    usage,
                    state.transform.output_tokens_estimate(),
                );
                let emit = concat_frames(&frames);
                add_sent(&state.runtime, &emit);
                state.finished = true;
                finalize(&state.runtime, UsageStatus::Success, None);
                return if emit.is_empty() {
                    None
                } else {
                    Some((Ok(emit), state))
                };
            }
            let emit = concat_frames(&frames);
            if !emit.is_empty() {
                add_sent(&state.runtime, &emit);
                return Some((Ok(emit), state));
            }
        }
    });
    Body::from_stream(GuardedBody {
        inner: Box::pin(stream),
        runtime,
    })
}

struct GeminiRelayState {
    upstream: BoxStream,
    first_chunk: Option<Bytes>,
    parser: SseParser,
    runtime: Arc<Mutex<Option<SseRuntime>>>,
    idle_timeout: Duration,
    deadline: Instant,
    finished: bool,
    transform: GeminiStream,
    request_id: String,
}

struct ResponsesRelayState {
    upstream: BoxStream,
    first_chunk: Option<Bytes>,
    parser: SseParser,
    runtime: Arc<Mutex<Option<SseRuntime>>>,
    idle_timeout: Duration,
    deadline: Instant,
    finished: bool,
    transform: ResponsesStream,
}

struct MessagesRelayState {
    upstream: BoxStream,
    first_chunk: Option<Bytes>,
    parser: SseParser,
    runtime: Arc<Mutex<Option<SseRuntime>>>,
    idle_timeout: Duration,
    deadline: Instant,
    finished: bool,
    transform: MessagesStream,
}

fn record_transform_usage(
    runtime: &Arc<Mutex<Option<SseRuntime>>>,
    usage: UsageTokens,
    output_estimate: i64,
) {
    if let Ok(mut guard) = runtime.lock()
        && let Some(rt) = guard.as_mut()
    {
        rt.add_usage(usage);
        if rt.usage.output_tokens.is_none() {
            rt.estimated_output_tokens = Some(output_estimate);
        }
    }
}

fn add_sent(runtime: &Arc<Mutex<Option<SseRuntime>>>, emit: &Bytes) {
    if emit.is_empty() {
        return;
    }
    if let Ok(mut guard) = runtime.lock()
        && let Some(rt) = guard.as_mut()
    {
        rt.bytes_sent += emit.len() as i64;
        rt.mark_ttft();
    }
}

fn concat_frames(frames: &[Bytes]) -> Bytes {
    if frames.is_empty() {
        return Bytes::new();
    }
    if frames.len() == 1 {
        return frames[0].clone();
    }
    let mut bytes = Vec::with_capacity(frames.iter().map(Bytes::len).sum());
    for frame in frames {
        bytes.extend_from_slice(frame);
    }
    Bytes::from(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concat_frames_keeps_order() {
        let frames = vec![Bytes::from_static(b"a"), Bytes::from_static(b"b")];
        assert_eq!(concat_frames(&frames), Bytes::from_static(b"ab"));
        assert_eq!(concat_frames(&[]), Bytes::new());
    }
}
