//! The core rules the shell's `core/` tested (spec §2.8, rows L11–L18), now dainty-launcher-kit's `core` and `protocol`:
//! the game's language, the backend's own text, and the servers' shared invite, status and username tables
//! through the launcher's path.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

use dainty_launcher_kit::bridge::types::GameLanguage;
use dainty_launcher_kit::core::game_language::{GAME_LOCALES, launch_locale, resolve};
use dainty_launcher_kit::core::i18n::{self as kit, CLOSE_ANYWAY, Language, translate};
use dainty_launcher_kit::core::text::js_number;
use dainty_protocol::account::{
    ACCOUNT_KEY_PREFIX, UsernameCheck, check_username, extract_account_key, is_plausible_account_key, is_valid_username,
};
use dainty_protocol::invite::{
    Invite, InviteError, ServerMode, format_invite, is_private_mode_host, is_tailscale_ipv4, is_valid_fingerprint,
    is_valid_host, parse_invite,
};
use dainty_protocol::status::{ServerStatus, clean_text, iso_date, parse_server_status};
use dainty_protocol::text::{is_unsafe_char, js_length, js_trim};
use serde::Deserialize;
use serde_json::Value;

// ---------------------------------------------------------------- game_language.rs

fn name(setting: GameLanguage) -> String {
    serde_json::to_value(setting).unwrap().as_str().unwrap().to_owned()
}

/// The shell's `game_locale`: the setting, `auto` from the system's locale.
fn game_locale(setting: GameLanguage, system: &str) -> &'static str {
    resolve(&name(setting), Some(system))
}

#[test]
fn auto_follows_the_system_when_the_game_has_that_language() {
    let launch = |locale: Option<&str>| launch_locale(locale.unwrap_or("en-US"));
    assert_eq!(launch(None), "en-US");
    assert_eq!(launch(Some("es-ES")), "es-ES");
    assert_eq!(launch(Some("pt-BR")), "pt-BR");
    assert_eq!(launch(Some("xx-XX")), "en-US");
    assert_eq!(game_locale(GameLanguage::Auto, "es-ES"), "es-ES");
    assert_eq!(game_locale(GameLanguage::Auto, "es-419"), "es-ES");
    assert_eq!(game_locale(GameLanguage::Auto, "de"), "de-DE");
    assert_eq!(game_locale(GameLanguage::Auto, "pt-PT"), "pt-BR");
    assert_eq!(
        game_locale(GameLanguage::Auto, "fi-FI"),
        "en-US",
        "no Finnish text in the game"
    );
    assert_eq!(game_locale(GameLanguage::Auto, ""), "en-US");
    assert_eq!(
        game_locale(GameLanguage::JaJp, "en-US"),
        "ja-JP",
        "a chosen language wins over the system"
    );
}

#[test]
fn every_setting_but_auto_is_a_game_locale() {
    let all = [
        GameLanguage::EnUs,
        GameLanguage::DeDe,
        GameLanguage::EsEs,
        GameLanguage::FrFr,
        GameLanguage::ItIt,
        GameLanguage::JaJp,
        GameLanguage::PtBr,
        GameLanguage::RuRu,
    ];
    for setting in all {
        let locale = name(setting);
        assert!(GAME_LOCALES.contains(&locale.as_str()), "{locale}");
        assert_eq!(game_locale(setting, "fi-FI"), locale);
    }
    assert_eq!(all.len(), GAME_LOCALES.len());
}

// ---------------------------------------------------------------- i18n.rs

#[test]
fn every_key_the_backend_shows_has_text_in_every_language() {
    for language in Language::ALL {
        for key in kit::KEYS {
            let text = translate(language, key, &[]);
            assert!(!text.trim().is_empty() && text != key, "{} {key}", language.code());
        }
    }
}

#[test]
fn every_page_language_is_one_the_catalog_has() {
    for language in Language::ALL {
        assert_eq!(Language::from_code(language.code()), Some(language));
    }
}

#[test]
fn translates_like_the_page() {
    assert_ne!(
        translate(Language::PtBr, CLOSE_ANYWAY, &[]),
        translate(Language::En, CLOSE_ANYWAY, &[])
    );
    assert_eq!(translate(Language::PtBr, "x {a}, {a}", &[("a", "Ana")]), "x Ana, Ana");
    assert_eq!(translate(Language::En, "no_such_key", &[]), "no_such_key");
    assert_eq!(translate(Language::En, "x {a} {a}", &[("a", "1")]), "x 1 1");
    assert_eq!(Language::from_code("pt-BR"), Some(Language::PtBr));
    assert_eq!(Language::from_code("pt"), None);
    assert_eq!(Language::ALL[0], Language::En);
}

#[test]
fn system_language_is_the_pages() {
    for (locale, language) in [
        ("pt-BR", Language::PtBr),
        ("pt-PT", Language::PtBr),
        ("pt_BR", Language::PtBr),
        ("PT-br", Language::PtBr),
        ("pt", Language::PtBr),
        ("ptx", Language::En),
        ("en-US", Language::En),
        ("en", Language::En),
        ("fi", Language::En),
        ("", Language::En),
    ] {
        assert_eq!(Language::from_system(locale), language, "{locale}");
    }
}

// ---------------------------------------------------------------- invite.rs

#[derive(Deserialize)]
struct InviteCases {
    accept: Vec<Accept>,
    reject: Vec<Reject>,
}

#[derive(Deserialize)]
struct Accept {
    input: String,
    invite: Invite,
}

#[derive(Deserialize)]
struct Reject {
    input: String,
    error: String,
}

/// The servers' table, through the launcher's path: what the launcher refuses is what they refuse.
#[test]
fn the_shared_invite_table() {
    let cases: InviteCases = serde_json::from_str(dainty_protocol::test_data::INVITE_CASES).unwrap();
    assert!(cases.accept.len() > 30 && cases.reject.len() > 150);
    for case in cases.accept {
        assert_eq!(parse_invite(&case.input), Ok(case.invite), "{:?}", case.input);
    }
    for case in cases.reject {
        assert_eq!(
            parse_invite(&case.input).map_err(InviteError::as_str),
            Err(case.error.as_str()),
            "{:?}",
            case.input
        );
    }
}

#[test]
fn invites_the_servers_build_parse_back() {
    let invite = Invite {
        mode: ServerMode::Public,
        host: "play.example.org".into(),
        port: 8443,
        code: "XXXX-YYYY".into(),
        name: "Aether & Ember".into(),
        share: None,
        fp: Some("3f".repeat(32)),
        version: dainty_protocol::invite::dainty_version(),
    };
    let link = format_invite(&invite).unwrap();
    assert!(link.starts_with("dainty://join?v=2&"));
    assert_eq!(parse_invite(&link), Ok(invite));
}

#[test]
fn dauntless_revived_invites_are_refused() {
    let link = format!(
        "dauntless-revived://join?v=2&mode=public&host=play.example.org&fp={}&code=ABCD&name=X",
        "3f".repeat(32)
    );
    assert_eq!(parse_invite(&link), Err(InviteError::Revived));
}

#[test]
fn host_rules() {
    assert!(is_private_mode_host("100.64.0.1"));
    assert!(is_private_mode_host("my-pc.tail-scale.ts.net"));
    assert!(is_private_mode_host("127.0.0.1"));
    assert!(is_private_mode_host("localhost"));
    assert!(!is_private_mode_host("203.0.113.9"));
    assert!(!is_private_mode_host("10.0.0.5"));
    assert!(!is_private_mode_host("example.com"));
    assert!(!is_private_mode_host("ts.net"));
    assert!(!is_valid_host("MyPC"));
    assert!(!is_valid_host("a_b"));
    assert!(is_tailscale_ipv4("100.127.255.255"));
    assert!(!is_tailscale_ipv4("100.128.0.1"));
    assert!(!is_valid_fingerprint(&"3F".repeat(32)));
}

// ---------------------------------------------------------------- status.rs

#[derive(Deserialize)]
struct StatusCase {
    input: Value,
    expected: Option<ServerStatus>,
    /// `JSON.stringify` of the TypeScript answer
    text: String,
}

#[test]
fn the_shared_status_table() {
    let cases: Vec<StatusCase> = serde_json::from_str(dainty_protocol::test_data::STATUS_CASES).unwrap();
    assert!(cases.len() > 90);
    for case in cases {
        let parsed = parse_server_status(&case.input);
        assert_eq!(parsed, case.expected, "{}", case.input);
        // The JSON the page receives over the bridge
        assert_eq!(serde_json::to_string(&parsed).unwrap(), case.text, "{}", case.input);
    }
}

#[test]
fn dates_far_out() {
    assert_eq!(
        iso_date("+200000-09-13T00:00:00Z").as_deref(),
        Some("+200000-09-13T00:00:00.000Z")
    );
    assert_eq!(
        iso_date("-000001-01-01T00:00:00Z").as_deref(),
        Some("-000001-01-01T00:00:00.000Z")
    );
    assert_eq!(iso_date("-000000-01-01T00:00:00Z"), None);
    assert_eq!(
        iso_date("2026-09-21T24:00:00Z").as_deref(),
        Some("2026-09-22T00:00:00.000Z")
    );
    assert_eq!(iso_date("2026-02-30T00:00:00Z"), None);
}

#[test]
fn text_is_cleaned_as_the_page_expects() {
    assert_eq!(
        clean_text(Some(&Value::from(" a\u{202e}\t\u{3000}b ")), 32).as_deref(),
        Some("a b")
    );
    assert_eq!(clean_text(Some(&Value::from("\u{7}")), 32), None);
    assert_eq!(clean_text(Some(&Value::from(5)), 32), None);
    assert_eq!(clean_text(Some(&Value::from("abcdef")), 3).as_deref(), Some("abc"));
}

// ---------------------------------------------------------------- text.rs

#[test]
fn trims_like_javascript() {
    assert_eq!(js_trim("\u{feff}\u{a0} x \u{3000}\u{2028}"), "x");
    assert_eq!(js_trim("\u{85}x"), "\u{85}x");
    assert_eq!(js_length("a\u{1f600}é"), 4);
    assert!(is_unsafe_char('\u{85}') && is_unsafe_char('\u{202e}') && !is_unsafe_char('\u{200b}'));
}

#[test]
fn numbers_like_javascript() {
    for (text, n) in [
        ("", 0.0),
        (" 720 ", 720.0),
        ("1e3", 1000.0),
        ("0x10", 16.0),
        (".5", 0.5),
        ("-2", -2.0),
        ("+3", 3.0),
        ("5.", 5.0),
    ] {
        assert_eq!(js_number(text), n, "{text:?}");
    }
    assert_eq!(js_number("-Infinity"), f64::NEG_INFINITY);
    for text in ["inf", "NaN", "1,5", "0x", "abc", "1 2", "e5"] {
        assert!(js_number(text).is_nan(), "{text:?}");
    }
}

// ---------------------------------------------------------------- username.rs

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UsernameCases {
    check: Vec<Check>,
    plausible_key: Vec<Plausible>,
    extract_key: Vec<Extract>,
}

#[derive(Deserialize)]
struct Check {
    input: Value,
    result: String,
}

#[derive(Deserialize)]
struct Plausible {
    input: String,
    ok: bool,
}

#[derive(Deserialize)]
struct Extract {
    input: String,
    key: Option<String>,
}

#[test]
fn the_shared_username_table() {
    let cases: UsernameCases = serde_json::from_str(dainty_protocol::test_data::USERNAME_CASES).unwrap();
    for case in cases.check {
        // The page may send a non-string, which TypeScript reads as empty
        let result = case.input.as_str().map_or(UsernameCheck::Empty, check_username);
        assert_eq!(result.as_str(), case.result, "{:?}", case.input);
        assert_eq!(
            case.input.as_str().is_some_and(is_valid_username),
            case.result == "ok",
            "{:?}",
            case.input
        );
    }
    for case in cases.plausible_key {
        assert_eq!(is_plausible_account_key(&case.input), case.ok, "{:?}", case.input);
    }
    for case in cases.extract_key {
        assert_eq!(
            extract_account_key(&case.input),
            case.key.as_deref(),
            "{:?}",
            case.input
        );
    }
}

#[test]
fn server_made_keys_are_reforged_keys() {
    let key = format!("{ACCOUNT_KEY_PREFIX}{}", "ab".repeat(24));
    assert!(is_plausible_account_key(&key));
    assert_eq!(
        extract_account_key(&format!("Server: X Key: {key} Saved")),
        Some(key.as_str())
    );
}
