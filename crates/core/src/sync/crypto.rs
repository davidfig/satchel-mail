//! The sync key, the sync code that carries it between computers, and
//! sealing each computer's file (0080).

use anyhow::{Result, anyhow, bail};
use aws_lc_rs::aead::{Aad, CHACHA20_POLY1305, LessSafeKey, NONCE_LEN, Nonce, UnboundKey};
use aws_lc_rs::rand::{SecureRandom, SystemRandom};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;

pub const KEY_LEN: usize = 32;
const KEY_CHECK_LABEL: &[u8] = b"satchel-sync key check v1";
const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

pub type Key = [u8; KEY_LEN];

pub fn new_key() -> Result<Key> {
    let mut k = [0u8; KEY_LEN];
    SystemRandom::new().fill(&mut k).map_err(|_| anyhow!("no randomness"))?;
    Ok(k)
}

/// The key as a sync code: base32 in groups of four, e.g. `ABCD-EFGH-…`.
pub fn code(key: &Key) -> String {
    let mut out = String::new();
    let (mut buf, mut bits) = (0u32, 0u32);
    for &b in key {
        buf = (buf << 8) | b as u32;
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(ALPHABET[((buf >> bits) & 31) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(ALPHABET[((buf << (5 - bits)) & 31) as usize] as char);
    }
    out.as_bytes().chunks(4).map(|c| std::str::from_utf8(c).unwrap()).collect::<Vec<_>>().join("-")
}

/// Reads a sync code as typed: any case, dashes and spaces ignored, and the
/// digits people mistake for letters (0, 1, 8) read as those letters.
pub fn parse_code(code: &str) -> Result<Key> {
    let mut bytes = Vec::with_capacity(KEY_LEN + 1);
    let (mut buf, mut bits) = (0u32, 0u32);
    for ch in code.chars().filter(|c| !c.is_whitespace() && *c != '-') {
        let ch = match ch.to_ascii_uppercase() {
            '0' => 'O',
            '1' => 'I',
            '8' => 'B',
            c => c,
        };
        let Some(v) = ALPHABET.iter().position(|&a| a as char == ch) else {
            bail!("a sync code has only letters A–Z and digits 2–7");
        };
        buf = (buf << 5) | v as u32;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            bytes.push((buf >> bits) as u8);
        }
        buf &= (1 << bits) - 1;
    }
    bytes.try_into().map_err(|_| anyhow!("that sync code is the wrong length"))
}

/// What the folder's header stores, so a wrong code is refused at join.
pub fn key_check(key: &Key) -> String {
    blake3::keyed_hash(key, KEY_CHECK_LABEL).to_hex().to_string()
}

fn aead(key: &Key) -> Result<LessSafeKey> {
    Ok(LessSafeKey::new(UnboundKey::new(&CHACHA20_POLY1305, key).map_err(|_| anyhow!("bad key"))?))
}

/// Seals `plain` for the computer `device`: base64 nonce and ciphertext.
pub fn seal(key: &Key, device: &str, plain: &[u8]) -> Result<(String, String)> {
    let mut n = [0u8; NONCE_LEN];
    SystemRandom::new().fill(&mut n).map_err(|_| anyhow!("no randomness"))?;
    let mut data = plain.to_vec();
    aead(key)?
        .seal_in_place_append_tag(Nonce::assume_unique_for_key(n), Aad::from(device.as_bytes()), &mut data)
        .map_err(|_| anyhow!("couldn't encrypt"))?;
    Ok((B64.encode(n), B64.encode(data)))
}

/// Opens what `seal` made for `device`. Fails if the key is wrong or the file
/// was changed or claims another computer's name.
pub fn open(key: &Key, device: &str, nonce: &str, data: &str) -> Result<Vec<u8>> {
    let n: [u8; NONCE_LEN] = B64.decode(nonce)?.try_into().map_err(|_| anyhow!("bad nonce"))?;
    let mut data = B64.decode(data)?;
    let plain = aead(key)?
        .open_in_place(Nonce::assume_unique_for_key(n), Aad::from(device.as_bytes()), &mut data)
        .map_err(|_| anyhow!("couldn't decrypt (another sync code, or the file was changed)"))?;
    Ok(plain.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_round_trips_as_typed() {
        let k = new_key().unwrap();
        let c = code(&k);
        assert_eq!(c.len(), 52 + 12);
        assert_eq!(parse_code(&c).unwrap(), k);
        assert_eq!(parse_code(&c.to_lowercase().replace('-', " ")).unwrap(), k);
        assert!(parse_code("ABCD").is_err());
    }

    #[test]
    fn sealed_for_one_device_only() {
        let k = new_key().unwrap();
        let (n, d) = seal(&k, "dev-a", b"hello").unwrap();
        assert_eq!(open(&k, "dev-a", &n, &d).unwrap(), b"hello");
        assert!(open(&k, "dev-b", &n, &d).is_err());
        assert!(open(&new_key().unwrap(), "dev-a", &n, &d).is_err());
    }
}
