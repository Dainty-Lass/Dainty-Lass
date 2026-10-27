//! Session tickets and the keys that check them (docs/dainty-lass-plan.md DN5, DX2; docs/security/network.md): a
//! token from another node is read whole or refused, never panics, and nothing the fuzzer writes verifies,
//! since it cannot hold a node's key. A public key from a peer is read only with the id derived from it.
#![no_main]

use std::sync::{Arc, OnceLock};

use libfuzzer_sys::fuzz_target;
use module_api::NodeIdentity;
use module_api::network::{GlobalId, IdKind, SessionTicket};
use network::Clock;
use network::home::ticket::MAX_TOKEN;
use network::home::{Expect, SigningKey, TicketConfig, TicketError, Tickets, VerifyingKey};

struct Fixture {
    tickets: Tickets,
    other: Tickets,
    account: GlobalId,
}

fn fixture() -> &'static Fixture {
    static FIXTURE: OnceLock<Fixture> = OnceLock::new();
    FIXTURE.get_or_init(|| {
        let node = |name: &str| NodeIdentity {
            node: name.parse().expect("a node name"),
            region: "oregon".parse().expect("a region"),
        };
        let clock = Arc::new(Clock::default());
        let make = |name: &str| {
            Tickets::new(
                &node(name),
                SigningKey::generate().expect("a key"),
                clock.clone(),
                TicketConfig::default(),
            )
        };
        let (tickets, other) = (make("germany-1"), make("oregon-1"));
        other.learn(tickets.node(), tickets.public_keys());
        Fixture {
            account: GlobalId::new(IdKind::Account, tickets.node()),
            tickets,
            other,
        }
    })
}

fuzz_target!(|data: &[u8]| {
    let f = fixture();
    let token = String::from_utf8_lossy(data).into_owned();
    let ticket = SessionTicket {
        account: f.account.clone(),
        audience: f.other.node().clone(),
        token: token.clone(),
    };
    let game = "1.14.7".parse().expect("a version");
    let expect = Expect {
        issuer: f.tickets.node(),
        audience: f.other.node(),
        game: Some(&game),
        account: None,
    };
    if token.len() > MAX_TOKEN {
        assert_eq!(f.other.peek(&ticket), Err(TicketError::Malformed));
    }
    // Reading the claims never panics; and no token made of fuzz input is a ticket the home signed
    let _ = f.other.peek(&ticket);
    assert!(f.other.verify(&ticket, &expect).is_err());
    assert!(f.other.redeem(&ticket, &expect).is_err());

    // A real ticket with its text cut or one byte changed never verifies as a different ticket
    let real = f
        .tickets
        .issue(&f.account, f.other.node(), &game, None)
        .expect("a ticket");
    assert!(f.other.verify(&real, &expect).is_ok());
    if let Some(&byte) = data.first() {
        let mut bytes = real.token.clone().into_bytes();
        let at = usize::from(byte) % bytes.len();
        let flipped = bytes[at] ^ data.get(1).copied().unwrap_or(1).max(1);
        if bytes[at] != flipped {
            bytes[at] = flipped;
            if let Ok(token) = String::from_utf8(bytes) {
                let edited = SessionTicket { token, ..real.clone() };
                assert!(f.other.verify(&edited, &expect).is_err());
            }
        }
    }

    // A public key from a peer
    if let Ok(key) = serde_json::from_slice::<VerifyingKey>(data) {
        let again = serde_json::to_vec(&key).expect("a key writes");
        assert_eq!(serde_json::from_slice::<VerifyingKey>(&again).expect("reads back"), key);
        assert_eq!(key.kid().len(), 16);
    }
});
