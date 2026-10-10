/// A buffer that accumulates peeked data from stdin.
/// When a command runs with stdin_source="inherit", the peeked data
/// is replayed first, then the remaining original stdin is forwarded.
pub struct PeekBuffer {
    data: Vec<u8>,
}

impl PeekBuffer {
    pub fn new() -> Self {
        PeekBuffer { data: Vec::new() }
    }

    pub fn append(&mut self, bytes: &[u8]) {
        self.data.extend_from_slice(bytes);
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// Take the buffered data (for replaying to a command's stdin).
    pub fn take(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.data)
    }

    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
}

#[derive(Debug, serde::Serialize)]
pub struct PeekResult {
    pub text: String,
    pub bytes_read: usize,
    pub truncated: bool,
}
