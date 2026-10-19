use aes_gcm::aead::{Aead, generic_array::GenericArray};
use aes_gcm::{Aes256Gcm, KeyInit};
use std::error::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const MAX_PAYLOAD: usize = 1024 * 1024;
const PAD_BLOCK: usize = 512;

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

fn pad_plaintext(data: &[u8]) -> Vec<u8> {
    let real_len = data.len() as u32;
    let total = 4 + data.len();
    let padded_len = ((total + PAD_BLOCK - 1) / PAD_BLOCK) * PAD_BLOCK;
    let mut out = Vec::with_capacity(padded_len);
    out.extend_from_slice(&real_len.to_be_bytes());
    out.extend_from_slice(data);
    let pad_needed = padded_len - total;
    if pad_needed > 0 {
        let mut padding = vec![0u8; pad_needed];
        let _ = getrandom::getrandom(&mut padding);
        out.extend_from_slice(&padding);
    }
    out
}

fn unpad_plaintext(data: &[u8]) -> Result<Vec<u8>, Box<dyn Error + Send + Sync>> {
    if data.len() < 4 {
        return Err("Padded plaintext too short".into());
    }
    let real_len = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as usize;
    if 4 + real_len > data.len() {
        return Err("Invalid padding length".into());
    }
    Ok(data[4..4 + real_len].to_vec())
}

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

pub async fn send_encrypted(
    socket: &mut TcpStream,
    data: &[u8],
    key: &[u8],
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let padded = pad_plaintext(data);
    let nonce = generate_nonce();
    let ciphertext = encrypt_data(&padded, key, &nonce)?;
    let payload_len = (12 + ciphertext.len()) as u32;
    socket.write_all(&payload_len.to_be_bytes()).await?;
    socket.write_all(&nonce).await?;
    socket.write_all(&ciphertext).await?;
    Ok(())
}

pub async fn recv_encrypted(
    socket: &mut TcpStream,
    key: &[u8],
) -> Result<Vec<u8>, Box<dyn Error + Send + Sync>> {
    let mut len_buf = [0u8; 4];
    socket.read_exact(&mut len_buf).await?;
    let payload_len = u32::from_be_bytes(len_buf) as usize;
    if payload_len < 12 || payload_len > MAX_PAYLOAD {
        return Err("Invalid payload length".into());
    }
    let mut payload = vec![0u8; payload_len];
    socket.read_exact(&mut payload).await?;
    let (nonce, ciphertext) = payload.split_at(12);
    let padded = decrypt_data(ciphertext, key, nonce)?;
    unpad_plaintext(&padded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encryption_decryption() {
        let key = b"an_example_very_secure_key!!12345";
        let nonce = generate_nonce();
        let data = b"Some secret data";

        let encrypted = encrypt_data(data, key, &nonce).expect("Failed to encrypt");
        let decrypted = decrypt_data(&encrypted, key, &nonce).expect("Failed to decrypt");

        assert_eq!(data.to_vec(), decrypted);
    }

    #[test]
    fn test_padding_roundtrip() {
        let data = b"test payload of arbitrary length";
        let padded = pad_plaintext(data);
        assert_eq!(padded.len() % PAD_BLOCK, 0);
        let unpadded = unpad_plaintext(&padded).unwrap();
        assert_eq!(data.to_vec(), unpadded);
    }

    #[test]
    fn test_padding_alignment() {
        for len in [0, 1, 100, 507, 508, 509, 1024] {
            let data = vec![0xABu8; len];
            let padded = pad_plaintext(&data);
            assert_eq!(padded.len() % PAD_BLOCK, 0);
            let unpadded = unpad_plaintext(&padded).unwrap();
            assert_eq!(data, unpadded);
        }
    }
}
