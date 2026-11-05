//! The code mail (`mail/mailConfig.ts`, `mail/codeEmail.ts`, `mail/mailSender.ts`): the one-time codes of email
//! verification, password reset and the Discord unlink, sent over SMTP. Port 587 starts plain and must upgrade
//! (STARTTLS), so the login never goes out in clear; port 465 (`RUSTIC_ISLES_SMTP_SECURE=tls`) speaks TLS from the
//! first byte. Nothing the user typed goes into the mail, only the code and fixed text, and a failure is logged by
//! its code only: an SMTP reply can repeat the recipient's address.

use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use module_api::BoxFuture;
use rustls::pki_types::ServerName;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_rustls::TlsConnector;

use crate::repo::auth::EmailCodePurpose;
use crate::settings::Settings;

pub const MAIL_FROM_DEFAULT: &str = "Rustic Isles <no-reply@mysticfox.dev>";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const IO_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Clone, PartialEq, Eq)]
pub struct SmtpSettings {
    pub host: String,
    pub port: u16,
    /// TLS from the first byte (port 465); otherwise STARTTLS, which is required
    pub implicit_tls: bool,
    pub user: String,
    pub password: String,
    /// The From header, for example `Rustic Isles <no-reply@mysticfox.dev>`
    pub from: String,
}

impl std::fmt::Debug for SmtpSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SmtpSettings")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("implicit_tls", &self.implicit_tls)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SmtpReading {
    Configured(SmtpSettings),
    /// The keys that are missing or invalid, never their values
    Problems(Vec<String>),
}

impl SmtpReading {
    pub fn configured(&self) -> bool {
        matches!(self, Self::Configured(_))
    }
}

fn trimmed<'a>(settings: &'a Settings, name: &str) -> &'a str {
    service_kit::js::trim(settings.raw(name).unwrap_or_default())
}

/// `ReadSmtpSettings`. Blank counts as missing; the port defaults to 587 and the mode to STARTTLS; an unknown mode is
/// a problem, since the code mail must never fall back to a mode the owner did not choose.
pub fn read_smtp_settings(settings: &Settings) -> SmtpReading {
    let mut problems = Vec::new();
    let host = trimmed(settings, "RUSTIC_ISLES_SMTP_HOST");
    if host.is_empty() {
        problems.push("RUSTIC_ISLES_SMTP_HOST".to_owned());
    }
    let port_text = trimmed(settings, "RUSTIC_ISLES_SMTP_PORT");
    let port = if port_text.is_empty() {
        Some(587)
    } else {
        // Number(text): a whole number from 1 to 65535
        port_text.parse::<u32>().ok().filter(|port| (1..=65535).contains(port))
    };
    if port.is_none() {
        problems.push("RUSTIC_ISLES_SMTP_PORT (a port number)".to_owned());
    }
    let secure = trimmed(settings, "RUSTIC_ISLES_SMTP_SECURE").to_lowercase();
    if !["", "starttls", "tls"].contains(&secure.as_str()) {
        problems.push("RUSTIC_ISLES_SMTP_SECURE (starttls or tls)".to_owned());
    }
    let user = trimmed(settings, "RUSTIC_ISLES_SMTP_USER");
    if user.is_empty() {
        problems.push("RUSTIC_ISLES_SMTP_USER".to_owned());
    }
    let password = settings.raw("RUSTIC_ISLES_SMTP_PASSWORD").unwrap_or_default();
    if service_kit::js::trim(password).is_empty() {
        problems.push("RUSTIC_ISLES_SMTP_PASSWORD".to_owned());
    }
    let Some(port) = port.filter(|_| problems.is_empty()) else {
        return SmtpReading::Problems(problems);
    };
    let from = Some(trimmed(settings, "RUSTIC_ISLES_MAIL_FROM"))
        .filter(|from| !from.is_empty())
        .unwrap_or(MAIL_FROM_DEFAULT);
    SmtpReading::Configured(SmtpSettings {
        host: host.to_owned(),
        port: port as u16,
        implicit_tls: secure == "tls",
        user: user.to_owned(),
        password: password.to_owned(),
        from: from.to_owned(),
    })
}

// ------------------------------------------------------------------ the message

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeEmail {
    pub subject: String,
    pub text: String,
    pub html: String,
}

struct Copy {
    subject: &'static str,
    kicker: &'static str,
    title: &'static str,
    lead: &'static str,
    ignore: &'static str,
}

const fn copy(purpose: EmailCodePurpose) -> Copy {
    match purpose {
        EmailCodePurpose::Verify => Copy {
            subject: "Your Rustic Isles code",
            kicker: "Email verification",
            title: "Confirm your email",
            lead: "Enter this code in the Rustic Isles launcher to verify your email address.",
            ignore: "If you didn't create a Rustic Isles account, ignore this email. Nothing happens without this code.",
        },
        EmailCodePurpose::Reset => Copy {
            subject: "Your Rustic Isles password reset code",
            kicker: "Password reset",
            title: "Reset your password",
            lead: "Enter this code in the Rustic Isles launcher to choose a new password.",
            ignore: "If you didn't ask to reset your password, ignore this email. Your password stays as it is.",
        },
        EmailCodePurpose::Unlink => Copy {
            subject: "Your Rustic Isles code to unlink Discord",
            kicker: "Discord unlink",
            title: "Unlink Discord",
            lead: "Enter this code in the Rustic Isles launcher to unlink Discord from your account.",
            ignore: "If you didn't ask to unlink Discord, ignore this email: Discord stays linked. Someone may know your password, so consider changing it.",
        },
    }
}

const FONT_DISPLAY: &str = "'Cinzel', 'Trajan Pro', Georgia, 'Times New Roman', serif";
const FONT_BODY: &str = "'Segoe UI', 'Inter', Roboto, Helvetica, Arial, sans-serif";
const FONT_MONO: &str = "'Cascadia Mono', Consolas, 'SFMono-Regular', Menlo, monospace";

fn escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        if matches!(c, '&' | '<' | '>' | '"' | '\'') {
            out.push_str(&format!("&#{};", c as u32));
        } else {
            out.push(c);
        }
    }
    out
}

/// `RenderCodeEmail`: in the launcher's look, every style inline, the layout tables, no images. The code must be the
/// six digits the caller generated, so a caller bug can never put other text into the mail.
pub fn render_code_email(purpose: EmailCodePurpose, code: &str, expires_in_minutes: i64) -> Result<CodeEmail, String> {
    if !super::crypto::is_email_code_format(code) {
        return Err("RenderCodeEmail: the code must be 6 digits".into());
    }
    if expires_in_minutes < 1 {
        return Err("RenderCodeEmail: bad expiry".into());
    }
    let copy = copy(purpose);
    let minutes = format!(
        "{expires_in_minutes} minute{}",
        if expires_in_minutes == 1 { "" } else { "s" }
    );
    let expiry = format!("This code expires in {minutes} and works only once.");
    let footer =
        "Rustic Isles is a community-run, open-source Dauntless server. It is not affiliated with Phoenix Labs.";
    let subject = format!("{}: {code}", copy.subject);
    let text = [
        "RUSTIC ISLES",
        "",
        copy.title,
        "",
        copy.lead,
        "",
        &format!("    {code}"),
        "",
        &expiry,
        "",
        copy.ignore,
        "",
        "--",
        footer,
    ]
    .join("\n");
    let preheader = escape(&format!("{}: your code is {code}. {expiry}", copy.title));
    let (subject_html, kicker, title, lead, code_html, expiry_html, ignore, footer_html) = (
        escape(copy.subject),
        escape(copy.kicker),
        escape(copy.title),
        escape(copy.lead),
        escape(code),
        escape(&expiry),
        escape(copy.ignore),
        escape(footer),
    );
    let html = format!(
        r##"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="color-scheme" content="dark">
<meta name="supported-color-schemes" content="dark">
<title>{subject_html}</title>
</head>
<body style="margin:0;padding:0;background-color:#050a12;">
<div style="display:none;max-height:0;overflow:hidden;opacity:0;color:#050a12;">{preheader}</div>
<table role="presentation" width="100%" cellspacing="0" cellpadding="0" border="0" style="background-color:#050a12;">
  <tr>
    <td align="center" style="padding:40px 16px;">
      <table role="presentation" width="100%" cellspacing="0" cellpadding="0" border="0" style="max-width:480px;">
        <tr>
          <td align="center" style="padding:0 0 28px 0;">
            <div style="font-family:{FONT_DISPLAY};font-size:30px;line-height:30px;color:#8cc6ff;">&#10022;</div>
            <div style="padding-top:12px;font-family:{FONT_DISPLAY};font-size:19px;line-height:24px;letter-spacing:5px;color:#eef3fb;">RUSTIC ISLES</div>
            <div style="padding-top:6px;font-family:{FONT_BODY};font-size:10px;line-height:14px;letter-spacing:4px;color:#8191a8;">DAUNTLESS SERVER</div>
          </td>
        </tr>
        <tr>
          <td style="background-color:#0a1320;border:1px solid #1d2b40;border-radius:22px;padding:32px 24px;">
            <div style="font-family:{FONT_BODY};font-size:11px;line-height:16px;font-weight:600;letter-spacing:3px;text-transform:uppercase;color:#e0c287;">{kicker}</div>
            <div style="padding-top:10px;font-family:{FONT_DISPLAY};font-size:26px;line-height:32px;color:#eef3fb;">{title}</div>
            <div style="width:44px;height:2px;margin-top:14px;background-color:#5ba3ff;line-height:2px;font-size:2px;">&nbsp;</div>
            <div style="padding-top:18px;font-family:{FONT_BODY};font-size:15px;line-height:23px;color:#b9c5d8;">{lead}</div>
            <table role="presentation" width="100%" cellspacing="0" cellpadding="0" border="0" style="margin-top:26px;">
              <tr>
                <td align="center" style="background-color:#0c1a2e;border:1px solid #2a4a75;border-radius:14px;padding:20px 8px;">
                  <div style="font-family:{FONT_MONO};font-size:32px;line-height:40px;font-weight:700;letter-spacing:7px;white-space:nowrap;color:#eef3fb;">{code_html}</div>
                </td>
              </tr>
            </table>
            <div style="padding-top:18px;font-family:{FONT_BODY};font-size:13px;line-height:20px;color:#8cc6ff;">{expiry_html}</div>
            <div style="padding-top:22px;margin-top:22px;border-top:1px solid #16243a;font-family:{FONT_BODY};font-size:13px;line-height:20px;color:#8191a8;">{ignore}</div>
          </td>
        </tr>
        <tr>
          <td align="center" style="padding:24px 12px 0 12px;font-family:{FONT_BODY};font-size:11px;line-height:17px;color:#5d6b82;">{footer_html}</td>
        </tr>
      </table>
    </td>
  </tr>
</table>
</body>
</html>
"##
    );
    Ok(CodeEmail { subject, text, html })
}

// ------------------------------------------------------------------ sending

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MailMessage {
    pub to: String,
    pub subject: String,
    pub text: String,
    pub html: String,
}

/// A failed send, described by what may be logged: a code or a class name, never the SMTP reply's text.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct MailError(pub String);

/// `DescribeMailError`.
pub fn describe_mail_error(error: &MailError) -> &str {
    &error.0
}

/// `MailTransport`; a test installs a recording one in `AuthRuntime::mail`.
pub trait MailTransport: Send + Sync {
    fn send<'a>(&'a self, settings: &'a Settings, message: MailMessage) -> BoxFuture<'a, Result<(), MailError>>;
}

/// The SMTP transport, built from the module's settings at each send.
#[derive(Debug, Clone, Copy, Default)]
pub struct SmtpTransport;

impl MailTransport for SmtpTransport {
    fn send<'a>(&'a self, settings: &'a Settings, message: MailMessage) -> BoxFuture<'a, Result<(), MailError>> {
        Box::pin(async move {
            let SmtpReading::Configured(smtp) = read_smtp_settings(settings) else {
                return Err(MailError("NOT_CONFIGURED".into()));
            };
            deliver(&smtp, &message).await
        })
    }
}

fn tls_connector() -> TlsConnector {
    let roots = rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let config = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    TlsConnector::from(Arc::new(config))
}

fn io_error(error: impl std::fmt::Display) -> MailError {
    let _ = error;
    MailError("EIO".into())
}

async fn deliver(settings: &SmtpSettings, message: &MailMessage) -> Result<(), MailError> {
    let tcp = timeout(
        CONNECT_TIMEOUT,
        TcpStream::connect((settings.host.as_str(), settings.port)),
    )
    .await
    .map_err(|_| MailError("ETIMEDOUT".into()))?
    .map_err(|_| MailError("ECONNECTION".into()))?;
    let name = ServerName::try_from(settings.host.clone()).map_err(|_| MailError("ESERVERNAME".into()))?;
    let connector = tls_connector();
    if settings.implicit_tls {
        let tls = connector
            .connect(name, tcp)
            .await
            .map_err(|_| MailError("ETLS".into()))?;
        let mut session = Session::new(tls);
        session.greet().await?;
        session.ehlo().await?;
        return session.transaction(settings, message).await;
    }
    let mut plain = Session::new(tcp);
    plain.greet().await?;
    let capabilities = plain.ehlo().await?;
    if !capabilities.iter().any(|line| line.eq_ignore_ascii_case("STARTTLS")) {
        // requireTLS: the login never goes out in clear
        return Err(MailError("ETLS".into()));
    }
    plain.expect("STARTTLS", 220).await?;
    let tcp = plain.into_inner();
    let tls = connector
        .connect(name, tcp)
        .await
        .map_err(|_| MailError("ETLS".into()))?;
    let mut session = Session::new(tls);
    session.ehlo().await?;
    session.transaction(settings, message).await
}

/// One SMTP conversation over any stream.
pub struct Session<S> {
    reader: BufReader<S>,
}

impl<S: AsyncRead + AsyncWrite + Unpin> Session<S> {
    pub fn new(stream: S) -> Self {
        Self {
            reader: BufReader::new(stream),
        }
    }

    pub fn into_inner(self) -> S {
        self.reader.into_inner()
    }

    /// One reply: its code and its lines (text after the code). A multi-line reply has `-` after the code until the last.
    async fn reply(&mut self) -> Result<(u16, Vec<String>), MailError> {
        let mut lines = Vec::new();
        loop {
            let mut line = String::new();
            let read = timeout(IO_TIMEOUT, self.reader.read_line(&mut line))
                .await
                .map_err(|_| MailError("ETIMEDOUT".into()))?
                .map_err(io_error)?;
            if read == 0 || line.len() < 4 {
                return Err(MailError("ECONNECTION".into()));
            }
            let code: u16 = line[..3].parse().map_err(|_| MailError("EPROTOCOL".into()))?;
            lines.push(line[4..].trim_end().to_owned());
            if line.as_bytes()[3] != b'-' {
                return Ok((code, lines));
            }
        }
    }

    async fn write(&mut self, text: &str) -> Result<(), MailError> {
        let stream = self.reader.get_mut();
        timeout(IO_TIMEOUT, async {
            stream.write_all(text.as_bytes()).await?;
            stream.flush().await
        })
        .await
        .map_err(|_| MailError("ETIMEDOUT".into()))?
        .map_err(io_error)
    }

    /// Sends one command and wants one reply code.
    pub async fn expect(&mut self, command: &str, want: u16) -> Result<Vec<String>, MailError> {
        self.write(&format!("{command}\r\n")).await?;
        let (code, lines) = self.reply().await?;
        if code == want {
            Ok(lines)
        } else {
            Err(MailError(format!("EPROTOCOL{code}")))
        }
    }

    pub async fn greet(&mut self) -> Result<(), MailError> {
        let (code, _) = self.reply().await?;
        if code == 220 {
            Ok(())
        } else {
            Err(MailError(format!("EPROTOCOL{code}")))
        }
    }

    pub async fn ehlo(&mut self) -> Result<Vec<String>, MailError> {
        self.expect("EHLO localhost", 250).await
    }

    /// AUTH, the envelope and the message, then QUIT.
    pub async fn transaction(&mut self, settings: &SmtpSettings, message: &MailMessage) -> Result<(), MailError> {
        let plain = BASE64.encode(format!("\0{}\0{}", settings.user, settings.password));
        self.expect(&format!("AUTH PLAIN {plain}"), 235)
            .await
            .map_err(|_| MailError("EAUTH".into()))?;
        self.expect(&format!("MAIL FROM:<{}>", address_of(&settings.from)), 250)
            .await?;
        self.expect(&format!("RCPT TO:<{}>", message.to), 250).await?;
        self.expect("DATA", 354).await?;
        let body = mime(&settings.from, message);
        self.write(&dot_stuff(&body)).await?;
        self.write(".\r\n").await?;
        let (code, _) = self.reply().await?;
        if code != 250 {
            return Err(MailError(format!("EPROTOCOL{code}")));
        }
        // QUIT's reply does not matter: the message is accepted
        let _ = self.write("QUIT\r\n").await;
        Ok(())
    }
}

/// The address inside `Name <address>`, or the whole text.
fn address_of(from: &str) -> &str {
    match (from.rfind('<'), from.rfind('>')) {
        (Some(open), Some(close)) if open < close => &from[open + 1..close],
        _ => from.trim(),
    }
}

fn wrapped_base64(text: &str) -> String {
    let encoded = BASE64.encode(text.as_bytes());
    encoded
        .as_bytes()
        .chunks(76)
        .map(|chunk| String::from_utf8_lossy(chunk).into_owned())
        .collect::<Vec<_>>()
        .join("\r\n")
}

/// A `multipart/alternative` message: the text, then the HTML, each base64 so no line is too long.
fn mime(from: &str, message: &MailMessage) -> String {
    let boundary = format!("=_mp_{}", super::crypto::random_hex(12));
    let subject = if message.subject.is_ascii() {
        message.subject.clone()
    } else {
        format!("=?UTF-8?B?{}?=", BASE64.encode(message.subject.as_bytes()))
    };
    format!(
        "From: {from}\r\nTo: {to}\r\nSubject: {subject}\r\nMIME-Version: 1.0\r\nContent-Type: multipart/alternative; boundary=\"{boundary}\"\r\n\r\n--{boundary}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: base64\r\n\r\n{text}\r\n--{boundary}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Transfer-Encoding: base64\r\n\r\n{html}\r\n--{boundary}--\r\n",
        to = message.to,
        text = wrapped_base64(&message.text),
        html = wrapped_base64(&message.html),
    )
}

/// A line that starts with a dot gets another (RFC 5321 4.5.2).
fn dot_stuff(body: &str) -> String {
    body.split("\r\n")
        .map(|line| {
            if line.starts_with('.') {
                format!(".{line}")
            } else {
                line.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\r\n")
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncReadExt, duplex};

    use super::*;
    use crate::repo::auth::EmailCodePurpose::{Reset, Unlink, Verify};

    fn complete() -> Vec<(&'static str, &'static str)> {
        vec![
            ("RUSTIC_ISLES_SMTP_HOST", "smtp.example.test"),
            ("RUSTIC_ISLES_SMTP_USER", "postmaster"),
            ("RUSTIC_ISLES_SMTP_PASSWORD", "SECRET_PASSWORD_VALUE"),
        ]
    }

    fn read(extra: &[(&'static str, &'static str)], drop: &str) -> SmtpReading {
        let pairs = complete()
            .into_iter()
            .filter(|(k, _)| *k != drop)
            .chain(extra.iter().copied());
        read_smtp_settings(&Settings::from_pairs(pairs))
    }

    #[test]
    fn complete_settings_are_port_587_starttls_and_the_default_from() {
        let SmtpReading::Configured(settings) = read(&[], "") else {
            panic!("not configured")
        };
        assert_eq!(
            (
                settings.host.as_str(),
                settings.port,
                settings.implicit_tls,
                settings.from.as_str()
            ),
            ("smtp.example.test", 587, false, "Rustic Isles <no-reply@mysticfox.dev>")
        );
        assert_eq!(settings.password, "SECRET_PASSWORD_VALUE");
    }

    #[test]
    fn the_port_the_mode_and_the_from_header_can_be_set() {
        let extra = [
            ("RUSTIC_ISLES_SMTP_PORT", "465"),
            ("RUSTIC_ISLES_SMTP_SECURE", "TLS"),
            ("RUSTIC_ISLES_MAIL_FROM", "Ops <ops@example.test>"),
        ];
        let SmtpReading::Configured(settings) = read(&extra, "") else {
            panic!("not configured")
        };
        assert_eq!(
            (settings.port, settings.implicit_tls, settings.from.as_str()),
            (465, true, "Ops <ops@example.test>")
        );
        let SmtpReading::Configured(starttls) = read(&[("RUSTIC_ISLES_SMTP_SECURE", " starttls ")], "") else {
            panic!("not configured")
        };
        assert!(!starttls.implicit_tls);
    }

    #[test]
    fn a_missing_or_blank_host_user_or_password_is_a_problem_named_by_its_key() {
        for key in [
            "RUSTIC_ISLES_SMTP_HOST",
            "RUSTIC_ISLES_SMTP_USER",
            "RUSTIC_ISLES_SMTP_PASSWORD",
        ] {
            for value in ["", "   "] {
                let reading = read(&[(key, value)], key);
                assert_eq!(reading, SmtpReading::Problems(vec![key.to_owned()]), "{key}={value:?}");
            }
            assert_eq!(read(&[], key), SmtpReading::Problems(vec![key.to_owned()]));
        }
    }

    #[test]
    fn an_invalid_port_or_mode_is_a_problem_and_no_value_is_in_it() {
        let SmtpReading::Problems(problems) = read(&[("RUSTIC_ISLES_SMTP_PORT", "SECRET_PORT_TEXT")], "") else {
            panic!("configured")
        };
        let joined = problems.join(",");
        assert!(joined.contains("RUSTIC_ISLES_SMTP_PORT") && !joined.contains("SECRET_PORT_TEXT"));
        let SmtpReading::Problems(problems) = read(&[("RUSTIC_ISLES_SMTP_SECURE", "ssl-maybe")], "") else {
            panic!("an unknown secure mode never falls back to plain text")
        };
        assert!(problems.join(",").contains("RUSTIC_ISLES_SMTP_SECURE"));
        for port in ["0", "65536", "587.5"] {
            assert!(!read(&[("RUSTIC_ISLES_SMTP_PORT", port)], "").configured(), "{port}");
        }
    }

    #[test]
    fn the_password_is_kept_exactly_as_set() {
        let SmtpReading::Configured(settings) = read(
            &[("RUSTIC_ISLES_SMTP_PASSWORD", "  spaced  ")],
            "RUSTIC_ISLES_SMTP_PASSWORD",
        ) else {
            panic!("not configured")
        };
        assert_eq!(settings.password, "  spaced  ");
    }

    #[test]
    fn a_code_email_carries_the_code_the_expiry_and_the_single_use_note() {
        let mail = render_code_email(Verify, "482913", 5).unwrap();
        assert_eq!(mail.subject, "Your Rustic Isles code: 482913");
        assert!(mail.text.contains("482913"));
        assert!(mail.text.contains("expires in 5 minutes and works only once"));
        assert!(mail.html.contains(">482913<"));
        assert!(mail.html.contains("expires in 5 minutes and works only once"));
        let lower = mail.html.to_lowercase();
        assert!(!lower.contains("<img") && !lower.contains("<script") && !lower.contains("<link"));
    }

    #[test]
    fn reset_and_unlink_have_their_own_subject_and_wording() {
        let reset = render_code_email(Reset, "000017", 1).unwrap();
        assert_eq!(reset.subject, "Your Rustic Isles password reset code: 000017");
        assert!(reset.html.contains("Reset your password"));
        assert!(reset.text.contains("expires in 1 minute and"));
        let unlink = render_code_email(Unlink, "552019", 5).unwrap();
        assert_eq!(unlink.subject, "Your Rustic Isles code to unlink Discord: 552019");
        assert!(unlink.html.contains("Unlink Discord"));
        assert!(unlink.text.contains("Discord stays linked"));
    }

    #[test]
    fn anything_but_six_digits_or_a_bad_expiry_is_refused() {
        for code in ["12345", "12345a", "<b>1</b>"] {
            assert!(render_code_email(Verify, code, 5).is_err(), "{code}");
        }
        assert!(render_code_email(Verify, "123456", 0).is_err());
    }

    #[test]
    fn the_envelope_address_is_the_one_in_angle_brackets() {
        assert_eq!(
            address_of("Rustic Isles <no-reply@mysticfox.dev>"),
            "no-reply@mysticfox.dev"
        );
        assert_eq!(address_of(" a@b.c "), "a@b.c");
        assert_eq!(dot_stuff("a\r\n.b\r\nc"), "a\r\n..b\r\nc");
    }

    /// A scripted server: what it sends first, then a reply for each line it reads.
    async fn serve(mut stream: tokio::io::DuplexStream, script: Vec<(&'static str, &'static str)>) -> Vec<String> {
        let mut seen = Vec::new();
        stream.write_all(b"220 hello\r\n").await.unwrap();
        let mut reader = BufReader::new(stream);
        let mut in_data = false;
        let mut expected = script.into_iter();
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).await.unwrap() == 0 {
                return seen;
            }
            let line = line.trim_end().to_owned();
            if in_data {
                if line == "." {
                    in_data = false;
                    reader.get_mut().write_all(b"250 queued\r\n").await.unwrap();
                }
                continue;
            }
            seen.push(line.clone());
            if line == "QUIT" {
                return seen;
            }
            let (prefix, answer) = expected.next().unwrap_or(("", "500 unexpected\r\n"));
            assert!(line.starts_with(prefix), "{line} vs {prefix}");
            if line == "DATA" {
                in_data = true;
            }
            reader.get_mut().write_all(answer.as_bytes()).await.unwrap();
        }
    }

    #[tokio::test]
    async fn a_message_goes_through_the_smtp_dialogue() {
        let (client, server) = duplex(64 * 1024);
        let script = vec![
            ("EHLO", "250-hi\r\n250 AUTH PLAIN\r\n"),
            ("AUTH PLAIN", "235 ok\r\n"),
            ("MAIL FROM:<no-reply@mysticfox.dev>", "250 ok\r\n"),
            ("RCPT TO:<fox@example.test>", "250 ok\r\n"),
            ("DATA", "354 go\r\n"),
        ];
        let server = tokio::spawn(serve(server, script));
        let settings = SmtpSettings {
            host: "smtp.example.test".into(),
            port: 465,
            implicit_tls: true,
            user: "postmaster".into(),
            password: "pw".into(),
            from: MAIL_FROM_DEFAULT.into(),
        };
        let mail = render_code_email(Verify, "123456", 5).unwrap();
        let message = MailMessage {
            to: "fox@example.test".into(),
            subject: mail.subject,
            text: mail.text,
            html: mail.html,
        };
        let mut session = Session::new(client);
        session.greet().await.unwrap();
        session.ehlo().await.unwrap();
        session.transaction(&settings, &message).await.unwrap();
        drop(session);
        let seen = server.await.unwrap();
        assert_eq!(seen.last().map(String::as_str), Some("QUIT"));
        assert!(seen.iter().any(|line| line == "RCPT TO:<fox@example.test>"));
    }

    #[tokio::test]
    async fn a_refused_login_is_an_auth_failure_without_the_servers_text() {
        let (client, server) = duplex(4096);
        let server = tokio::spawn(serve(
            server,
            vec![
                ("EHLO", "250 hi\r\n"),
                ("AUTH PLAIN", "535 5.7.8 fox@example.test rejected\r\n"),
            ],
        ));
        let settings = SmtpSettings {
            host: "h".into(),
            port: 465,
            implicit_tls: true,
            user: "u".into(),
            password: "p".into(),
            from: MAIL_FROM_DEFAULT.into(),
        };
        let message = MailMessage {
            to: "fox@example.test".into(),
            subject: "s".into(),
            text: "t".into(),
            html: "h".into(),
        };
        let mut session = Session::new(client);
        session.greet().await.unwrap();
        session.ehlo().await.unwrap();
        let error = session.transaction(&settings, &message).await.unwrap_err();
        assert_eq!(error, MailError("EAUTH".into()));
        drop(session);
        let _ = server.await;
    }

    #[tokio::test]
    async fn an_unconfigured_transport_refuses() {
        let message = MailMessage {
            to: "a@b.c".into(),
            subject: "s".into(),
            text: "t".into(),
            html: "h".into(),
        };
        let error = SmtpTransport.send(&Settings::default(), message).await.unwrap_err();
        assert_eq!(error, MailError("NOT_CONFIGURED".into()));
        let mut buffer = Vec::new();
        let _ = duplex(8).0.read(&mut buffer).await;
    }
}
