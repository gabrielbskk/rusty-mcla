//! Decryption of a PS3 save file protected by `PARAM.PFD` (port of `tools/ps3dec.py`, the scheme of flatz's pfdtool).
//!
//! A PS3 save folder holds `PARAM.SFO`, `PARAM.PFD` (the protected-file database), `ICON0.PNG` and `RAGE.SAV`. The PFD lists every protected file with a
//! 64-byte file key, itself AES-CBC encrypted with the console's *syscon manager key*; that key is decrypted with the same key using an IV made from
//! the game's *secure file id*. The file is then AES-ECB with a counter-block whitening (`out = D(in) ^ E(block number)`).
//!
//! **Key material.** [`SYSCON_KEY`] is the PS3's public save-data manager key (it is the same on every console and widely published) and
//! [`SECURE_FILE_ID`] is the id of this game's saves as listed in the community `games.conf` (the **US release, BLUS30190**; the other regions have
//! their own ids, which have not been looked up, so a save from another region is reported as "cannot decrypt"). The user allowed these constants in
//! the launcher on 2026-10-15 (docs/LAUNCHER.md, decision 18; DECISIONS.md D17). Nothing here can decrypt anything but a save of this game.

use crate::{Error, Result};
use aes::cipher::generic_array::GenericArray;
use aes::cipher::{BlockDecrypt, BlockEncrypt, KeyInit};
use aes::Aes128;

/// The PS3's save-data manager ("syscon") key.
pub const SYSCON_KEY: [u8; 16] = [0xD4, 0x13, 0xB8, 0x96, 0x63, 0xE1, 0xFE, 0x9F, 0x75, 0x14, 0x3D, 0x3B, 0xB4, 0x56, 0x52, 0x74];
/// Secure file id of Midnight Club: Los Angeles (BLUS30190).
pub const SECURE_FILE_ID: [u8; 16] = [0x9B, 0x95, 0x45, 0xED, 0x1C, 0x6B, 0x5A, 0xFD, 0xAC, 0x01, 0x2D, 0x15, 0x3F, 0xB2, 0x2C, 0x0A];
/// How a decrypted save starts (the plain save's first twelve bytes and the first entry's name): version 15, nine entries, the 5-letter name `Stat..`.
const KNOWN: [u8; 16] = [0, 0, 0, 0x0F, 0, 0, 0, 9, 0, 0, 0, 5, b'S', b't', b'a', b't'];

const ENTRY: usize = 272;

fn cipher(key: &[u8]) -> Aes128 {
    Aes128::new(GenericArray::from_slice(&key[..16]))
}

fn xor16(a: &[u8], b: &[u8]) -> [u8; 16] {
    let mut o = [0u8; 16];
    for i in 0..16 {
        o[i] = a[i] ^ b[i];
    }
    o
}

fn cbc_decrypt(key: &[u8], iv: &[u8; 16], data: &[u8]) -> Vec<u8> {
    let c = cipher(key);
    let mut prev = *iv;
    let mut out = Vec::with_capacity(data.len());
    for chunk in data.chunks_exact(16) {
        let mut b = GenericArray::clone_from_slice(chunk);
        c.decrypt_block(&mut b);
        out.extend_from_slice(&xor16(&b, &prev));
        prev.copy_from_slice(chunk);
    }
    out
}

/// One 16-byte block of the file: `D(in) ^ E(block number)`.
fn file_block(c: &Aes128, data: &[u8], i: usize) -> [u8; 16] {
    let mut ctr = [0u8; 16];
    ctr[..8].copy_from_slice(&(i as u64).to_be_bytes());
    let mut ctr = GenericArray::clone_from_slice(&ctr);
    c.encrypt_block(&mut ctr);
    let mut b = GenericArray::clone_from_slice(&data[i * 16..i * 16 + 16]);
    c.decrypt_block(&mut b);
    xor16(&b, &ctr)
}

fn be64(b: &[u8], o: usize) -> u64 {
    b.get(o..o + 8).map(|s| u64::from_be_bytes(s.try_into().unwrap_or([0; 8]))).unwrap_or(0)
}

/// The IV that decrypts the file keys: four fixed bytes with the secure file id's bytes laid in around them (`pfd_generate_hash_key_for_secure_file_id`).
fn secure_iv() -> [u8; 16] {
    let mut ins = [0u8; 16];
    let mut j = 0;
    for (i, slot) in ins.iter_mut().enumerate() {
        *slot = match i {
            1 => 11,
            2 => 15,
            5 => 14,
            8 => 10,
            _ => {
                j += 1;
                SECURE_FILE_ID[j - 1]
            }
        };
    }
    ins
}

/// The files `pfd` (the bytes of `PARAM.PFD`) protects: (name, size, file key).
fn entries(pfd: &[u8]) -> Result<Vec<(String, usize, [u8; 64])>> {
    if pfd.len() < 0x78 || &pfd[4..8] != b"PFDB" {
        return Err(Error::new("pfd", "PARAM.PFD is not a protected-file database"));
    }
    let (cap, used) = (be64(pfd, 0x60), be64(pfd, 0x70));
    if cap > 1 << 16 || used > cap {
        return Err(Error::new("pfd", "PARAM.PFD is damaged"));
    }
    let ent0 = 0x78 + cap as usize * 8;
    let mut out = Vec::new();
    for k in 0..used as usize {
        let Some(e) = pfd.get(ent0 + k * ENTRY..ent0 + (k + 1) * ENTRY) else { break };
        let name: String = e[8..73].iter().take_while(|c| **c != 0).map(|c| *c as char).collect();
        if !name.is_empty() {
            let mut key = [0u8; 64];
            key.copy_from_slice(&e[80..144]);
            out.push((name, be64(e, 264) as usize, key));
        }
    }
    Ok(out)
}

/// Names of the protected files, for choosing the save among them.
pub fn protected_files(pfd: &[u8]) -> Result<Vec<String>> {
    Ok(entries(pfd)?.into_iter().map(|e| e.0).collect())
}

/// Decrypts the protected file `name` (as `PARAM.PFD` lists it, compared without case) whose encrypted bytes are `data`.
/// `Err("pfd")` = the database is not readable or does not list the file; `Err("key")` = the file does not decrypt to a save of this game
/// (another game, another region's secure file id, or a damaged file).
pub fn decrypt(pfd: &[u8], name: &str, data: &[u8]) -> Result<Vec<u8>> {
    let list = entries(pfd)?;
    let Some((_, size, file_key)) = list.iter().find(|e| e.0.eq_ignore_ascii_case(name)) else {
        return Err(Error::new("pfd", format!("PARAM.PFD does not list {name}")));
    };
    // the file key is AES-CBC encrypted with the console key, the secure-id IV; its first sixteen bytes are this file's key
    let key = cbc_decrypt(&SYSCON_KEY, &secure_iv(), file_key);
    let c = cipher(&key[..16]);
    if data.len() < 16 || file_block(&c, data, 0) != KNOWN {
        return Err(Error::new("key", format!("{name} does not decrypt to a save of this game")));
    }
    let mut plain = Vec::with_capacity(data.len());
    for i in 0..data.len() / 16 {
        plain.extend_from_slice(&file_block(&c, data, i));
    }
    plain.truncate((*size).min(plain.len()));
    Ok(plain)
}

/// Test helper: builds a `PARAM.PFD` that lists `name` with a file key made from `key`, and encrypts `plain` the way a console does.
#[doc(hidden)]
pub mod testing {
    use super::*;
    use aes::cipher::generic_array::GenericArray as G;

    fn cbc_encrypt(key: &[u8], iv: &[u8; 16], data: &[u8]) -> Vec<u8> {
        let c = cipher(key);
        let mut prev = *iv;
        let mut out = Vec::new();
        for chunk in data.chunks_exact(16) {
            let mut b = G::clone_from_slice(&xor16(chunk, &prev));
            c.encrypt_block(&mut b);
            prev.copy_from_slice(&b);
            out.extend_from_slice(&b);
        }
        out
    }

    /// `(PARAM.PFD bytes, encrypted file)` for a file `name` of content `plain`.
    pub fn protect(name: &str, plain: &[u8], key: [u8; 16]) -> (Vec<u8>, Vec<u8>) {
        let mut padded = plain.to_vec();
        padded.resize((plain.len() + 15) / 16 * 16, 0);
        let c = cipher(&key);
        let mut enc = Vec::new();
        for i in 0..padded.len() / 16 {
            let mut ctr = [0u8; 16];
            ctr[..8].copy_from_slice(&(i as u64).to_be_bytes());
            let mut ctr = G::clone_from_slice(&ctr);
            c.encrypt_block(&mut ctr);
            let mut b = G::clone_from_slice(&xor16(&padded[i * 16..i * 16 + 16], &ctr));
            c.encrypt_block(&mut b);
            enc.extend_from_slice(&b);
        }
        let mut file_key = [0u8; 64];
        file_key[..16].copy_from_slice(&key);
        let file_key = cbc_encrypt(&SYSCON_KEY, &secure_iv(), &file_key);
        let cap = 2u64;
        let mut pfd = vec![0u8; 0x78 + cap as usize * 8 + ENTRY * cap as usize];
        pfd[4..8].copy_from_slice(b"PFDB");
        pfd[0x60..0x68].copy_from_slice(&cap.to_be_bytes());
        pfd[0x70..0x78].copy_from_slice(&1u64.to_be_bytes());
        let e0 = 0x78 + cap as usize * 8;
        pfd[e0 + 8..e0 + 8 + name.len()].copy_from_slice(name.as_bytes());
        pfd[e0 + 80..e0 + 144].copy_from_slice(&file_key);
        pfd[e0 + 264..e0 + 272].copy_from_slice(&(plain.len() as u64).to_be_bytes());
        (pfd, enc)
    }
}
