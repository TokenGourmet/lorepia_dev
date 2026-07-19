use crate::{Result, RuntimeError, RuntimeErrorKind};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SseFrame {
    pub(crate) event: Option<String>,
    pub(crate) data: String,
}

pub(crate) struct SseFramer {
    buffer: Vec<u8>,
    event: Option<String>,
    data_lines: Vec<String>,
    current_event_bytes: usize,
    max_frame_bytes: usize,
    first_line: bool,
}

impl SseFramer {
    pub(crate) fn new(max_frame_bytes: usize) -> Self {
        Self {
            buffer: Vec::new(),
            event: None,
            data_lines: Vec::new(),
            current_event_bytes: 0,
            max_frame_bytes,
            first_line: true,
        }
    }

    pub(crate) fn push(&mut self, bytes: &[u8]) -> Result<Vec<SseFrame>> {
        let mut frames = Vec::new();
        let mut remaining = bytes;
        while let Some(newline) = remaining.iter().position(|byte| *byte == b'\n') {
            if self.buffer.len().saturating_add(newline) > self.max_frame_bytes {
                return Err(too_large());
            }
            self.buffer.extend_from_slice(&remaining[..newline]);
            let mut line = std::mem::take(&mut self.buffer);
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            if self.first_line {
                self.first_line = false;
                if line.starts_with(&[0xef, 0xbb, 0xbf]) {
                    line.drain(..3);
                }
            }
            self.process_line(&line, &mut frames)?;
            remaining = &remaining[newline + 1..];
        }
        if self.buffer.len().saturating_add(remaining.len()) > self.max_frame_bytes {
            return Err(too_large());
        }
        self.buffer.extend_from_slice(remaining);
        Ok(frames)
    }

    pub(crate) fn finish(self) -> Result<()> {
        if self.buffer.is_empty() && self.event.is_none() && self.data_lines.is_empty() {
            Ok(())
        } else {
            Err(RuntimeError::new(
                RuntimeErrorKind::StreamProtocol,
                "INCOMPLETE_SSE_FRAME",
                "SSE stream ended in the middle of an event",
            ))
        }
    }

    fn process_line(&mut self, line: &[u8], frames: &mut Vec<SseFrame>) -> Result<()> {
        if line.len() > self.max_frame_bytes {
            return Err(too_large());
        }
        if line.is_empty() {
            if !self.data_lines.is_empty() {
                frames.push(SseFrame {
                    event: self.event.take(),
                    data: self.data_lines.join("\n"),
                });
            } else {
                self.event = None;
            }
            self.data_lines.clear();
            self.current_event_bytes = 0;
            return Ok(());
        }
        if line[0] == b':' {
            return Ok(());
        }
        self.current_event_bytes = self
            .current_event_bytes
            .saturating_add(line.len().saturating_add(1));
        if self.current_event_bytes > self.max_frame_bytes {
            return Err(too_large());
        }
        let line = std::str::from_utf8(line).map_err(|_| {
            RuntimeError::new(
                RuntimeErrorKind::StreamProtocol,
                "INVALID_SSE_UTF8",
                "SSE fields must be valid UTF-8",
            )
        })?;
        let (field, mut value) = line.split_once(':').unwrap_or((line, ""));
        if value.starts_with(' ') {
            value = &value[1..];
        }
        match field {
            "event" => self.event = Some(value.to_owned()),
            "data" => {
                self.data_lines.push(value.to_owned());
            }
            _ => {}
        }
        Ok(())
    }
}

fn too_large() -> RuntimeError {
    RuntimeError::new(
        RuntimeErrorKind::StreamTooLarge,
        "SSE_FRAME_TOO_LARGE",
        "SSE frame exceeded the configured byte limit",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconstructs_frames_at_every_byte_boundary() {
        let fixture = b": ping\r\nevent: custom\r\ndata: {\"a\":\r\ndata: 1}\r\n\r\n";
        for split in 0..=fixture.len() {
            let mut framer = SseFramer::new(1024);
            let mut frames = framer.push(&fixture[..split]).unwrap();
            frames.extend(framer.push(&fixture[split..]).unwrap());
            framer.finish().unwrap();
            assert_eq!(
                frames,
                vec![SseFrame {
                    event: Some("custom".into()),
                    data: "{\"a\":\n1}".into(),
                }],
                "split {split}"
            );
        }
    }

    #[test]
    fn rejects_oversized_and_incomplete_frames() {
        let mut oversized = SseFramer::new(8);
        assert!(oversized.push(b"data: 123456789\n").is_err());

        let mut incomplete = SseFramer::new(64);
        incomplete.push(b"data: {}\n").unwrap();
        assert!(incomplete.finish().is_err());
    }

    #[test]
    fn accepts_one_large_chunk_containing_many_bounded_events() {
        let fixture = "data: {}\n\n".repeat(100);
        assert!(fixture.len() > 64);
        let mut framer = SseFramer::new(64);
        let frames = framer.push(fixture.as_bytes()).unwrap();
        framer.finish().unwrap();
        assert_eq!(frames.len(), 100);
    }
}
