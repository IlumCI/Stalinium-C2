use ml_kem::MlKem768;
use ml_kem::kem::Decapsulate;
use ml_kem::{Encoded, EncodedSizeUser, KemCore};
use x25519_dalek::{EphemeralSecret, PublicKey};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use rand::rngs::OsRng;
use std::error::Error;

use super::kdf::derive_session_key;

pub struct HandshakeResult {
    pub session_key: [u8; 32],
}

// ML-KEM-768 sizes
const MLKEM_EK_LEN: usize = 1184;
const MLKEM_CT_LEN: usize = 1088;

pub async fn server_handshake(
    socket: &mut TcpStream,
) -> Result<HandshakeResult, Box<dyn Error + Send + Sync>> {
    let x25519_secret = EphemeralSecret::random_from_rng(OsRng);
    let x25519_public = PublicKey::from(&x25519_secret);

    let (mlkem_dk, mlkem_ek) = MlKem768::generate(&mut OsRng);
    let ek_bytes: Encoded<ml_kem::EncapsulationKey<MlKem768>> = mlkem_ek.as_bytes();

    let mut server_hello = Vec::with_capacity(4 + 32 + MLKEM_EK_LEN);
    let payload_len = (32 + MLKEM_EK_LEN) as u32;
    server_hello.extend_from_slice(&payload_len.to_be_bytes());
    server_hello.extend_from_slice(x25519_public.as_bytes());
    server_hello.extend_from_slice(&ek_bytes);
    socket.write_all(&server_hello).await?;

    let mut len_buf = [0u8; 4];
    socket.read_exact(&mut len_buf).await?;
    let client_payload_len = u32::from_be_bytes(len_buf) as usize;
    if client_payload_len != 32 + MLKEM_CT_LEN {
        return Err("Invalid client hello length".into());
    }

    let mut client_hello = vec![0u8; client_payload_len];
    socket.read_exact(&mut client_hello).await?;

    let mut client_x25519_bytes = [0u8; 32];
    client_x25519_bytes.copy_from_slice(&client_hello[..32]);
    let client_x25519_public = PublicKey::from(client_x25519_bytes);

    let x25519_shared = x25519_secret.diffie_hellman(&client_x25519_public);

    let ct_bytes = &client_hello[32..];
    let ct = ml_kem::Ciphertext::<MlKem768>::from_bytes(
        ct_bytes.try_into().map_err(|_| "Invalid ML-KEM ciphertext length")?,
    );
    let mlkem_shared = mlkem_dk
        .decapsulate(&ct)
        .map_err(|_| "ML-KEM decapsulation failed")?;

    let mut mlkem_ss = [0u8; 32];
    mlkem_ss.copy_from_slice(mlkem_shared.as_ref());

    let session_key = derive_session_key(x25519_shared.as_bytes(), &mlkem_ss);

    Ok(HandshakeResult { session_key })
}
