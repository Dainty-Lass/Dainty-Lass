// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The code mail (spec §7.2, §7.4): SMTP settings, the message, and a small SMTP client that never sends the login
//! in clear. Failures are codes only; the server's reply text is never kept.

use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use dainty_module_api::BoxFuture;
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};
use tokio::net::TcpStream;

use crate::AuthSettings;
use crate::store::Purpose;
use crate::util;

/// The From header when `RUSTIC_ISLES_MAIL_FROM` is unset.
pub const DEFAULT_FROM: &str = "Rustic Isles <no-reply@mysticfox.dev>";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const IO_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_REPLY_LINE: usize = 4096;
const MAX_REPLY_LINES: usize = 128;

/// How the SMTP connection is secured (`RUSTIC_ISLES_SMTP_SECURE`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmtpSecurity {
    /// Required STARTTLS on a plain connection
    StartTls,
    /// TLS from the first byte
    Tls,
}

/// The host's SMTP settings; Debug leaves out the password.
#[derive(Clone, PartialEq, Eq)]
pub struct SmtpConfig {
    /// The SMTP server's host name.
    pub host: String,
    /// The SMTP port (default 587).
    pub port: u16,
    /// STARTTLS or TLS from the start.
    pub security: SmtpSecurity,
    /// The SMTP login.
    pub user: String,
    /// The SMTP password; never logged.
    pub password: String,
    /// The From header.
    pub from: String,
}

// The password stays out of logs (§13)
impl std::fmt::Debug for SmtpConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SmtpConfig")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("security", &self.security)
            .field("user", &self.user)
            .field("from", &self.from)
            .finish_non_exhaustive()
    }
}

impl SmtpConfig {
    /// The settings, or the keys that are missing or invalid (names only, never values).
    pub fn from_settings(settings: &dyn AuthSettings) -> Result<Self, Vec<&'static str>> {
        let trimmed = |name: &str| dainty_service_kit::js::trim(settings.raw(name).unwrap_or("")).to_owned();
        let mut problems = Vec::new();
        let host = trimmed("RUSTIC_ISLES_SMTP_HOST");
        if host.is_empty() {
            problems.push("RUSTIC_ISLES_SMTP_HOST");
        }
        let port_text = trimmed("RUSTIC_ISLES_SMTP_PORT");
        let port = if port_text.is_empty() {
            Some(587)
        } else if port_text.bytes().all(|b| b.is_ascii_digit()) {
            port_text.parse::<u16>().ok().filter(|&p| p >= 1)
        } else {
            None
        };
        if port.is_none() {
            problems.push("RUSTIC_ISLES_SMTP_PORT (a port number)");
        }
        let security = match trimmed("RUSTIC_ISLES_SMTP_SECURE").to_ascii_lowercase().as_str() {
            "" | "starttls" => Some(SmtpSecurity::StartTls),
            "tls" => Some(SmtpSecurity::Tls),
            _ => None,
        };
        if security.is_none() {
            problems.push("RUSTIC_ISLES_SMTP_SECURE (starttls or tls)");
        }
        let user = trimmed("RUSTIC_ISLES_SMTP_USER");
        if user.is_empty() {
            problems.push("RUSTIC_ISLES_SMTP_USER");
        }
        let password = settings.raw("RUSTIC_ISLES_SMTP_PASSWORD").unwrap_or("").to_owned();
        if dainty_service_kit::js::trim(&password).is_empty() {
            problems.push("RUSTIC_ISLES_SMTP_PASSWORD");
        }
        let from = trimmed("RUSTIC_ISLES_MAIL_FROM");
        let from = if from.is_empty() { DEFAULT_FROM.to_owned() } else { from };
        match (port, security) {
            (Some(port), Some(security)) if problems.is_empty() => Ok(Self {
                host,
                port,
                security,
                user,
                password,
                from,
            }),
            _ => Err(problems),
        }
    }

    /// The address inside the From header's angle brackets, or the trimmed text.
    pub fn envelope_from(&self) -> String {
        match (self.from.rfind('<'), self.from.rfind('>')) {
            (Some(open), Some(close)) if open < close => self.from[open + 1..close].trim().to_owned(),
            _ => self.from.trim().to_owned(),
        }
    }
}

/// One rendered email.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MailMessage {
    /// The recipient's address.
    pub to: String,
    /// The subject line.
    pub subject: String,
    /// The plain-text body.
    pub text: String,
    /// The HTML body.
    pub html: String,
}

/// A failure, by code only (§7.4).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{code}")]
pub struct MailError {
    /// A nodemailer-style code such as `ECONNECTION` or `ETIMEDOUT`.
    pub code: String,
}

impl MailError {
    /// A failure with `code`.
    pub fn new(code: impl Into<String>) -> Self {
        Self { code: code.into() }
    }
}

/// How mail is sent, a trait so tests send none.
pub trait MailTransport: Send + Sync {
    /// Sends `message` through the SMTP server in `settings`.
    fn send<'a>(&'a self, settings: &'a SmtpConfig, message: MailMessage) -> BoxFuture<'a, Result<(), MailError>>;
}

fn subject_of(purpose: Purpose) -> &'static str {
    match purpose {
        Purpose::Verify => "Your Rustic Isles code",
        Purpose::Reset => "Your Rustic Isles password reset code",
        Purpose::Unlink => "Your Rustic Isles code to unlink Discord",
    }
}

fn escape_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
    out
}

/// The code mail: refuses a code that isn't six digits or an expiry under a minute (`ERENDER`).
pub fn render_code_mail(
    to: &str,
    purpose: Purpose,
    code: &str,
    expires_minutes: u32,
) -> Result<MailMessage, MailError> {
    if !crate::email::is_code_shape(code) || expires_minutes < 1 || to.contains(['\r', '\n']) {
        return Err(MailError::new("ERENDER"));
    }
    let unit = if expires_minutes == 1 { "minute" } else { "minutes" };
    let expiry = format!("This code expires in {expires_minutes} {unit} and works only once.");
    let (heading, intro, outro) = match purpose {
        Purpose::Verify => (
            "Confirm your email",
            "Enter this code in the launcher to confirm your email address.",
            "If you didn't create a Rustic Isles account, you can ignore this email.",
        ),
        Purpose::Reset => (
            "Reset your password",
            "Enter this code in the launcher to choose a new password.",
            "If you didn't ask to reset your password, you can ignore this email; your password stays the same.",
        ),
        Purpose::Unlink => (
            "Unlink Discord",
            "Enter this code in the launcher to unlink Discord from your account.",
            "If you didn't ask for this, ignore this email: Discord stays linked.",
        ),
    };
    let text = format!("{heading}\n\n{intro}\n\n{code}\n\n{expiry}\n\n{outro}\n");
    let html = format!(
        "<!DOCTYPE html>\n<html lang=\"en\">\n<head><meta charset=\"utf-8\"><title>{title}</title></head>\n\
         <body style=\"font-family:sans-serif;color:#222;background:#f6f4ef;padding:24px\">\n\
         <div style=\"max-width:480px;margin:auto;background:#fff;padding:24px;border-radius:8px\">\n\
         <h1 style=\"font-size:20px\">{heading}</h1>\n<p>{intro}</p>\n\
         <p style=\"font-size:32px;letter-spacing:6px;font-weight:bold\">{code}</p>\n<p>{expiry}</p>\n\
         <p style=\"color:#666\">{outro}</p>\n</div>\n</body>\n</html>\n",
        title = escape_html(subject_of(purpose)),
        heading = escape_html(heading),
        intro = escape_html(intro),
        code = escape_html(code),
        expiry = escape_html(&expiry),
        outro = escape_html(outro),
    );
    Ok(MailMessage {
        to: to.to_owned(),
        subject: format!("{}: {code}", subject_of(purpose)),
        text,
        html,
    })
}

fn encode_header(text: &str) -> String {
    if text.is_ascii() {
        text.to_owned()
    } else {
        format!("=?UTF-8?B?{}?=", STANDARD.encode(text))
    }
}

fn wrapped_base64(text: &str) -> String {
    let encoded = STANDARD.encode(text);
    let lines: Vec<&str> = encoded
        .as_bytes()
        .chunks(76)
        .map(|chunk| std::str::from_utf8(chunk).unwrap_or(""))
        .collect();
    lines.join("\r\n")
}

/// The MIME message, CRLF line ends, before dot-stuffing.
pub fn build_message(from: &str, message: &MailMessage) -> Result<String, MailError> {
    if [from, &message.to, &message.subject]
        .iter()
        .any(|h| h.contains(['\r', '\n']))
    {
        return Err(MailError::new("ERENDER"));
    }
    let boundary = format!("=_mp_{}", util::random_hex::<12>());
    Ok(format!(
        "From: {from}\r\nTo: {to}\r\nSubject: {subject}\r\nMIME-Version: 1.0\r\n\
         Content-Type: multipart/alternative; boundary=\"{boundary}\"\r\n\r\n\
         --{boundary}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: base64\r\n\r\n{text}\r\n\
         --{boundary}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Transfer-Encoding: base64\r\n\r\n{html}\r\n\
         --{boundary}--\r\n",
        to = message.to,
        subject = encode_header(&message.subject),
        text = wrapped_base64(&message.text),
        html = wrapped_base64(&message.html),
    ))
}

/// A leading `.` on any line is doubled.
pub fn dot_stuff(message: &str) -> String {
    let mut out = String::with_capacity(message.len() + 8);
    for (i, line) in message.split("\r\n").enumerate() {
        if i > 0 {
            out.push_str("\r\n");
        }
        if line.starts_with('.') {
            out.push('.');
        }
        out.push_str(line);
    }
    out
}

async fn timed<T>(future: impl Future<Output = std::io::Result<T>>) -> Result<T, MailError> {
    match tokio::time::timeout(IO_TIMEOUT, future).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(_)) => Err(MailError::new("EIO")),
        Err(_) => Err(MailError::new("ETIMEDOUT")),
    }
}

async fn write_line<S: AsyncWrite + Unpin>(stream: &mut S, line: &str) -> Result<(), MailError> {
    timed(stream.write_all(format!("{line}\r\n").as_bytes())).await?;
    timed(stream.flush()).await
}

/// One reply: `NNN-` lines continue, `NNN ` ends it. The code and the lines' text after the code.
async fn read_reply<S: AsyncRead + Unpin>(stream: &mut S) -> Result<(u16, Vec<String>), MailError> {
    let mut lines = Vec::new();
    loop {
        let mut line = Vec::new();
        loop {
            let mut byte = [0u8; 1];
            let n = timed(stream.read(&mut byte)).await?;
            if n == 0 {
                return Err(MailError::new("ECONNECTION"));
            }
            if byte[0] == b'\n' {
                break;
            }
            if line.len() >= MAX_REPLY_LINE {
                return Err(MailError::new("EPROTOCOL"));
            }
            line.push(byte[0]);
        }
        if line.last() == Some(&b'\r') {
            line.pop();
        }
        let text = String::from_utf8_lossy(&line).into_owned();
        if text.len() < 4 {
            return Err(MailError::new("ECONNECTION"));
        }
        let code = text[..3].parse::<u16>().map_err(|_| MailError::new("EPROTOCOL"))?;
        let more = text.as_bytes()[3] == b'-';
        lines.push(text[4..].to_owned());
        if !more {
            return Ok((code, lines));
        }
        if lines.len() >= MAX_REPLY_LINES {
            return Err(MailError::new("EPROTOCOL"));
        }
    }
}

async fn expect<S: AsyncRead + Unpin>(stream: &mut S, wanted: u16) -> Result<Vec<String>, MailError> {
    let (code, lines) = read_reply(stream).await?;
    if code != wanted {
        return Err(MailError::new(format!("EPROTOCOL{code}")));
    }
    Ok(lines)
}

/// Greeting and `EHLO`: the extension lines.
async fn hello<S: AsyncRead + AsyncWrite + Unpin>(stream: &mut S, greet: bool) -> Result<Vec<String>, MailError> {
    if greet {
        expect(stream, 220).await?;
    }
    write_line(stream, "EHLO localhost").await?;
    expect(stream, 250).await
}

/// After the (encrypted) `EHLO`: the login, the envelope, the message and `QUIT`.
async fn deliver<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    config: &SmtpConfig,
    message: &MailMessage,
) -> Result<(), MailError> {
    let body = dot_stuff(&build_message(&config.from, message)?);
    let login = STANDARD.encode(format!("\0{}\0{}", config.user, config.password));
    write_line(stream, &format!("AUTH PLAIN {login}")).await?;
    let (code, _) = read_reply(stream).await?;
    if code != 235 {
        return Err(MailError::new("EAUTH"));
    }
    write_line(stream, &format!("MAIL FROM:<{}>", config.envelope_from())).await?;
    expect(stream, 250).await?;
    write_line(stream, &format!("RCPT TO:<{}>", message.to)).await?;
    expect(stream, 250).await?;
    write_line(stream, "DATA").await?;
    expect(stream, 354).await?;
    let body = body.strip_suffix("\r\n").unwrap_or(&body);
    timed(stream.write_all(body.as_bytes())).await?;
    write_line(stream, "\r\n.").await?;
    expect(stream, 250).await?;
    // The reply to QUIT doesn't matter: the message is accepted
    let _ = write_line(stream, "QUIT").await;
    let _ = tokio::time::timeout(Duration::from_secs(2), read_reply(stream)).await;
    Ok(())
}

/// The plain part of a STARTTLS dialogue, up to the upgrade.
async fn start_tls<S: AsyncRead + AsyncWrite + Unpin>(stream: &mut S) -> Result<(), MailError> {
    let extensions = hello(stream, true).await?;
    if !extensions
        .iter()
        .any(|line| line.trim().eq_ignore_ascii_case("STARTTLS"))
    {
        // The login never goes in clear
        return Err(MailError::new("ETLS"));
    }
    write_line(stream, "STARTTLS").await?;
    expect(stream, 220).await?;
    Ok(())
}

/// SMTP over TCP and rustls with webpki roots.
#[derive(Debug, Clone, Default)]
pub struct SmtpTransport;

impl SmtpTransport {
    async fn run(config: &SmtpConfig, message: &MailMessage) -> Result<(), MailError> {
        let connect = TcpStream::connect((config.host.as_str(), config.port));
        let mut tcp = match tokio::time::timeout(CONNECT_TIMEOUT, connect).await {
            Ok(Ok(stream)) => stream,
            Ok(Err(_)) => return Err(MailError::new("ECONNECTION")),
            Err(_) => return Err(MailError::new("ETIMEDOUT")),
        };
        let name =
            rustls::pki_types::ServerName::try_from(config.host.clone()).map_err(|_| MailError::new("ESERVERNAME"))?;
        let tls = dainty_http::tls_config().map_err(|_| MailError::new("ETLS"))?;
        let connector = tokio_rustls::TlsConnector::from(Arc::new(tls));
        if config.security == SmtpSecurity::StartTls {
            start_tls(&mut tcp).await?;
        }
        let mut stream = match tokio::time::timeout(IO_TIMEOUT, connector.connect(name, tcp)).await {
            Ok(Ok(stream)) => stream,
            Ok(Err(_)) => return Err(MailError::new("ETLS")),
            Err(_) => return Err(MailError::new("ETIMEDOUT")),
        };
        hello(&mut stream, config.security == SmtpSecurity::Tls).await?;
        deliver(&mut stream, config, message).await
    }
}

impl MailTransport for SmtpTransport {
    fn send<'a>(&'a self, settings: &'a SmtpConfig, message: MailMessage) -> BoxFuture<'a, Result<(), MailError>> {
        Box::pin(async move { Self::run(settings, &message).await })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::MapSettings;
    use tokio::io::AsyncBufReadExt as _;

    fn complete() -> Vec<(&'static str, &'static str)> {
        vec![
            ("RUSTIC_ISLES_SMTP_HOST", "smtp.example.test"),
            ("RUSTIC_ISLES_SMTP_USER", "mailer"),
            ("RUSTIC_ISLES_SMTP_PASSWORD", " p4ss "),
        ]
    }

    fn read(pairs: &[(&str, &str)]) -> Result<SmtpConfig, Vec<&'static str>> {
        SmtpConfig::from_settings(&MapSettings::new(pairs))
    }

    #[test]
    fn settings() {
        let config = read(&complete()).unwrap();
        assert_eq!((config.port, config.security), (587, SmtpSecurity::StartTls));
        assert_eq!(config.from, DEFAULT_FROM);
        assert_eq!(config.password, " p4ss ");
        assert!(!format!("{config:?}").contains("p4ss"));
        let mut tls = complete();
        tls.extend([
            ("RUSTIC_ISLES_SMTP_PORT", "465"),
            ("RUSTIC_ISLES_SMTP_SECURE", "TLS"),
            ("RUSTIC_ISLES_MAIL_FROM", "Fox <fox@x.test>"),
        ]);
        let config = read(&tls).unwrap();
        assert_eq!(
            (config.port, config.security, config.envelope_from().as_str()),
            (465, SmtpSecurity::Tls, "fox@x.test")
        );
        let mut spaced = complete();
        spaced.push(("RUSTIC_ISLES_SMTP_SECURE", " starttls "));
        assert!(read(&spaced).is_ok());
        for key in [
            "RUSTIC_ISLES_SMTP_HOST",
            "RUSTIC_ISLES_SMTP_USER",
            "RUSTIC_ISLES_SMTP_PASSWORD",
        ] {
            let missing: Vec<_> = complete().into_iter().filter(|(k, _)| *k != key).collect();
            assert_eq!(read(&missing).unwrap_err(), [key]);
            let mut blank = missing.clone();
            blank.push((key, "  "));
            assert_eq!(read(&blank).unwrap_err(), [key]);
        }
        for port in ["0", "65536", "587.5", "x"] {
            let mut bad = complete();
            bad.push(("RUSTIC_ISLES_SMTP_PORT", port));
            assert_eq!(read(&bad).unwrap_err(), ["RUSTIC_ISLES_SMTP_PORT (a port number)"]);
        }
        let mut bad = complete();
        bad.push(("RUSTIC_ISLES_SMTP_SECURE", "ssl3"));
        assert_eq!(read(&bad).unwrap_err(), ["RUSTIC_ISLES_SMTP_SECURE (starttls or tls)"]);
    }

    #[test]
    fn rendering() {
        let verify = render_code_mail("fox@example.test", Purpose::Verify, "482913", 5).unwrap();
        assert_eq!(verify.subject, "Your Rustic Isles code: 482913");
        let reset = render_code_mail("fox@example.test", Purpose::Reset, "482913", 1).unwrap();
        assert_eq!(reset.subject, "Your Rustic Isles password reset code: 482913");
        assert!(reset.html.contains("Reset your password"));
        assert!(
            reset
                .text
                .contains("This code expires in 1 minute and works only once.")
        );
        let unlink = render_code_mail("fox@example.test", Purpose::Unlink, "482913", 5).unwrap();
        assert_eq!(unlink.subject, "Your Rustic Isles code to unlink Discord: 482913");
        assert!(unlink.html.contains("Unlink Discord") && unlink.text.contains("Discord stays linked"));
        for mail in [&verify, &reset, &unlink] {
            assert!(mail.html.contains(">482913<"));
            assert!(mail.text.contains("This code expires in") && mail.html.contains("works only once."));
            for tag in ["<img", "<script", "<link"] {
                assert!(!mail.html.contains(tag));
            }
        }
        assert_eq!(
            render_code_mail("a@b.co", Purpose::Verify, "12345", 5)
                .unwrap_err()
                .code,
            "ERENDER"
        );
        assert_eq!(
            render_code_mail("a@b.co", Purpose::Verify, "123456", 0)
                .unwrap_err()
                .code,
            "ERENDER"
        );
    }

    #[test]
    fn messages_stuff_dots() {
        assert_eq!(dot_stuff("a\r\n.b\r\n..c"), "a\r\n..b\r\n...c");
        let message = MailMessage {
            to: "fox@example.test".into(),
            subject: "Caf\u{e9}".into(),
            text: "t".into(),
            html: "h".into(),
        };
        let built = build_message(DEFAULT_FROM, &message).unwrap();
        assert!(built.contains("Subject: =?UTF-8?B?Q2Fmw6k=?=\r\n"));
        assert!(built.contains("boundary=\"=_mp_"));
    }

    fn config() -> SmtpConfig {
        let mut pairs = complete();
        pairs.push(("RUSTIC_ISLES_MAIL_FROM", "Fox <fox-sender@example.test>"));
        read(&pairs).unwrap()
    }

    /// A scripted server: answers each command with the next reply and records what it got.
    async fn scripted(replies: Vec<&'static str>) -> (tokio::io::DuplexStream, tokio::task::JoinHandle<Vec<String>>) {
        let (client, server) = tokio::io::duplex(64 * 1024);
        let task = tokio::spawn(async move {
            let (read, mut write) = tokio::io::split(server);
            let mut reader = tokio::io::BufReader::new(read);
            let mut seen = Vec::new();
            let mut replies = replies.into_iter();
            if let Some(greeting) = replies.next() {
                write.write_all(greeting.as_bytes()).await.unwrap();
            }
            let mut in_data = false;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).await.unwrap_or(0) == 0 {
                    break;
                }
                let line = line.trim_end().to_owned();
                if in_data && line != "." {
                    continue;
                }
                in_data = line == "DATA";
                seen.push(line);
                match replies.next() {
                    Some(reply) => write.write_all(reply.as_bytes()).await.unwrap(),
                    None => break,
                }
            }
            seen
        });
        (client, task)
    }

    #[tokio::test]
    async fn a_dialogue_ends_with_quit() {
        let (mut client, server) = scripted(vec![
            "220 hi\r\n",
            "250-x\r\n250 AUTH PLAIN\r\n",
            "235 ok\r\n",
            "250 ok\r\n",
            "250 ok\r\n",
            "354 go\r\n",
            "250 queued\r\n",
            "221 bye\r\n",
        ])
        .await;
        let message = render_code_mail("fox@example.test", Purpose::Verify, "482913", 5).unwrap();
        hello(&mut client, true).await.unwrap();
        deliver(&mut client, &config(), &message).await.unwrap();
        drop(client);
        let seen = server.await.unwrap();
        assert_eq!(seen[0], "EHLO localhost");
        assert!(seen[1].starts_with("AUTH PLAIN "));
        assert_eq!(seen[2], "MAIL FROM:<fox-sender@example.test>");
        assert_eq!(seen[3], "RCPT TO:<fox@example.test>");
        assert_eq!(seen.last().map(String::as_str), Some("QUIT"));
    }

    #[tokio::test]
    async fn refusals_are_codes() {
        let (mut client, _server) = scripted(vec!["220 hi\r\n", "250 ok\r\n", "535 secret server text\r\n"]).await;
        let message = render_code_mail("fox@example.test", Purpose::Verify, "482913", 5).unwrap();
        hello(&mut client, true).await.unwrap();
        let error = deliver(&mut client, &config(), &message).await.unwrap_err();
        assert_eq!(error.code, "EAUTH");
        assert!(!error.to_string().contains("secret"));

        let (mut client, _server) = scripted(vec!["220 hi\r\n", "250-x\r\n250 SIZE 1\r\n"]).await;
        assert_eq!(start_tls(&mut client).await.unwrap_err().code, "ETLS");
        let (mut client, _server) = scripted(vec!["220 hi\r\n", "250-x\r\n250 starttls\r\n", "220 go\r\n"]).await;
        assert!(start_tls(&mut client).await.is_ok());
        let (mut client, _server) = scripted(vec!["22\r\n"]).await;
        assert_eq!(hello(&mut client, true).await.unwrap_err().code, "ECONNECTION");
    }
}
