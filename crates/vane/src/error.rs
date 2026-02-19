use std::fmt;

#[derive(Debug)]
pub struct VaneCliError {
    pub message: String,
}

impl VaneCliError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for VaneCliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for VaneCliError {}
