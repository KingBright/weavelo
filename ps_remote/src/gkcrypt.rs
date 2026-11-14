//! PS5 Takion stream cipher (AES-128-CTR with 128-bit LE counter) & 16-byte-IV AES-128-GMAC (`gkcrypt`).

use aes::Aes128;
use aes::cipher::generic_array::GenericArray;
use aes::cipher::{BlockEncrypt, KeyInit};
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

pub const GKCRYPT_BLOCK_SIZE: usize = 16;
pub const GKCRYPT_GMAC_SIZE: usize = 4;
pub const GMAC_KEY_REFRESH_KEY_POS: u64 = 45_000;
pub const GMAC_KEY_REFRESH_IV_OFFSET: u64 = 44_910;

/// Tracks high-order bits of Takion 32-bit `key_pos_low` wire counters across wraparounds.
#[derive(Debug, Default, Clone)]
pub struct KeyState {
    prev: u64,
}

impl KeyState {
    pub fn new() -> Self {
        Self { prev: 0 }
    }

    /// Expands a 32-bit wire `key_pos_low` to a 64-bit counter nearest `prev`.
    pub fn request_pos(&mut self, low: u32, commit: bool) -> u64 {
        let prev_high = self.prev >> 32;
        let low64 = low as u64;
        let mut best = (prev_high << 32) | low64;
        let mut best_dist = self.prev.abs_diff(best);

        for delta in [-1i64, 1i64] {
            let Some(cand_high) = prev_high.checked_add_signed(delta) else {
                continue;
            };
            let cand = (cand_high << 32) | low64;
            let dist = self.prev.abs_diff(cand);
            if dist < best_dist {
                best = cand;
                best_dist = dist;
            }
        }

        if commit {
            self.prev = best;
        }
        best
    }

    pub fn commit(&mut self, pos: u64) {
        self.prev = pos;
    }
}

#[derive(Debug, Clone)]
pub struct GkCrypt {
    pub index: u8,
    pub key_base: [u8; GKCRYPT_BLOCK_SIZE],
    pub iv: [u8; GKCRYPT_BLOCK_SIZE],
    pub key_gmac_base: [u8; GKCRYPT_BLOCK_SIZE],
    pub key_gmac_current: [u8; GKCRYPT_BLOCK_SIZE],
    pub key_gmac_index_current: u64,
}

impl GkCrypt {
    /// Derives base keystream key/IV and initial GMAC key from `handshake_key` and `ecdh_secret`.
    pub fn new(index: u8, handshake_key: &[u8; 16], ecdh_secret: &[u8; 32]) -> Self {
        let (key_base, iv) = gen_key_iv(index, handshake_key, ecdh_secret);
        let key_gmac_base = gen_gmac_key(0, &key_base, &iv);
        Self {
            index,
            key_base,
            iv,
            key_gmac_base,
            key_gmac_current: key_gmac_base,
            key_gmac_index_current: 0,
        }
    }

    /// Constructs a `GkCrypt` instance with an explicit current GMAC key and IV
    /// (matches Chiaki's raw GMAC test fixture where `key_gmac_base` is zero-initialized).
    pub fn from_raw_gmac(key_gmac_current: [u8; 16], iv: [u8; 16]) -> Self {
        Self {
            index: 0,
            key_base: [0; 16],
            iv,
            key_gmac_base: [0; 16],
            key_gmac_current,
            key_gmac_index_current: 0,
        }
    }

    /// Generates `out.len()` bytes of keystream starting at block `key_pos / 16` (`key_pos` in bytes).
    pub fn gen_key_stream(&self, key_pos: u64, out: &mut [u8]) {
        let cipher = Aes128::new(GenericArray::from_slice(&self.key_base));
        let start_block = key_pos / (GKCRYPT_BLOCK_SIZE as u64);
        let mut block_offset = 0u64;
        for chunk in out.chunks_mut(GKCRYPT_BLOCK_SIZE) {
            let mut blk = counter_add(&self.iv, start_block + block_offset);
            cipher.encrypt_block(GenericArray::from_mut_slice(&mut blk));
            chunk.copy_from_slice(&blk[..chunk.len()]);
            block_offset += 1;
        }
    }

    /// XORs `buf` in-place with keystream starting at byte position `key_pos`.
    pub fn crypt(&self, key_pos: u64, buf: &mut [u8]) {
        if buf.is_empty() {
            return;
        }
        let block_start = key_pos / (GKCRYPT_BLOCK_SIZE as u64);
        let byte_offset = (key_pos % (GKCRYPT_BLOCK_SIZE as u64)) as usize;
        let cipher = Aes128::new(GenericArray::from_slice(&self.key_base));

        let mut remaining = buf;
        let mut cur_block = block_start;
        let mut offset_in_block = byte_offset;

        while !remaining.is_empty() {
            let mut key_block = counter_add(&self.iv, cur_block);
            cipher.encrypt_block(GenericArray::from_mut_slice(&mut key_block));
            let take = (GKCRYPT_BLOCK_SIZE - offset_in_block).min(remaining.len());
            for i in 0..take {
                remaining[i] ^= key_block[offset_in_block + i];
            }
            remaining = &mut remaining[take..];
            cur_block += 1;
            offset_in_block = 0;
        }
    }

    /// Computes 4-byte Takion AES-128-GMAC over `buf` with 16-byte IV `counter_add(iv, key_pos)`.
    pub fn gmac(&mut self, key_pos: u64, buf: &[u8]) -> [u8; GKCRYPT_GMAC_SIZE] {
        let key_index = if key_pos > 0 {
            (key_pos - 1) / GMAC_KEY_REFRESH_KEY_POS
        } else {
            0
        };
        let key = if key_index == self.key_gmac_index_current {
            self.key_gmac_current
        } else {
            let derived = gen_gmac_key(key_index, &self.key_gmac_base, &self.iv);
            if key_index > self.key_gmac_index_current {
                self.key_gmac_index_current = key_index;
                self.key_gmac_current = derived;
            }
            derived
        };

        let iv = counter_add(&self.iv, key_pos / (GKCRYPT_BLOCK_SIZE as u64));
        aes_128_gmac_iv16(&key, &iv, buf)
    }
}

/// 128-bit little-endian addition: `counter + offset`.
#[inline]
pub fn counter_add(iv: &[u8; 16], offset: u64) -> [u8; 16] {
    u128::from_le_bytes(*iv)
        .wrapping_add(offset as u128)
        .to_le_bytes()
}

pub fn gen_key_iv(index: u8, handshake_key: &[u8; 16], ecdh_secret: &[u8; 32]) -> ([u8; 16], [u8; 16]) {
    let mut mac =
        <HmacSha256 as Mac>::new_from_slice(ecdh_secret).expect("HMAC accepts 32-byte secret");
    mac.update(&[1u8, index, 0u8]);
    mac.update(handshake_key);
    mac.update(&[1u8, 0u8]);
    let digest: [u8; 32] = mac.finalize().into_bytes().into();
    let mut key_base = [0u8; 16];
    let mut iv = [0u8; 16];
    key_base.copy_from_slice(&digest[..16]);
    iv.copy_from_slice(&digest[16..32]);
    (key_base, iv)
}

pub fn gen_gmac_key(index: u64, key_base: &[u8; 16], iv: &[u8; 16]) -> [u8; 16] {
    let offset = index.saturating_mul(GMAC_KEY_REFRESH_IV_OFFSET);
    let c = counter_add(iv, offset);
    let mut hasher = Sha256::new();
    hasher.update(key_base);
    hasher.update(c);
    let digest: [u8; 32] = hasher.finalize().into();
    let mut key_out = [0u8; 16];
    for i in 0..16 {
        key_out[i] = digest[i] ^ digest[i + 16];
    }
    key_out
}

/// NIST SP 800-38D AES-128-GMAC supporting 16-byte IVs and empty plaintext.
fn aes_128_gmac_iv16(key: &[u8; 16], iv: &[u8; 16], aad: &[u8]) -> [u8; 4] {
    let cipher = Aes128::new(GenericArray::from_slice(key));
    let mut h_bytes = [0u8; 16];
    cipher.encrypt_block(GenericArray::from_mut_slice(&mut h_bytes));
    let h = u128::from_be_bytes(h_bytes);

    // Non-96-bit IV: J0 = GHASH(H, IV || 0^64 || [bit_len(IV)]_64)
    let mut j0 = 0u128;
    j0 = ghash_mul(j0 ^ u128::from_be_bytes(*iv), h);
    j0 = ghash_mul(j0 ^ (128u128), h);

    // Authenticate AAD with empty ciphertext: S = GHASH(H, pad(AAD) || [bit_len(AAD)]_64 || [0]_64)
    let mut s = 0u128;
    for chunk in aad.chunks(16) {
        let mut blk = [0u8; 16];
        blk[..chunk.len()].copy_from_slice(chunk);
        s = ghash_mul(s ^ u128::from_be_bytes(blk), h);
    }
    let len_block = ((aad.len() as u128) * 8) << 64;
    s = ghash_mul(s ^ len_block, h);

    let mut enc_j0 = j0.to_be_bytes();
    cipher.encrypt_block(GenericArray::from_mut_slice(&mut enc_j0));
    let tag = (u128::from_be_bytes(enc_j0) ^ s).to_be_bytes();
    [tag[0], tag[1], tag[2], tag[3]]
}

#[inline]
fn ghash_mul(x: u128, h: u128) -> u128 {
    let mut z = 0u128;
    let mut v = h;
    for i in (0..128).rev() {
        if ((x >> i) & 1) != 0 {
            z ^= v;
        }
        let lsb = (v & 1) != 0;
        v >>= 1;
        if lsb {
            v ^= 0xe100_0000_0000_0000_0000_0000_0000_0000u128;
        }
    }
    z
}
