//! The `events` query (the module-sharing plan's A4): the registry's events as the controller's recorder kept
//! them (A5), oldest first, for the dashboard to page through with `after`.

use async_graphql::{Context, SimpleObject};
use dainty_module_api::{ErrorKind, EventName};

use crate::module_control::control;
use crate::schema::{Code, fail};

/// Rows `events` returns when `limit` is left out.
pub const DEFAULT_LIMIT: i64 = 100;
/// The most rows one `events` call returns.
pub const MAX_LIMIT: i64 = 1000;
/// The most names one `events` call filters on.
pub const MAX_NAMES: usize = 32;

/// One kept event, as the controller's recorder hands it over.
#[derive(Clone, Debug, PartialEq)]
pub struct KeptEvent {
    /// Row number.
    pub seq: i64,
    /// Unix milliseconds.
    pub at_ms: i64,
    /// The event's name.
    pub name: String,
    /// The emitting module's id; `None` for the host.
    pub source: Option<String>,
    /// The event's data.
    pub payload: serde_json::Value,
}

/// One kept event.
#[derive(SimpleObject, Clone, Debug, PartialEq)]
pub struct EventEntry {
    /// Row number; the next page's `after`.
    pub seq: i64,
    /// Unix milliseconds the recorder received it.
    pub at_ms: i64,
    /// Such as game.server.down, or events.lagged where the recorder fell behind.
    pub name: String,
    /// The module that emitted it; null for the host.
    pub source: Option<String>,
    /// The event's data; never a secret or an invite code.
    pub payload: async_graphql::Json<serde_json::Value>,
}

/// The `events` query.
pub async fn list(
    ctx: &Context<'_>,
    after: Option<i64>,
    names: Option<Vec<String>>,
    limit: Option<i64>,
) -> async_graphql::Result<Vec<EventEntry>> {
    let after = after.unwrap_or(0);
    if after < 0 {
        return Err(fail(Code::BadInput, "after is a row number, 0 or more"));
    }
    let limit = limit.unwrap_or(DEFAULT_LIMIT);
    if !(1..=MAX_LIMIT).contains(&limit) {
        return Err(fail(Code::BadInput, format!("limit is 1 to {MAX_LIMIT}")));
    }
    let names = names.unwrap_or_default();
    if names.len() > MAX_NAMES {
        return Err(fail(Code::BadInput, format!("names takes at most {MAX_NAMES}")));
    }
    if names.iter().any(|n| EventName::new(n).is_err()) {
        return Err(fail(Code::BadInput, "names are event names, such as game.server.down"));
    }
    control(ctx)?
        .events(after, names, limit)
        .await
        .map(|kept| {
            kept.into_iter()
                .map(|e| EventEntry {
                    seq: e.seq,
                    at_ms: e.at_ms,
                    name: e.name,
                    source: e.source,
                    payload: async_graphql::Json(e.payload),
                })
                .collect()
        })
        .map_err(|e| match e.kind {
            ErrorKind::Unsupported => fail(Code::NotSupported, "this server keeps no events"),
            _ => fail(Code::BackendUnavailable, "the events database did not answer"),
        })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "a panic is how a test fails")]

    use std::sync::{Arc, Mutex};

    use async_graphql::{EmptyMutation, EmptySubscription, Object, Schema};
    use dainty_module_api::{BoxFuture, ModuleError};
    use serde_json::json;

    use super::*;
    use crate::module_control::{Module, ModuleControl, Modules, Pending};

    #[derive(Default)]
    struct Fake(Mutex<Option<(i64, Vec<String>, i64)>>);

    impl ModuleControl for Fake {
        fn list(&self) -> Vec<Module> {
            Vec::new()
        }

        fn get(&self, id: &str) -> Result<Module, String> {
            Err(format!("no module {id}"))
        }

        fn load<'a>(&'a self, id: &'a str) -> Pending<'a, Module> {
            Box::pin(async move { Err(format!("no module {id}")) })
        }

        fn unload<'a>(&'a self, id: &'a str) -> Pending<'a, Module> {
            self.load(id)
        }

        fn restart<'a>(&'a self, id: &'a str) -> Pending<'a, Module> {
            self.load(id)
        }

        fn events(
            &self,
            after: i64,
            names: Vec<String>,
            limit: i64,
        ) -> BoxFuture<'_, Result<Vec<KeptEvent>, ModuleError>> {
            *self.0.lock().unwrap() = Some((after, names, limit));
            Box::pin(async move {
                Ok(vec![KeptEvent {
                    seq: after + 1,
                    at_ms: 1_700_000_000_000,
                    name: "game.server.down".into(),
                    source: Some("rustic-isles".into()),
                    payload: json!({"id": "hub-1"}),
                }])
            })
        }
    }

    struct Q;

    #[Object]
    impl Q {
        async fn events(
            &self,
            ctx: &Context<'_>,
            after: Option<i64>,
            names: Option<Vec<String>>,
            limit: Option<i64>,
        ) -> async_graphql::Result<Vec<EventEntry>> {
            list(ctx, after, names, limit).await
        }
    }

    async fn run(modules: Modules, query: &str) -> serde_json::Value {
        let schema = Schema::build(Q, EmptyMutation, EmptySubscription)
            .data(modules)
            .finish();
        serde_json::to_value(schema.execute(query).await).unwrap()
    }

    #[tokio::test]
    async fn events_pages_through_the_recorder() {
        let fake = Arc::new(Fake::default());
        let modules = Modules(Some(fake.clone()));
        let got = run(
            modules.clone(),
            "{ events(after: 4, names: [\"game.server.down\"], limit: 2) { seq atMs name source payload } }",
        )
        .await;
        assert_eq!(
            got["data"]["events"],
            json!([{"seq": 5, "atMs": 1_700_000_000_000_i64, "name": "game.server.down", "source": "rustic-isles",
                    "payload": {"id": "hub-1"}}])
        );
        assert_eq!(
            *fake.0.lock().unwrap(),
            Some((4, vec!["game.server.down".to_owned()], 2))
        );
        run(modules.clone(), "{ events { seq } }").await;
        assert_eq!(*fake.0.lock().unwrap(), Some((0, Vec::new(), DEFAULT_LIMIT)));

        for bad in [
            "{ events(after: -1) { seq } }",
            "{ events(limit: 0) { seq } }",
            "{ events(limit: 1001) { seq } }",
            "{ events(names: [\"Not An Event\"]) { seq } }",
        ] {
            let refused = run(modules.clone(), bad).await;
            assert_eq!(refused["errors"][0]["extensions"]["code"], "BAD_INPUT", "{bad}");
        }
    }

    #[tokio::test]
    async fn events_needs_a_recorder() {
        struct Bare;
        impl ModuleControl for Bare {
            fn list(&self) -> Vec<Module> {
                Vec::new()
            }
            fn get(&self, id: &str) -> Result<Module, String> {
                Err(id.to_owned())
            }
            fn load<'a>(&'a self, id: &'a str) -> Pending<'a, Module> {
                Box::pin(async move { Err(id.to_owned()) })
            }
            fn unload<'a>(&'a self, id: &'a str) -> Pending<'a, Module> {
                self.load(id)
            }
            fn restart<'a>(&'a self, id: &'a str) -> Pending<'a, Module> {
                self.load(id)
            }
        }
        let none = run(Modules(Some(Arc::new(Bare))), "{ events { seq } }").await;
        assert_eq!(none["errors"][0]["extensions"]["code"], "NOT_SUPPORTED");
        let alone = run(Modules::default(), "{ events { seq } }").await;
        assert_eq!(alone["errors"][0]["extensions"]["code"], "MODULES_UNAVAILABLE");
    }
}
