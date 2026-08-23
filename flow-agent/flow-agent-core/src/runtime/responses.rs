use crate::runtime::types::RuntimeError;
use proto::parse_unique_json;

pub(crate) const MAX_RESPONSES_LINE_BYTES: usize = 256 * 1024;
pub(crate) const MAX_RESPONSES_EVENT_BYTES: usize = 1024 * 1024;
pub(crate) const MAX_RESPONSES_DECODED_STREAM_BYTES: usize = 32 * 1024 * 1024;
pub(crate) const MAX_RESPONSES_EVENTS: usize = 4_096;
const MAX_RESPONSES_WIRE_STREAM_BYTES: usize =
    MAX_RESPONSES_DECODED_STREAM_BYTES + MAX_RESPONSES_EVENT_BYTES;

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct ParsedSseStream {
    pub(crate) values: Vec<serde_json::Value>,
    pub(crate) decoded_stream_bytes: usize,
    pub(crate) dispatched_events: usize,
    pub(crate) terminal_sentinel: bool,
}

pub(crate) struct SseDecoder {
    pending_line: Vec<u8>,
    event_data: Vec<u8>,
    wire_stream_bytes: usize,
    parsed: ParsedSseStream,
}

impl Default for SseDecoder {
    fn default() -> Self {
        Self {
            pending_line: Vec::new(),
            event_data: Vec::new(),
            wire_stream_bytes: 0,
            parsed: ParsedSseStream {
                values: Vec::new(),
                decoded_stream_bytes: 0,
                dispatched_events: 0,
                terminal_sentinel: false,
            },
        }
    }
}

impl SseDecoder {
    pub(crate) fn push(&mut self, bytes: &[u8]) -> Result<(), RuntimeError> {
        self.wire_stream_bytes = validate_wire_stream_bytes(self.wire_stream_bytes, bytes.len())?;
        let mut remaining = bytes;
        while let Some(newline) = remaining.iter().position(|byte| *byte == b'\n') {
            self.append_line_fragment(&remaining[..newline])?;
            self.finish_line()?;
            remaining = &remaining[newline + 1..];
        }
        self.append_line_fragment(remaining)
    }

    pub(crate) fn finish(self) -> Result<ParsedSseStream, RuntimeError> {
        if !self.pending_line.is_empty() || !self.event_data.is_empty() {
            return Err(response_protocol(
                "response stream ended before an SSE event was terminated".to_owned(),
            ));
        }
        Ok(self.parsed)
    }

    pub(crate) fn terminal_sentinel(&self) -> bool {
        self.parsed.terminal_sentinel
    }

    #[cfg(test)]
    pub(crate) fn decoded_stream_bytes(&self) -> usize {
        self.parsed.decoded_stream_bytes
    }

    #[cfg(test)]
    pub(crate) fn retained_value_count(&self) -> usize {
        self.parsed.values.len()
    }

    #[cfg(test)]
    pub(crate) fn pending_line_bytes(&self) -> usize {
        self.pending_line.len()
    }

    fn append_line_fragment(&mut self, bytes: &[u8]) -> Result<(), RuntimeError> {
        let prospective_len = self.pending_line.len().checked_add(bytes.len());
        let prospective_last = bytes.last().or_else(|| self.pending_line.last());
        if prospective_len.is_none_or(|len| {
            len > MAX_RESPONSES_LINE_BYTES.saturating_add(1)
                || (len > MAX_RESPONSES_LINE_BYTES && prospective_last != Some(&b'\r'))
        }) {
            return Err(response_protocol(format!(
                "response stream line exceeds {MAX_RESPONSES_LINE_BYTES} bytes"
            )));
        }
        self.pending_line.extend_from_slice(bytes);
        Ok(())
    }

    fn finish_line(&mut self) -> Result<(), RuntimeError> {
        strip_carriage_return(&mut self.pending_line);
        enforce_line_budget(self.pending_line.len())?;
        if self.pending_line.is_empty() {
            dispatch_event(&mut self.event_data, &mut self.parsed)?;
        } else if !self.pending_line.starts_with(b":")
            && let Some(data) = self.pending_line.strip_prefix(b"data:")
        {
            let data = data.strip_prefix(b" ").unwrap_or(data);
            if !self.event_data.is_empty() {
                append_event_data(&mut self.event_data, b"\n")?;
            }
            append_event_data(&mut self.event_data, data)?;
        }
        self.pending_line.clear();
        Ok(())
    }
}

fn validate_wire_stream_bytes(current: usize, incoming: usize) -> Result<usize, RuntimeError> {
    let next = current
        .checked_add(incoming)
        .ok_or_else(|| response_protocol("response stream byte count overflowed".to_owned()))?;
    if next > MAX_RESPONSES_WIRE_STREAM_BYTES {
        return Err(response_protocol(format!(
            "decoded response stream wire data is {next} bytes; maximum is {MAX_RESPONSES_WIRE_STREAM_BYTES}"
        )));
    }
    Ok(next)
}

fn decoded_item_bytes(item: &serde_json::Value) -> Result<usize, RuntimeError> {
    Ok(proto::canonical_json(item)
        .map_err(|error| {
            response_protocol(format!("response item is not canonicalizable: {error}"))
        })?
        .len())
}

fn validate_decoded_stream_bytes(
    decoded_stream_bytes: usize,
    item_bytes: usize,
) -> Result<usize, RuntimeError> {
    let next = decoded_stream_bytes
        .checked_add(item_bytes)
        .ok_or_else(|| {
            response_protocol("decoded response stream byte count overflowed".to_owned())
        })?;
    if next > MAX_RESPONSES_DECODED_STREAM_BYTES {
        return Err(response_protocol(format!(
            "decoded response stream is {next} bytes; maximum is {MAX_RESPONSES_DECODED_STREAM_BYTES}"
        )));
    }
    Ok(next)
}

fn strip_carriage_return(line: &mut Vec<u8>) {
    if line.last() == Some(&b'\r') {
        line.pop();
    }
}

fn enforce_line_budget(bytes: usize) -> Result<(), RuntimeError> {
    if bytes > MAX_RESPONSES_LINE_BYTES {
        return Err(response_protocol(format!(
            "response stream line is {bytes} bytes; maximum is {MAX_RESPONSES_LINE_BYTES}"
        )));
    }
    Ok(())
}

fn append_event_data(event_data: &mut Vec<u8>, bytes: &[u8]) -> Result<(), RuntimeError> {
    let next = event_data
        .len()
        .checked_add(bytes.len())
        .ok_or_else(|| response_protocol("response event byte count overflowed".to_owned()))?;
    if next > MAX_RESPONSES_EVENT_BYTES {
        return Err(response_protocol(format!(
            "response event is {next} bytes; maximum is {MAX_RESPONSES_EVENT_BYTES}"
        )));
    }
    event_data.extend_from_slice(bytes);
    Ok(())
}

fn dispatch_event(
    event_data: &mut Vec<u8>,
    parsed: &mut ParsedSseStream,
) -> Result<(), RuntimeError> {
    if event_data.is_empty() {
        return Ok(());
    }
    if parsed.dispatched_events == MAX_RESPONSES_EVENTS {
        return Err(response_protocol(format!(
            "response stream contains more than {MAX_RESPONSES_EVENTS} events"
        )));
    }
    if parsed.terminal_sentinel {
        return Err(response_protocol(
            "response stream contains data after its terminal sentinel".to_owned(),
        ));
    }
    parsed.dispatched_events += 1;
    if event_data.as_slice() == b"[DONE]" {
        parsed.terminal_sentinel = true;
        event_data.clear();
        return Ok(());
    }
    let event_text = std::str::from_utf8(event_data).map_err(|error| {
        response_protocol(format!("response event is not valid UTF-8: {error}"))
    })?;
    let value = parse_unique_json(event_text).map_err(|error| {
        response_protocol(format!(
            "response event is not duplicate-free JSON: {error}"
        ))
    })?;
    let item_bytes = decoded_item_bytes(&value)?;
    parsed.decoded_stream_bytes =
        validate_decoded_stream_bytes(parsed.decoded_stream_bytes, item_bytes)?;
    parsed.values.push(value);
    event_data.clear();
    Ok(())
}

fn response_protocol(message: String) -> RuntimeError {
    RuntimeError::Protocol(format!("OpenAI Responses protocol: {message}"))
}
