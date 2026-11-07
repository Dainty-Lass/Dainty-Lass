//! What a person or the admin API's bot types to filter, page and revert the change log
//! (docs/security/history.md, DH3b): an actor, a time, an account and a cursor. None of it may panic; an actor
//! that passes is one the change log can record and parse back, a selection that passes has its start before
//! its end, a cursor that passes is at most the longest cursor and survives being written back.
#![no_main]

use dainty_admin::history::{MAX_CURSOR, Selection, normalize_cursor, parse_actor, parse_time};
use libfuzzer_sys::arbitrary::{self, Arbitrary};
use libfuzzer_sys::fuzz_target;

#[derive(Debug, Arbitrary)]
struct Input<'a> {
    account: Option<&'a str>,
    actor: Option<&'a str>,
    since: Option<&'a str>,
    until: Option<&'a str>,
    cursor: &'a str,
    now: i64,
}

fuzz_target!(|input: Input<'_>| {
    if let Ok(actor) = parse_actor(input.actor.unwrap_or_default()) {
        assert_eq!(dainty_store::Actor::parse(&actor.as_text()), Some(actor));
    }
    let _ = parse_time(input.since.unwrap_or_default(), input.now);
    let selection = Selection {
        account: input.account.map(str::to_owned),
        actor: input.actor.map(str::to_owned),
        since: input.since.map(str::to_owned),
        until: input.until.map(str::to_owned),
    };
    if let Ok(filter) = selection.to_filter(input.now) {
        if let (Some(since), Some(until)) = (filter.since, filter.until) {
            assert!(since < until);
        }
        if let Some(account) = &filter.account {
            assert!(!account.is_empty() && !account.chars().any(char::is_control));
        }
    }
    if let Some(cursor) = normalize_cursor(input.cursor) {
        assert!(cursor.len() <= MAX_CURSOR || input.cursor.len() <= MAX_CURSOR);
        assert_eq!(normalize_cursor(&cursor), Some(cursor));
    }
});
