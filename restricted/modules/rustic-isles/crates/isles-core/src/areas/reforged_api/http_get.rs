//! A plain `GET` over HTTP/1.1 to a service on this machine (the Director's `/api/instances`), the way Node's
//! `fetch` does it for `reforgedApi.ts`: any failure is `None`, and the caller bounds the time.

use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// Decodes a `Transfer-Encoding: chunked` body; `None` when it is malformed.
fn dechunk(mut data: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let line_end = data.windows(2).position(|w| w == b"\r\n")?;
        let size_text = std::str::from_utf8(&data[..line_end]).ok()?;
        let size = usize::from_str_radix(size_text.split(';').next()?.trim(), 16).ok()?;
        data = &data[line_end + 2..];
        if size == 0 {
            return Some(out);
        }
        out.extend_from_slice(data.get(..size)?);
        data = data.get(size + 2..)?;
    }
}

/// The JSON of a 2xx answer; `None` for any other answer, any error, or a body that isn't JSON.
pub async fn get_json(authority: &str, path: &str) -> Option<Value> {
    let mut stream = TcpStream::connect(authority).await.ok()?;
    let request = format!("GET {path} HTTP/1.1\r\nHost: {authority}\r\nAccept: */*\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).await.ok()?;
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).await.ok()?;
    parse_answer(&raw)
}

pub fn parse_answer(raw: &[u8]) -> Option<Value> {
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n")?;
    let head = std::str::from_utf8(&raw[..split]).ok()?;
    let status: u16 = head.lines().next()?.split(' ').nth(1)?.parse().ok()?;
    if !(200..300).contains(&status) {
        return None;
    }
    let chunked = head.lines().skip(1).any(|line| {
        line.to_ascii_lowercase().starts_with("transfer-encoding:") && line.to_ascii_lowercase().contains("chunked")
    });
    let body = &raw[split + 4..];
    let body = if chunked { dechunk(body)? } else { body.to_vec() };
    serde_json::from_slice(&body).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_are_read_plain_and_chunked() {
        let plain = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n[1,2]";
        assert_eq!(parse_answer(plain), Some(serde_json::json!([1, 2])));
        let chunked = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\n[1,\r\n2\r\n2]\r\n0\r\n\r\n";
        assert_eq!(parse_answer(chunked), Some(serde_json::json!([1, 2])));
        assert_eq!(parse_answer(b"HTTP/1.1 500 Oops\r\n\r\n[]"), None);
        assert_eq!(parse_answer(b"HTTP/1.1 200 OK\r\n\r\nnot json"), None);
        assert_eq!(parse_answer(b"garbage"), None);
    }
}
