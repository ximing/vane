use std::fmt;

#[derive(Debug)]
pub struct VaneCliError {
    pub message: String,
    skip: bool,
}

impl VaneCliError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            skip: false,
        }
    }

    /// Recoverable skip (oversized file, etc.). The writer logs and continues.
    pub fn skip(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            skip: true,
        }
    }

    pub fn is_skip(&self) -> bool {
        self.skip
    }
}

impl fmt::Display for VaneCliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for VaneCliError {}
