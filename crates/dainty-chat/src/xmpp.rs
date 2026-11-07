// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! XMPP text: the frame parser (ltx's behaviour, not XML's), escaping, JIDs, the token bucket and the trace's
//! redaction.

/// RFC 7395's WebSocket framing (`<open/>`, `<close/>`).
pub const NS_FRAMING: &str = "urn:ietf:params:xml:ns:xmpp-framing";
/// SASL, for the login.
pub const NS_SASL: &str = "urn:ietf:params:xml:ns:xmpp-sasl";
/// Resource binding.
pub const NS_BIND: &str = "urn:ietf:params:xml:ns:xmpp-bind";
/// The legacy session establishment the client still asks for.
pub const NS_SESSION: &str = "urn:ietf:params:xml:ns:xmpp-session";
/// The `stream:` prefix.
pub const NS_STREAMS: &str = "http://etherx.jabber.org/streams";
/// Stream error conditions.
pub const NS_STREAM_ERRORS: &str = "urn:ietf:params:xml:ns:xmpp-streams";
/// Stanza error conditions.
pub const NS_STANZAS: &str = "urn:ietf:params:xml:ns:xmpp-stanzas";
/// The client's default namespace.
pub const NS_CLIENT: &str = "jabber:client";
/// XEP-0199 ping.
pub const NS_PING: &str = "urn:xmpp:ping";
/// XEP-0045 multi-user chat, on join.
pub const NS_MUC: &str = "http://jabber.org/protocol/muc";
/// XEP-0045 multi-user chat, on presence and messages.
pub const NS_MUC_USER: &str = "http://jabber.org/protocol/muc#user";
/// XEP-0203 delayed delivery, on history.
pub const NS_DELAY: &str = "urn:xmpp:delay";
/// The 1.14.7 client's built-in XMPP domain
pub const DEFAULT_DOMAIN: &str = "prod.ol.epicgames.com";

/// The longest trace line, in UTF-16 units
const TRACE_LIMIT: usize = 2048;

/// A child of an element.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Node {
    /// A nested element.
    Element(Element),
    /// Text, unescaped.
    Text(String),
}

/// An element as the frame parser reads it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Element {
    /// The tag name as written, prefix included.
    pub name: String,
    /// Attributes in document order, values unescaped.
    pub attrs: Vec<(String, String)>,
    /// Children in document order.
    pub children: Vec<Node>,
}

impl Element {
    /// The value of the attribute named exactly `name`.
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
    }

    /// The element's own text children, joined.
    pub fn text(&self) -> String {
        let mut out = String::new();
        for node in &self.children {
            if let Node::Text(text) = node {
                out.push_str(text);
            }
        }
        out
    }

    /// The first child element whose local name, lower-cased, is `name`.
    pub fn child(&self, name: &str) -> Option<&Element> {
        self.elements().find(|e| local_name(&e.name) == name)
    }

    /// The child elements, skipping text.
    pub fn elements(&self) -> impl Iterator<Item = &Element> {
        self.children.iter().filter_map(|node| match node {
            Node::Element(e) => Some(e),
            Node::Text(_) => None,
        })
    }
}

/// The element's own text, or empty for `None`.
pub fn text_of(element: Option<&Element>) -> String {
    element.map(Element::text).unwrap_or_default()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Text,
    TagName,
    InTag,
    AttrName,
    AttrEq,
    AttrQuote,
    AttrValue,
}

/// The open elements and the first finished root.
#[derive(Default)]
struct Tree {
    open: Vec<Element>,
    root: Option<Element>,
}

impl Tree {
    fn close(&mut self) {
        let Some(done) = self.open.pop() else { return };
        match self.open.last_mut() {
            Some(parent) => parent.children.push(Node::Element(done)),
            None => {
                if self.root.is_none() {
                    self.root = Some(done);
                }
            }
        }
    }
}

/// One frame as ltx 3.1.2's parser builds it: the first complete root element, or none.
pub fn parse_frame(input: &str) -> Option<Element> {
    let bytes = input.as_bytes();
    let mut tree = Tree::default();
    let mut mode = Mode::Text;
    let mut mark: usize = 0;
    let mut name = String::new();
    let mut attrs: Vec<(String, String)> = Vec::new();
    let mut end_tag = false;
    let mut self_closing = false;
    let mut quote = b'"';
    let mut attr_name = String::new();
    let mut pos = 0;
    while pos < bytes.len() {
        let byte = bytes[pos];
        match mode {
            Mode::Text => {
                let Some(offset) = bytes[pos..].iter().position(|&b| b == b'<') else {
                    break;
                };
                let lt = pos + offset;
                if lt > mark {
                    let text = unescape(&input[mark..lt])?;
                    if let Some(parent) = tree.open.last_mut() {
                        parent.children.push(Node::Text(text));
                    }
                }
                mode = Mode::TagName;
                mark = lt + 1;
                attrs.clear();
                pos = lt + 1;
                continue;
            }
            Mode::TagName => {
                if byte == b'/' && pos == mark {
                    end_tag = true;
                    mark = pos + 1;
                } else if byte == b'!' || byte == b'?' {
                    return None;
                } else if byte <= 32 || byte == b'/' || byte == b'>' {
                    name = input[mark..pos].to_owned();
                    mode = Mode::InTag;
                    continue;
                }
            }
            Mode::InTag => {
                if byte == b'>' {
                    if end_tag {
                        if tree.open.last().is_some_and(|e| e.name == name) {
                            tree.close();
                        }
                    } else {
                        tree.open.push(Element {
                            name: std::mem::take(&mut name),
                            attrs: std::mem::take(&mut attrs),
                            children: Vec::new(),
                        });
                        if self_closing {
                            tree.close();
                        }
                    }
                    end_tag = false;
                    self_closing = false;
                    mode = Mode::Text;
                    mark = pos + 1;
                } else if byte == b'/' {
                    self_closing = true;
                } else if byte > 32 {
                    mode = Mode::AttrName;
                    mark = pos;
                }
            }
            Mode::AttrName => {
                if byte <= 32 || byte == b'=' {
                    attr_name = input[mark..pos].to_owned();
                    mode = Mode::AttrEq;
                    continue;
                }
            }
            Mode::AttrEq => {
                if byte == b'=' {
                    mode = Mode::AttrQuote;
                }
            }
            Mode::AttrQuote => {
                if byte == b'"' || byte == b'\'' {
                    quote = byte;
                    mode = Mode::AttrValue;
                    mark = pos + 1;
                }
            }
            Mode::AttrValue => {
                let Some(offset) = bytes[pos..].iter().position(|&b| b == quote) else {
                    break;
                };
                let end = pos + offset;
                let value = unescape(&input[mark..end])?;
                match attrs.iter_mut().find(|(n, _)| *n == attr_name) {
                    Some(slot) => slot.1 = value,
                    None => attrs.push((std::mem::take(&mut attr_name), value)),
                }
                mode = Mode::InTag;
                pos = end + 1;
                continue;
            }
        }
        pos += 1;
    }
    tree.root
}

/// `<!` or `<?` anywhere: refused before parsing.
pub fn has_markup_declaration(text: &str) -> bool {
    text.contains("<!") || text.contains("<?")
}

/// ltx's unescape: the first `;` after an `&` ends the entity, and a lone `&` with no `;` after it stays.
fn unescape(text: &str) -> Option<String> {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(amp) = rest.find('&') {
        let Some(semi) = rest[amp..].find(';').map(|s| amp + s) else {
            break;
        };
        out.push_str(&rest[..amp]);
        let entity = &rest[amp + 1..semi];
        match entity {
            "amp" => out.push('&'),
            "lt" => out.push('<'),
            "gt" => out.push('>'),
            "quot" => out.push('"'),
            "apos" => out.push('\''),
            _ => {
                let body = entity.strip_prefix('#')?;
                let code = match body.strip_prefix('x') {
                    Some(hex) => parse_int(hex, 16)?,
                    None => parse_int(body, 10)?,
                };
                out.push(xml_char(code)?);
            }
        }
        rest = &rest[semi + 1..];
    }
    out.push_str(rest);
    Some(out)
}

/// JavaScript's `parseInt(text, radix)`, as far as a character reference needs it: none when it gives `NaN`.
fn parse_int(text: &str, radix: u32) -> Option<i64> {
    let text = text.trim_start_matches(dainty_service_kit::js::is_space);
    let (negative, text) = match text.as_bytes().first() {
        Some(b'-') => (true, &text[1..]),
        Some(b'+') => (false, &text[1..]),
        _ => (false, text),
    };
    let text = if radix == 16 {
        text.strip_prefix("0x")
            .or_else(|| text.strip_prefix("0X"))
            .unwrap_or(text)
    } else {
        text
    };
    let digits: Vec<u32> = text.chars().map_while(|c| c.to_digit(radix)).collect();
    if digits.is_empty() {
        return None;
    }
    // Past 15 digits JavaScript's number has lost the code anyway; none of those is a legal char
    let value = digits
        .iter()
        .take(15)
        .fold(0_i64, |acc, &d| acc * i64::from(radix) + i64::from(d));
    Some(if negative { -value } else { value })
}

fn xml_char(code: i64) -> Option<char> {
    let legal = matches!(code, 0x9 | 0xA | 0xD | 0x20..=0xD7FF | 0xE000..=0xFFFD | 0x10000..=0x10FFFF);
    if !legal {
        return None;
    }
    char::from_u32(u32::try_from(code).ok()?)
}

/// `text` with `& < > " '` escaped, for text and attribute values alike.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

/// The name without its prefix, lower-cased.
pub fn local_name(name: &str) -> String {
    name.rsplit(':').next().unwrap_or(name).to_lowercase()
}

/// Client text made safe for a log line: printable ASCII only, cut at `max` chars.
pub fn log_text(text: &str, max: usize) -> String {
    let mut out: String = text
        .chars()
        .take(max)
        .map(|c| if ('\u{21}'..='\u{7e}').contains(&c) { c } else { '?' })
        .collect();
    if text.chars().nth(max).is_some() {
        out.push_str("...");
    }
    out
}

/// Whether `text` is 1 to 253 ASCII letters, digits, `.` and `-`.
pub fn is_host_name(text: &str) -> bool {
    (1..=253).contains(&text.len())
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
}

/// A stream's `to`: a host name with an optional port.
pub fn is_xmpp_domain(text: &str) -> bool {
    if text.is_empty() || text.len() > 259 {
        return false;
    }
    match text.split_once(':') {
        None => is_host_name(text),
        Some((host, port)) => {
            is_host_name(host)
                && (1..=5).contains(&port.len())
                && port.bytes().all(|b| b.is_ascii_digit())
                && port.parse::<u32>().is_ok_and(|p| (1..=65535).contains(&p))
        }
    }
}

/// A JID split into its parts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Jid {
    /// The local part, before `@`.
    pub local: String,
    /// The domain, between `@` and the first `/`.
    pub domain: String,
    /// Everything after the first `/`.
    pub resource: Option<String>,
}

/// `local@domain[/resource]`, split as the Node chat split it: the resource is all after the first `/`.
pub fn parse_jid(text: &str) -> Option<Jid> {
    let (bare, resource) = match text.split_once('/') {
        Some((bare, resource)) => (bare, Some(resource.to_owned())),
        None => (text, None),
    };
    let (local, domain) = bare.split_once('@')?;
    if local.is_empty() || domain.is_empty() {
        return None;
    }
    Some(Jid {
        local: local.to_owned(),
        domain: domain.to_owned(),
        resource,
    })
}

/// Whether `text` holds any Unicode control character.
pub fn has_control_character(text: &str) -> bool {
    text.chars().any(char::is_control)
}

/// A token bucket, refilled continuously.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bucket {
    tokens: f64,
    at: i64,
}

impl Bucket {
    /// A bucket holding `burst` tokens at `now` (unix milliseconds).
    pub fn new(burst: u32, now: i64) -> Self {
        Self {
            tokens: f64::from(burst),
            at: now,
        }
    }

    /// Takes a token if one is left after refilling one per `refill_ms` up to `burst`; `false` when empty.
    pub fn take(&mut self, burst: u32, refill_ms: f64, now: i64) -> bool {
        #[allow(clippy::cast_precision_loss, reason = "ms between two clock reads, far below 2^53")]
        let elapsed = now.saturating_sub(self.at).max(0) as f64;
        self.tokens = (self.tokens + elapsed / refill_ms).min(f64::from(burst));
        self.at = now;
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

/// A frame or stanza as the trace may show it: no credentials, no message text, bounded.
pub fn redact_frame(text: &str) -> String {
    let mut out = text.to_owned();
    for tag in ["auth", "password", "body"] {
        out = redact_tag(&out, tag);
    }
    let out = crate::redact::redact(&out);
    if out.encode_utf16().count() <= TRACE_LIMIT {
        return out;
    }
    let mut units = 0;
    let mut cut = String::new();
    for c in out.chars() {
        units += c.len_utf16();
        if units > TRACE_LIMIT {
            break;
        }
        cut.push(c);
    }
    cut.push_str("...");
    cut
}

fn find_ascii_ci(haystack: &str, needle: &str, from: usize) -> Option<usize> {
    let hay = haystack.as_bytes();
    let needle = needle.as_bytes();
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    (from..=hay.len() - needle.len()).find(|&i| hay[i..i + needle.len()].eq_ignore_ascii_case(needle))
}

fn redact_tag(text: &str, tag: &str) -> String {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut out = String::with_capacity(text.len());
    let mut from = 0;
    let mut search = 0;
    while let Some(start) = find_ascii_ci(text, &open, search) {
        let after = start + open.len();
        let next = text.as_bytes().get(after).copied();
        if next.is_some_and(|b| b.is_ascii_alphanumeric() || b == b'_') {
            search = after;
            continue;
        }
        let Some(gt) = text[after..].find('>').map(|i| after + i) else {
            break;
        };
        let Some(end) = find_ascii_ci(text, &close, gt + 1) else {
            break;
        };
        out.push_str(&text[from..=gt]);
        if tag == "body" {
            out.push_str(&format!("[{} chars]", text[gt + 1..end].chars().count()));
        } else {
            out.push_str("[redacted]");
        }
        from = end;
        search = end + close.len();
    }
    out.push_str(&text[from..]);
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "a panic is how a test fails")]
    use super::*;

    fn el(name: &str) -> Element {
        Element {
            name: name.into(),
            ..Element::default()
        }
    }

    #[test]
    fn frames_parse_as_ltx_parses_them() {
        let a = parse_frame("<a><b></c></b></a>").unwrap();
        assert_eq!(a.name, "a");
        assert_eq!(a.children, vec![Node::Element(el("b"))]);

        let a = parse_frame(r#"<a b c="1"/>"#).unwrap();
        assert_eq!(a.attrs, vec![("b".to_owned(), "1".to_owned())]);

        assert_eq!(parse_frame("<a><b/>"), None);
        assert_eq!(parse_frame("text only"), None);
        assert_eq!(parse_frame("<a/><b/>").unwrap().name, "a");
        let features = parse_frame(r#"<stream:features xmlns:stream="x"><bind/></stream:features>"#).unwrap();
        assert_eq!(features.name, "stream:features");
        assert_eq!(local_name(&features.name), "features");

        let m = parse_frame(r#"pre<message to='a@b' x="1" x="2"><body>a &amp; b &lt;c&gt;</body>tail</message>post"#)
            .unwrap();
        assert_eq!(m.attr("x"), Some("2"));
        assert_eq!(m.attrs[0].0, "to");
        assert_eq!(m.attrs[1], ("x".to_owned(), "2".to_owned()));
        assert_eq!(text_of(m.child("body")), "a & b <c>");
        assert_eq!(m.text(), "tail");
        assert_eq!(text_of(None), "");

        assert_eq!(
            parse_frame("<a>&#65;&#x42;&#x0x43; &#  68;</a>").unwrap().text(),
            "ABC D"
        );
        assert_eq!(parse_frame("<a>x & y</a>").unwrap().text(), "x & y");
        assert_eq!(parse_frame("<a>a & b; c</a>"), None);
        assert_eq!(parse_frame("<a>&#X41;</a>"), None);
        assert_eq!(parse_frame("<a>&#0;</a>"), None);
        assert_eq!(parse_frame("<a>&#xD800;</a>"), None);
        assert_eq!(parse_frame("<a>&nbsp;</a>"), None);
        assert_eq!(parse_frame("<a b='&bad;'/>"), None);
        assert_eq!(parse_frame("<!a/>"), None);
        assert_eq!(parse_frame("<a?/>"), None);
        assert!(has_markup_declaration("x<!DOCTYPE"));
        assert!(has_markup_declaration("<?xml"));
        assert!(!has_markup_declaration("<a>!?</a>"));

        let deep = "<a>".repeat(100_000);
        assert_eq!(parse_frame(&deep), None);
        // As deep as a FRAME_LIMIT frame can nest
        let nested = format!("{}{}", "<a>".repeat(4_700), "</a>".repeat(4_700));
        assert!(parse_frame(&nested).is_some());
    }

    #[test]
    fn helpers_match_node() {
        assert_eq!(escape(r#"<a href="x">&'"#), "&lt;a href=&quot;x&quot;&gt;&amp;&apos;");
        assert_eq!(log_text("ab cé", 3), "ab?...");
        assert_eq!(log_text("abc", 3), "abc");
        assert_eq!(local_name("Stream:FEATURES"), "features");
        assert_eq!(local_name("a:b:c"), "c");

        assert!(is_host_name("prod.ol.epicgames.com"));
        assert!(!is_host_name(""));
        assert!(!is_host_name(&"a".repeat(254)));
        assert!(is_xmpp_domain("127.0.0.1:61000"));
        assert!(is_xmpp_domain("dom"));
        for bad in ["host:0", "host:123456", "host:", "bad host", "", "h:65536"] {
            assert!(!is_xmpp_domain(bad), "{bad}");
        }

        let jid = parse_jid("City-1@muc.dom/Name:UID-a:V2:x/y").unwrap();
        assert_eq!(jid.local, "City-1");
        assert_eq!(jid.domain, "muc.dom");
        assert_eq!(jid.resource.as_deref(), Some("Name:UID-a:V2:x/y"));
        assert_eq!(parse_jid("a@b@c").unwrap().domain, "b@c");
        assert_eq!(parse_jid("a@b/").unwrap().resource.as_deref(), Some(""));
        assert_eq!(parse_jid("@dom"), None);
        assert_eq!(parse_jid("a@"), None);
        assert_eq!(parse_jid("nodomain"), None);

        assert!(has_control_character("a\u{7f}"));
        assert!(!has_control_character("a b"));

        let mut bucket = Bucket::new(2, 0);
        assert!(bucket.take(2, 1000.0, 0));
        assert!(bucket.take(2, 1000.0, 0));
        assert!(!bucket.take(2, 1000.0, 0));
        assert!(!bucket.take(2, 1000.0, 999));
        assert!(bucket.take(2, 1000.0, 1000));
        let mut back = Bucket::new(1, 100);
        assert!(back.take(1, 1000.0, 50));
        assert!(!back.take(1, 1000.0, 50));
    }

    #[test]
    fn the_trace_is_redacted() {
        assert_eq!(
            redact_frame("<auth mechanism='PLAIN'>c2VjcmV0</auth>"),
            "<auth mechanism='PLAIN'>[redacted]</auth>"
        );
        assert_eq!(
            redact_frame("<message><body>héllo</body></message>"),
            "<message><body>[5 chars]</body></message>"
        );
        assert_eq!(redact_frame("<AUTH>x</Auth>"), "<AUTH>[redacted]</Auth>");
        assert_eq!(redact_frame("<authx>x</authx>"), "<authx>x</authx>");
        assert_eq!(
            redact_frame("<body>a</body><body>bc</body>"),
            "<body>[1 chars]</body><body>[2 chars]</body>"
        );
        assert_eq!(redact_frame("<body>open"), "<body>open");
        assert_eq!(
            redact_frame("<password>p</password>"),
            "<password>[redacted]</password>"
        );
        let long = format!("<a>{}</a>", "é ".repeat(3000));
        let cut = redact_frame(&long);
        assert!(cut.ends_with("..."));
        assert_eq!(cut.encode_utf16().count(), 2048 + 3);
        assert!(redact_frame(&format!("<a>{}</a>", "x".repeat(70))).contains("<redacted>"));
    }
}
