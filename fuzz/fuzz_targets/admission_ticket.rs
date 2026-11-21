// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The join tickets a host's transport and the backend read (spec-rd27-rifts-admission-relay.md §3.2): a payload that
//! decodes is within the transport's ranges and is the one canonical encoding of its claims, and a signature check
//! never changes what the framing checks read.
#![no_main]

use std::sync::OnceLock;

use libfuzzer_sys::fuzz_target;
use rifts_admission::keys::signing_key_from_pem;
use rifts_admission::ticket::{decode_claims, encode_claims, parse_unverified};
use rifts_admission::{Es256Tickets, KeyId, P256KeyGenerator, TicketClaims};

fn authority() -> &'static Es256Tickets {
    static TICKETS: OnceLock<Es256Tickets> = OnceLock::new();
    TICKETS.get_or_init(|| {
        let (_, private_pem, _) = P256KeyGenerator
            .generate_with_pems(KeyId("fuzz".to_owned()))
            .expect("a new key");
        let signing = signing_key_from_pem(private_pem.expose()).expect("the key reads back");
        Es256Tickets {
            key_id: KeyId("fuzz".to_owned()),
            verifying: *signing.verifying_key(),
            signing: Some(signing),
            default_lifetime: 75.0,
        }
    })
}

fn in_transport_ranges(claims: &TicketClaims) {
    let bindings = &claims.bindings;
    assert!((1..=32).contains(&claims.key_id.0.len()));
    assert!((1..=96).contains(&bindings.account.0.len()));
    assert!((1..=128).contains(&bindings.hunt_definition_id.len()));
    assert!((1..=192).contains(&bindings.party_id.len()));
    assert!((1..=999).contains(&bindings.virtual_port));
    assert!(bindings.party_revision < 1 << 53);
    assert_ne!(bindings.joiner_peer.0, 0);
    assert_ne!(bindings.host_peer.0, 0);
    assert!((1..=5).contains(&(bindings.session.0[6] >> 4)));
    assert!((0x8..=0xb).contains(&(bindings.session.0[8] >> 4)));
}

fuzz_target!(|data: &[u8]| {
    if let Ok(claims) = decode_claims(data) {
        in_transport_ranges(&claims);
        assert_eq!(encode_claims(&claims).as_deref(), Ok(data));
    }
    let Ok(token) = std::str::from_utf8(data) else {
        return;
    };
    let Ok(unverified) = parse_unverified(token) else {
        return;
    };
    in_transport_ranges(&unverified);
    if let Ok(verified) = authority().verify(token, unverified.issued_at) {
        assert_eq!(verified, unverified);
    }
});
