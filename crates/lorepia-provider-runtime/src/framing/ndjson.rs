use crate::{Result, RuntimeError, RuntimeErrorKind};

pub(crate) struct NdjsonFramer {
    buffer: Vec<u8>,
    max_frame_bytes: usize,
}

impl NdjsonFramer {
    pub(crate) fn new(max_frame_bytes: usize) -> Self {
        Self {
            buffer: Vec::new(),
            max_frame_bytes,
        }
    }

    pub(crate) fn push(&mut self, bytes: &[u8]) -> Result<Vec<String>> {
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
            if !line.is_empty() {
                frames.push(parse_line(line)?);
            }
            remaining = &remaining[newline + 1..];
        }
        if self.buffer.len().saturating_add(remaining.len()) > self.max_frame_bytes {
            return Err(too_large());
        }
        self.buffer.extend_from_slice(remaining);
        Ok(frames)
    }

    pub(crate) fn finish(mut self) -> Result<Vec<String>> {
        if self.buffer.last() == Some(&b'\r') {
            self.buffer.pop();
        }
        if self.buffer.is_empty() {
            Ok(Vec::new())
        } else {
            Ok(vec![parse_line(self.buffer)?])
        }
    }
}

fn parse_line(line: Vec<u8>) -> Result<String> {
    String::from_utf8(line).map_err(|_| {
        RuntimeError::new(
            RuntimeErrorKind::StreamProtocol,
            "INVALID_NDJSON_UTF8",
            "NDJSON frames must be valid UTF-8",
        )
    })
}

fn too_large() -> RuntimeError {
    RuntimeError::new(
        RuntimeErrorKind::StreamTooLarge,
        "NDJSON_FRAME_TOO_LARGE",
        "NDJSON frame exceeded the configured byte limit",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconstructs_lines_at_every_byte_boundary() {
        let fixture = b"{\"message\":{\"content\":\"A\"}}\r\n{\"done\":true}";
        for split in 0..=fixture.len() {
            let mut framer = NdjsonFramer::new(1024);
            let mut frames = framer.push(&fixture[..split]).unwrap();
            frames.extend(framer.push(&fixture[split..]).unwrap());
            frames.extend(framer.finish().unwrap());
            assert_eq!(frames.len(), 2, "split {split}");
        }
    }

    #[test]
    fn rejects_an_oversized_partial_line() {
        let mut framer = NdjsonFramer::new(4);
        assert!(framer.push(b"12345").is_err());
    }

    #[test]
    fn accepts_one_large_chunk_containing_many_bounded_lines() {
        let fixture = "{}\n".repeat(100);
        assert!(fixture.len() > 64);
        let mut framer = NdjsonFramer::new(64);
        let frames = framer.push(fixture.as_bytes()).unwrap();
        assert!(framer.finish().unwrap().is_empty());
        assert_eq!(frames.len(), 100);
    }
}
