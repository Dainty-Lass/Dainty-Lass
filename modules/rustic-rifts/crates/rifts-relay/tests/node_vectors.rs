//! Replays what Mystic Paradox's own `relayCredentials.ts`, `p2pIcePolicy.ts` and `iceSessionRules.ts` answered
//! (`vectors/node-relay.json`, made by rifts-admission's `tests/vectors/make-vectors.mts` in `node:22`).

// The helpers are test code too, outside the #[test] functions clippy exempts
#![allow(clippy::unwrap_used)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use rifts_relay::rules::{SignalKind, filter_relay_only, parse_ice_signal};
use rifts_relay::{
    CloudflareAnalytics, ProviderAnswer, ProviderHttp, ProviderRequest, ProviderUnreachable, RelayConfig, TurnRest,
    derive_pair_key, mint_turn_rest, normalize_provider_ice_servers, read_monthly_relay_egress_bytes,
    relay_account_tag,
};
use rifts_seam::{AccountId, BoxFuture, Secret, UnixMillis, poll_once};
use serde_json::{Value, json};

fn vectors() -> Value {
    serde_json::from_str(include_str!("vectors/node-relay.json")).unwrap()
}

fn text(value: &Value) -> &str {
    value.as_str().unwrap()
}

fn account(value: &Value) -> AccountId {
    AccountId(text(value).to_owned())
}

#[test]
fn relay_tags_and_turn_rest_credentials_are_nodes() {
    let vectors = vectors();
    for entry in vectors["tags"].as_array().unwrap() {
        assert_eq!(relay_account_tag(&account(&entry["account"])), text(&entry["tag"]));
    }
    for entry in vectors["turnRest"].as_array().unwrap() {
        let rest = TurnRest {
            urls: vec!["turn:turn.example.org:3478".to_owned()],
            secret: Secret::new(text(&entry["secret"]).to_owned()),
        };
        let now = UnixMillis(entry["nowMs"].as_i64().unwrap());
        let ttl = u32::try_from(entry["ttl"].as_u64().unwrap()).unwrap();
        let minted = mint_turn_rest(&rest, &account(&entry["account"]), ttl, now);
        assert_eq!(minted.username, text(&entry["username"]));
        assert_eq!(minted.credential.expose(), text(&entry["credential"]));
        assert_eq!(json!(minted.urls), entry["urls"]);
    }
}

#[test]
fn pair_keys_are_nodes() {
    for entry in vectors()["pairKeys"].as_array().unwrap() {
        let key = derive_pair_key(
            &Secret::new(text(&entry["secret"]).to_owned()),
            text(&entry["sessionId"]),
            entry["generation"].as_u64().unwrap(),
            &account(&entry["host"]),
            &account(&entry["joiner"]),
        );
        assert_eq!(key.expose(), text(&entry["key"]));
    }
}

#[test]
fn signals_parse_and_filter_as_node_does() {
    let vectors = vectors();
    for entry in vectors["signals"].as_array().unwrap() {
        let parsed = parse_ice_signal(&entry["body"]).map(|signal| {
            json!({ "generation": signal.generation, "toAccountId": signal.to.0, "kind": signal.kind.as_str(),
                "data": signal.data })
        });
        assert_eq!(parsed.unwrap_or(Value::Null), entry["parsed"], "{}", entry["body"]);
    }
    for entry in vectors["filters"].as_array().unwrap() {
        let kind = match text(&entry["kind"]) {
            "DESCRIPTION" => SignalKind::Description,
            "CANDIDATE" => SignalKind::Candidate,
            _ => SignalKind::GatheringDone,
        };
        let kept = filter_relay_only(kind, text(&entry["data"]));
        assert_eq!(json!(kept), entry["kept"], "{}", entry["data"]);
    }
}

#[test]
fn provider_answers_normalize_as_node_does() {
    for entry in vectors()["provider"].as_array().unwrap() {
        let normalized = normalize_provider_ice_servers(Some(&entry["body"]));
        let mut expected = json!({ "stunUrls": normalized.stun_urls });
        if let Some((urls, username, credential)) = &normalized.turn {
            expected["turn"] = json!({ "urls": urls, "username": username, "credential": credential.expose() });
        }
        assert_eq!(expected, entry["normalized"], "{}", entry["body"]);
    }
}

#[test]
fn settings_read_as_node_reads_them() {
    for entry in vectors()["configs"].as_array().unwrap() {
        let settings: HashMap<String, String> = entry["env"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), text(v).to_owned()))
            .collect();
        let config = RelayConfig::from_settings(&settings);
        let summary: Value = serde_json::from_str(&config.summary_json()).unwrap();
        assert_eq!(summary, entry["summary"], "{}", entry["env"]);
        assert_eq!(json!(config.stun_urls), entry["stunUrls"]);
        assert_eq!(
            json!(config.turn_rest.as_ref().map(|rest| &rest.urls)),
            entry["turnRestUrls"]
        );
        assert_eq!(
            json!(config.cloudflare.as_ref().map(|key| &key.key_id)),
            entry["cloudflareKeyId"]
        );
        assert_eq!(json!(config.credential_ttl_seconds), entry["maxTtl"]);
        assert_eq!(json!(config.hub_credential_ttl_seconds), entry["hubTtl"]);
        assert_eq!(json!(config.configs_per_hour), entry["configsPerHour"]);
        assert_eq!(json!(config.monthly_quota_bytes), entry["quotaBytes"]);
        assert_eq!(
            json!(config.analytics.as_ref().map(|a| &a.account_id)),
            entry["analytics"]
        );
    }
}

#[derive(Default)]
struct Recorder(Mutex<Vec<ProviderRequest>>);

impl ProviderHttp for Recorder {
    fn post(&self, request: ProviderRequest) -> BoxFuture<'_, Result<ProviderAnswer, ProviderUnreachable>> {
        self.0.lock().unwrap().push(request);
        Box::pin(async {
            Ok(ProviderAnswer {
                status: 500,
                json: None,
            })
        })
    }
}

#[test]
fn the_usage_query_is_nodes_byte_for_byte() {
    let analytics = CloudflareAnalytics {
        account_id: "abcdef0123456789abcdef0123456789".to_owned(),
        api_token: Secret::new("analytics-token-1".to_owned()),
    };
    for entry in vectors()["usageQueries"].as_array().unwrap() {
        let recorder = Arc::new(Recorder::default());
        let who = account(&entry["account"]);
        let read = read_monthly_relay_egress_bytes(&analytics, &who, UnixMillis(1_800_000_000_000), recorder.as_ref());
        assert_eq!(poll_once(read), Some(None));
        let request = recorder.0.lock().unwrap().remove(0);
        assert_eq!(request.url, text(&entry["url"]));
        assert_eq!(request.body, text(&entry["body"]));
    }
}
