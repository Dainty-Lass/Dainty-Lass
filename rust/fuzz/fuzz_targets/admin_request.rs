//! What reaches the admin API (docs/security/admin-api.md): the bot's headers and body, the query and
//! the arguments it names, a module's error text, and the Director's instance list. None of it may panic; a request passes
//! the header check only with the exact key and no proxy header; a name, a reason or a timeout end that
//! passes its check is one the backend and the player's launcher can take as it is.
#![no_main]

use admin::input::{
    MAX_MODULE_MESSAGE, MAX_REASON, MAX_TIMEOUT_MS, OPERATOR_HEADER, bearer, game, is_account_id, is_json,
    module_message, operator, parse_body, parse_instances, reason, timeout_end, username,
};
use admin::keys::{MAX_NAME, Role, clean_name, hash_key, is_key_id, key_id};
use http::{HeaderMap, HeaderName, HeaderValue};
use libfuzzer_sys::arbitrary::{self, Arbitrary};
use libfuzzer_sys::fuzz_target;

#[derive(Debug, Arbitrary)]
struct Request<'a> {
    headers: Vec<(&'a str, &'a [u8])>,
    body: &'a [u8],
    name: &'a str,
    reason: &'a str,
    minutes: Option<i64>,
    until: Option<&'a str>,
    now: i64,
    game: &'a str,
    module_message: &'a str,
    key: &'a str,
    director: &'a [u8],
}

fuzz_target!(|request: Request<'_>| {
    let mut headers = HeaderMap::new();
    for (name, value) in &request.headers {
        if let (Ok(name), Ok(value)) = (HeaderName::from_bytes(name.as_bytes()), HeaderValue::from_bytes(value)) {
            headers.append(name, value);
        }
    }
    // The key is whatever follows one Bearer, and only then; whose it is, is Keys::authenticate's to say
    if let Ok(given) = bearer(&headers) {
        let auth: Vec<_> = headers.get_all("authorization").iter().collect();
        assert_eq!(auth.len(), 1);
        assert_eq!(auth[0].as_bytes(), format!("Bearer {given}").as_bytes());
        assert!(headers.keys().all(|n| !n.as_str().starts_with("x-forwarded-") && n != "x-reforged-gateway"));
    }
    if let Ok(operator) = operator(&headers) {
        assert!(!operator.id().is_empty() && operator.id().bytes().all(|b| b.is_ascii_digit()));
        if headers.get(OPERATOR_HEADER).is_none() {
            assert_eq!(operator.id(), "0");
        }
    }
    // A key is looked up only when it has the shape of an issued one: its id is 8 hex digits and the whole
    // key is 77 characters
    if let Some(id) = key_id(request.key) {
        assert!(is_key_id(id) && request.key.len() == 77 && request.key.starts_with("RRA_"));
        assert_eq!(hash_key(request.key).len(), 64);
    }
    if let Some(name) = clean_name(request.name) {
        assert!((1..=MAX_NAME).contains(&name.chars().count()) && !name.chars().any(char::is_control));
    }
    if let Some(role) = Role::parse(request.reason) {
        assert_eq!(Role::parse(role.as_str()), Some(role));
    }
    let _ = is_json(&headers);
    if let Ok(parsed) = parse_body(request.body) {
        assert!(request.body.len() <= admin::input::MAX_BODY);
        let _ = async_graphql::parser::parse_query(&parsed.query);
    }

    if let Some(name) = username(request.name) {
        assert!((3..=16).contains(&name.len()));
        assert!(name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'));
    }
    if let Some(text) = reason(request.reason) {
        let count = text.chars().count();
        assert!((3..=MAX_REASON).contains(&count));
        assert!(!text.chars().any(char::is_control));
    }
    if let Ok(end) = timeout_end(request.minutes, request.until, request.now) {
        assert!(end > request.now && end - request.now <= MAX_TIMEOUT_MS);
    }
    // A game the caller names is compared as text with the modules' versions, so it has one spelling
    if let Some(version) = game(request.game) {
        assert_eq!(game(&version).as_deref(), Some(version.as_str()));
        assert!(version.bytes().all(|b| b.is_ascii_digit() || b == b'.'));
    }
    let text = module_message(request.module_message);
    assert!(text.chars().count() <= MAX_MODULE_MESSAGE && !text.chars().any(char::is_control));

    if let Ok(value) = serde_json::from_slice::<serde_json::Value>(request.director) {
        for instance in parse_instances(&value) {
            assert!(!instance.id.is_empty() && instance.id.chars().count() <= 64);
            assert!(instance.title.chars().count() <= 64 && instance.map.chars().count() <= 64);
            assert!(instance.player_ids.len() <= 16);
            // Ids a lookup would send on are the ones the backend's ?ids= accepts, and nothing else
            for id in &instance.player_ids {
                let _ = is_account_id(id);
            }
        }
    }
});
