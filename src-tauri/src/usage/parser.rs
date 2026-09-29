//! Bounded observation only: bytes forwarded by the gateway are never altered.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::Write;
const MAX_EVENT: usize = 2 * 1024 * 1024;

#[derive(Default, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tokens {
    pub input: Option<u64>,
    pub output: Option<u64>,
    pub cache_read: Option<u64>,
    pub cache_write: Option<u64>,
    pub cache_write_hour: Option<u64>,
    pub input_images: Option<u64>,
    pub output_images: Option<u64>,
    pub images: Option<u64>,
    pub context: Option<u64>,
}
impl Tokens {
    #[cfg(test)]
    pub fn total(&self) -> u64 {
        [self.input, self.output, self.cache_read, self.cache_write]
            .into_iter()
            .flatten()
            .sum()
    }
    pub fn reported(&self) -> bool {
        self.input.is_some() || self.output.is_some() || self.images.is_some()
    }
}
#[derive(Default, Clone, Debug)]
pub struct Observation {
    pub tokens: Tokens,
    pub model: Option<String>,
    pub service_tier: Option<String>,
    pub first_token_ms: Option<u64>,
    pub incomplete: bool,
    pub terminal: Option<Terminal>,
    pub expects_terminal: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Terminal {
    Success,
    Limited,
    Rejected,
    Failure,
    Cancelled,
    Unknown,
}
impl Terminal {
    pub fn outcome(self) -> &'static str {
        match self {
            Self::Success => "OK",
            Self::Limited => "OUTPUT_LIMIT",
            Self::Rejected => "BUSINESS_REJECTED",
            Self::Failure => "UPSTREAM_ERROR",
            Self::Cancelled => "CANCELLED",
            Self::Unknown => "UNKNOWN_TERMINAL",
        }
    }
}
fn count(v: &Value, paths: &[&str]) -> Option<u64> {
    paths
        .iter()
        .find_map(|p| v.pointer(p).and_then(Value::as_u64))
        .filter(|n| *n <= 1_000_000_000_000)
}
pub fn identifier(v: Option<&Value>) -> Option<String> {
    v.and_then(Value::as_str)
        .filter(|s| !s.is_empty() && s.len() <= 256 && !s.chars().any(char::is_control))
        .map(str::to_owned)
}
impl Observation {
    pub fn value(&mut self, outer: &Value, elapsed: u64) {
        let value = outer
            .get("response")
            .or_else(|| outer.get("message"))
            .unwrap_or(outer);
        if let Some(model) = identifier(value.get("model")) {
            self.model = Some(model);
        }
        if let Some(tier) = value
            .get("service_tier")
            .and_then(Value::as_str)
            .filter(|s| {
                matches!(
                    *s,
                    "default" | "auto" | "priority" | "flex" | "batch" | "fast"
                )
            })
        {
            self.service_tier = Some(tier.to_owned());
        }
        let event = outer.get("type").and_then(Value::as_str).unwrap_or("");
        self.expects_terminal |= event.starts_with("response.") || outer.get("choices").is_some();
        if self.terminal.is_none() {
            let status = value.get("status").and_then(Value::as_str).unwrap_or("");
            let terminal = match event {
                "response.completed" | "response.done" | "message_stop" => Some(Terminal::Success),
                "response.cancelled" | "response.canceled" => Some(Terminal::Cancelled),
                "response.failed" | "error" => Some(error_terminal(value)),
                "response.incomplete" => Some(incomplete_terminal(value)),
                _ => match status {
                    "completed" => Some(Terminal::Success),
                    "failed" => Some(error_terminal(value)),
                    "incomplete" => Some(incomplete_terminal(value)),
                    "cancelled" => Some(Terminal::Cancelled),
                    _ if value.get("error").is_some_and(|e| !e.is_null()) => {
                        Some(error_terminal(value))
                    }
                    _ => None,
                },
            };
            // A chat completion finishes only once all reported choices finish.
            let choices = outer.get("choices").and_then(Value::as_array);
            self.terminal = terminal.or_else(|| {
                choices
                    .filter(|c| {
                        !c.is_empty()
                            && c.iter()
                                .all(|v| v.get("finish_reason").is_some_and(|v| !v.is_null()))
                    })
                    .map(|c| {
                        if c.iter().any(|v| v["finish_reason"] == "content_filter") {
                            Terminal::Rejected
                        } else if c.iter().any(|v| v["finish_reason"] == "length") {
                            Terminal::Limited
                        } else {
                            Terminal::Success
                        }
                    })
            });
        }
        let delta = event.ends_with(".delta")
            || event == "content_block_delta"
            || outer
                .pointer("/choices/0/delta/content")
                .and_then(Value::as_str)
                .is_some_and(|s| !s.is_empty());
        if delta && self.first_token_ms.is_none() {
            self.first_token_ms = Some(elapsed);
        }
        if let Some(u) = value.get("usage").or_else(|| outer.get("usage")) {
            let read = count(
                u,
                &[
                    "/cache_read_input_tokens",
                    "/input_tokens_details/cached_tokens",
                    "/prompt_tokens_details/cached_tokens",
                ],
            );
            let short = count(u, &["/cache_creation/ephemeral_5m_input_tokens"]);
            let hour = count(u, &["/cache_creation/ephemeral_1h_input_tokens"]);
            let write = count(u, &["/cache_creation_input_tokens"]).or_else(|| {
                (short.is_some() || hour.is_some())
                    .then_some(short.unwrap_or(0) + hour.unwrap_or(0))
            });
            if let Some(n) = read {
                self.tokens.cache_read = Some(n);
            }
            if let Some(n) = write {
                self.tokens.cache_write = Some(n);
            }
            if let Some(n) = hour {
                self.tokens.cache_write_hour = Some(n);
            }
            if let Some(input) = count(u, &["/input_tokens", "/prompt_tokens"]) {
                // Anthropic reports fresh input; OpenAI-style input includes cache.
                let fresh = u.get("cache_read_input_tokens").is_some()
                    || u.get("cache_creation_input_tokens").is_some()
                    || event == "message_start"
                    || value.get("type").and_then(Value::as_str) == Some("message");
                self.tokens.input = Some(if fresh {
                    input
                } else {
                    input
                        .saturating_sub(self.tokens.cache_read.unwrap_or(0))
                        .saturating_sub(self.tokens.cache_write.unwrap_or(0))
                });
                self.tokens.context = Some(if fresh {
                    input
                        + self.tokens.cache_read.unwrap_or(0)
                        + self.tokens.cache_write.unwrap_or(0)
                } else {
                    input
                });
            }
            if let Some(n) = count(u, &["/output_tokens", "/completion_tokens"]) {
                self.tokens.output = Some(n);
            }
            if let Some(n) = count(
                u,
                &[
                    "/input_tokens_details/image_tokens",
                    "/prompt_tokens_details/image_tokens",
                ],
            ) {
                self.tokens.input_images = Some(n);
            }
            if let Some(n) = count(
                u,
                &[
                    "/output_tokens_details/image_tokens",
                    "/completion_tokens_details/image_tokens",
                ],
            ) {
                self.tokens.output_images = Some(n);
            }
        }
        if let Some(u) = value.get("usageMetadata") {
            let input = count(u, &["/promptTokenCount"]);
            let read = count(u, &["/cachedContentTokenCount"]);
            if let Some(n) = read {
                self.tokens.cache_read = Some(n);
            }
            if let Some(n) = input {
                self.tokens.context = Some(n);
                self.tokens.input = Some(n.saturating_sub(read.unwrap_or(0)));
            }
            if let Some(output) = count(u, &["/candidatesTokenCount"]) {
                self.tokens.output = Some(output + count(u, &["/thoughtsTokenCount"]).unwrap_or(0));
            }
        }
        if let Some(data) = value.get("data").and_then(Value::as_array) {
            if !data.is_empty()
                && data
                    .iter()
                    .all(|v| v.get("url").is_some() || v.get("b64_json").is_some())
            {
                self.tokens.images = Some(data.len() as u64);
            }
        }
    }
}
fn error_terminal(value: &Value) -> Terminal {
    let e = value.get("error").unwrap_or(value);
    let code = e
        .get("code")
        .or_else(|| e.get("type"))
        .and_then(Value::as_str)
        .unwrap_or("");
    if matches!(
        code,
        "invalid_request_error"
            | "invalid_request"
            | "context_length_exceeded"
            | "response_not_found"
            | "content_filter"
    ) {
        Terminal::Rejected
    } else {
        Terminal::Failure
    }
}
fn incomplete_terminal(value: &Value) -> Terminal {
    match value
        .pointer("/incomplete_details/reason")
        .and_then(Value::as_str)
        .unwrap_or("")
    {
        "max_output_tokens" | "max_tokens" => Terminal::Limited,
        "content_filter" => Terminal::Rejected,
        "server_error" | "rate_limit_exceeded" => Terminal::Failure,
        _ => Terminal::Unknown,
    }
}
struct Sink {
    observation: Observation,
    buffer: Vec<u8>,
    data: Vec<u8>,
    stream: bool,
    overflow: bool,
    event_overflow: bool,
    elapsed: u64,
}
impl Sink {
    fn new(stream: bool) -> Self {
        Self {
            observation: Observation::default(),
            buffer: vec![],
            data: vec![],
            stream,
            overflow: false,
            event_overflow: false,
            elapsed: 0,
        }
    }
    fn parse(&mut self, data: &[u8]) {
        if data.trim_ascii() == b"[DONE]" {
            self.observation.terminal.get_or_insert(Terminal::Success);
            return;
        }
        if let Ok(v) = serde_json::from_slice(data) {
            self.observation.value(&v, self.elapsed);
        }
    }
    fn line(&mut self) {
        if self.overflow {
            self.event_overflow = true;
            self.observation.incomplete = true;
        } else {
            let line = self.buffer.strip_suffix(b"\n").unwrap_or(&self.buffer);
            let line = line.strip_suffix(b"\r").unwrap_or(line);
            if line.is_empty() {
                if !self.event_overflow && !self.data.is_empty() {
                    let data = std::mem::take(&mut self.data);
                    self.parse(&data);
                }
                self.data.clear();
                self.event_overflow = false;
            } else if let Some(data) = line.strip_prefix(b"data:") {
                if self.data.len() + data.len() < MAX_EVENT {
                    self.data.extend_from_slice(data);
                    self.data.push(b'\n');
                } else {
                    self.event_overflow = true;
                    self.observation.incomplete = true;
                }
            }
        }
        self.buffer.clear();
        self.overflow = false;
    }
    fn finish(&mut self) {
        if self.stream {
            if !self.buffer.is_empty() {
                self.line();
            }
            if !self.event_overflow {
                let data = std::mem::take(&mut self.data);
                self.parse(&data);
            }
        } else if !self.overflow {
            let data = std::mem::take(&mut self.buffer);
            self.parse(&data);
        }
    }
}
impl Write for Sink {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.stream {
            for segment in bytes.split_inclusive(|b| *b == b'\n') {
                if !self.overflow && self.buffer.len() + segment.len() <= MAX_EVENT {
                    self.buffer.extend_from_slice(segment);
                } else {
                    self.overflow = true;
                    self.buffer.clear();
                }
                if segment.ends_with(b"\n") {
                    self.line();
                }
            }
        } else if !self.overflow && self.buffer.len() + bytes.len() <= MAX_EVENT {
            self.buffer.extend_from_slice(bytes);
        } else {
            self.overflow = true;
            self.observation.incomplete = true;
            self.buffer.clear();
            return Err(std::io::Error::other("usage observation limit"));
        }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
enum Decoder {
    Plain(Sink),
    Gzip(flate2::write::GzDecoder<Sink>),
    Deflate(flate2::write::ZlibDecoder<Sink>),
    Zstd(zstd::stream::write::Decoder<'static, Sink>),
}
pub struct Observer {
    decoder: Decoder,
    failed: bool,
}
impl Observer {
    pub fn new(stream: bool, encoding: &str) -> Self {
        let sink = Sink::new(stream);
        let mut failed = false;
        let decoder = match encoding {
            "" | "identity" => Decoder::Plain(sink),
            "gzip" => Decoder::Gzip(flate2::write::GzDecoder::new(sink)),
            "deflate" => Decoder::Deflate(flate2::write::ZlibDecoder::new(sink)),
            "zstd" => Decoder::Zstd(zstd::stream::write::Decoder::new(sink).expect("zstd decoder")),
            _ => {
                failed = true;
                Decoder::Plain(sink)
            }
        };
        Self { decoder, failed }
    }
    fn sink(&mut self) -> &mut Sink {
        match &mut self.decoder {
            Decoder::Plain(s) => s,
            Decoder::Gzip(d) => d.get_mut(),
            Decoder::Deflate(d) => d.get_mut(),
            Decoder::Zstd(d) => d.get_mut(),
        }
    }
    pub fn feed(&mut self, bytes: &[u8], elapsed: u64) {
        if self.failed {
            return;
        }
        self.sink().elapsed = elapsed;
        let result = match &mut self.decoder {
            Decoder::Plain(s) => s.write_all(bytes),
            Decoder::Gzip(d) => d.write_all(bytes),
            Decoder::Deflate(d) => d.write_all(bytes),
            Decoder::Zstd(d) => d.write_all(bytes),
        };
        if result.is_err() {
            self.failed = true;
        }
    }
    pub fn snapshot(&mut self, finish: bool) -> Observation {
        if finish {
            let result = match &mut self.decoder {
                Decoder::Plain(_) => Ok(()),
                Decoder::Gzip(d) => d.try_finish(),
                Decoder::Deflate(d) => d.try_finish(),
                Decoder::Zstd(d) => d.flush(),
            };
            if result.is_err() {
                self.failed = true;
            }
            self.sink().finish();
        }
        let mut result = self.sink().observation.clone();
        result.incomplete |= self.failed;
        result
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn protocol_terminal_and_error_categories_are_idempotent() {
        for (value, expected) in [
            (serde_json::json!({"status":"completed"}), Terminal::Success),
            (
                serde_json::json!({"status":"failed","error":{"code":"server_error"}}),
                Terminal::Failure,
            ),
            (
                serde_json::json!({"type":"error","error":{"code":"rate_limit_exceeded"}}),
                Terminal::Failure,
            ),
            (
                serde_json::json!({"type":"error","error":{"type":"invalid_request_error"}}),
                Terminal::Rejected,
            ),
            (
                serde_json::json!({"type":"response.incomplete","response":{"incomplete_details":{"reason":"max_output_tokens"}}}),
                Terminal::Limited,
            ),
            (
                serde_json::json!({"choices":[{"finish_reason":"length"}]}),
                Terminal::Limited,
            ),
            (
                serde_json::json!({"choices":[{"finish_reason":"content_filter"}]}),
                Terminal::Rejected,
            ),
            (
                serde_json::json!({"type":"response.cancelled"}),
                Terminal::Cancelled,
            ),
        ] {
            let mut o = Observation::default();
            o.value(&value, 10);
            assert_eq!(o.terminal, Some(expected));
            o.value(&serde_json::json!({"type":"response.completed"}), 11);
            assert_eq!(o.terminal, Some(expected));
        }
        let mut observer = Observer::new(true, "");
        for chunk in b"data: {\"type\":\"response.failed\"}\n\ndata: [DONE]\n\n".chunks(3) {
            observer.feed(chunk, 1);
        }
        assert_eq!(observer.snapshot(true).terminal, Some(Terminal::Failure));
    }
    #[test]
    fn fragmented_sse_duplicate_usage_and_cache_normalization() {
        let bytes = b"data: {\"type\":\"response.output_text.delta\",\"delta\":\"x\"}\r\n\r\ndata: {\"response\":{\"model\":\"gpt-fixture\",\"usage\":{\"input_tokens\":100,\"output_tokens\":12,\"input_tokens_details\":{\"cached_tokens\":80},\"output_tokens_details\":{\"reasoning_tokens\":5}}}}\n\n";
        let mut o = Observer::new(true, "");
        for chunk in bytes.chunks(3) {
            o.feed(chunk, 5);
        }
        o.feed(&bytes[67..], 8);
        let v = o.snapshot(true);
        assert_eq!(v.tokens.input, Some(20));
        assert_eq!(v.tokens.output, Some(12));
        assert_eq!(v.tokens.total(), 112);
        assert_eq!(v.first_token_ms, Some(5));
    }
    #[test]
    fn gzip_and_zstd_preserve_usage_without_unbounded_buffering() {
        let raw = br#"{"usage":{"prompt_tokens":17,"completion_tokens":2}}"#;
        let mut g = flate2::write::GzEncoder::new(vec![], flate2::Compression::fast());
        g.write_all(raw).unwrap();
        for (encoding, bytes) in [
            ("gzip", g.finish().unwrap()),
            ("zstd", zstd::encode_all(&raw[..], 1).unwrap()),
        ] {
            let mut o = Observer::new(false, encoding);
            for c in bytes.chunks(2) {
                o.feed(c, 0);
            }
            assert_eq!(o.snapshot(true).tokens.total(), 19);
        }
        let mut o = Observer::new(false, "");
        o.feed(&vec![b' '; MAX_EVENT + 1], 0);
        assert!(o.snapshot(true).incomplete);
    }
    #[test]
    fn anthropic_usage_updates_do_not_subtract_cache_twice() {
        let mut o = Observation::default();
        o.value(&serde_json::json!({"type":"message_start","message":{"type":"message","usage":{"input_tokens":10,"cache_read_input_tokens":100,"cache_creation_input_tokens":20}}}),0);
        o.value(
            &serde_json::json!({"type":"message_delta","usage":{"output_tokens":5}}),
            1,
        );
        assert_eq!(o.tokens.total(), 135);
        assert_eq!(o.tokens.context, Some(130));
    }
}
