//! MPT2 admission tickets (RF4): the token text `ConsumeAdmissionTicket` reads from a host's transport and the raw
//! payload under it. It checks that only the canonical payload for its claims decodes, that every accepted claim is in
//! the range the transport takes, and that a verification under a fixed key agrees with the unverified parse.
#![no_main]

use std::sync::OnceLock;

use libfuzzer_sys::fuzz_target;
use rifts_admission::ticket::{decode_claims, encode_claims, parse_unverified};
use rifts_admission::{Es256Tickets, KeyId, P256KeyGenerator, TicketClaims};

fn tickets() -> Option<&'static Es256Tickets> {
    static TICKETS: OnceLock<Option<Es256Tickets>> = OnceLock::new();
    TICKETS
        .get_or_init(|| {
            let (_, private_pem, _) = P256KeyGenerator.generate_with_pems(KeyId("fuzz".to_owned())).ok()?;
            let signing = rifts_admission::keys::signing_key_from_pem(private_pem.expose())?;
            Some(Es256Tickets {
                key_id: KeyId("fuzz".to_owned()),
                verifying: *signing.verifying_key(),
                signing: Some(signing),
                default_lifetime: 75.0,
            })
        })
        .as_ref()
}

fn sane(claims: &TicketClaims) {
    let bindings = &claims.bindings;
    assert!((1..=999).contains(&bindings.virtual_port));
    assert!(bindings.joiner_peer.0 != 0 && bindings.host_peer.0 != 0);
    assert!(bindings.party_revision <= 9_007_199_254_740_991);
    assert!((1..=32).contains(&claims.key_id.0.len()));
    assert!(bindings.account.0.is_ascii() && bindings.party_id.is_ascii() && bindings.hunt_definition_id.is_ascii());
}

fuzz_target!(|data: &[u8]| {
    if let Ok(claims) = decode_claims(data) {
        sane(&claims);
        assert_eq!(encode_claims(&claims).ok().as_deref(), Some(data));
    }
    let Ok(token) = std::str::from_utf8(data) else {
        return;
    };
    let unverified = parse_unverified(token);
    if let Ok(claims) = &unverified {
        sane(claims);
    }
    if let Some(tickets) = tickets() {
        if let Ok(claims) = tickets.verify(token, claims_time(&unverified)) {
            assert_eq!(Ok(&claims), unverified.as_ref());
        }
    }
});

fn claims_time(unverified: &Result<TicketClaims, rifts_admission::TicketError>) -> u64 {
    unverified.as_ref().map_or(0, |claims| claims.issued_at)
}
