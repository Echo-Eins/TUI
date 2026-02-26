/// A buffer that accumulates raw bytes and produces valid UTF-8 strings,
/// correctly handling multi-byte characters split across read boundaries.
///
/// When a read boundary falls in the middle of a multi-byte UTF-8 sequence,
/// `String::from_utf8_lossy` would replace the incomplete bytes with U+FFFD.
/// This buffer saves incomplete trailing bytes and prepends them to the next chunk.

pub struct Utf8AccumulationBuffer {
    /// Incomplete trailing bytes from the previous chunk (max 3 bytes for a 4-byte sequence).
    tail: Vec<u8>,
}

impl Utf8AccumulationBuffer {
    pub fn new() -> Self {
        Self {
            tail: Vec::with_capacity(4),
        }
    }

    /// Feed a chunk of raw bytes and return the valid UTF-8 string.
    ///
    /// Incomplete trailing bytes are saved internally and will be prepended
    /// to the next call. The returned string is always valid UTF-8.
    pub fn push(&mut self, chunk: &[u8]) -> String {
        if chunk.is_empty() && self.tail.is_empty() {
            return String::new();
        }

        // Combine saved tail with new chunk
        let mut combined = Vec::with_capacity(self.tail.len() + chunk.len());
        combined.extend_from_slice(&self.tail);
        combined.extend_from_slice(chunk);
        self.tail.clear();

        // Find the last valid UTF-8 boundary
        let valid_up_to = find_valid_utf8_boundary(&combined);

        // Save any trailing incomplete bytes
        if valid_up_to < combined.len() {
            self.tail.extend_from_slice(&combined[valid_up_to..]);
        }

        // The portion up to `valid_up_to` is guaranteed valid UTF-8
        // (because we only cut at char boundaries)
        match std::str::from_utf8(&combined[..valid_up_to]) {
            Ok(s) => s.to_string(),
            Err(_) => {
                // Fallback: should not happen if find_valid_utf8_boundary is correct,
                // but be safe and use lossy conversion
                String::from_utf8_lossy(&combined[..valid_up_to]).to_string()
            }
        }
    }

    /// Flush any remaining bytes as lossy UTF-8 (call at end of stream).
    pub fn flush(&mut self) -> String {
        if self.tail.is_empty() {
            return String::new();
        }
        let result = String::from_utf8_lossy(&self.tail).to_string();
        self.tail.clear();
        result
    }
}

/// Find the byte index up to which the input is valid UTF-8.
///
/// If the last byte(s) form an incomplete multi-byte sequence,
/// returns the index before that sequence starts.
fn find_valid_utf8_boundary(bytes: &[u8]) -> usize {
    match std::str::from_utf8(bytes) {
        Ok(_) => bytes.len(), // all valid
        Err(e) => {
            let valid = e.valid_up_to();
            // Check if the error is due to an incomplete sequence at the end
            // (as opposed to genuinely invalid bytes in the middle)
            if e.error_len().is_none() {
                // Incomplete sequence — save trailing bytes
                valid
            } else {
                // Invalid byte(s) in the middle — skip them and try to continue
                // For simplicity, we return up to the valid part
                // The caller will save the rest as tail which will be re-evaluated
                valid + e.error_len().unwrap()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_passthrough() {
        let mut buf = Utf8AccumulationBuffer::new();
        assert_eq!(buf.push(b"hello world"), "hello world");
    }

    #[test]
    fn split_two_byte_char() {
        // Cyrillic 'п' = 0xD0 0xBF (2 bytes)
        let mut buf = Utf8AccumulationBuffer::new();
        // First chunk: just the leading byte
        let result1 = buf.push(&[0xD0]);
        assert_eq!(result1, "");
        // Second chunk: the continuation byte
        let result2 = buf.push(&[0xBF]);
        assert_eq!(result2, "п");
    }

    #[test]
    fn split_three_byte_char() {
        // Euro sign '€' = 0xE2 0x82 0xAC (3 bytes)
        let mut buf = Utf8AccumulationBuffer::new();
        // First chunk: first two bytes
        let result1 = buf.push(&[0xE2, 0x82]);
        assert_eq!(result1, "");
        // Second chunk: last byte
        let result2 = buf.push(&[0xAC]);
        assert_eq!(result2, "€");
    }

    #[test]
    fn split_four_byte_char() {
        // Emoji '😀' = 0xF0 0x9F 0x98 0x80 (4 bytes)
        let mut buf = Utf8AccumulationBuffer::new();
        // First chunk: first two bytes
        let result1 = buf.push(&[0xF0, 0x9F]);
        assert_eq!(result1, "");
        // Second chunk: last two bytes
        let result2 = buf.push(&[0x98, 0x80]);
        assert_eq!(result2, "😀");
    }

    #[test]
    fn mixed_ascii_and_multibyte() {
        // "hello п" where 'п' is split
        let mut buf = Utf8AccumulationBuffer::new();
        let result1 = buf.push(b"hello \xD0");
        assert_eq!(result1, "hello ");
        let result2 = buf.push(b"\xBF world");
        assert_eq!(result2, "п world");
    }

    #[test]
    fn complete_multibyte_in_single_chunk() {
        let mut buf = Utf8AccumulationBuffer::new();
        let result = buf.push("привет".as_bytes());
        assert_eq!(result, "привет");
    }

    #[test]
    fn empty_input() {
        let mut buf = Utf8AccumulationBuffer::new();
        assert_eq!(buf.push(b""), "");
    }

    #[test]
    fn flush_incomplete() {
        let mut buf = Utf8AccumulationBuffer::new();
        let _ = buf.push(&[0xD0]); // incomplete Cyrillic
        let flushed = buf.flush();
        // Should produce a replacement character
        assert!(!flushed.is_empty());
    }

    #[test]
    fn flush_empty() {
        let mut buf = Utf8AccumulationBuffer::new();
        assert_eq!(buf.flush(), "");
    }
}
