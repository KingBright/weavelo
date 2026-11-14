//! PS5 Remote Play session/registration obfuscation & AES-128-CFB128 cipher (`rpcrypt`).

use aes::Aes128;
use aes::cipher::generic_array::GenericArray;
use aes::cipher::{BlockEncrypt, KeyInit};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use thiserror::Error;

use crate::keys_ps5::{HMAC_KEY_PS5, KEYS_A_PS5, KEYS_B_PS5, PS5_KEYS_0, PS5_KEYS_1};

type HmacSha256 = Hmac<Sha256>;

pub const RPCRYPT_KEY_SIZE: usize = 16;

#[derive(Debug, Error)]
pub enum RpcryptError {
    #[error("rpcrypt key index {0} out of range (must be < 32)")]
    KeyOffsetOutOfRange(usize),
}

#[derive(Debug, Clone)]
pub struct Rpcrypt {
    pub bright: [u8; RPCRYPT_KEY_SIZE],
    pub ambassador: [u8; RPCRYPT_KEY_SIZE],
}

impl Rpcrypt {
    /// Initializes registration encryption from the console `ambassador`, `key_0_off`, and 8-digit PIN.
    pub fn init_regist_ps5(
        ambassador: [u8; RPCRYPT_KEY_SIZE],
        key_0_off: usize,
        pin: u32,
    ) -> Result<Self, RpcryptError> {
        if key_0_off >= 0x20 {
            return Err(RpcryptError::KeyOffsetOutOfRange(key_0_off));
        }

        let mut bright = [0u8; RPCRYPT_KEY_SIZE];
        for i in 0..RPCRYPT_KEY_SIZE {
            bright[i] = PS5_KEYS_0[i * 0x20 + key_0_off];
        }
        let pin_be = pin.to_be_bytes();
        bright[0xc] ^= pin_be[0];
        bright[0xd] ^= pin_be[1];
        bright[0xe] ^= pin_be[2];
        bright[0xf] ^= pin_be[3];

        Ok(Self { bright, ambassador })
    }

    /// Initializes session control encryption from decoded `RP-Nonce` and random local `morning`.
    pub fn init_auth_ps5(
        nonce: &[u8; RPCRYPT_KEY_SIZE],
        morning: &[u8; RPCRYPT_KEY_SIZE],
    ) -> Self {
        let (bright, ambassador) = bright_ambassador_ps5(nonce, morning);
        Self { bright, ambassador }
    }

    /// Computes per-counter 16-byte IV using HMAC-SHA256(`HMAC_KEY_PS5`, `ambassador || counter_be`).
    pub fn generate_iv(&self, counter: u64) -> [u8; RPCRYPT_KEY_SIZE] {
        let mut mac =
            <HmacSha256 as Mac>::new_from_slice(&HMAC_KEY_PS5).expect("HMAC accepts 16-byte key");
        mac.update(&self.ambassador);
        mac.update(&counter.to_be_bytes());
        let digest: [u8; 32] = mac.finalize().into_bytes().into();
        let mut iv = [0u8; RPCRYPT_KEY_SIZE];
        iv.copy_from_slice(&digest[..RPCRYPT_KEY_SIZE]);
        iv
    }

    /// Encrypts bytes with AES-128-CFB128 under `(bright, iv(counter))`.
    pub fn encrypt(&self, counter: u64, input: &[u8]) -> Vec<u8> {
        let iv = self.generate_iv(counter);
        aes_128_cfb128_crypt(&self.bright, &iv, input, true)
    }

    /// Decrypts bytes with AES-128-CFB128 under `(bright, iv(counter))`.
    pub fn decrypt(&self, counter: u64, input: &[u8]) -> Vec<u8> {
        let iv = self.generate_iv(counter);
        aes_128_cfb128_crypt(&self.bright, &iv, input, false)
    }

    /// Encrypts `LaunchSpec` payloads by XORing plaintext with the encrypted all-zero keystream.
    pub fn encrypt_zero_keystream_xor(&self, counter: u64, plaintext: &[u8]) -> Vec<u8> {
        let zeros = vec![0u8; plaintext.len()];
        let mut keystream = self.encrypt(counter, &zeros);
        for (dst, &p) in keystream.iter_mut().zip(plaintext.iter()) {
            *dst ^= p;
        }
        keystream
    }
}

/// Derives PS5 `(bright, ambassador)` pair from decoded `RP-Nonce` and local `morning`.
pub fn bright_ambassador_ps5(
    nonce: &[u8; RPCRYPT_KEY_SIZE],
    morning: &[u8; RPCRYPT_KEY_SIZE],
) -> ([u8; RPCRYPT_KEY_SIZE], [u8; RPCRYPT_KEY_SIZE]) {
    let key_a = &KEYS_A_PS5[((nonce[0] >> 3) as usize) * 0x70..];
    let mut ambassador = [0u8; RPCRYPT_KEY_SIZE];
    for i in 0..RPCRYPT_KEY_SIZE {
        let v = nonce[i].wrapping_sub(0x2d).wrapping_sub(i as u8) ^ key_a[i];
        ambassador[i] = v;
    }

    let key_b = &KEYS_B_PS5[((nonce[7] >> 3) as usize) * 0x70..];
    let mut bright = [0u8; RPCRYPT_KEY_SIZE];
    for i in 0..RPCRYPT_KEY_SIZE {
        let v = morning[i]
            .wrapping_add(0x18)
            .wrapping_add(i as u8)
            ^ nonce[i]
            ^ key_b[i];
        bright[i] = v;
    }

    (bright, ambassador)
}

/// Computes `RP-Auth` registration `aeropause` from PS5 key table index `key_1_off` and `ambassador`.
pub fn aeropause_ps5(
    key_1_off: usize,
    ambassador: &[u8; RPCRYPT_KEY_SIZE],
) -> Result<[u8; RPCRYPT_KEY_SIZE], RpcryptError> {
    if key_1_off >= 0x20 {
        return Err(RpcryptError::KeyOffsetOutOfRange(key_1_off));
    }
    let wurzelbert = (-0x2d_i8) as u8;
    let mut out = [0u8; RPCRYPT_KEY_SIZE];
    for i in 0..RPCRYPT_KEY_SIZE {
        let k = PS5_KEYS_1[i * 0x20 + key_1_off];
        out[i] = (ambassador[i] ^ k)
            .wrapping_add(wurzelbert)
            .wrapping_add(i as u8);
    }
    Ok(out)
}

/// Reverses `aeropause` back into `ambassador` (`RP-Nonce` in regist response), matching Chiaki's server-side helper.
pub fn ambassador_from_aeropause_ps5(
    key_1_off: usize,
    aeropause: &[u8; RPCRYPT_KEY_SIZE],
) -> Result<[u8; RPCRYPT_KEY_SIZE], RpcryptError> {
    if key_1_off >= 0x20 {
        return Err(RpcryptError::KeyOffsetOutOfRange(key_1_off));
    }
    let wurzelbert = (-0x2d_i8) as u8;
    let mut out = [0u8; RPCRYPT_KEY_SIZE];
    for i in 0..RPCRYPT_KEY_SIZE {
        let k = PS5_KEYS_1[i * 0x20 + key_1_off];
        out[i] = aeropause[i]
            .wrapping_sub(wurzelbert)
            .wrapping_sub(i as u8)
            ^ k;
    }
    Ok(out)
}

fn aes_128_cfb128_crypt(
    key: &[u8; RPCRYPT_KEY_SIZE],
    iv: &[u8; RPCRYPT_KEY_SIZE],
    input: &[u8],
    encrypt: bool,
) -> Vec<u8> {
    let cipher = Aes128::new(GenericArray::from_slice(key));
    let mut feedback = *iv;
    let mut out = vec![0u8; input.len()];
    let mut n = 0usize;

    for (dst, &src) in out.iter_mut().zip(input.iter()) {
        if n == 0 {
            cipher.encrypt_block(GenericArray::from_mut_slice(&mut feedback));
        }
        let c = src ^ feedback[n];
        *dst = c;
        feedback[n] = if encrypt { c } else { src };
        n = (n + 1) & 0x0f;
    }

    out
}
