//! SHA-256 for the evidence chain, the traffic window ids and the capability digests: RustCrypto's `sha2` behind the
//! chaining API the callers use.

use sha2::Digest as _;

#[derive(Clone, Default)]
pub struct Sha256(sha2::Sha256);

impl Sha256 {
    pub fn update(&mut self, data: &[u8]) -> &mut Self {
        self.0.update(data);
        self
    }

    pub fn finish(self) -> [u8; 32] {
        self.0.finalize().into()
    }

    pub fn finish_hex(self) -> String {
        hex(&self.finish())
    }
}

pub fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from(DIGITS[usize::from(byte >> 4)]));
        out.push(char::from(DIGITS[usize::from(byte & 15)]));
    }
    out
}

pub fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::default();
    hasher.update(data);
    hasher.finish_hex()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_vectors() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha256_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
        assert_eq!(
            sha256_hex(&[b'a'; 1_000_000]),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
    }

    #[test]
    fn split_updates_match_one_update() {
        let data: Vec<u8> = (0..=255u8).cycle().take(1000).collect();
        for split in [0, 1, 55, 56, 63, 64, 65, 999] {
            let mut hasher = Sha256::default();
            hasher.update(&data[..split]).update(&data[split..]);
            assert_eq!(hasher.finish_hex(), sha256_hex(&data), "split at {split}");
        }
    }
}
