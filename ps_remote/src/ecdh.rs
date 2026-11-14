//! secp256k1 ECDH key exchange with HMAC-SHA256 public key signatures.

use hmac::{Hmac, Mac};
use k256::ecdh::diffie_hellman;
use k256::elliptic_curve::sec1::ToEncodedPoint;
use k256::{PublicKey, SecretKey};
use rand::Rng;
use sha2::Sha256;
use thiserror::Error;

type HmacSha256 = Hmac<Sha256>;

pub const ECDH_SECRET_SIZE: usize = 32;
pub const ECDH_PUB_KEY_SIZE: usize = 65;
pub const HANDSHAKE_KEY_SIZE: usize = 16;

#[derive(Debug, Error)]
pub enum EcdhError {
    #[error("invalid secp256k1 key material")]
    InvalidKey,
    #[error("remote public key signature mismatch")]
    SignatureMismatch,
}

pub struct Ecdh {
    secret: SecretKey,
    public: PublicKey,
}

impl Ecdh {
    /// Generates a fresh random secp256k1 keypair.
    pub fn generate() -> Self {
        loop {
            let mut raw = [0u8; 32];
            rand::rng().fill_bytes(&mut raw);
            if let Ok(secret) = SecretKey::from_slice(&raw) {
                let public = secret.public_key();
                return Self { secret, public };
            }
        }
    }

    /// Reconstructs a keypair from raw private and uncompressed public key bytes.
    pub fn from_keypair(priv_bytes: &[u8; 32], pub_bytes: &[u8; 65]) -> Result<Self, EcdhError> {
        let secret = SecretKey::from_slice(priv_bytes).map_err(|_| EcdhError::InvalidKey)?;
        let public = PublicKey::from_sec1_bytes(pub_bytes).map_err(|_| EcdhError::InvalidKey)?;
        if secret.public_key() != public {
            return Err(EcdhError::InvalidKey);
        }
        Ok(Self { secret, public })
    }

    /// Returns uncompressed 65-byte SEC1 public key (`0x04 || X || Y`) and HMAC-SHA256 signature.
    pub fn local_pub_key_and_sig(
        &self,
        handshake_key: &[u8; HANDSHAKE_KEY_SIZE],
    ) -> ([u8; ECDH_PUB_KEY_SIZE], [u8; 32]) {
        let encoded = self.public.to_encoded_point(false);
        let mut pub_key = [0u8; ECDH_PUB_KEY_SIZE];
        pub_key.copy_from_slice(encoded.as_bytes());

        let mut mac = <HmacSha256 as Mac>::new_from_slice(handshake_key)
            .expect("HMAC accepts 16-byte handshake key");
        mac.update(&pub_key);
        let sig: [u8; 32] = mac.finalize().into_bytes().into();
        (pub_key, sig)
    }

    /// Verifies optional remote HMAC signature and derives the 32-byte raw ECDH shared secret.
    pub fn derive_secret(
        &self,
        remote_pub_key: &[u8],
        handshake_key: &[u8; HANDSHAKE_KEY_SIZE],
        remote_sig: Option<&[u8]>,
    ) -> Result<[u8; ECDH_SECRET_SIZE], EcdhError> {
        if let Some(sig) = remote_sig {
            let mut mac = <HmacSha256 as Mac>::new_from_slice(handshake_key)
                .expect("HMAC accepts 16-byte handshake key");
            mac.update(remote_pub_key);
            mac.verify_slice(sig)
                .map_err(|_| EcdhError::SignatureMismatch)?;
        }

        let remote_pk =
            PublicKey::from_sec1_bytes(remote_pub_key).map_err(|_| EcdhError::InvalidKey)?;
        let shared = diffie_hellman(self.secret.to_nonzero_scalar(), remote_pk.as_affine());
        let mut secret = [0u8; ECDH_SECRET_SIZE];
        secret.copy_from_slice(shared.raw_secret_bytes());
        Ok(secret)
    }
}
