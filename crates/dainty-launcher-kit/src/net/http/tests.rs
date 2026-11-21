// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT OR Apache-2.0

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

use std::time::Duration;

use hyper::Method;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use super::to_file::{FileRequestError, FileRequestOptions, request_to_file};
use super::*;
use crate::net::pinned::certificate_fingerprint;
use crate::net::pinned::tests::{acceptor, certificate};

const SECONDS: Duration = Duration::from_secs(3);

fn local(port: u16) -> Endpoint {
    Endpoint {
        host: "127.0.0.1".to_owned(),
        port,
        pin: None,
    }
}

/// Reads one request (head and a `content-length` body) and answers its text.
async fn read_request<S: AsyncRead + Unpin>(stream: &mut S) -> String {
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    while !buf.ends_with(b"\r\n\r\n") {
        if stream.read(&mut byte).await.unwrap_or(0) == 0 {
            break;
        }
        buf.push(byte[0]);
    }
    let head = String::from_utf8_lossy(&buf).into_owned();
    let length = head
        .lines()
        .find_map(|line| line.strip_prefix("content-length: "))
        .map_or(0, |len| len.trim().parse::<usize>().unwrap());
    let mut body = vec![0u8; length];
    stream.read_exact(&mut body).await.unwrap();
    head + &String::from_utf8_lossy(&body)
}

async fn answer<S: AsyncRead + AsyncWrite + Unpin>(mut stream: S, reply: Vec<u8>, hang: bool) -> String {
    let request = read_request(&mut stream).await;
    let _ = stream.write_all(&reply).await;
    let _ = stream.flush().await;
    if hang {
        std::future::pending::<()>().await;
    }
    request
}

/// A plain server that answers one request with `reply` and, with `hang`, then holds the connection open.
async fn plain_server(reply: &[u8], hang: bool) -> (u16, JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let reply = reply.to_vec();
    let task = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        answer(tcp, reply, hang).await
    });
    (port, task)
}

/// The same over TLS, with the certificate's fingerprint.
async fn tls_server(reply: &[u8]) -> (u16, String, JoinHandle<String>) {
    let (cert, key) = certificate();
    let pin = certificate_fingerprint(cert.as_ref());
    let tls = acceptor(cert, key);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let reply = reply.to_vec();
    let task = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        match tls.accept(tcp).await {
            Ok(stream) => answer(stream, reply, false).await,
            Err(_) => String::new(),
        }
    });
    (port, pin, task)
}

fn header_lines(request: &str) -> Vec<&str> {
    request.split("\r\n\r\n").next().unwrap().lines().skip(1).collect()
}

const OK_JSON: &[u8] = b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 8\r\n\r\n{\"a\":1}\n";

#[tokio::test]
async fn a_get_sends_accept_then_host() {
    let (port, server) = plain_server(OK_JSON, false).await;
    let response = request(&local(port), "/v1/status?x=1", RequestOptions::get(SECONDS, 1024))
        .await
        .unwrap();
    assert_eq!(response.status, 200);
    assert_eq!(parse_json_body(&response), Some(serde_json::json!({ "a": 1 })));
    let seen = server.await.unwrap();
    assert!(seen.starts_with("GET /v1/status?x=1 HTTP/1.1\r\n"), "{seen}");
    assert_eq!(
        header_lines(&seen),
        ["accept: application/json", &format!("host: 127.0.0.1:{port}")]
    );
}

#[tokio::test]
async fn a_body_follows_the_callers_headers_with_its_type_and_length() {
    let (port, server) = plain_server(OK_JSON, false).await;
    let options = RequestOptions {
        method: Method::POST,
        headers: vec![("x-account-key", "DLK_secret".to_owned())],
        body: Some("{\"b\":2}".to_owned()),
        ..RequestOptions::get(SECONDS, 1024)
    };
    request(&local(port), "/v1/register", options).await.unwrap();
    let seen = server.await.unwrap();
    assert!(seen.starts_with("POST /v1/register HTTP/1.1\r\n"), "{seen}");
    assert_eq!(
        header_lines(&seen),
        [
            "accept: application/json",
            &format!("host: 127.0.0.1:{port}"),
            "x-account-key: DLK_secret",
            "content-type: application/json",
            "content-length: 7",
        ]
    );
    assert!(seen.ends_with("\r\n\r\n{\"b\":2}"), "{seen}");
}

#[tokio::test]
async fn a_callers_accept_replaces_the_default() {
    let (port, server) = plain_server(OK_JSON, false).await;
    let mut options = RequestOptions::get(SECONDS, 1024);
    options.headers.push(("accept", "application/octet-stream".to_owned()));
    request(&local(port), "/x", options).await.unwrap();
    let seen = server.await.unwrap();
    assert_eq!(
        header_lines(&seen),
        [&format!("host: 127.0.0.1:{port}"), "accept: application/octet-stream"]
    );
}

#[tokio::test]
async fn a_redirect_is_answered_not_followed() {
    let reply = b"HTTP/1.1 302 Found\r\nlocation: http://127.0.0.1:1/elsewhere\r\ncontent-length: 0\r\n\r\n";
    let (port, server) = plain_server(reply, false).await;
    let response = request(&local(port), "/x", RequestOptions::get(SECONDS, 1024))
        .await
        .unwrap();
    assert_eq!(response.status, 302);
    assert!(response.body.is_empty());
    server.await.unwrap();
}

#[tokio::test]
async fn a_body_over_the_limit_is_too_large() {
    let (port, _server) = plain_server(OK_JSON, false).await;
    let err = request(&local(port), "/x", RequestOptions::get(SECONDS, 7))
        .await
        .unwrap_err();
    assert_eq!(err, HttpError::TooLarge);
}

#[tokio::test]
async fn the_deadline_covers_the_whole_exchange() {
    // The head arrives, the body never does
    let reply = b"HTTP/1.1 200 OK\r\ncontent-length: 10\r\n\r\nabc";
    let (port, server) = plain_server(reply, true).await;
    let options = RequestOptions::get(Duration::from_millis(300), 1024);
    let err = request(&local(port), "/x", options).await.unwrap_err();
    assert_eq!(err, HttpError::Timeout);
    server.abort();
}

#[tokio::test]
async fn a_cancel_aborts_in_flight_and_before_starting() {
    let reply = b"HTTP/1.1 200 OK\r\ncontent-length: 10\r\n\r\nabc";
    let (port, server) = plain_server(reply, true).await;
    let token = CancellationToken::new();
    let options = RequestOptions {
        cancel: Some(token.clone()),
        ..RequestOptions::get(SECONDS, 1024)
    };
    let canceller = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(150)).await;
        token.cancel();
    });
    let err = request(&local(port), "/x", options).await.unwrap_err();
    assert_eq!(err, HttpError::Aborted);
    canceller.await.unwrap();
    server.abort();

    let token = CancellationToken::new();
    token.cancel();
    let options = RequestOptions {
        cancel: Some(token),
        ..RequestOptions::get(SECONDS, 1024)
    };
    // Port 9 is never listened on here; an already cancelled request does not connect
    let err = request(&local(9), "/x", options).await.unwrap_err();
    assert_eq!(err, HttpError::Aborted);
}

#[tokio::test]
async fn a_pinned_request_names_the_port_and_a_wrong_pin_reports_the_one_seen() {
    let (port, pin, server) = tls_server(OK_JSON).await;
    let endpoint = Endpoint {
        pin: Some(pin.clone()),
        ..local(port)
    };
    let response = request(&endpoint, "/x", RequestOptions::get(SECONDS, 1024))
        .await
        .unwrap();
    assert_eq!(response.status, 200);
    let seen = server.await.unwrap();
    assert_eq!(
        header_lines(&seen),
        ["accept: application/json", &format!("host: 127.0.0.1:{port}")]
    );

    let (port, pin, server) = tls_server(OK_JSON).await;
    let endpoint = Endpoint {
        pin: Some("0".repeat(64)),
        ..local(port)
    };
    let err = request(&endpoint, "/x", RequestOptions::get(SECONDS, 1024))
        .await
        .unwrap_err();
    assert_eq!(err, HttpError::Pin { actual: pin });
    server.await.unwrap();
}

#[test]
fn the_host_header_drops_the_port_only_for_pinned_443() {
    let pinned = |port| Endpoint {
        host: "play.example.org".to_owned(),
        port,
        pin: Some("a".repeat(64)),
    };
    assert_eq!(host_header(&pinned(443)), "play.example.org");
    assert_eq!(host_header(&pinned(8443)), "play.example.org:8443");
    assert_eq!(host_header(&local(80)), "127.0.0.1:80");
    assert_eq!(host_header(&local(443)), "127.0.0.1:443");
}

#[test]
fn endpoints_are_checked_before_connecting() {
    let fp = "ab".repeat(32);
    let with = |host: &str, port: u16, pin: Option<&str>| Endpoint {
        host: host.to_owned(),
        port,
        pin: pin.map(str::to_owned),
    };
    for ok in [
        with("127.0.0.1", 1, None),
        with("localhost", 61000, None),
        with("100.64.0.1", 61000, None),
        with("box.tail1234.ts.net", 61000, None),
        with("play.example.org", 443, Some(&fp)),
        with("203.0.113.5", 8443, Some(&fp)),
    ] {
        assert!(validate_endpoint(&ok).is_ok(), "{ok:?}");
    }
    for bad in [
        with("127.0.0.1", 0, None),
        with("", 1, None),
        with("Play.example.org", 443, Some(&fp)),
        with("play example.org", 443, Some(&fp)),
        with("::1", 443, Some(&fp)),
        with("play.example.org", 443, Some(&fp.to_uppercase())),
        with("play.example.org", 443, Some("ab:cd")),
        with("203.0.113.5", 61000, None),
        with("192.168.1.10", 61000, None),
        with("play.example.org", 61000, None),
    ] {
        assert!(matches!(validate_endpoint(&bad), Err(HttpError::Io(_))), "{bad:?}");
    }
    assert_eq!(
        check_endpoint(&with("play.example.org", 443, Some(&fp))),
        Ok(Some(fp.clone()))
    );
    assert_eq!(check_endpoint(&with("127.0.0.1", 1, None)), Ok(None));
}

#[test]
fn a_pinned_target_exists_only_with_a_pin() {
    let fp = "cd".repeat(32);
    let endpoint = Endpoint {
        host: "play.example.org".to_owned(),
        port: 8443,
        pin: Some(fp.clone()),
    };
    assert_eq!(
        pinned_target(&endpoint),
        Some(PinnedTarget {
            host: "play.example.org".to_owned(),
            port: 8443,
            fingerprint: fp,
        })
    );
    assert_eq!(pinned_target(&local(1)), None);
}

#[tokio::test]
async fn a_path_must_start_with_a_slash() {
    let err = request(&local(1), "x", RequestOptions::get(SECONDS, 1))
        .await
        .unwrap_err();
    assert!(matches!(err, HttpError::Protocol(_)), "{err:?}");
}

#[test]
fn debug_text_never_shows_header_values() {
    let options = RequestOptions {
        headers: vec![("x-account-key", "DLK_hidden".to_owned())],
        body: Some("{\"key\":\"DLK_body\"}".to_owned()),
        ..RequestOptions::get(SECONDS, 1)
    };
    let text = format!("{options:?}");
    assert!(text.contains("x-account-key"), "{text}");
    assert!(!text.contains("DLK_"), "{text}");

    let file = FileRequestOptions {
        headers: vec![("x-account-key", "DLK_hidden".to_owned())],
        idle_timeout: SECONDS,
        max_bytes: 1,
        cancel: None,
        on_bytes: None,
    };
    let text = format!("{file:?}");
    assert!(text.contains("x-account-key") && !text.contains("DLK_"), "{text}");
}

#[test]
fn json_bodies_are_read_lossily_and_empty_is_none() {
    let response = |body: &[u8]| HttpResponse {
        status: 200,
        body: Bytes::copy_from_slice(body),
    };
    assert_eq!(parse_json_body(&response(b"")), None);
    assert_eq!(parse_json_body(&response(b"not json")), None);
    assert_eq!(parse_json_body(&response(b"[1]")), Some(serde_json::json!([1])));
    assert_eq!(
        parse_json_body(&response(b"{\"name\":\"a\xffb\"}")),
        Some(serde_json::json!({ "name": "a\u{fffd}b" }))
    );
}

// --- to a file ---

fn file_options<'a>(max_bytes: u64, on_bytes: Option<&'a mut (dyn FnMut(u64) + Send)>) -> FileRequestOptions<'a> {
    FileRequestOptions {
        headers: Vec::new(),
        idle_timeout: SECONDS,
        max_bytes,
        cancel: None,
        on_bytes,
    }
}

#[tokio::test]
async fn a_200_is_written_with_each_chunk_reported() {
    let reply = b"HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\n\r\n3\r\nabc\r\n2\r\nde\r\n0\r\n\r\n";
    let (port, server) = plain_server(reply, false).await;
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("sub").join("file.bin");
    let mut sizes = Vec::new();
    let mut on_bytes = |n: u64| sizes.push(n);
    let status = request_to_file(&local(port), "/f?v=1", &target, file_options(5, Some(&mut on_bytes)))
        .await
        .unwrap();
    assert_eq!(status, 200);
    assert_eq!(std::fs::read(&target).unwrap(), b"abcde");
    assert_eq!(sizes.iter().sum::<u64>(), 5);
    let seen = server.await.unwrap();
    assert!(seen.starts_with("GET /f?v=1 HTTP/1.1\r\n"), "{seen}");
    assert_eq!(
        header_lines(&seen),
        ["accept: application/octet-stream", &format!("host: 127.0.0.1:{port}")]
    );
}

#[tokio::test]
async fn another_status_writes_nothing() {
    let reply = b"HTTP/1.1 404 Not Found\r\ncontent-length: 4\r\n\r\nnope";
    let (port, _server) = plain_server(reply, false).await;
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("file.bin");
    let status = request_to_file(&local(port), "/f", &target, file_options(100, None))
        .await
        .unwrap();
    assert_eq!(status, 404);
    assert!(!target.exists());
}

#[tokio::test]
async fn a_declared_length_over_the_limit_creates_nothing() {
    let reply = b"HTTP/1.1 200 OK\r\ncontent-length: 10\r\n\r\n0123456789";
    let (port, _server) = plain_server(reply, false).await;
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("file.bin");
    let err = request_to_file(&local(port), "/f", &target, file_options(9, None))
        .await
        .unwrap_err();
    assert!(matches!(err, FileRequestError::Http(HttpError::TooLarge)), "{err:?}");
    assert!(!target.exists());
}

#[tokio::test]
async fn a_running_total_over_the_limit_removes_the_file() {
    let reply = b"HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\n\r\n3\r\nabc\r\n3\r\ndef\r\n0\r\n\r\n";
    let (port, _server) = plain_server(reply, false).await;
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("file.bin");
    let err = request_to_file(&local(port), "/f", &target, file_options(5, None))
        .await
        .unwrap_err();
    assert!(matches!(err, FileRequestError::Http(HttpError::TooLarge)), "{err:?}");
    assert!(!target.exists());
}

#[tokio::test]
async fn an_idle_body_times_out_and_removes_the_file() {
    let reply = b"HTTP/1.1 200 OK\r\ncontent-length: 10\r\n\r\nabc";
    let (port, server) = plain_server(reply, true).await;
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("file.bin");
    let options = FileRequestOptions {
        idle_timeout: Duration::from_millis(300),
        ..file_options(100, None)
    };
    let err = request_to_file(&local(port), "/f", &target, options).await.unwrap_err();
    assert!(matches!(err, FileRequestError::Http(HttpError::Timeout)), "{err:?}");
    assert!(!target.exists());
    server.abort();
}

#[tokio::test]
async fn a_cancel_mid_body_removes_the_file() {
    let reply = b"HTTP/1.1 200 OK\r\ncontent-length: 10\r\n\r\nabc";
    let (port, server) = plain_server(reply, true).await;
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("file.bin");
    let token = CancellationToken::new();
    let options = FileRequestOptions {
        cancel: Some(token.clone()),
        ..file_options(100, None)
    };
    let canceller = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        token.cancel();
    });
    let err = request_to_file(&local(port), "/f", &target, options).await.unwrap_err();
    assert!(matches!(err, FileRequestError::Http(HttpError::Aborted)), "{err:?}");
    assert!(!target.exists());
    canceller.await.unwrap();
    server.abort();
}

#[tokio::test]
async fn a_disk_failure_is_its_own_kind() {
    let reply = b"HTTP/1.1 200 OK\r\ncontent-length: 3\r\n\r\nabc";
    let (port, _server) = plain_server(reply, false).await;
    let dir = tempfile::tempdir().unwrap();
    let blocker = dir.path().join("blocker");
    std::fs::write(&blocker, b"a file, not a folder").unwrap();
    let err = request_to_file(&local(port), "/f", &blocker.join("file.bin"), file_options(100, None))
        .await
        .unwrap_err();
    assert!(matches!(err, FileRequestError::Disk(_)), "{err:?}");
}

/// Answers each connection with `replies` in turn, closing at once on an empty one; the requests read, in order.
async fn serial_server(replies: Vec<&'static [u8]>) -> (u16, JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let task = tokio::spawn(async move {
        let mut seen = Vec::new();
        for reply in replies {
            let accepted = tokio::time::timeout(Duration::from_secs(2), listener.accept()).await;
            let Ok(Ok((mut tcp, _))) = accepted else { break };
            seen.push(read_request(&mut tcp).await);
            let _ = tcp.write_all(reply).await;
            let _ = tcp.shutdown().await;
        }
        seen
    });
    (port, task)
}

#[tokio::test]
async fn a_get_that_breaks_is_sent_once_more_to_the_same_server() {
    let (port, server) = serial_server(vec![b"", OK_JSON]).await;
    let response = request(&local(port), "/x", RequestOptions::get(SECONDS, 1024))
        .await
        .unwrap();
    assert_eq!(response.status, 200);
    assert_eq!(server.await.unwrap().len(), 2);
}

#[tokio::test]
async fn a_write_is_never_sent_twice() {
    let (port, server) = serial_server(vec![b"", OK_JSON]).await;
    let options = RequestOptions {
        method: Method::POST,
        body: Some("{}".to_owned()),
        ..RequestOptions::get(SECONDS, 1024)
    };
    let err = request(&local(port), "/x", options).await.unwrap_err();
    assert_eq!(err.kind(), FailureKind::Other);
    assert_eq!(server.await.unwrap().len(), 1);
}

#[tokio::test]
async fn a_closed_port_is_refused() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let err = request(&local(port), "/x", RequestOptions::get(SECONDS, 1024))
        .await
        .unwrap_err();
    assert_eq!(err.kind(), FailureKind::Refused, "{err}");
}

#[tokio::test]
async fn a_name_that_does_not_resolve_is_dns() {
    // `.invalid` never resolves (RFC 6761)
    let endpoint = Endpoint {
        host: "dainty-lass-test.invalid".to_owned(),
        port: 443,
        pin: Some("0".repeat(64)),
    };
    let err = request(&endpoint, "/x", RequestOptions::get(SECONDS, 1024))
        .await
        .unwrap_err();
    assert_eq!(err.kind(), FailureKind::Dns, "{err}");
}

#[tokio::test]
async fn a_server_that_does_not_speak_tls_is_a_tls_failure_not_a_pin_mismatch() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        for _ in 0..2 {
            let (mut tcp, _) = listener.accept().await.unwrap();
            let _ = tcp
                .write_all(b"HTTP/1.1 400 Bad Request\r\ncontent-length: 0\r\n\r\n")
                .await;
            let _ = tcp.shutdown().await;
        }
    });
    let endpoint = Endpoint {
        pin: Some("0".repeat(64)),
        ..local(port)
    };
    let err = request(&endpoint, "/x", RequestOptions::get(SECONDS, 1024))
        .await
        .unwrap_err();
    assert_eq!(err.kind(), FailureKind::Tls, "{err}");
    server.await.unwrap();
}

#[test]
fn error_statuses_have_a_kind() {
    assert_eq!(FailureKind::of_status(200), None);
    assert_eq!(FailureKind::of_status(302), None);
    assert_eq!(FailureKind::of_status(404), Some(FailureKind::Status));
    assert_eq!(FailureKind::of_status(503), Some(FailureKind::Status));
}
