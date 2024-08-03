use std::fs;
use std::path::Path;
use std::io::{self, Write};
use log::{info, error};
use serde::{Deserialize, Serialize};

/// Reads a file and returns its contents as a String.
pub fn read_file_to_string(path: &str) -> io::Result<String> {
    fs::read_to_string(path)
}

/// Writes a string to a file.
pub fn write_string_to_file(path: &str, content: &str) -> io::Result<()> {
    let mut file = fs::File::create(path)?;
    file.write_all(content.as_bytes())
}

/// Appends a string to a file.
pub fn append_string_to_file(path: &str, content: &str) -> io::Result<()> {
    let mut file = fs::OpenOptions::new().append(true).open(path)?;
    file.write_all(content.as_bytes())
}

/// Logs an error message to a log file.
pub fn log_error(message: &str) {
    error!("{}", message);
}

/// Logs an info message to a log file.
pub fn log_info(message: &str) {
    info!("{}", message);
}

/// Serializes an object to a JSON string.
pub fn serialize_to_json<T: Serialize>(data: &T) -> Result<String, serde_json::Error> {
    serde_json::to_string(data)
}

/// Deserializes a JSON string to an object.
pub fn deserialize_from_json<T: for<'de> Deserialize<'de>>(data: &str) -> Result<T, serde_json::Error> {
    serde_json::from_str(data)
}

/// Creates a directory if it does not exist.
pub fn create_directory_if_not_exists(path: &str) -> io::Result<()> {
    if !Path::new(path).exists() {
        fs::create_dir_all(path)?;
    }
    Ok(())
}

/// Deletes a file if it exists.
pub fn delete_file_if_exists(path: &str) -> io::Result<()> {
    if Path::new(path).exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}

/// Checks if a file exists.
pub fn file_exists(path: &str) -> bool {
    Path::new(path).exists()
}

/// Reads the contents of a file into a Vec<u8>.
pub fn read_file_to_bytes(path: &str) -> io::Result<Vec<u8>> {
    fs::read(path)
}

/// Writes bytes to a file.
pub fn write_bytes_to_file(path: &str, content: &[u8]) -> io::Result<()> {
    fs::write(path, content)
}
