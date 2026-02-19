use std::fmt;

#[derive(Debug)]
pub struct VaneCliError {
    pub message: String,
}

impl fmt::Display for VaneCliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for VaneCliError {}
