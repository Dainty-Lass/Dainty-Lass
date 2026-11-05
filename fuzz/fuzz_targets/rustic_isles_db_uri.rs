//! The addresses the database tool is given (spec-rustic-isles-db §4.1, §9.1, §15): arbitrary text as a MongoDB
//! URI and as a Postgres URL. Nothing panics, and a parsed address's `Debug` does not depend on its password.
#![no_main]

use libfuzzer_sys::fuzz_target;
use rustic_isles_db::mongo::MongoUri;
use rustic_isles_db::parse_postgres_url;

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    if let Ok(uri) = MongoUri::parse(&text) {
        let mut other = uri.clone();
        other.password = other.password.map(|p| format!("{p}-other"));
        assert_eq!(format!("{uri:?}"), format!("{other:?}"), "the password reached Debug");
    }
    if let Ok(config) = parse_postgres_url(&text, true) {
        let mut other = config.clone();
        other.password.push_str("-other");
        assert_eq!(
            format!("{config:?}"),
            format!("{other:?}"),
            "the password reached Debug"
        );
    }
});
