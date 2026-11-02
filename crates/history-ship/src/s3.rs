//! An S3-compatible bucket through signed requests (SigV4) on the workspace's HTTP client: PUT for the
//! server, ListObjectsV2 and GET for the owner. No SDK: three calls don't need one, and this keeps rustls
//! with aws-lc-rs and no OpenSSL (D10).
//!
//! HTTPS only, except to a loopback address (a local MinIO). Redirects are errors. An error carries the
//! status and S3's `<Code>`, never a header or a signature.

use std::sync::Arc;
use std::time::Duration;

use aws_lc_rs::hmac;
use base64::Engine as _;
use http_body_util::{BodyExt, Full, Limited};
use hyper::body::Bytes;
use hyper::{Method, Request, StatusCode};
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioExecutor;
use sha2::{Digest, Sha256};

use crate::Error;
use crate::bucket::{Bucket, BucketRead, ObjectInfo};
use crate::layout::{hex, sha256_hex};

type Https = hyper_rustls::HttpsConnector<HttpConnector>;

const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
/// A listing page or an error body is never this large.
const MAX_SMALL_BODY: usize = 16 << 20;

/// Where the bucket is and the credentials for it.
#[derive(Clone, PartialEq, Eq)]
pub struct S3Config {
    /// `https://s3.eu-central-003.backblazeb2.com`, `https://<account>.r2.cloudflarestorage.com`,
    /// `https://s3.us-east-1.amazonaws.com`; `http://127.0.0.1:9000` for a local MinIO only.
    pub endpoint: String,
    /// `us-east-1` and so on; R2 takes `auto`.
    pub region: String,
    pub bucket: String,
    /// `https://endpoint/bucket/key` instead of `https://bucket.endpoint/key`: MinIO, and R2 or B2 if
    /// asked; AWS wants virtual-hosted names.
    pub path_style: bool,
    pub access_key_id: String,
    pub secret_access_key: String,
}

// By hand: the secret stays out of every log line.
impl std::fmt::Debug for S3Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("S3Config")
            .field("endpoint", &self.endpoint)
            .field("region", &self.region)
            .field("bucket", &self.bucket)
            .field("path_style", &self.path_style)
            .field("access_key_id", &self.access_key_id)
            .finish_non_exhaustive()
    }
}

/// A bucket's name as S3 allows it: 3-63 of lower-case letters, digits, `-` and `.`.
fn valid_bucket(name: &str) -> bool {
    (3..=63).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'.')
        && name.bytes().next().is_some_and(|b| b.is_ascii_alphanumeric())
        && name.bytes().last().is_some_and(|b| b.is_ascii_alphanumeric())
}

#[derive(Clone)]
pub struct S3Bucket {
    config: Arc<S3Config>,
    scheme: String,
    /// The host (and port) requests go to.
    host: String,
    /// The path before the key: `/bucket` or empty.
    base_path: String,
    client: Client<Https, Full<Bytes>>,
    timeout: Duration,
    max_object: usize,
}

impl S3Bucket {
    pub fn new(config: S3Config) -> Result<Self, Error> {
        let url = url::Url::parse(&config.endpoint)
            .map_err(|e| Error::Config(format!("the bucket endpoint {:?}: {e}", config.endpoint)))?;
        let host = url
            .host_str()
            .ok_or_else(|| Error::Config("the bucket endpoint names no host".into()))?;
        let local = store::is_local_address(host);
        match url.scheme() {
            "https" => {}
            "http" if local => {}
            _ => {
                return Err(Error::Config(format!(
                    "the bucket endpoint {} must be https:// (http:// only to this machine)",
                    config.endpoint
                )));
            }
        }
        if url.path() != "/" || url.query().is_some() || !url.username().is_empty() || url.password().is_some() {
            return Err(Error::Config(
                "the bucket endpoint is a scheme, a host and a port, no more".into(),
            ));
        }
        if !valid_bucket(&config.bucket) {
            return Err(Error::Config(format!("{:?} is not a bucket name", config.bucket)));
        }
        if config.region.is_empty() || config.access_key_id.is_empty() || config.secret_access_key.is_empty() {
            return Err(Error::Config(
                "the bucket needs a region and both halves of its key".into(),
            ));
        }
        let authority = match url.port() {
            Some(port) => format!("{host}:{port}"),
            None => host.to_owned(),
        };
        let (host, base_path) = if config.path_style {
            (authority, format!("/{}", config.bucket))
        } else {
            (format!("{}.{authority}", config.bucket), String::new())
        };
        // aws-lc-rs by name, as crates/content does: with another provider compiled in there is no default
        let builder = hyper_rustls::HttpsConnectorBuilder::new();
        let builder = match builder.with_provider_and_webpki_roots(rustls::crypto::aws_lc_rs::default_provider()) {
            Ok(builder) => builder,
            Err(_) => hyper_rustls::HttpsConnectorBuilder::new().with_webpki_roots(),
        };
        let connector = builder.https_or_http().enable_http1().build();
        Ok(Self {
            scheme: url.scheme().to_owned(),
            host,
            base_path,
            config: Arc::new(config),
            client: Client::builder(TokioExecutor::new()).build(connector),
            timeout: Duration::from_secs(600),
            max_object: usize::try_from(crate::layout::MAX_SNAPSHOT_BYTES).unwrap_or(usize::MAX),
        })
    }

    /// How long one request may take, upload included (10 minutes unless set).
    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    async fn send(
        &self,
        method: Method,
        key: Option<&str>,
        query: &[(&str, &str)],
        extra: &[(&str, String)],
        body: Vec<u8>,
        max_reply: usize,
    ) -> Result<(StatusCode, Bytes), Error> {
        let path = match key {
            Some(key) => format!("{}/{}", self.base_path, uri_encode(key, false)),
            None if self.base_path.is_empty() => "/".to_owned(),
            None => self.base_path.clone(),
        };
        let payload = if body.is_empty() {
            EMPTY_SHA256.to_owned()
        } else {
            sha256_hex(&body)
        };
        let amz_date = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
        let mut headers: Vec<(String, String)> = vec![
            ("host".into(), self.host.clone()),
            ("x-amz-content-sha256".into(), payload.clone()),
            ("x-amz-date".into(), amz_date.clone()),
        ];
        headers.extend(extra.iter().map(|(k, v)| ((*k).to_owned(), v.clone())));
        let query: Vec<(String, String)> = query.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect();
        let authorization = sign(
            &self.config,
            method.as_str(),
            &path,
            &query,
            &mut headers,
            &payload,
            &amz_date,
        );
        let mut uri = format!("{}://{}{path}", self.scheme, self.host);
        if !query.is_empty() {
            uri.push('?');
            uri.push_str(&canonical_query(&query));
        }
        let mut request = Request::builder().method(method.clone()).uri(&uri);
        for (name, value) in &headers {
            if name != "host" {
                request = request.header(name.as_str(), value.as_str());
            }
        }
        let request = request
            .header("authorization", authorization)
            .body(Full::new(Bytes::from(body)))
            .map_err(|e| Error::Bucket(format!("{method} request: {e}")))?;
        let work = async {
            let response = self
                .client
                .request(request)
                .await
                .map_err(|e| Error::Bucket(format!("{method} {}: {e}", key.unwrap_or("/"))))?;
            let status = response.status();
            let limit = if status.is_success() { max_reply } else { MAX_SMALL_BODY };
            let body = Limited::new(response.into_body(), limit)
                .collect()
                .await
                .map(|b| b.to_bytes())
                .map_err(|e| Error::Bucket(format!("{method} {}: the reply: {e}", key.unwrap_or("/"))))?;
            Ok::<_, Error>((status, body))
        };
        let (status, body) = tokio::time::timeout(self.timeout, work)
            .await
            .map_err(|_| Error::Bucket(format!("{method} {}: timed out", key.unwrap_or("/"))))??;
        if !status.is_success() {
            let code = xml_text(&String::from_utf8_lossy(&body), "Code").unwrap_or_default();
            return Err(Error::Bucket(format!(
                "{method} {}: {status} {code}",
                key.unwrap_or("/")
            )));
        }
        Ok((status, body))
    }

    /// Creates the bucket with object lock on and a default retention of `days` in `mode`
    /// (`GOVERNANCE` or `COMPLIANCE`). The owner's credentials: for setup and the MinIO test, never the server.
    pub async fn create_locked(&self, mode: &str, days: u32) -> Result<(), Error> {
        if !matches!(mode, "GOVERNANCE" | "COMPLIANCE") || days == 0 {
            return Err(Error::Config(
                "object lock takes GOVERNANCE or COMPLIANCE and at least a day".into(),
            ));
        }
        let mut extra = vec![("x-amz-bucket-object-lock-enabled", "true".to_owned())];
        let region_body = if self.config.region == "us-east-1" || self.config.region == "auto" {
            Vec::new()
        } else {
            format!(
                "<CreateBucketConfiguration xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\">\
                 <LocationConstraint>{}</LocationConstraint></CreateBucketConfiguration>",
                self.config.region
            )
            .into_bytes()
        };
        self.send(Method::PUT, None, &[], &extra, region_body, MAX_SMALL_BODY)
            .await?;
        let rule = format!(
            "<ObjectLockConfiguration xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\">\
             <ObjectLockEnabled>Enabled</ObjectLockEnabled><Rule><DefaultRetention>\
             <Mode>{mode}</Mode><Days>{days}</Days></DefaultRetention></Rule></ObjectLockConfiguration>"
        )
        .into_bytes();
        extra.clear();
        extra.push(("x-amz-checksum-sha256", checksum(&rule)));
        extra.push(("content-type", "application/xml".to_owned()));
        self.send(Method::PUT, None, &[("object-lock", "")], &extra, rule, MAX_SMALL_BODY)
            .await?;
        Ok(())
    }
}

fn checksum(body: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(Sha256::digest(body))
}

impl Bucket for S3Bucket {
    async fn put(&self, key: &str, body: Vec<u8>, content_type: &'static str) -> Result<(), Error> {
        // A checksum header: S3 wants one on every PUT to an object-locked bucket, and checks it.
        let extra = [
            ("content-type", content_type.to_owned()),
            ("x-amz-checksum-sha256", checksum(&body)),
        ];
        self.send(Method::PUT, Some(key), &[], &extra, body, MAX_SMALL_BODY)
            .await?;
        Ok(())
    }
}

impl BucketRead for S3Bucket {
    async fn list(&self, prefix: &str, start_after: Option<&str>) -> Result<Vec<ObjectInfo>, Error> {
        let mut out = Vec::new();
        let mut token: Option<String> = None;
        loop {
            let mut query = vec![("list-type", "2"), ("prefix", prefix)];
            if let Some(start) = start_after {
                query.push(("start-after", start));
            }
            if let Some(token) = &token {
                query.push(("continuation-token", token.as_str()));
            }
            let (_, body) = self
                .send(Method::GET, None, &query, &[], Vec::new(), MAX_SMALL_BODY)
                .await?;
            let page = parse_list(&String::from_utf8_lossy(&body)).map_err(Error::Bucket)?;
            out.extend(page.objects);
            match page.next {
                Some(next) if page.truncated => token = Some(next),
                _ => break,
            }
        }
        Ok(out)
    }

    async fn get(&self, key: &str) -> Result<Vec<u8>, Error> {
        let (_, body) = self
            .send(Method::GET, Some(key), &[], &[], Vec::new(), self.max_object)
            .await?;
        Ok(body.to_vec())
    }
}

// ------------------------------------------------------------------ SigV4

/// RFC 3986 unreserved characters stay; everything else is `%XX`. `/` too when `slash` (query values).
pub(crate) fn uri_encode(text: &str, slash: bool) -> String {
    let mut out = String::with_capacity(text.len());
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') || (b == b'/' && !slash) {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn canonical_query(query: &[(String, String)]) -> String {
    let mut pairs: Vec<(String, String)> = query
        .iter()
        .map(|(k, v)| (uri_encode(k, true), uri_encode(v, true)))
        .collect();
    pairs.sort();
    pairs
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&")
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    hmac::sign(&hmac::Key::new(hmac::HMAC_SHA256, key), data)
        .as_ref()
        .to_vec()
}

/// The `Authorization` header for a request. `path` is already encoded; `headers` (names lower case) are
/// all signed, and come back sorted.
pub(crate) fn sign(
    config: &S3Config,
    method: &str,
    path: &str,
    query: &[(String, String)],
    headers: &mut [(String, String)],
    payload_sha256: &str,
    amz_date: &str,
) -> String {
    headers.sort_by(|a, b| a.0.cmp(&b.0));
    let canonical_headers: String = headers.iter().map(|(k, v)| format!("{k}:{}\n", v.trim())).collect();
    let signed: Vec<&str> = headers.iter().map(|(k, _)| k.as_str()).collect();
    let signed = signed.join(";");
    let request = format!(
        "{method}\n{path}\n{}\n{canonical_headers}\n{signed}\n{payload_sha256}",
        canonical_query(query)
    );
    let date = amz_date.get(..8).unwrap_or_default();
    let scope = format!("{date}/{}/s3/aws4_request", config.region);
    let to_sign = format!(
        "AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}",
        hex(&Sha256::digest(request.as_bytes()))
    );
    let mut key = hmac_sha256(format!("AWS4{}", config.secret_access_key).as_bytes(), date.as_bytes());
    for part in [config.region.as_str(), "s3", "aws4_request"] {
        key = hmac_sha256(&key, part.as_bytes());
    }
    let signature = hex(&hmac_sha256(&key, to_sign.as_bytes()));
    format!(
        "AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={signed}, Signature={signature}",
        config.access_key_id
    )
}

// ------------------------------------------------------------------ the listing's XML

/// One page of ListObjectsV2.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ListPage {
    pub objects: Vec<ObjectInfo>,
    pub truncated: bool,
    pub next: Option<String>,
}

fn unescape(text: &str) -> Result<String, String> {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let end = rest.find(';').ok_or("an unterminated entity")?;
        let entity = &rest[1..end];
        let c = match entity {
            "amp" => '&',
            "lt" => '<',
            "gt" => '>',
            "quot" => '"',
            "apos" => '\'',
            _ => {
                let code = if let Some(hex) = entity.strip_prefix("#x") {
                    u32::from_str_radix(hex, 16).ok()
                } else if let Some(dec) = entity.strip_prefix('#') {
                    dec.parse().ok()
                } else {
                    None
                };
                code.and_then(char::from_u32).ok_or("an unknown entity")?
            }
        };
        out.push(c);
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

/// The text of the first `<tag>` in `xml`, unescaped.
fn xml_text(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)? + start;
    unescape(&xml[start..end]).ok()
}

/// A ListObjectsV2 reply. The bucket is the owner's, but anyone with the server's key wrote the names.
pub fn parse_list(xml: &str) -> Result<ListPage, String> {
    if !xml.contains("<ListBucketResult") {
        return Err("not a ListBucketResult".into());
    }
    let mut page = ListPage::default();
    let mut rest = xml;
    while let Some(start) = rest.find("<Contents>") {
        let after = &rest[start + "<Contents>".len()..];
        let end = after.find("</Contents>").ok_or("an unterminated Contents")?;
        let item = &after[..end];
        let key = xml_text(item, "Key").ok_or("a Contents without a Key")?;
        let modified = xml_text(item, "LastModified").ok_or("a Contents without LastModified")?;
        let last_modified = chrono::DateTime::parse_from_rfc3339(&modified)
            .map_err(|e| format!("LastModified {modified:?}: {e}"))?
            .timestamp_millis();
        let size = xml_text(item, "Size").and_then(|s| s.parse().ok()).unwrap_or(0);
        page.objects.push(ObjectInfo {
            key,
            last_modified,
            size,
        });
        rest = &after[end + "</Contents>".len()..];
    }
    page.truncated = xml_text(xml, "IsTruncated").is_some_and(|t| t == "true");
    page.next = xml_text(xml, "NextContinuationToken");
    Ok(page)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    /// The worked examples in AWS's "Signature Calculations for the Authorization Header" for S3.
    fn example() -> S3Config {
        S3Config {
            endpoint: "https://s3.amazonaws.com".into(),
            region: "us-east-1".into(),
            bucket: "examplebucket".into(),
            path_style: false,
            access_key_id: "AKIAIOSFODNN7EXAMPLE".into(),
            secret_access_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".into(),
        }
    }

    fn headers(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs.iter().map(|(k, v)| ((*k).into(), (*v).into())).collect()
    }

    #[test]
    fn sigv4_matches_aws_examples() {
        let config = example();
        let date = "20130524T000000Z";
        let get = sign(
            &config,
            "GET",
            "/test.txt",
            &[],
            &mut headers(&[
                ("host", "examplebucket.s3.amazonaws.com"),
                ("range", "bytes=0-9"),
                ("x-amz-content-sha256", EMPTY_SHA256),
                ("x-amz-date", date),
            ]),
            EMPTY_SHA256,
            date,
        );
        assert!(
            get.ends_with("Signature=f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41"),
            "{get}"
        );
        assert!(get.starts_with(
            "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request, \
             SignedHeaders=host;range;x-amz-content-sha256;x-amz-date,"
        ));

        let body = b"Welcome to Amazon S3.";
        let payload = sha256_hex(body);
        let put = sign(
            &config,
            "PUT",
            &format!("/{}", uri_encode("test$file.text", false)),
            &[],
            &mut headers(&[
                ("date", "Fri, 24 May 2013 00:00:00 GMT"),
                ("host", "examplebucket.s3.amazonaws.com"),
                ("x-amz-content-sha256", &payload),
                ("x-amz-date", date),
                ("x-amz-storage-class", "REDUCED_REDUNDANCY"),
            ]),
            &payload,
            date,
        );
        assert!(
            put.ends_with("Signature=98ad721746da40c64f1a55b78f14c238d841ea1380cd77a1b5971af0ece108bd"),
            "{put}"
        );

        let list = sign(
            &config,
            "GET",
            "/",
            &[("max-keys".into(), "2".into()), ("prefix".into(), "J".into())],
            &mut headers(&[
                ("host", "examplebucket.s3.amazonaws.com"),
                ("x-amz-content-sha256", EMPTY_SHA256),
                ("x-amz-date", date),
            ]),
            EMPTY_SHA256,
            date,
        );
        assert!(
            list.ends_with("Signature=34b48302e7b5fa45bde8084f4b7868a86f0a534bc59db6670ed5711ef69dc6f7"),
            "{list}"
        );
    }

    #[test]
    fn query_values_are_encoded_slashes_and_all() {
        let q = canonical_query(&[
            ("prefix".into(), "srv/log/".into()),
            ("list-type".into(), "2".into()),
            ("object-lock".into(), String::new()),
        ]);
        assert_eq!(q, "list-type=2&object-lock=&prefix=srv%2Flog%2F");
        assert_eq!(uri_encode("a b/c~", false), "a%20b/c~");
    }

    #[test]
    fn endpoints_and_debug() {
        let ok = S3Config {
            endpoint: "http://127.0.0.1:9000".into(),
            path_style: true,
            ..example()
        };
        let bucket = S3Bucket::new(ok.clone()).unwrap();
        assert_eq!(
            (bucket.host.as_str(), bucket.base_path.as_str()),
            ("127.0.0.1:9000", "/examplebucket")
        );
        let virtual_hosted = S3Bucket::new(example()).unwrap();
        assert_eq!(virtual_hosted.host, "examplebucket.s3.amazonaws.com");
        for endpoint in [
            "http://s3.example.org",
            "ftp://127.0.0.1",
            "https://h/path",
            "https://u:p@h",
        ] {
            let bad = S3Config {
                endpoint: endpoint.into(),
                ..ok.clone()
            };
            assert!(S3Bucket::new(bad).is_err(), "{endpoint}");
        }
        for name in ["ab", "Upper", "-lead", "trail-", "a_b"] {
            let bad = S3Config {
                bucket: name.into(),
                ..ok.clone()
            };
            assert!(S3Bucket::new(bad).is_err(), "{name}");
        }
        assert!(!format!("{ok:?}").contains("EXAMPLEKEY"));
    }

    #[test]
    fn listings_parse() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<ListBucketResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/"><Name>b</Name><Prefix>p/</Prefix>
<Contents><Key>p/a&amp;b</Key><LastModified>2026-10-26T12:00:00.000Z</LastModified><ETag>&quot;x&quot;</ETag><Size>12</Size></Contents>
<Contents><Key>p/c</Key><LastModified>2026-10-26T12:00:01Z</LastModified><Size>0</Size></Contents>
<IsTruncated>true</IsTruncated><NextContinuationToken>tok/en=</NextContinuationToken></ListBucketResult>"#;
        let page = parse_list(xml).unwrap();
        assert_eq!(page.objects.len(), 2);
        assert_eq!(page.objects[0].key, "p/a&b");
        assert_eq!(page.objects[0].size, 12);
        assert_eq!(page.objects[1].last_modified - page.objects[0].last_modified, 1000);
        assert!(page.truncated);
        assert_eq!(page.next.as_deref(), Some("tok/en="));
        assert!(parse_list("<Error><Code>AccessDenied</Code></Error>").is_err());
        assert!(parse_list("<ListBucketResult><Contents><Key>x</Key>").is_err());
        assert_eq!(
            xml_text("<Error><Code>AccessDenied</Code></Error>", "Code").as_deref(),
            Some("AccessDenied")
        );
    }
}
