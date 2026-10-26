use super::*;
use crate::P256KeyGenerator;

const NOW: u64 = 1_800_000_000;

fn authority(key_id: &str) -> Es256Tickets {
    let (_, private_pem, _) = P256KeyGenerator.generate_with_pems(KeyId(key_id.to_owned())).unwrap();
    let signing = crate::keys::signing_key_from_pem(private_pem.expose()).unwrap();
    Es256Tickets {
        key_id: KeyId(key_id.to_owned()),
        verifying: *signing.verifying_key(),
        signing: Some(signing),
        default_lifetime: 75.0,
    }
}

pub(crate) fn bindings(virtual_port: u16) -> TicketBindings {
    TicketBindings {
        account: AccountId("account-1".to_owned()),
        joiner_peer: PeerId(76_561_198_000_000_001),
        host_peer: PeerId(76_561_198_000_000_002),
        session: "01234567-89ab-4def-8123-456789abcdef".parse().unwrap(),
        generation: 7,
        party_id: "party_abc==".to_owned(),
        party_revision: 12,
        hunt_definition_id: "hunt/island-1".to_owned(),
        build: 392_819,
        bridge_protocol: 1,
        virtual_port,
    }
}

fn issue(tickets: &Es256Tickets, now: u64, virtual_port: u16) -> Result<MintedTicket, TicketError> {
    tickets.mint(bindings(virtual_port), now, Some(75))
}

fn parts(token: &str) -> (String, Vec<u8>, String) {
    let mut split = token.split('.');
    let prefix = split.next().unwrap().to_owned();
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(split.next().unwrap())
        .unwrap();
    (prefix, payload, split.next().unwrap().to_owned())
}

// admissionTickets.test.ts "admission ticket round-trips every production binding"
#[test]
fn a_ticket_round_trips_every_binding() {
    let tickets = authority("test-key-1");
    let issued = issue(&tickets, NOW, 1).unwrap();
    let claims = tickets.verify(issued.text.expose(), NOW + 10).unwrap();
    assert_eq!(claims.key_id, KeyId("test-key-1".to_owned()));
    assert_eq!(claims.bindings, bindings(1));
    assert_eq!(
        claims.bindings.session.to_string(),
        "01234567-89ab-4def-8123-456789abcdef"
    );
    assert_eq!(
        (claims.issued_at, claims.not_before, claims.expires_at),
        (NOW, NOW, NOW + 75)
    );
    assert_eq!(issued.jti_digest, hash_jti(&claims.jti.to_string()).unwrap());
    assert_eq!(claims, issued.claims);
}

// "admission ticket rejects tampering and expiry"
#[test]
fn a_ticket_refuses_tampering_and_expiry() {
    let tickets = authority("test-key-1");
    let issued = issue(&tickets, NOW, 1).unwrap();
    let (prefix, mut payload, signature) = parts(issued.text.expose());
    let last = payload.len() - 1;
    payload[last] ^= 1;
    let tampered = format!("{prefix}.{}.{signature}", base64url(&payload));
    assert_eq!(tickets.verify(&tampered, NOW + 10), Err(TicketError::BadSignature));
    let expired = tickets.verify(issued.text.expose(), NOW + 200).unwrap_err();
    assert!(expired.is_expired() && expired.to_string().contains("expired"));
}

// "admission ticket restricts Steam P2P virtual ports to the logical range"
#[test]
fn virtual_ports_stay_in_the_logical_range() {
    let tickets = authority("test-key-1");
    assert!(issue(&tickets, NOW, 999).is_ok());
    assert_eq!(
        issue(&tickets, NOW, 0).unwrap_err().to_string(),
        "virtualPort must be a nonzero Steam logical port"
    );
    assert_eq!(
        issue(&tickets, NOW, 1000).unwrap_err().to_string(),
        "virtualPort is outside its unsigned range"
    );
}

#[test]
fn the_payload_layout_is_the_transports() {
    let tickets = authority("k");
    let issued = issue(&tickets, NOW, 3).unwrap();
    let (prefix, payload, signature) = parts(issued.text.expose());
    assert_eq!(prefix, "mpt1");
    assert_eq!(payload.len(), FIXED_BYTES + 1 + 9 + 13 + 11);
    assert_eq!(&payload[..6], b"MPT2\x02\x01");
    assert_eq!(&payload[6..18], &[0, 9, 0, 13, 0, 11, 0, 1, 0, 3, 0, 0]);
    assert_eq!(&payload[18..26], &[0, 0, 0, 7, 0, 5, 0xfe, 0x73]);
    assert_eq!(u64::from_be_bytes(payload[34..42].try_into().unwrap()), NOW);
    assert_eq!(
        u64::from_be_bytes(payload[58..66].try_into().unwrap()),
        76_561_198_000_000_001
    );
    assert_eq!(&payload[74..90], &bindings(3).session.0);
    assert_eq!(&payload[106..], b"kaccount-1hunt/island-1party_abc==");
    assert_eq!(signature.len(), 86);
    assert_eq!(decode_claims(&payload).unwrap(), issued.claims);
}

#[test]
fn only_the_active_key_and_a_sane_lifetime_pass() {
    let tickets = authority("key-a");
    let issued = issue(&tickets, NOW, 1).unwrap();
    let other_id = Es256Tickets {
        key_id: KeyId("key-b".to_owned()),
        ..tickets.clone()
    };
    assert_eq!(
        other_id.verify(issued.text.expose(), NOW),
        Err(TicketError::KeyNotActive)
    );
    let other_key = authority("key-a");
    assert_eq!(
        other_key.verify(issued.text.expose(), NOW),
        Err(TicketError::BadSignature)
    );
    // 30 seconds of skew either side
    assert!(tickets.verify(issued.text.expose(), NOW - 30).is_ok());
    assert_eq!(
        tickets.verify(issued.text.expose(), NOW - 31),
        Err(TicketError::NotYetValid)
    );
    assert!(tickets.verify(issued.text.expose(), NOW + 75 + 30).is_ok());
    assert_eq!(
        tickets.verify(issued.text.expose(), NOW + 75 + 31),
        Err(TicketError::Expired)
    );

    let mut claims = issued.claims.clone();
    claims.expires_at = NOW + 181;
    let long = tickets.sign(claims).unwrap();
    assert_eq!(
        tickets.verify(long.text.expose(), NOW),
        Err(TicketError::InvalidLifetime)
    );
    let mut claims = issued.claims;
    claims.not_before = NOW - 1;
    let early = tickets.sign(claims).unwrap();
    assert_eq!(
        tickets.verify(early.text.expose(), NOW),
        Err(TicketError::InvalidLifetime)
    );
}

#[test]
fn the_minting_lifetime_is_checked_as_node_checks_it() {
    let tickets = authority("k");
    assert_eq!(
        tickets.mint(bindings(1), NOW, Some(14)).unwrap_err(),
        TicketError::LifetimeSetting
    );
    assert_eq!(
        tickets.mint(bindings(1), NOW, Some(181)).unwrap_err(),
        TicketError::LifetimeSetting
    );
    assert_eq!(
        tickets.mint(bindings(1), NOW, Some(180)).unwrap().claims.expires_at,
        NOW + 180
    );
    assert_eq!(
        tickets.mint(bindings(1), NOW, None).unwrap().claims.expires_at,
        NOW + 75
    );
    let unset = Es256Tickets {
        default_lifetime: f64::NAN,
        ..tickets.clone()
    };
    assert_eq!(
        unset.mint(bindings(1), NOW, None).unwrap_err(),
        TicketError::LifetimeSetting
    );
    let fractional = Es256Tickets {
        default_lifetime: 75.5,
        ..tickets.clone()
    };
    assert_eq!(
        fractional.mint(bindings(1), NOW, None).unwrap_err(),
        TicketError::LifetimeSetting
    );
    let verify_only = Es256Tickets {
        signing: None,
        ..tickets
    };
    assert_eq!(
        verify_only.mint(bindings(1), NOW, None).unwrap_err(),
        TicketError::NoSigningKey
    );
}

#[test]
fn identifiers_and_peers_are_checked_before_signing() {
    let tickets = authority("k");
    let with = |change: fn(&mut TicketBindings)| {
        let mut bindings = bindings(1);
        change(&mut bindings);
        tickets.mint(bindings, NOW, None).unwrap_err()
    };
    assert_eq!(
        with(|b| b.account = AccountId("a b".to_owned())),
        TicketError::UnsafeIdentifier
    );
    assert_eq!(
        with(|b| b.account = AccountId("a".repeat(97))),
        TicketError::UnsafeIdentifier
    );
    assert_eq!(with(|b| b.party_id = String::new()), TicketError::UnsafeIdentifier);
    assert_eq!(
        with(|b| b.hunt_definition_id = "hunt=1".to_owned()),
        TicketError::UnsafeIdentifier
    );
    assert_eq!(with(|b| b.joiner_peer = PeerId(0)), TicketError::InvalidPeerId);
    assert_eq!(
        with(|b| b.party_revision = 1 << 53),
        TicketError::OutOfRange("partyRevision")
    );
    assert_eq!(with(|b| b.session = SessionUuid([0; 16])), TicketError::SessionNotUuid);
    let bad_key = Es256Tickets {
        key_id: KeyId("a/b".to_owned()),
        ..tickets.clone()
    };
    assert_eq!(
        bad_key.mint(bindings(1), NOW, None).unwrap_err(),
        TicketError::UnsafeIdentifier
    );
    // Node allows a zero generation, build and bridge protocol (the transport does not)
    assert!(
        tickets
            .mint(
                TicketBindings {
                    generation: 0,
                    build: 0,
                    bridge_protocol: 0,
                    ..bindings(1)
                },
                NOW,
                None
            )
            .is_ok()
    );
}

#[test]
fn only_canonical_encodings_pass() {
    let tickets = authority("k");
    let issued = issue(&tickets, NOW, 1).unwrap();
    let token = issued.text.expose();
    let (prefix, payload, signature) = parts(token);
    let body = base64url(&payload);
    assert_eq!(
        tickets.verify(&format!("{token}="), NOW),
        Err(TicketError::NoncanonicalEncoding)
    );
    assert_eq!(
        tickets.verify(&format!("{prefix}.{body}=.{signature}"), NOW),
        Err(TicketError::NoncanonicalEncoding)
    );
    let standard = body.replace('-', "+").replace('_', "/");
    if standard != body {
        assert_eq!(
            tickets.verify(&format!("{prefix}.{standard}.{signature}"), NOW),
            Err(TicketError::NoncanonicalEncoding)
        );
    }
    assert_eq!(
        tickets.verify(&format!("{prefix}.{body}.{}", &signature[..85]), NOW),
        Err(TicketError::NoncanonicalEncoding)
    );
    assert_eq!(
        tickets.verify(&format!("mpt2.{body}.{signature}"), NOW),
        Err(TicketError::MalformedToken)
    );
    assert_eq!(
        tickets.verify(&format!("{prefix}.{body}"), NOW),
        Err(TicketError::MalformedToken)
    );
    assert_eq!(
        tickets.verify(&format!("{prefix}..{signature}"), NOW),
        Err(TicketError::MalformedToken)
    );
    assert_eq!(
        tickets.verify(&format!("{token}.x"), NOW),
        Err(TicketError::MalformedToken)
    );
    assert_eq!(
        tickets.verify(&"a".repeat(4097), NOW),
        Err(TicketError::MissingOrOversized)
    );
    // A non-zero trailing bit decodes to the same bytes in Node, then fails its round trip
    let last = body.chars().last().unwrap();
    let alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let bumped = alphabet.chars().nth(alphabet.find(last).unwrap() + 1).unwrap();
    if !body.len().is_multiple_of(4) {
        let trailing = format!("{prefix}.{}{bumped}.{signature}", &body[..body.len() - 1]);
        assert!(tickets.verify(&trailing, NOW).is_err());
    }
}

#[test]
fn a_high_s_signature_passes_as_it_does_for_node_and_bcrypt() {
    let tickets = authority("k");
    let issued = issue(&tickets, NOW, 1).unwrap();
    let (prefix, payload, signature) = parts(issued.text.expose());
    let signature = Signature::from_slice(
        &base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(signature)
            .unwrap(),
    )
    .unwrap();
    let (r, s) = signature.split_scalars();
    let flipped = Signature::from_scalars(r.to_bytes(), (-*s).to_bytes()).unwrap();
    let token = format!("{prefix}.{}.{}", base64url(&payload), base64url(&flipped.to_bytes()));
    assert!(tickets.verify(&token, NOW).is_ok());
    // r = 0 is refused like OpenSSL's verify refuses it
    let zero = [&[0u8; 32][..], &flipped.to_bytes()[32..]].concat();
    let token = format!("{prefix}.{}.{}", base64url(&payload), base64url(&zero));
    assert_eq!(tickets.verify(&token, NOW), Err(TicketError::BadSignature));
}

#[test]
fn the_payload_framing_refuses_anything_noncanonical() {
    let tickets = authority("k");
    let payload = encode_claims(&issue(&tickets, NOW, 1).unwrap().claims).unwrap();
    let bent = |at: usize, value: u8| {
        let mut copy = payload.clone();
        copy[at] = value;
        decode_claims(&copy)
    };
    assert_eq!(decode_claims(&payload[..105]), Err(TicketError::MalformedPayload));
    assert_eq!(bent(0, b'X'), Err(TicketError::MalformedPayload));
    assert_eq!(bent(4, 3), Err(TicketError::Framing)); // version
    assert_eq!(bent(17, 1), Err(TicketError::Framing)); // reserved
    assert_eq!(bent(5, 2), Err(TicketError::Framing)); // key length no longer adds up
    assert_eq!(bent(5, 0), Err(TicketError::Framing));
    assert_eq!(bent(15, 0), Err(TicketError::ZeroVirtualPort));
    assert_eq!(bent(14, 4), Err(TicketError::OutOfRange("virtualPort")));
    assert_eq!(bent(80, 0x05), Err(TicketError::SessionNotUuid)); // version nibble 0
    assert_eq!(bent(106, b' '), Err(TicketError::UnsafeIdentifier));
    assert_eq!(bent(106, 0xff), Err(TicketError::UnsafeIdentifier));
    // A time with no exact double: Node's Number() changes it, so its re-encoding differs
    let mut inexact = payload.clone();
    inexact[34..42].copy_from_slice(&((1u64 << 60) + 1).to_be_bytes());
    assert_eq!(decode_claims(&inexact), Err(TicketError::NoncanonicalPayload));
    let mut joiner_zero = payload.clone();
    joiner_zero[58..66].copy_from_slice(&[0; 8]);
    assert_eq!(decode_claims(&joiner_zero), Err(TicketError::InvalidPeerId));
    let mut trailing = payload.clone();
    trailing.push(b'x');
    assert_eq!(decode_claims(&trailing), Err(TicketError::Framing));
}

#[test]
fn session_uuids_and_jtis_parse_as_node_matches_them() {
    assert!("01234567-89AB-4DEF-8123-456789ABCDEF".parse::<SessionUuid>().is_ok());
    assert_eq!(
        "01234567-89ab-4def-8123-456789abcdef"
            .parse::<SessionUuid>()
            .unwrap()
            .to_string(),
        "01234567-89ab-4def-8123-456789abcdef"
    );
    for bad in [
        "01234567-89ab-0def-8123-456789abcdef",
        "01234567-89ab-6def-8123-456789abcdef",
        "01234567-89ab-4def-7123-456789abcdef",
        "0123456789ab4def8123456789abcdef",
        "01234567-89ab-4def-8123-456789abcdeg",
        "01234567-89ab-4def-8123_456789abcdef",
    ] {
        assert_eq!(bad.parse::<SessionUuid>(), Err(TicketError::SessionNotUuid), "{bad}");
    }
    assert_eq!(hash_jti("zz"), Err(TicketError::InvalidJti));
    // SHA-256 of sixteen zero bytes
    assert_eq!(
        hash_jti(&"0".repeat(32)).unwrap(),
        "374708fff7719dd5979ec875d56cd2286f6d3cf7ec317a3b25632aab28ec37bb"
    );
    assert_eq!(hash_jti(&"A".repeat(32)).unwrap(), hash_jti(&"a".repeat(32)).unwrap());
}

#[test]
fn the_unverified_parser_reads_but_never_trusts() {
    let tickets = authority("k");
    let issued = issue(&tickets, NOW, 1).unwrap();
    assert_eq!(parse_unverified(issued.text.expose()).unwrap(), issued.claims);
    assert!(parse_unverified("mpt1.AAAA.AAAA").is_err());
}
