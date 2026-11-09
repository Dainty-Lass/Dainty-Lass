// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

use p256::elliptic_curve::scalar::IsHigh as _;
use p256::elliptic_curve::subtle::ConstantTimeEq as _;
use p256::elliptic_curve::{Generate as _, PrimeField as _};
use rustic_rifts_relay::wire::format_uuid;
use sha2::Sha256;

use super::B64URL as URL_SAFE_NO_PAD;
use super::*;
use crate::Stub;

const NOW: u64 = 1_793_000_000;
const SESSION: &str = "01234567-89ab-4def-8123-456789abcdef";

fn key() -> SigningKey {
    SigningKey::try_generate_from_rng(&mut rand::rngs::SysRng).unwrap()
}

fn authority(signing: SigningKey) -> Es256Tickets {
    Es256Tickets {
        key_id: KeyId("k".to_owned()),
        verifying: *signing.verifying_key(),
        signing: Some(signing),
        default_lifetime: 75.0,
    }
}

fn bindings() -> TicketBindings {
    TicketBindings {
        account: AccountId("account-1".to_owned()),
        joiner_peer: PeerId(76_561_198_000_000_001),
        host_peer: PeerId(76_561_198_000_000_002),
        session: SESSION.parse().unwrap(),
        generation: 7,
        party_id: "party_abc==".to_owned(),
        party_revision: 12,
        hunt_definition_id: "hunt/island-1".to_owned(),
        build: 392_819,
        bridge_protocol: 1,
        virtual_port: 3,
    }
}

fn claims() -> TicketClaims {
    TicketClaims {
        key_id: KeyId("k".to_owned()),
        jti: Jti([0x5a; 16]),
        bindings: bindings(),
        issued_at: NOW,
        not_before: NOW,
        expires_at: NOW + 75,
    }
}

fn token(payload: &[u8], signature: &[u8]) -> String {
    format!(
        "rpt1.{}.{}",
        URL_SAFE_NO_PAD.encode(payload),
        URL_SAFE_NO_PAD.encode(signature)
    )
}

fn parts(text: &str) -> (Vec<u8>, Vec<u8>) {
    let pieces: Vec<&str> = text.split('.').collect();
    (
        URL_SAFE_NO_PAD.decode(pieces[1]).unwrap(),
        URL_SAFE_NO_PAD.decode(pieces[2]).unwrap(),
    )
}

#[test]
fn every_binding_round_trips() {
    let tickets = authority(key());
    let minted = tickets.mint(bindings(), NOW, None).unwrap();
    let text = minted.text.expose();
    assert!(text.starts_with("rpt1."));
    assert_eq!(text.rsplit('.').next().unwrap().len(), 86);
    assert_eq!(minted.claims.bindings, bindings());
    assert_eq!(minted.claims.issued_at, NOW);
    assert_eq!(minted.claims.not_before, NOW);
    assert_eq!(minted.claims.expires_at, NOW + 75);
    assert_eq!(minted.jti_digest, minted.claims.jti_digest());
    assert_eq!(tickets.verify(text, NOW), Ok(minted.claims.clone()));

    let (payload, _) = parts(text);
    assert_eq!(decode_claims(&payload), Ok(minted.claims.clone()));
    assert_eq!(encode_claims(&minted.claims).unwrap(), payload);
    assert_eq!(parse_unverified(text), Ok(minted.claims));
}

#[test]
fn payload_layout() {
    let payload = encode_claims(&claims()).unwrap();
    assert_eq!(&payload[..6], b"RPT2\x02\x01");
    assert_eq!(
        payload[6..18],
        [0x00, 0x09, 0x00, 0x0d, 0x00, 0x0b, 0x00, 0x01, 0x00, 0x03, 0x00, 0x00]
    );
    assert_eq!(payload[18..26], [0x00, 0x00, 0x00, 0x07, 0x00, 0x05, 0xfe, 0x73]);
    assert_eq!(payload[26..34], 12_u64.to_be_bytes());
    assert_eq!(payload[34..42], NOW.to_be_bytes());
    assert_eq!(payload[42..50], NOW.to_be_bytes());
    assert_eq!(payload[50..58], (NOW + 75).to_be_bytes());
    assert_eq!(payload[58..66], 76_561_198_000_000_001_u64.to_be_bytes());
    assert_eq!(payload[66..74], 76_561_198_000_000_002_u64.to_be_bytes());
    assert_eq!(format_uuid(&payload[74..90].try_into().unwrap()), SESSION);
    assert_eq!(payload[90..106], [0x5a; 16]);
    assert_eq!(&payload[106..], b"kaccount-1hunt/island-1party_abc==");
    assert_eq!(payload.len(), FIXED_BYTES + 34);
}

#[test]
fn tampering_and_expiry() {
    let tickets = authority(key());
    let text = tickets.mint(bindings(), NOW, None).unwrap().text.expose().to_owned();
    let (payload, mut signature) = parts(&text);
    signature[10] ^= 1;
    assert_eq!(
        tickets.verify(&token(&payload, &signature), NOW),
        Err(TicketError::BadSignature)
    );

    let mut tampered = payload.clone();
    tampered[21] ^= 1;
    let (_, signature) = parts(&text);
    assert_eq!(
        tickets.verify(&token(&tampered, &signature), NOW),
        Err(TicketError::BadSignature)
    );

    let expired = tickets.verify(&text, NOW + 106).unwrap_err();
    assert_eq!(expired, TicketError::Expired);
    assert!(expired.is_expired());
    assert!(expired.to_string().contains("expired"));
    assert!(!TicketError::BadSignature.is_expired());
}

#[test]
fn virtual_port_range() {
    let tickets = authority(key());
    let port = |virtual_port| {
        tickets.mint(
            TicketBindings {
                virtual_port,
                ..bindings()
            },
            NOW,
            None,
        )
    };
    assert!(port(999).is_ok());
    assert_eq!(port(1000).unwrap_err(), TicketError::OutOfRange("virtualPort"));
    assert_eq!(port(0).unwrap_err(), TicketError::ZeroVirtualPort);
    assert_eq!(
        TicketError::OutOfRange("virtualPort").to_string(),
        "virtualPort is outside its unsigned range"
    );
}

#[test]
fn keys_and_clock_edges() {
    let signing = key();
    let tickets = authority(signing.clone());
    let text = tickets.mint(bindings(), NOW, None).unwrap().text.expose().to_owned();

    let other_id = Es256Tickets {
        key_id: KeyId("k2".to_owned()),
        ..tickets.clone()
    };
    assert_eq!(other_id.verify(&text, NOW), Err(TicketError::KeyNotActive));
    let other_key = authority(key());
    assert_eq!(other_key.verify(&text, NOW), Err(TicketError::BadSignature));

    assert!(tickets.verify(&text, NOW - 30).is_ok());
    assert_eq!(tickets.verify(&text, NOW - 31), Err(TicketError::NotYetValid));
    assert!(tickets.verify(&text, NOW + 105).is_ok());
    assert_eq!(tickets.verify(&text, NOW + 106), Err(TicketError::Expired));

    let long = tickets
        .sign(TicketClaims {
            expires_at: NOW + 181,
            ..claims()
        })
        .unwrap();
    assert_eq!(
        tickets.verify(long.text.expose(), NOW),
        Err(TicketError::InvalidLifetime)
    );
    let early = tickets
        .sign(TicketClaims {
            not_before: NOW - 1,
            ..claims()
        })
        .unwrap();
    assert_eq!(
        tickets.verify(early.text.expose(), NOW),
        Err(TicketError::InvalidLifetime)
    );
    let backwards = TicketClaims {
        expires_at: NOW - 1,
        ..claims()
    };
    assert_eq!(check_lifetime(&backwards, NOW), Err(TicketError::InvalidLifetime));
    let max = TicketClaims {
        issued_at: u64::MAX - 10,
        not_before: u64::MAX - 10,
        expires_at: u64::MAX,
        ..claims()
    };
    assert_eq!(check_lifetime(&max, u64::MAX), Ok(()));
}

#[test]
fn minting_lifetimes() {
    let tickets = authority(key());
    let mint = |tickets: &Es256Tickets, lifetime| tickets.mint(bindings(), NOW, lifetime);
    assert_eq!(mint(&tickets, Some(14)).unwrap_err(), TicketError::LifetimeSetting);
    assert_eq!(mint(&tickets, Some(181)).unwrap_err(), TicketError::LifetimeSetting);
    assert_eq!(mint(&tickets, Some(180)).unwrap().claims.expires_at, NOW + 180);
    assert_eq!(mint(&tickets, Some(15)).unwrap().claims.expires_at, NOW + 15);
    assert_eq!(mint(&tickets, None).unwrap().claims.expires_at, NOW + 75);
    for bad in [f64::NAN, 75.5, 0.0, f64::INFINITY] {
        let defaulted = Es256Tickets {
            default_lifetime: bad,
            ..tickets.clone()
        };
        assert_eq!(
            mint(&defaulted, None).unwrap_err(),
            TicketError::LifetimeSetting,
            "{bad}"
        );
    }
    let verify_only = Es256Tickets {
        signing: None,
        ..tickets.clone()
    };
    assert_eq!(mint(&verify_only, None).unwrap_err(), TicketError::NoSigningKey);
    // The payload checks come before the missing key.
    let bad_port = TicketBindings {
        virtual_port: 0,
        ..bindings()
    };
    assert_eq!(
        verify_only.mint(bad_port, NOW, None).unwrap_err(),
        TicketError::ZeroVirtualPort
    );

    let through_trait = TicketAuthority::mint(&tickets, bindings(), UnixMillis(1_500_999), None).unwrap();
    assert_eq!(through_trait.claims.issued_at, 1_500);
    let negative = TicketAuthority::mint(&tickets, bindings(), UnixMillis(-1), None).unwrap();
    assert_eq!(negative.claims.issued_at, 0);
    assert_eq!(
        TicketAuthority::verify(&tickets, "x", UnixMillis(0)),
        Err(AdmissionError::Ticket(TicketError::MalformedToken))
    );
    assert_eq!(TicketAuthority::key_id(&tickets), KeyId("k".to_owned()));
    assert!(matches!(
        TicketAuthority::mint(&Stub, bindings(), UnixMillis(0), None),
        Err(AdmissionError::Unimplemented(_))
    ));
}

#[test]
fn identifier_and_peer_checks() {
    let encode = |change: &dyn Fn(&mut TicketClaims)| {
        let mut claims = claims();
        change(&mut claims);
        encode_claims(&claims).map(drop)
    };
    let unsafe_id = Err(TicketError::UnsafeIdentifier);
    assert_eq!(encode(&|c| c.bindings.account.0 = "a b".to_owned()), unsafe_id);
    assert_eq!(encode(&|c| c.bindings.account.0 = "a".repeat(97)), unsafe_id);
    assert_eq!(encode(&|c| c.bindings.account.0 = "a".repeat(96)), Ok(()));
    assert_eq!(encode(&|c| c.bindings.party_id = String::new()), unsafe_id);
    assert_eq!(
        encode(&|c| c.bindings.hunt_definition_id = "hunt=1".to_owned()),
        unsafe_id
    );
    assert_eq!(encode(&|c| c.key_id.0 = "a/b".to_owned()), unsafe_id);
    assert_eq!(encode(&|c| c.key_id.0 = "k".repeat(33)), unsafe_id);
    assert_eq!(encode(&|c| c.bindings.party_id = "p:+/=-._9".to_owned()), Ok(()));
    assert_eq!(
        encode(&|c| c.bindings.joiner_peer = PeerId(0)),
        Err(TicketError::InvalidPeerId)
    );
    assert_eq!(
        encode(&|c| c.bindings.host_peer = PeerId(0)),
        Err(TicketError::InvalidPeerId)
    );
    assert_eq!(
        encode(&|c| c.bindings.party_revision = 1 << 53),
        Err(TicketError::OutOfRange("partyRevision"))
    );
    assert_eq!(encode(&|c| c.bindings.party_revision = (1 << 53) - 1), Ok(()));
    assert_eq!(
        encode(&|c| c.bindings.session = SessionUuid([0; 16])),
        Err(TicketError::SessionNotUuid)
    );
    // Identifiers are checked before ports and peers.
    assert_eq!(
        encode(&|c| {
            c.bindings.account.0 = String::new();
            c.bindings.virtual_port = 0;
        }),
        unsafe_id
    );
    assert_eq!(
        encode(&|c| {
            c.bindings.generation = 0;
            c.bindings.build = 0;
            c.bindings.bridge_protocol = 0;
        }),
        Ok(())
    );
    assert!(safe_key_id("a.b_c-1"));
    assert!(!safe_key_id(""));
}

#[test]
fn canonical_encodings() {
    let tickets = authority(key());
    let text = tickets.mint(bindings(), NOW, None).unwrap().text.expose().to_owned();
    let pieces: Vec<&str> = text.split('.').collect();
    let (payload, signature) = (pieces[1], pieces[2]);
    let verify = |token: &str| tickets.verify(token, NOW);
    let noncanonical = Err(TicketError::NoncanonicalEncoding);

    assert_eq!(verify(&format!("{text}=")), noncanonical);
    let padded = base64::engine::general_purpose::URL_SAFE.encode(parts(&text).0);
    assert!(padded.ends_with('='));
    assert_eq!(verify(&format!("rpt1.{padded}.{signature}")), noncanonical);
    assert_eq!(verify(&format!("rpt1.+{}.{signature}", &payload[1..])), noncanonical);
    assert_eq!(verify(&format!("rpt1.{payload}.{}", &signature[..85])), noncanonical);
    assert_eq!(
        verify(&format!("rpt1.{payload}.{}", &signature[..84])),
        noncanonical,
        "63 bytes"
    );

    // 140 payload bytes leave two spare bits in the last character.
    let last = payload.chars().last().unwrap();
    let alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let value = alphabet.find(last).unwrap();
    assert_eq!(value & 3, 0);
    let bent = alphabet.as_bytes()[value | 1] as char;
    let trailing = format!("rpt1.{}{bent}.{signature}", &payload[..payload.len() - 1]);
    assert_eq!(verify(&trailing), noncanonical);

    let malformed = Err(TicketError::MalformedToken);
    assert_eq!(verify(&format!("rpt2.{payload}.{signature}")), malformed);
    assert_eq!(verify(&format!("mpt1.{payload}.{signature}")), malformed);
    assert_eq!(verify(&format!("rpt1.{payload}")), malformed);
    assert_eq!(verify(&format!("rpt1..{signature}")), malformed);
    assert_eq!(verify(&format!("rpt1.{payload}.")), malformed);
    assert_eq!(verify(&format!("rpt1.{payload}.{signature}.x")), malformed);
    assert_eq!(verify(""), malformed);
    assert_eq!(verify(&"a".repeat(4097)), Err(TicketError::MissingOrOversized));
    assert_eq!(verify(&format!("{text}{}", ".".repeat(4096 - text.len()))), malformed);
}

#[test]
fn high_s_signatures_verify() {
    let signing = key();
    let tickets = authority(signing.clone());
    let payload = encode_claims(&claims()).unwrap();
    let signature: Signature = signing.sign(&payload);
    let low = signature.normalize_s();
    let high_s = -low.s();
    let high = Signature::from_scalars(low.r().to_repr(), high_s.to_repr()).unwrap();
    assert!(bool::from(high.s().is_high()));
    assert!(!bool::from(high.s().ct_eq(&low.s())));
    assert_eq!(tickets.verify(&token(&payload, &low.to_bytes()), NOW), Ok(claims()));
    assert_eq!(tickets.verify(&token(&payload, &high.to_bytes()), NOW), Ok(claims()));

    let mut zero_r = high.to_bytes().to_vec();
    zero_r[..32].fill(0);
    assert_eq!(
        tickets.verify(&token(&payload, &zero_r), NOW),
        Err(TicketError::BadSignature)
    );
    let mut beyond = high.to_bytes().to_vec();
    beyond[32..].fill(0xff);
    assert_eq!(
        tickets.verify(&token(&payload, &beyond), NOW),
        Err(TicketError::BadSignature)
    );
}

#[test]
fn payload_framing() {
    let payload = encode_claims(&claims()).unwrap();
    let decode = |change: &dyn Fn(&mut Vec<u8>)| {
        let mut bytes = payload.clone();
        change(&mut bytes);
        decode_claims(&bytes)
    };
    assert_eq!(decode(&|_| ()), Ok(claims()));
    assert_eq!(decode_claims(&payload[..105]), Err(TicketError::MalformedPayload));
    assert_eq!(decode(&|b| b[0] = b'M'), Err(TicketError::MalformedPayload));
    assert_eq!(decode(&|b| b[4] = 3), Err(TicketError::Framing));
    assert_eq!(decode(&|b| b[17] = 1), Err(TicketError::Framing));
    assert_eq!(decode(&|b| b[5] = 2), Err(TicketError::Framing));
    assert_eq!(decode(&|b| b[5] = 0), Err(TicketError::Framing));
    assert_eq!(decode(&|b| b.push(b'x')), Err(TicketError::Framing));
    assert_eq!(
        decode(&|b| b[14..16].copy_from_slice(&[0, 0])),
        Err(TicketError::ZeroVirtualPort)
    );
    assert_eq!(
        decode(&|b| b[14..16].copy_from_slice(&1000_u16.to_be_bytes())),
        Err(TicketError::OutOfRange("virtualPort"))
    );
    assert_eq!(decode(&|b| b[80] &= 0x0f), Err(TicketError::SessionNotUuid));
    assert_eq!(decode(&|b| b[106] = b' '), Err(TicketError::UnsafeIdentifier));
    assert_eq!(decode(&|b| b[106] = 0xff), Err(TicketError::UnsafeIdentifier));
    assert_eq!(decode(&|b| b[58..66].fill(0)), Err(TicketError::InvalidPeerId));
    assert_eq!(
        decode(&|b| b[26..34].copy_from_slice(&(1_u64 << 53).to_be_bytes())),
        Err(TicketError::OutOfRange("partyRevision"))
    );
    let inexact = (1_u64 << 53) + 1;
    assert_eq!(
        decode(&|b| b[34..42].copy_from_slice(&inexact.to_be_bytes())),
        Err(TicketError::NoncanonicalPayload)
    );
    assert_eq!(
        decode(&|b| b[50..58].copy_from_slice(&u64::MAX.to_be_bytes())),
        Err(TicketError::NoncanonicalPayload)
    );
    assert!(exact_double(1 << 60));
    assert!(!exact_double(u64::MAX));
}

#[test]
fn session_and_jti_text() {
    let lower: SessionUuid = SESSION.parse().unwrap();
    let upper: SessionUuid = SESSION.to_uppercase().parse().unwrap();
    assert_eq!(lower, upper);
    assert_eq!(upper.to_string(), SESSION);
    for bad in [
        "0123456789ab-4def-8123-456789abcdef",
        "01234567-89ab-4def-8123-456789abcde",
        "01234567-89ab-4def-8123_456789abcdef",
        "01234567-89ab-4def-8123-456789abcdeg",
        "01234567-89ab-0def-8123-456789abcdef",
        "01234567-89ab-6def-8123-456789abcdef",
        "01234567-89ab-4def-7123-456789abcdef",
        "01234567-89ab-4def-c123-456789abcdef",
        "01234567-89ab-4def-8123--56789abcdef",
    ] {
        assert_eq!(bad.parse::<SessionUuid>(), Err(TicketError::SessionNotUuid), "{bad}");
    }
    assert!("01234567-89ab-1def-b123-456789abcdef".parse::<SessionUuid>().is_ok());
    assert_eq!(SessionUuid::from_bytes([0; 16]), Err(TicketError::SessionNotUuid));

    assert_eq!(hash_jti("zz"), Err(TicketError::InvalidJti));
    assert_eq!(hash_jti(&"A".repeat(32)), hash_jti(&"a".repeat(32)));
    let digest = hash_jti(&"a".repeat(32)).unwrap();
    assert_eq!(digest.len(), 64);
    assert_eq!(digest, hex::encode(Sha256::digest([0xaa; 16])));
    let jti = Jti::parse("00112233445566778899AABBCCDDEEFF").unwrap();
    assert_eq!(jti.to_string(), "00112233445566778899aabbccddeeff");
    assert_eq!(
        Jti::parse("00112233445566778899aabbccddeef"),
        Err(TicketError::InvalidJti)
    );
    assert_ne!(Jti::random().unwrap(), Jti::random().unwrap());
}

#[test]
fn unverified_parsing() {
    let tickets = authority(key());
    let minted = tickets.mint(bindings(), NOW, None).unwrap();
    let (payload, _) = parts(minted.text.expose());
    assert_eq!(parse_unverified(&token(&payload, &[7; 64])), Ok(minted.claims));
    assert!(parse_unverified("rpt1.AAAA.AAAA").is_err());
    assert_eq!(
        parse_unverified(&token(&payload, &[0; 64])),
        Err(TicketError::BadSignature)
    );
}

#[test]
fn debug_hides_the_key() {
    let signing = key();
    let tickets = authority(signing.clone());
    let debug = format!("{tickets:?}");
    assert!(debug.contains("signing: Some(\"<redacted>\")"), "{debug}");
    assert!(debug.contains("default_lifetime: 75.0"));
    assert!(!debug.contains(&hex::encode(signing.to_bytes())));
}
