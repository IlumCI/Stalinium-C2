use aes_gcm::aead::{Aead, generic_array::GenericArray};
use aes_gcm::{Aes256Gcm, KeyInit};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use std::error::Error;

const MAX_PAYLOAD: usize = 1024 * 1024;

pub fn generate_nonce() -> [u8; 12] {
    let mut nonce = [0u8; 12];
    getrandom::getrandom(&mut nonce).expect("Failed to generate nonce");
    nonce
}

pub fn encrypt_data(data: &[u8], key: &[u8], nonce: &[u8]) -> Result<Vec<u8>, Box<dyn Error>> {
    let cipher = Aes256Gcm::new(GenericArray::from_slice(key));
    let nonce = GenericArray::from_slice(nonce);
    cipher.encrypt(nonce, data)
        .map_err(|e| format!("Encryption failed: {}", e).into())
}

pub fn decrypt_data(ciphertext: &[u8], key: &[u8], nonce: &[u8]) -> Result<Vec<u8>, Box<dyn Error>> {
    let cipher = Aes256Gcm::new(GenericArray::from_slice(key));
    let nonce = GenericArray::from_slice(nonce);
    cipher.decrypt(nonce, ciphertext)
        .map_err(|e| format!("Decryption failed: {}", e).into())
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
    decrypt_data(ciphertext, key, nonce)
}
