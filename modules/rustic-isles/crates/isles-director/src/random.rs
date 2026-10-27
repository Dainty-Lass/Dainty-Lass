//! Random ids and choices (`crypto.randomUUID`, `crypto.randomInt`), from the system's generator.

/// A version 4 UUID in lower-case hex, as `crypto.randomUUID()` writes it.
pub fn uuid_v4() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// An integer in `[min, max)`, without modulo bias.
pub fn int_in(min: usize, max: usize) -> usize {
    let range = max.saturating_sub(min) as u64;
    if range <= 1 {
        return min;
    }
    let zone = u64::MAX - (u64::MAX % range);
    loop {
        let bytes = uuid::Uuid::new_v4().into_bytes();
        let mut word = [0u8; 8];
        word.copy_from_slice(&bytes[..8]);
        let n = u64::from_le_bytes(word);
        if n < zone {
            return min + (n % range) as usize;
        }
    }
}
