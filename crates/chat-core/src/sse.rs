//! Minimal Server-Sent Events decoder: yields the `data:` payload of each event.

#[derive(Debug, Default)]
pub(crate) struct SseDecoder {
    buf: Vec<u8>,
}

impl SseDecoder {
    /// Feeds raw bytes (which may split events or UTF-8 sequences anywhere)
    /// and returns the payloads of all events completed so far.
    pub(crate) fn push(&mut self, chunk: &[u8]) -> Vec<String> {
        self.buf.extend(chunk.iter().filter(|&&b| b != b'\r'));
        let mut events = Vec::new();
        while let Some(end) = self.buf.windows(2).position(|w| w == b"\n\n") {
            let block: Vec<u8> = self.buf.drain(..end + 2).collect();
            let block = String::from_utf8_lossy(&block);
            let data: Vec<&str> = block
                .lines()
                .filter_map(|line| line.strip_prefix("data:"))
                .map(|d| d.strip_prefix(' ').unwrap_or(d))
                .collect();
            if !data.is_empty() {
                events.push(data.join("\n"));
            }
        }
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_split_chunks_and_crlf() {
        let mut d = SseDecoder::default();
        assert!(d.push(b"event: a\r\ndata: {\"x\"").is_empty());
        assert_eq!(
            d.push(b":1}\r\n\r\ndata: two\n\n: comment\n\n"),
            vec![r#"{"x":1}"#, "two"]
        );
    }

    #[test]
    fn keeps_multibyte_characters_split_across_chunks() {
        let bytes = "data: あ\n\n".as_bytes();
        let mut d = SseDecoder::default();
        assert!(d.push(&bytes[..7]).is_empty());
        assert_eq!(d.push(&bytes[7..]), vec!["あ"]);
    }

    #[test]
    fn joins_multiline_data() {
        let mut d = SseDecoder::default();
        assert_eq!(d.push(b"data: a\ndata:b\n\n"), vec!["a\nb"]);
    }
}
