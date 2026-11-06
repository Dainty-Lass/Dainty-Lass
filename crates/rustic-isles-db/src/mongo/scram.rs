// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! SCRAM-SHA-256 (RFC 7677; spec §4.6). Two departures from restricted, both security (coordinator item 5): the
//! server's signature `v=` is checked, and an iteration count above [`MAX_ITERATIONS`] is refused.

use base64::Engine as _;
use hmac::{Hmac, KeyInit as _, Mac as _};
use sha2::{Digest as _, Sha256};

/// A server asking for more is a CPU sink, not a stronger hash.
pub const MAX_ITERATIONS: u32 = 1_000_000;

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

fn hmac(key: &[u8], data: &[u8]) -> [u8; 32] {
    // HMAC takes a key of any length
    let Ok(mut mac) = <Hmac<Sha256>>::new_from_slice(key) else {
        return [0; 32];
    };
    mac.update(data);
    mac.finalize().into_bytes().into()
}

fn salted_password(password: &[u8], salt: &[u8], iterations: u32) -> [u8; 32] {
    let mut block = salt.to_vec();
    block.extend_from_slice(&1u32.to_be_bytes());
    let mut u = hmac(password, &block);
    let mut out = u;
    for _ in 1..iterations {
        u = hmac(password, &u);
        for (o, b) in out.iter_mut().zip(u) {
            *o ^= b;
        }
    }
    out
}

/// One conversation: the client nonce in, the client-first, client-final and the server signature to expect.
pub struct Scram {
    password: String,
    nonce: String,
    first_bare: String,
    server_signature: Option<[u8; 32]>,
}

impl Scram {
    pub fn new(user: &str, password: &str, nonce: String) -> Self {
        let user = user.replace('=', "=3D").replace(',', "=2C");
        Self {
            password: password.to_owned(),
            first_bare: format!("n={user},r={nonce}"),
            nonce,
            server_signature: None,
        }
    }

    /// A nonce from 24 random bytes.
    pub fn random_nonce() -> String {
        B64.encode(rand::random::<[u8; 24]>())
    }

    pub fn client_first(&self) -> String {
        format!("n,,{}", self.first_bare)
    }

    /// The client-final for the server-first message.
    pub fn client_final(&mut self, server_first: &[u8]) -> Result<String, String> {
        let text = std::str::from_utf8(server_first).map_err(|e| format!("invalid UTF-8 in saslStart: {e}"))?;
        let (mut nonce, mut salt, mut iterations) = (None, None, None);
        for part in text.split(',') {
            if let Some(v) = part.strip_prefix("r=") {
                nonce = Some(v);
            } else if let Some(v) = part.strip_prefix("s=") {
                salt = Some(v);
            } else if let Some(v) = part.strip_prefix("i=") {
                iterations = v.parse::<u32>().ok();
            }
        }
        let nonce = nonce.ok_or("missing r= in SCRAM")?;
        let salt = B64
            .decode(salt.ok_or("missing s= in SCRAM")?)
            .map_err(|e| format!("invalid salt base64: {e}"))?;
        let iterations = iterations.ok_or("missing i= in SCRAM")?;
        if iterations == 0 || iterations > MAX_ITERATIONS {
            return Err(format!(
                "SCRAM iteration count {iterations} is outside 1 to {MAX_ITERATIONS}"
            ));
        }
        if !nonce.starts_with(&self.nonce) {
            return Err("server nonce did not prefix client nonce".into());
        }
        let salted = salted_password(self.password.as_bytes(), &salt, iterations);
        let client_key = hmac(&salted, b"Client Key");
        let stored_key: [u8; 32] = Sha256::digest(client_key).into();
        let without_proof = format!("c=biws,r={nonce}");
        let auth_message = format!("{},{text},{without_proof}", self.first_bare);
        let signature = hmac(&stored_key, auth_message.as_bytes());
        let proof: Vec<u8> = client_key.iter().zip(signature).map(|(k, s)| k ^ s).collect();
        let server_key = hmac(&salted, b"Server Key");
        self.server_signature = Some(hmac(&server_key, auth_message.as_bytes()));
        Ok(format!("{without_proof},p={}", B64.encode(proof)))
    }

    /// Checks the server-final message's `v=` against the signature this conversation expects.
    pub fn verify_server_final(&self, server_final: &[u8]) -> Result<(), String> {
        let expected = self.server_signature.ok_or("server-final before server-first")?;
        let text = std::str::from_utf8(server_final).map_err(|e| format!("invalid UTF-8 in saslContinue: {e}"))?;
        let mut signature = None;
        for part in text.split(',') {
            if let Some(e) = part.strip_prefix("e=") {
                return Err(format!("server refused the proof: {e}"));
            }
            if let Some(v) = part.strip_prefix("v=") {
                signature = Some(v);
            }
        }
        let signature = B64
            .decode(signature.ok_or("missing v= in SCRAM")?)
            .map_err(|e| format!("invalid server signature base64: {e}"))?;
        if signature.len() != expected.len()
            || signature.iter().zip(expected).fold(0u8, |acc, (a, b)| acc | (a ^ b)) != 0
        {
            return Err("server signature did not match".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // RFC 7677 section 3's example conversation
    const NONCE: &str = "rOprNGfwEbeRWgbNEkqO";
    const SERVER_FIRST: &str = "r=rOprNGfwEbeRWgbNEkqO%hvYDpWUa2RaTCAfuxFIlj)hNlF$k0,s=W22ZaJ0SNY7soEsUEjb6gQ==,i=4096";

    #[test]
    fn rfc_7677_vector() {
        let mut scram = Scram::new("user", "pencil", NONCE.into());
        assert_eq!(scram.client_first(), "n,,n=user,r=rOprNGfwEbeRWgbNEkqO");
        assert_eq!(
            scram.client_final(SERVER_FIRST.as_bytes()).unwrap(),
            "c=biws,r=rOprNGfwEbeRWgbNEkqO%hvYDpWUa2RaTCAfuxFIlj)hNlF$k0,p=dHzbZapWIk4jUhN+Ute9ytag9zjfMHgsqmmiz7AndVQ="
        );
        scram
            .verify_server_final(b"v=6rriTRBi23WpRR/wtup+mMhUZUn/dB5nLTJRsjl95G4=")
            .unwrap();
        assert_eq!(
            scram
                .verify_server_final(b"v=AAAATRBi23WpRR/wtup+mMhUZUn/dB5nLTJRsjl95G4=")
                .unwrap_err(),
            "server signature did not match"
        );
        assert_eq!(scram.verify_server_final(b"").unwrap_err(), "missing v= in SCRAM");
    }

    #[test]
    fn iteration_cap_and_malformed_server_first() {
        let mut scram = Scram::new("user", "pencil", NONCE.into());
        let greedy = SERVER_FIRST.replace("i=4096", "i=1000001");
        assert!(
            scram
                .client_final(greedy.as_bytes())
                .unwrap_err()
                .contains("iteration count 1000001")
        );
        let wrong = SERVER_FIRST.replace("rOprNGfwEbeRWgbNEkqO%", "other%");
        assert_eq!(
            scram.client_final(wrong.as_bytes()).unwrap_err(),
            "server nonce did not prefix client nonce"
        );
        assert_eq!(scram.client_final(b"s=AA==,i=1").unwrap_err(), "missing r= in SCRAM");
        assert_eq!(scram.client_final(b"r=x,i=1").unwrap_err(), "missing s= in SCRAM");
        assert_eq!(scram.client_final(b"r=x,s=AA==").unwrap_err(), "missing i= in SCRAM");
    }

    #[test]
    fn user_name_is_escaped() {
        assert_eq!(
            Scram::new("a=b,c", "p", "n".into()).client_first(),
            "n,,n=a=3Db=2Cc,r=n"
        );
    }
}
