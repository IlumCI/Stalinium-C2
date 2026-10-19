use aes_gcm::aead::{Aead, generic_array::GenericArray};
use aes_gcm::{Aes256Gcm, KeyInit};
use std::error::Error;
use crate::commands::Command;

#[derive(Debug)]
pub enum EncryptionError {
    EncryptionError(String),
    DecryptionError(String),
    KeyError(String),
}

impl std::fmt::Display for EncryptionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl Error for EncryptionError {}

pub fn encrypt_data(data: &[u8], key: &[u8], nonce: &[u8]) -> Result<Vec<u8>, EncryptionError> {
    let cipher = Aes256Gcm::new(GenericArray::from_slice(key));
    let nonce = GenericArray::from_slice(nonce);

    cipher
        .encrypt(nonce, data)
        .map_err(|e| EncryptionError::EncryptionError(e.to_string()))
}

pub fn decrypt_data(encrypted_data: &[u8], key: &[u8], nonce: &[u8]) -> Result<Vec<u8>, EncryptionError> {
    let cipher = Aes256Gcm::new(GenericArray::from_slice(key));
    let nonce = GenericArray::from_slice(nonce);

    cipher
        .decrypt(nonce, encrypted_data)
        .map_err(|e| EncryptionError::DecryptionError(e.to_string()))
}

pub fn generate_nonce() -> [u8; 12] {
    let mut nonce = [0u8; 12];
    getrandom::getrandom(&mut nonce).expect("Failed to generate nonce");
    nonce
}

/// Stub: decrypts a command. In a full implementation, this would decrypt
/// encrypted fields using the configured encryption key and nonce.
pub fn decrypt_command(command: &Command) -> Result<Command, EncryptionError> {
    // Passthrough for now -- encrypted transport not yet wired
    Ok(command.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encryption_decryption() {
        let key = b"an_example_very_secure_key!!12345"; // 32 bytes for AES-256
        let nonce = generate_nonce();
        let data = b"Some secret data";

        let encrypted = encrypt_data(data, key, &nonce).expect("Failed to encrypt");
        let decrypted = decrypt_data(&encrypted, key, &nonce).expect("Failed to decrypt");

        assert_eq!(data.to_vec(), decrypted);
    }
}
