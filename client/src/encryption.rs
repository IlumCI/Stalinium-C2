use aes_gcm::aead::{Aead, generic_array::GenericArray};
use aes_gcm::{Aes256Gcm, KeyInit};
use std::error::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const MAX_PAYLOAD: usize = 1024 * 1024;

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
    cipher.encrypt(nonce, data)
        .map_err(|e| EncryptionError::EncryptionError(e.to_string()))
}

pub fn decrypt_data(encrypted_data: &[u8], key: &[u8], nonce: &[u8]) -> Result<Vec<u8>, EncryptionError> {
    let cipher = Aes256Gcm::new(GenericArray::from_slice(key));
    let nonce = GenericArray::from_slice(nonce);
    cipher.decrypt(nonce, encrypted_data)
        .map_err(|e| EncryptionError::DecryptionError(e.to_string()))
}

pub fn generate_nonce() -> [u8; 12] {
    let mut nonce = [0u8; 12];
    getrandom::getrandom(&mut nonce).expect("Failed to generate nonce");
    nonce
}

/// Send length-prefixed encrypted data.
/// Wire format: [4 bytes BE length][12 bytes nonce][N bytes ciphertext]
pub async fn send_encrypted(
    socket: &mut TcpStream,
    data: &[u8],
    key: &[u8],
) -> Result<(), Box<dyn Error>> {
    let nonce = generate_nonce();
    let ciphertext = encrypt_data(data, key, &nonce)?;
    let payload_len = (12 + ciphertext.len()) as u32;
    socket.write_all(&payload_len.to_be_bytes()).await?;
    socket.write_all(&nonce).await?;
    socket.write_all(&ciphertext).await?;
    Ok(())
}

/// Receive and decrypt length-prefixed data.
pub async fn recv_encrypted(
    socket: &mut TcpStream,
    key: &[u8],
) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut len_buf = [0u8; 4];
    socket.read_exact(&mut len_buf).await?;
    let payload_len = u32::from_be_bytes(len_buf) as usize;
    if payload_len < 12 || payload_len > MAX_PAYLOAD {
        return Err("Invalid payload length".into());
    }
    let mut payload = vec![0u8; payload_len];
    socket.read_exact(&mut payload).await?;
    let (nonce, ciphertext) = payload.split_at(12);
    Ok(decrypt_data(ciphertext, key, nonce)?)
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
