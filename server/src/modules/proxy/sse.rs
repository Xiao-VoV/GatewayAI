//! SSE 解析/封装：增量解析上游 SSE（任意 chunk 切分），供 Chat 旁路检查与 Messages 转换使用。
//!
//! 只做字节级行解析，不保存正文语义；超过 [`SSE_LINE_BUFFER_MAX`] 后停止解析（调用方仍继续转发字节），
//! 避免恶意/异常上游把内存打满。

use bytes::Bytes;

use super::settings::SSE_LINE_BUFFER_MAX;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    pub event: Option<String>,
    pub data: String,
}

impl SseEvent {
    pub fn is_done(&self) -> bool {
        self.data.trim() == "[DONE]"
    }
}

#[derive(Debug, Default)]
pub struct SseParser {
    buf: Vec<u8>,
    event_name: Option<String>,
    data_lines: Vec<String>,
    overflowed: bool,
}

impl SseParser {
    pub fn new() -> Self {
        Self::default()
    }

    #[cfg(test)]
    pub fn is_overflowed(&self) -> bool {
        self.overflowed
    }

    /// 喂入一个上游 chunk，返回本次可完整解析出的 SSE 事件。
    pub fn push(&mut self, chunk: &[u8]) -> Vec<SseEvent> {
        let mut events = Vec::new();
        if self.overflowed {
            return events;
        }
        self.buf.extend_from_slice(chunk);
        while let Some(position) = self.buf.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = self.buf.drain(..=position).collect();
            let line = trim_line(&line);
            self.handle_line(line, &mut events);
        }
        // 单个事件超过上限：放弃解析（继续原样转发），避免内存放大。
        if self.buf.len() > SSE_LINE_BUFFER_MAX {
            self.overflowed = true;
            self.buf.clear();
            self.data_lines.clear();
            self.event_name = None;
            events.clear();
        }
        events
    }

    /// 流结束时冲刷未以空行结尾的最后一个事件（流式场景通常由 EOF 结束；测试使用）。
    #[cfg(test)]
    pub fn finish(&mut self) -> Option<SseEvent> {
        if self.overflowed {
            return None;
        }
        if !self.buf.is_empty() {
            let rest = std::mem::take(&mut self.buf);
            let line = trim_line(&rest);
            let mut ignored = Vec::new();
            self.handle_line(line, &mut ignored);
        }
        self.dispatch()
    }

    fn handle_line(&mut self, line: &[u8], events: &mut Vec<SseEvent>) {
        if line.is_empty() {
            if let Some(event) = self.dispatch() {
                events.push(event);
            }
            return;
        }
        if line.starts_with(b":") {
            return; // comment / heartbeat
        }
        let (field, value) = match line.iter().position(|byte| *byte == b':') {
            Some(position) => (&line[..position], &line[position + 1..]),
            None => (line, &b""[..]),
        };
        let value = std::str::from_utf8(value)
            .map(|value| value.strip_prefix(' ').unwrap_or(value))
            .unwrap_or_default();
        match field {
            b"event" => self.event_name = Some(value.to_string()),
            b"data" => self.data_lines.push(value.to_string()),
            // id/retry 等字段对代理无意义，忽略。
            _ => {}
        }
    }

    fn dispatch(&mut self) -> Option<SseEvent> {
        let event_name = self.event_name.take();
        if self.data_lines.is_empty() {
            return None;
        }
        let data = self.data_lines.join("\n");
        self.data_lines.clear();
        Some(SseEvent {
            event: event_name,
            data,
        })
    }
}

fn trim_line(line: &[u8]) -> &[u8] {
    let line = line.strip_suffix(b"\n").unwrap_or(line);
    line.strip_suffix(b"\r").unwrap_or(line)
}

pub fn frame(event: Option<&str>, data: &str) -> Bytes {
    let mut out = String::with_capacity(data.len() + 32);
    if let Some(event) = event {
        out.push_str("event: ");
        out.push_str(event);
        out.push('\n');
    }
    for line in data.split('\n') {
        out.push_str("data: ");
        out.push_str(line);
        out.push('\n');
    }
    out.push('\n');
    Bytes::from(out)
}

pub fn json_frame(event: Option<&str>, value: &serde_json::Value) -> Bytes {
    frame(event, &value.to_string())
}

#[cfg(test)]
pub fn done_frame() -> Bytes {
    frame(None, "[DONE]")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_events_split_across_chunks() {
        let mut parser = SseParser::new();
        assert!(parser.push(b"data: {\"a\":").is_empty());
        let events = parser.push(b"1}\n\ndata: [DONE]\n\n");
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].data, "{\"a\":1}");
        assert!(events[1].is_done());
    }

    #[test]
    fn parses_named_events_and_crlf() {
        let mut parser = SseParser::new();
        let events = parser.push(b"event: error\r\ndata: {\"type\":\"error\"}\r\n\r\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event.as_deref(), Some("error"));
        assert_eq!(events[0].data, "{\"type\":\"error\"}");
    }

    #[test]
    fn joins_multiline_data_and_ignores_comments() {
        let mut parser = SseParser::new();
        let events = parser.push(b": ping\ndata: line1\ndata: line2\n\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, "line1\nline2");
    }

    #[test]
    fn finish_flushes_unterminated_event() {
        let mut parser = SseParser::new();
        assert!(parser.push(b"data: [DONE]").is_empty());
        let event = parser.finish().expect("unterminated event flushed");
        assert!(event.is_done());
    }

    #[test]
    fn overflow_stops_parsing_without_panicking() {
        let mut parser = SseParser::new();
        let huge = vec![b'x'; SSE_LINE_BUFFER_MAX + 1024];
        assert!(parser.push(&huge).is_empty());
        assert!(parser.is_overflowed());
        assert!(parser.push(b"data: [DONE]\n\n").is_empty());
        assert!(parser.finish().is_none());
    }

    #[test]
    fn frames_are_sse_compliant() {
        let value = serde_json::json!({"type": "message_stop"});
        assert_eq!(frame(Some("message_stop"), &value.to_string()), {
            Bytes::from("event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n")
        });
        assert_eq!(done_frame(), Bytes::from("data: [DONE]\n\n"));
    }
}
