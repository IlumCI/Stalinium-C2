use aes_gcm::aead::{Aead, generic_array::GenericArray};
use aes_gcm::{Aes256Gcm, KeyInit};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use std::error::Error;

const MAX_PAYLOAD: usize = 1024 * 1024;
const PAD_BLOCK: usize = 512;

pub fn generate_nonce() -> [u8; 12] {
    let mut nonce = [0u8; 12];
    getrandom::getrandom(&mut nonce).expect("Failed to generate nonce");
    nonce
}

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

pub fn encrypt_data(data: &[u8], key: &[u8], nonce: &[u8]) -> Result<Vec<u8>, Box<dyn Error + Send + Sync>> {
    let cipher = Aes256Gcm::new(GenericArray::from_slice(key));
    let nonce = GenericArray::from_slice(nonce);
    cipher.encrypt(nonce, data)
        .map_err(|e| -> Box<dyn Error + Send + Sync> { format!("Encryption failed: {}", e).into() })
}

pub fn decrypt_data(ciphertext: &[u8], key: &[u8], nonce: &[u8]) -> Result<Vec<u8>, Box<dyn Error + Send + Sync>> {
    let cipher = Aes256Gcm::new(GenericArray::from_slice(key));
    let nonce = GenericArray::from_slice(nonce);
    cipher.decrypt(nonce, ciphertext)
        .map_err(|e| -> Box<dyn Error + Send + Sync> { format!("Decryption failed: {}", e).into() })
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
