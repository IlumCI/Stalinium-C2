use std::error::Error;
use std::fmt;

#[derive(Debug)]
pub enum C2Error {
    NetworkError(String),
    CommandError(String),
    UnknownError(String),
}

impl fmt::Display for C2Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            C2Error::NetworkError(msg) => write!(f, "Network error: {}", msg),
            C2Error::CommandError(msg) => write!(f, "Command error: {}", msg),
            C2Error::UnknownError(msg) => write!(f, "Unknown error: {}", msg),
        }
    }
}

impl Error for C2Error {}

impl From<tokio::io::Error> for C2Error {
    fn from(error: tokio::io::Error) -> Self {
        C2Error::NetworkError(error.to_string())
    }
}

impl From<serde_json::Error> for C2Error {
    fn from(error: serde_json::Error) -> Self {
        C2Error::CommandError(error.to_string())
    }
}

pub fn handle_error<E: Error>(error: E) {
    eprintln!("Error occurred: {}", error);
    // Implement advanced error handling logic here
    // For example, retry logic, logging, alerting, etc.
}
