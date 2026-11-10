// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! Fuzz target for the Discord bot (`docs/security/discord.md`, module sharing A7): an interaction as twilight's
//! model reads it, and what dainty-dash answers. Parsing never panics; an interaction that parses has ids that are not 0 and a token that is
//! safe in a URL path and never shown by `Debug`; a signup link is an http(s) URL with no credentials.
#![no_main]

use dainty_discord::interaction::{Interaction, MAX_INTERACTION_TOKEN, parse_interaction};
use dainty_discord::signup::{MAX_URL, parse_answer};
use libfuzzer_sys::fuzz_target;

fn check(interaction: &Interaction) {
    let token = interaction.token.expose();
    assert!(!token.is_empty() && token.len() <= MAX_INTERACTION_TOKEN);
    assert!(token.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-')));
    assert!(interaction.id.get() != 0 && interaction.user_id.get() != 0 && interaction.application_id.get() != 0);
    if token.len() >= 16 {
        assert!(!format!("{interaction:?}").contains(token));
    }
}

fuzz_target!(|data: &[u8]| {
    if let Ok(interaction) = parse_interaction(data) {
        check(&interaction);
    }
    if let Some(url) = parse_answer(data) {
        assert!(url.len() <= MAX_URL * 3);
        let parsed = url::Url::parse(&url).expect("a signup link is a URL");
        assert!(matches!(parsed.scheme(), "http" | "https"));
        assert!(parsed.username().is_empty() && parsed.password().is_none());
    }
});
