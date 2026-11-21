// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! The OAuth provider's redirect back to the launcher (RFC 8252 section 7.3): one listener on
//! `127.0.0.1:61190` for the length of a sign-in, which takes the browser's `GET /oauth/callback?code=..&state=..`
//! for the `state` it was opened with and answers a page the player can close. Anything else is answered and
//! ignored; nothing it reads is trusted past the `state` check (`docs/security/oauth-redirect.md`).

use std::io;
use std::net::{Ipv4Addr, SocketAddr};
use std::time::Duration;

use dainty_protocol::account::{OAUTH_REDIRECT_PATH, OAUTH_REDIRECT_PORT};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::CancellationToken;

use crate::core::i18n::{self, Language};

/// The most a request head may be.
pub const MAX_HEAD: usize = 8 * 1024;
const READ_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_CODE: usize = 2048;

/// What one request to the listener was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Callback {
    /// The provider's code for our `state`.
    Code(String),
    /// The player declined, or the provider refused, for our `state`.
    Denied,
    /// The callback path with another `state`, or none: not ours.
    WrongState,
    /// Any other request (a favicon, a probe), or one that does not parse.
    Unrelated,
}

/// Why waiting ended without a code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RedirectError {
    /// Another program holds the port.
    PortBusy,
    /// The player declined at the provider.
    Denied,
    /// No redirect came in time.
    TimedOut,
    /// The player cancelled in the launcher.
    Cancelled,
}

fn query_pairs(query: &str) -> Vec<(String, String)> {
    url::form_urlencoded::parse(query.as_bytes()).into_owned().collect()
}

fn single<'a>(pairs: &'a [(String, String)], name: &str) -> Option<&'a str> {
    let mut found = pairs.iter().filter(|(k, _)| k == name);
    let first = found.next()?;
    found.next().is_none().then_some(first.1.as_str())
}

/// Reads one request head: `GET <OAUTH_REDIRECT_PATH>?…` with `state` equal to `expected`, each parameter once.
#[must_use]
pub fn parse_request(head: &[u8], expected: &str) -> Callback {
    let Some(line) = head.split(|b| *b == b'\n').next() else {
        return Callback::Unrelated;
    };
    let Ok(line) = std::str::from_utf8(line) else {
        return Callback::Unrelated;
    };
    let mut parts = line.trim_end_matches('\r').split(' ');
    let (Some("GET"), Some(target), Some(version), None) = (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Callback::Unrelated;
    };
    if !version.starts_with("HTTP/1.") {
        return Callback::Unrelated;
    }
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    if path != OAUTH_REDIRECT_PATH {
        return Callback::Unrelated;
    }
    let pairs = query_pairs(query);
    // The state is compared before anything else is read, so another tab's redirect can't end this sign-in
    let ours = single(&pairs, "state")
        .is_some_and(|state| state.len() == expected.len() && subtle_eq(state.as_bytes(), expected.as_bytes()));
    if !ours || expected.is_empty() {
        return Callback::WrongState;
    }
    if pairs.iter().any(|(k, _)| k == "error") {
        return Callback::Denied;
    }
    match single(&pairs, "code") {
        Some(code) if !code.is_empty() && code.len() <= MAX_CODE && code.bytes().all(|b| b.is_ascii_graphic()) => {
            Callback::Code(code.to_owned())
        }
        _ => Callback::Denied,
    }
}

fn subtle_eq(a: &[u8], b: &[u8]) -> bool {
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// The page the browser shows: `key` from the backend's catalog, nothing from the request.
#[must_use]
pub fn page(language: Language, status: u16, key: &str) -> Vec<u8> {
    let title = escape(&i18n::translate(language, i18n::APP_TITLE, &[]));
    let text = escape(&i18n::translate(language, key, &[]));
    let body = format!(
        "<!doctype html><html lang=\"{}\"><meta charset=\"utf-8\"><title>{title}</title>\
         <body style=\"font-family:system-ui,sans-serif;max-width:32rem;margin:4rem auto;padding:0 1rem\">\
         <h1 style=\"font-size:1.25rem\">{title}</h1><p>{text}</p></body></html>",
        language.code()
    );
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        _ => "Not Found",
    };
    format!(
        "HTTP/1.1 {status} {reason}\r\ncontent-type: text/html; charset=utf-8\r\ncontent-length: {}\r\n\
         cache-control: no-store\r\ncontent-security-policy: default-src 'none'; style-src 'unsafe-inline'\r\n\
         referrer-policy: no-referrer\r\nx-content-type-options: nosniff\r\nconnection: close\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

/// Takes the redirect port; [`RedirectError::PortBusy`] when another program has it.
pub async fn bind() -> Result<TcpListener, RedirectError> {
    bind_on(OAUTH_REDIRECT_PORT).await
}

/// [`bind`] on another port (tests).
pub async fn bind_on(port: u16) -> Result<TcpListener, RedirectError> {
    TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, port)))
        .await
        .map_err(|_| RedirectError::PortBusy)
}

async fn read_head(stream: &mut TcpStream) -> io::Result<Vec<u8>> {
    let mut head = Vec::with_capacity(1024);
    let mut buf = [0u8; 1024];
    while !head.windows(4).any(|w| w == b"\r\n\r\n") && head.len() < MAX_HEAD {
        let n = stream.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        head.extend_from_slice(&buf[..n]);
    }
    head.truncate(MAX_HEAD);
    Ok(head)
}

async fn answer(stream: &mut TcpStream, bytes: &[u8]) {
    let _ = tokio::time::timeout(READ_TIMEOUT, async {
        let _ = stream.write_all(bytes).await;
        let _ = stream.shutdown().await;
    })
    .await;
}

/// Waits for the redirect carrying `state`, answering every request on the way; the code, or why there is none.
pub async fn wait(
    listener: TcpListener,
    state: &str,
    language: Language,
    cancel: &CancellationToken,
    timeout: Duration,
) -> Result<String, RedirectError> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let accepted = tokio::select! {
            () = cancel.cancelled() => return Err(RedirectError::Cancelled),
            () = tokio::time::sleep_until(deadline) => return Err(RedirectError::TimedOut),
            accepted = listener.accept() => accepted,
        };
        let Ok((mut stream, peer)) = accepted else {
            continue;
        };
        if !peer.ip().is_loopback() {
            continue;
        }
        let head = match tokio::time::timeout(READ_TIMEOUT, read_head(&mut stream)).await {
            Ok(Ok(head)) => head,
            _ => continue,
        };
        match parse_request(&head, state) {
            Callback::Code(code) => {
                answer(&mut stream, &page(language, 200, i18n::OAUTH_PAGE_DONE)).await;
                return Ok(code);
            }
            Callback::Denied => {
                answer(&mut stream, &page(language, 200, i18n::OAUTH_PAGE_FAILED)).await;
                return Err(RedirectError::Denied);
            }
            Callback::WrongState => answer(&mut stream, &page(language, 400, i18n::OAUTH_PAGE_FAILED)).await,
            Callback::Unrelated => answer(&mut stream, &page(language, 404, i18n::OAUTH_PAGE_FAILED)).await,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "a panic is how a test fails")]

    use super::*;

    const STATE: &str = "0123456789abcdef";

    fn get(target: &str) -> Vec<u8> {
        format!("GET {target} HTTP/1.1\r\nHost: 127.0.0.1:61190\r\n\r\n").into_bytes()
    }

    #[test]
    fn requests() {
        let ok = get(&format!("/oauth/callback?code=abc%2Bd&state={STATE}"));
        assert_eq!(parse_request(&ok, STATE), Callback::Code("abc+d".into()));
        let denied = get(&format!("/oauth/callback?error=access_denied&state={STATE}"));
        assert_eq!(parse_request(&denied, STATE), Callback::Denied);
        let no_code = get(&format!("/oauth/callback?state={STATE}"));
        assert_eq!(parse_request(&no_code, STATE), Callback::Denied);
        for wrong in [
            "/oauth/callback?code=abc&state=other",
            "/oauth/callback?code=abc",
            "/oauth/callback?code=abc&state=0123456789abcdef&state=0123456789abcdef",
        ] {
            assert_eq!(parse_request(&get(wrong), STATE), Callback::WrongState, "{wrong}");
        }
        assert_eq!(
            parse_request(&get(&format!("/oauth/callback?code=x&state={STATE}")), ""),
            Callback::WrongState
        );
        for unrelated in [
            get("/favicon.ico"),
            get(&format!("/oauth/callback/?code=abc&state={STATE}")),
            format!("POST /oauth/callback?code=abc&state={STATE} HTTP/1.1\r\n\r\n").into_bytes(),
            b"\xff\xfe".to_vec(),
            Vec::new(),
        ] {
            assert_eq!(parse_request(&unrelated, STATE), Callback::Unrelated);
        }
        let twice = get(&format!("/oauth/callback?code=a&code=b&state={STATE}"));
        assert_eq!(parse_request(&twice, STATE), Callback::Denied);
    }

    #[test]
    fn the_page_holds_nothing_from_the_request() {
        let page = String::from_utf8(page(Language::PtBr, 200, i18n::OAUTH_PAGE_DONE)).unwrap();
        assert!(page.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(page.contains("lang=\"pt-BR\"") && page.contains("default-src 'none'"));
        assert!(!page.contains("<script"));
    }

    #[tokio::test]
    async fn waits_past_other_requests_for_its_own() {
        let listener = bind_on(0).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let cancel = CancellationToken::new();
        let waiting = tokio::spawn({
            let cancel = cancel.clone();
            async move { wait(listener, STATE, Language::En, &cancel, Duration::from_secs(10)).await }
        });
        let send = |target: String| async move {
            let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            stream.write_all(&get(&target)).await.unwrap();
            let mut answer = String::new();
            stream.read_to_string(&mut answer).await.unwrap();
            answer
        };
        assert!(send("/favicon.ico".into()).await.starts_with("HTTP/1.1 404"));
        assert!(
            send("/oauth/callback?code=x&state=nope".into())
                .await
                .starts_with("HTTP/1.1 400")
        );
        assert!(
            send(format!("/oauth/callback?code=the-code&state={STATE}"))
                .await
                .starts_with("HTTP/1.1 200")
        );
        assert_eq!(waiting.await.unwrap(), Ok("the-code".into()));

        let listener = bind_on(0).await.unwrap();
        cancel.cancel();
        assert_eq!(
            wait(listener, STATE, Language::En, &cancel, Duration::from_secs(10)).await,
            Err(RedirectError::Cancelled)
        );
        let listener = bind_on(0).await.unwrap();
        let fresh = CancellationToken::new();
        assert_eq!(
            wait(listener, STATE, Language::En, &fresh, Duration::from_millis(50)).await,
            Err(RedirectError::TimedOut)
        );
    }
}
