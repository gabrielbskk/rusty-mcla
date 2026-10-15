//! RPF3 table-of-contents / script payload encryption: AES-256-ECB applied 16 times.
//! The key is not part of this project: it is derived from the user's own disc ([`crate::keys`]).

use aes::Aes256;
use cipher::{BlockDecrypt, KeyInit};

/// Decrypt `data` in place (a trailing partial block is left alone).
pub fn decrypt_toc(data: &mut [u8], key: &[u8; 32]) {
    let cipher = Aes256::new(key.into());
    let whole = data.len() / 16 * 16;
    for _round in 0..16 {
        for chunk in data[..whole].chunks_exact_mut(16) {
            cipher.decrypt_block(chunk.into());
        }
    }
}
