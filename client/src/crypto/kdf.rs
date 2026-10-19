use hkdf::Hkdf;
use sha2::Sha256;

const SALT: &[u8] = b"stalinium-c2-hybrid-kex-v1";
const INFO: &[u8] = b"session-key";

pub fn derive_session_key(x25519_shared: &[u8; 32], mlkem_shared: &[u8; 32]) -> [u8; 32] {
    let mut ikm = Vec::with_capacity(64);
    ikm.extend_from_slice(x25519_shared);
    ikm.extend_from_slice(mlkem_shared);

    let hk = Hkdf::<Sha256>::new(Some(SALT), &ikm);
    let mut okm = [0u8; 32];
    hk.expand(INFO, &mut okm)
        .expect("HKDF-SHA256 expand failed for 32-byte output");
    okm
}
