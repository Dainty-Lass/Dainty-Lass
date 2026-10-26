//! The calls only a Dauntless Revived server answers (DR's `net/hostapi.rs` at 24b7624): its content
//! server's game manifest, the hunt region and the end of a play session. DR's device signals are not
//! ported (the server treats them as optional).

use hyper::Method;
use serde_json::Value;

use super::{REVIVED, key_headers_as, options};
use crate::net::http::{Endpoint, HttpError, parse_json_body, request};

const CONTENT_MANIFEST_PATH: &str = "/content/v1/manifest";

/// Public mode: is the host's content server behind the gateway?
pub async fn probe_content(endpoint: &Endpoint) -> bool {
    request(endpoint, CONTENT_MANIFEST_PATH, options(Method::HEAD, 8000, 1024))
        .await
        .is_ok_and(|response| response.status == 200)
}

pub async fn fetch_content_manifest(endpoint: &Endpoint) -> Option<Value> {
    let response = request(
        endpoint,
        CONTENT_MANIFEST_PATH,
        options(Method::GET, 20000, 8 * 1024 * 1024),
    )
    .await
    .ok()?;
    if response.status == 200 {
        parse_json_body(&response)
    } else {
        None
    }
}

pub async fn save_hunt_region(endpoint: &Endpoint, key: &str, region: &str) -> Result<bool, HttpError> {
    let mut request_options = options(Method::POST, 10000, 4096);
    request_options.headers = key_headers_as(REVIVED, endpoint, Some(key));
    request_options.body = Some(serde_json::json!({ "region": region }).to_string());
    let response = request(endpoint, &format!("{}/HuntRegion", REVIVED.api), request_options).await?;
    let body = parse_json_body(&response);
    Ok(response.status == 200
        && body
            .as_ref()
            .and_then(|body| body.get("region"))
            .and_then(Value::as_str)
            == Some(region))
}

pub async fn end_playing(endpoint: &Endpoint, key: &str) -> Result<(), HttpError> {
    let mut request_options = options(Method::POST, 5000, 4096);
    request_options.headers = key_headers_as(REVIVED, endpoint, Some(key));
    request(endpoint, &format!("{}/PlayingEnded", REVIVED.api), request_options)
        .await
        .map(|_| ())
}
