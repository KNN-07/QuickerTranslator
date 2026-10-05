use serde_json::Value;

use crate::models::{AppError, AppResult};

/// A fully framed SSE event. Unknown fields, comments, IDs and retry hints are ignored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    pub event: Option<String>,
    pub data: String,
}

/// An adapter alone decides which native protocol event proves successful completion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SseControl {
    Continue,
    Done,
}

/// Incremental byte framing keeps split UTF-8 intact until a complete line is available.
pub struct SseDecoder {
    line: Vec<u8>,
    event: Option<String>,
    data: String,
    has_data: bool,
    after_cr: bool,
    first_line: bool,
}

impl Default for SseDecoder {
    fn default() -> Self {
        Self {
            line: Vec::new(),
            event: None,
            data: String::new(),
            has_data: false,
            after_cr: false,
            first_line: true,
        }
    }
}

impl SseDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Dispatch immediately without allocating a per-network-chunk event collection.
    /// Done stops decoding at the successful terminal, even if more bytes follow it.
    pub fn feed<F>(&mut self, bytes: &[u8], on_event: &mut F) -> AppResult<SseControl>
    where
        F: FnMut(SseEvent) -> AppResult<SseControl>,
    {
        for &byte in bytes {
            if self.after_cr {
                self.after_cr = false;
                if byte == b'\n' {
                    continue;
                }
            }
            let event = match byte {
                b'\r' => {
                    self.after_cr = true;
                    self.complete_line()?
                }
                b'\n' => self.complete_line()?,
                _ => {
                    self.line.push(byte);
                    None
                }
            };
            if let Some(event) = event {
                if on_event(event)? == SseControl::Done {
                    return Ok(SseControl::Done);
                }
            }
        }
        Ok(SseControl::Continue)
    }

    #[cfg(test)]
    fn push(&mut self, bytes: &[u8]) -> AppResult<Vec<SseEvent>> {
        let mut events = Vec::new();
        self.feed(bytes, &mut |event| {
            events.push(event);
            Ok(SseControl::Continue)
        })?;
        Ok(events)
    }

    /// EOF does not dispatch an event without its blank-line delimiter.
    pub fn finish(&self) -> AppResult<()> {
        if !self.line.is_empty() || self.has_data || self.event.is_some() {
            return Err(AppError::new(
                "ai_interrupted",
                "The provider stream ended in an unfinished event.",
            ));
        }
        Ok(())
    }

    fn complete_line(&mut self) -> AppResult<Option<SseEvent>> {
        let mut line = std::str::from_utf8(&self.line).map_err(|_| {
            AppError::new("ai_protocol", "The provider stream contains invalid UTF-8.")
        })?;
        if self.first_line {
            self.first_line = false;
            line = line.strip_prefix('\u{feff}').unwrap_or(line);
        }
        let mut completed = None;
        if line.is_empty() {
            if self.has_data {
                // Each data line contributes one LF; the final one is not event data.
                self.data.pop();
                completed = Some(SseEvent {
                    event: self.event.take(),
                    data: std::mem::take(&mut self.data),
                });
                self.has_data = false;
            } else {
                self.event = None;
            }
        } else if !line.starts_with(':') {
            let (field, value) = line.split_once(':').unwrap_or((line, ""));
            let value = value.strip_prefix(' ').unwrap_or(value);
            match field {
                "event" => self.event = (!value.is_empty()).then(|| value.to_owned()),
                "data" => {
                    self.data.push_str(value);
                    self.data.push('\n');
                    self.has_data = true;
                }
                _ => {}
            }
        }
        self.line.clear();
        Ok(completed)
    }
}

/// Call only for events whose native protocol requires JSON, not pings or [DONE].
pub fn json_data(event: &SseEvent) -> AppResult<Value> {
    serde_json::from_str(&event.data).map_err(|_| {
        AppError::new("ai_protocol", "The provider sent malformed event JSON.")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_byte_boundary_preserves_utf8_crlf_and_multiline_data() {
        let input = concat!(
            "\u{feff}: keepalive\r\n",
            "id: ignored\r\nretry: 100\r\n",
            "event: output\r\n",
            "data: {\r\n",
            "data: \"text\":\"Xin chào. 🙂\"}\r\n\r\n",
            "event: ping\ndata:\n\n",
            "data: [DONE]\r\r",
        ).as_bytes();
        let expected = vec![
            SseEvent {
                event: Some("output".into()),
                data: "{\n\"text\":\"Xin chào. 🙂\"}".into(),
            },
            SseEvent { event: Some("ping".into()), data: String::new() },
            SseEvent { event: None, data: "[DONE]".into() },
        ];
        for boundary in 0..=input.len() {
            let mut decoder = SseDecoder::new();
            let mut events = decoder.push(&input[..boundary]).unwrap();
            events.extend(decoder.push(&input[boundary..]).unwrap());
            assert_eq!(events, expected, "split at {boundary}");
            decoder.finish().unwrap();
        }
        let mut decoder = SseDecoder::new();
        let mut events = Vec::new();
        for byte in input {
            events.extend(decoder.push(std::slice::from_ref(byte)).unwrap());
        }
        assert_eq!(events, expected);
        assert_eq!(json_data(&events[0]).unwrap()["text"], "Xin chào. 🙂");
        decoder.finish().unwrap();
    }

    #[test]
    fn only_one_optional_space_is_removed_and_fields_reset() {
        let mut decoder = SseDecoder::new();
        let events = decoder.push(b"event: discarded\n\ndata:  keep\nunknown: ignored\n\ndata\n\n").unwrap();
        assert_eq!(events, vec![
            SseEvent { event: None, data: " keep".into() },
            SseEvent { event: None, data: String::new() },
        ]);
    }

    #[test]
    fn malformed_utf8_and_json_are_sanitized() {
        let error = SseDecoder::new().push(b"data: \xffsecret\n\n").unwrap_err();
        assert_eq!(error.code, "ai_protocol");
        assert!(!error.message.contains("secret"));
        let error = json_data(&SseEvent { event: None, data: "private invalid {".into() }).unwrap_err();
        assert_eq!(error.code, "ai_protocol");
        assert!(!error.message.contains("private"));
    }

    #[test]
    fn eof_never_completes_pending_events() {
        for input in [b"data: unfinished".as_slice(), b"data: framed line\n", b"event: unfinished\n"] {
            let mut decoder = SseDecoder::new();
            assert!(decoder.push(input).unwrap().is_empty());
            assert_eq!(decoder.finish().unwrap_err().code, "ai_interrupted");
        }
        let mut decoder = SseDecoder::new();
        assert!(decoder.push(b": comment\n\n").unwrap().is_empty());
        decoder.finish().unwrap();
    }

    #[test]
    fn terminal_dispatch_stops_before_unrelated_trailing_bytes() {
        let mut decoder = SseDecoder::new();
        let mut called = 0;
        let control = decoder.feed(b"data: [DONE]\n\ndata: \xff\n\n", &mut |event| {
            called += 1;
            assert_eq!(event.data, "[DONE]");
            Ok(SseControl::Done)
        }).unwrap();
        assert_eq!(control, SseControl::Done);
        assert_eq!(called, 1);
    }
}
