use ml_kem::MlKem768;
use ml_kem::{EncodedSizeUser, KemCore};
use ml_kem::kem::Encapsulate;
use x25519_dalek::{EphemeralSecret, PublicKey};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use rand::rngs::OsRng;
use std::error::Error;

use super::kdf::derive_session_key;

pub struct HandshakeResult {
    pub session_key: [u8; 32],
}

const MLKEM_EK_LEN: usize = 1184;
const MLKEM_CT_LEN: usize = 1088;

type MlKemEk = <MlKem768 as KemCore>::EncapsulationKey;

pub async fn client_handshake(
    socket: &mut TcpStream,
) -> Result<HandshakeResult, Box<dyn Error + Send + Sync>> {
    let mut len_buf = [0u8; 4];
    socket.read_exact(&mut len_buf).await?;
    let server_payload_len = u32::from_be_bytes(len_buf) as usize;
    if server_payload_len != 32 + MLKEM_EK_LEN {
        return Err("Invalid server hello length".into());
    }

    let mut server_hello = vec![0u8; server_payload_len];
    socket.read_exact(&mut server_hello).await?;

    let mut server_x25519_bytes = [0u8; 32];
    server_x25519_bytes.copy_from_slice(&server_hello[..32]);
    let server_x25519_public = PublicKey::from(server_x25519_bytes);

    let ek_bytes = &server_hello[32..];
    let ek_arr = ml_kem::array::Array::clone_from_slice(ek_bytes);
    let mlkem_ek = <MlKemEk as EncodedSizeUser>::from_bytes(&ek_arr);

    let x25519_secret = EphemeralSecret::random_from_rng(OsRng);
    let x25519_public = PublicKey::from(&x25519_secret);

    let x25519_shared = x25519_secret.diffie_hellman(&server_x25519_public);

    let (ct, mlkem_shared) = mlkem_ek
        .encapsulate(&mut OsRng)
        .map_err(|_| "ML-KEM encapsulation failed")?;

    let mut client_hello = Vec::with_capacity(4 + 32 + MLKEM_CT_LEN);
    let payload_len = (32 + MLKEM_CT_LEN) as u32;
    client_hello.extend_from_slice(&payload_len.to_be_bytes());
    client_hello.extend_from_slice(x25519_public.as_bytes());
    client_hello.extend_from_slice(ct.as_ref());
    socket.write_all(&client_hello).await?;

    let mut mlkem_ss = [0u8; 32];
    mlkem_ss.copy_from_slice(mlkem_shared.as_ref());

    let session_key = derive_session_key(x25519_shared.as_bytes(), &mlkem_ss);

    Ok(HandshakeResult { session_key })
}
