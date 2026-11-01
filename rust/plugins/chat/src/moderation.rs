//! New here: the staff dashboard's chat filter, chat log and reports (dainty-dashboard
//! `docs/integration.md`, "Ingest API"). The filter is polled from the dashboard and applied before delivery;
//! every line, with the verdict, and every `/report` go to it in the background. Without a dashboard
//! configured all of it is off and chat behaves as before.
//!
//! The matcher follows the dashboard's `lib/chat/filter.ts`, so both rate a line the same. Two differences
//! the `regex` crate forces: `word` terms check their edges by hand (no look-behind), and a `regex` term that
//! uses look-around or back-references does not compile here and is skipped, as `filter.ts` skips a bad one.

use std::sync::{Arc, PoisonError, RwLock};
use std::time::Duration;

use http_body_util::{BodyExt, Full, Limited};
use hyper::Request;
use hyper::body::Bytes;
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioExecutor;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

/// What a line is checked against: the whole filter, or (per term) only some channels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    City,
    Party,
    Guild,
    Direct,
    System,
}

/// Party and hunt rooms are the dashboard's `party`; the network and regional rooms are Ramsgate's `city`.
pub fn channel_of(kind: &module_api::chat::RoomKind) -> Channel {
    use module_api::chat::RoomKind;
    match kind {
        RoomKind::Party(_) | RoomKind::Hunt(_) => Channel::Party,
        RoomKind::Network | RoomKind::Region(_) => Channel::City,
    }
}

impl Channel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::City => "city",
            Self::Party => "party",
            Self::Guild => "guild",
            Self::Direct => "direct",
            Self::System => "system",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Flag,
    Mask,
    Block,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    Allowed,
    Flagged,
    Masked,
    Blocked,
}

impl Verdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Allowed => "allowed",
            Self::Flagged => "flagged",
            Self::Masked => "masked",
            Self::Blocked => "blocked",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MatchType {
    Word,
    Substring,
    Regex,
}

/// One term as `GET /api/ingest/chat-filter` lists it.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Term {
    pub id: i64,
    pub pattern: String,
    pub match_type: MatchType,
    pub action: Action,
    #[serde(default)]
    pub channels: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct FilterDocument {
    version: i64,
    terms: Vec<Term>,
}

struct Compiled {
    action: Action,
    channels: Option<Vec<String>>,
    regex: regex::Regex,
    word: bool,
}

/// A compiled filter. The empty one allows everything.
#[derive(Default)]
pub struct Filter {
    pub version: Option<i64>,
    terms: Vec<Compiled>,
}

/// What the filter made of a line: the verdict and the text recipients see.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rated {
    pub verdict: Verdict,
    pub shown: String,
}

fn leet(ch: char) -> Option<&'static str> {
    Some(match ch {
        'a' => "[a@4]",
        'e' => "[e3]",
        'i' => "[i1!|]",
        'o' => "[o0]",
        's' => "[s5$]",
        't' => "[t7]",
        'l' => "[l1|]",
        _ => return None,
    })
}

/// A term's pattern as a case-insensitive regex (`compilePattern`); `None` when it does not compile.
fn compile_pattern(pattern: &str, match_type: MatchType) -> Option<regex::Regex> {
    if pattern.trim().is_empty() || pattern.chars().count() > 200 {
        return None;
    }
    let body = match match_type {
        MatchType::Regex => pattern.to_owned(),
        MatchType::Word | MatchType::Substring => pattern
            .to_lowercase()
            .chars()
            .map(|ch| leet(ch).map_or_else(|| regex::escape(&ch.to_string()), str::to_owned))
            .collect::<Vec<_>>()
            .join(r"[\s._-]*"),
    };
    regex::RegexBuilder::new(&body)
        .case_insensitive(true)
        .size_limit(1 << 20)
        .build()
        .ok()
}

fn is_word_char(ch: char) -> bool {
    ch.is_alphabetic() || ch.is_numeric()
}

impl Filter {
    pub fn compile(version: Option<i64>, terms: &[Term]) -> Self {
        let terms = terms
            .iter()
            .filter_map(|t| {
                Some(Compiled {
                    action: t.action,
                    channels: t.channels.clone(),
                    regex: compile_pattern(&t.pattern, t.match_type)?,
                    word: t.match_type == MatchType::Word,
                })
            })
            .collect();
        Self { version, terms }
    }

    /// Byte spans each term matches, with its action (`findMatches`).
    fn matches(&self, text: &str, channel: Channel) -> Vec<(Action, usize, usize)> {
        let mut found = Vec::new();
        for term in &self.terms {
            if term
                .channels
                .as_ref()
                .is_some_and(|c| !c.iter().any(|c| c == channel.as_str()))
            {
                continue;
            }
            let mut at = 0;
            while at <= text.len() {
                let Some(m) = term.regex.find_at(text, at) else {
                    break;
                };
                let edges_ok = !term.word
                    || (!text[..m.start()].chars().next_back().is_some_and(is_word_char)
                        && !text[m.end()..].chars().next().is_some_and(is_word_char));
                if !m.is_empty() && edges_ok {
                    found.push((term.action, m.start(), m.end()));
                    at = m.end();
                } else {
                    // Look again one character on, as look-behind would have let the next start match
                    at = m.start() + text[m.start()..].chars().next().map_or(1, char::len_utf8);
                }
            }
        }
        found.sort_by_key(|f| f.1);
        found
    }

    pub fn rate(&self, text: &str, channel: Channel) -> Rated {
        let found = self.matches(text, channel);
        let verdict = match found.iter().map(|f| f.0).max() {
            None => Verdict::Allowed,
            Some(Action::Flag) => Verdict::Flagged,
            Some(Action::Mask) => Verdict::Masked,
            Some(Action::Block) => Verdict::Blocked,
        };
        let shown = if verdict == Verdict::Masked {
            text.char_indices()
                .map(|(i, ch)| {
                    let hidden = found.iter().any(|&(a, s, e)| a != Action::Flag && (s..e).contains(&i));
                    if hidden && !ch.is_whitespace() { '*' } else { ch }
                })
                .collect()
        } else {
            text.to_owned()
        };
        Rated { verdict, shown }
    }
}

/// Where the dashboard is: `CHAT_DASHBOARD_URL` and `CHAT_DASHBOARD_INGEST_KEY` in chat's env file.
#[derive(Clone, PartialEq, Eq)]
pub struct DashboardSettings {
    pub url: String,
    /// The dashboard refuses a key under 32 characters. Never logged.
    pub key: String,
}

impl std::fmt::Debug for DashboardSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DashboardSettings")
            .field("url", &self.url)
            .finish_non_exhaustive()
    }
}

impl DashboardSettings {
    pub fn problems(&self) -> Vec<String> {
        let mut problems = Vec::new();
        if !(self.url.starts_with("http://") || self.url.starts_with("https://")) {
            problems.push("the dashboard URL is http:// or https://".to_owned());
        }
        if self.key.chars().count() < 32 {
            problems.push("the dashboard ingest key is at least 32 characters".to_owned());
        }
        problems
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Person {
    pub user_id: String,
    pub name: String,
}

/// One line for `POST /api/ingest/chat`, with the original text.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Line {
    pub id: String,
    pub at: i64,
    pub channel: &'static str,
    pub room_id: Option<String>,
    pub sender: Person,
    pub recipient: Option<Person>,
    pub body: String,
    pub verdict: Verdict,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReportContext {
    #[serde(rename = "roomId")]
    pub room_id: Option<String>,
    pub at: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub id: String,
    pub source: &'static str,
    pub reporter: Person,
    pub target: TargetPerson,
    pub category: &'static str,
    pub description: String,
    pub context: ReportContext,
}

#[derive(Debug, Clone, Serialize)]
pub struct TargetPerson {
    #[serde(rename = "userId", skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    pub name: String,
}

pub const REPORT_CATEGORIES: [&str; 8] = [
    "harassment",
    "hate_speech",
    "cheating",
    "griefing",
    "spam",
    "inappropriate_name",
    "real_money_trading",
    "other",
];

/// `/report <player> [category] <what happened>`: the target, the category (`other` when the second word
/// isn't one) and the text. `None` when the line is not a report command; `Some(Err)` when it is malformed.
pub fn parse_report(body: &str) -> Option<Result<(String, &'static str, String), ()>> {
    let rest = body.trim_start();
    let rest = rest
        .strip_prefix("/report")
        .filter(|r| r.is_empty() || r.starts_with(char::is_whitespace))?;
    let mut words = rest.split_whitespace();
    let Some(target) = words.next().filter(|t| t.chars().count() <= 64) else {
        return Some(Err(()));
    };
    let words: Vec<&str> = words.collect();
    let (category, text) = match words.first().map(|w| w.to_ascii_lowercase()) {
        Some(w) => match REPORT_CATEGORIES.iter().find(|c| **c == w) {
            Some(c) => (*c, words[1..].join(" ")),
            None => ("other", words.join(" ")),
        },
        None => ("other", String::new()),
    };
    Some(Ok((target.to_owned(), category, text.chars().take(2000).collect())))
}

/// Lines waiting to go; past this the newest are dropped rather than holding chat's memory hostage.
const QUEUE: usize = 10_000;
const BATCH: usize = 500;
const POLL: Duration = Duration::from_secs(30);
const TIMEOUT: Duration = Duration::from_secs(10);

enum Outgoing {
    Line(Line),
    Report(Report),
}

/// Chat's handle on the dashboard: the current filter and the queue to it. Cheap to clone.
#[derive(Clone, Default)]
pub struct Moderation {
    filter: Arc<RwLock<Arc<Filter>>>,
    out: Option<mpsc::Sender<Outgoing>>,
}

impl Moderation {
    /// Off: everything allowed, nothing sent.
    pub fn off() -> Self {
        Self::default()
    }

    /// A fixed filter and no dashboard (tests).
    pub fn with_filter(filter: Filter) -> Self {
        Self {
            filter: Arc::new(RwLock::new(Arc::new(filter))),
            out: None,
        }
    }

    /// Starts polling the filter and shipping lines and reports. Needs a Tokio runtime.
    pub fn start(settings: DashboardSettings) -> Self {
        let (out, queue) = mpsc::channel(QUEUE);
        let moderation = Self {
            filter: Arc::default(),
            out: Some(out),
        };
        let dashboard = Arc::new(Dashboard::new(settings));
        tokio::spawn(poll_filter(dashboard.clone(), moderation.filter.clone()));
        tokio::spawn(ship(dashboard, queue));
        moderation
    }

    pub fn is_on(&self) -> bool {
        self.out.is_some()
    }

    /// Rates one line about to be delivered, and counts it.
    pub fn rate(&self, text: &str, channel: Channel) -> Rated {
        let filter = self.filter.read().unwrap_or_else(PoisonError::into_inner).clone();
        let rated = filter.rate(text, channel);
        service_kit::metrics::count(
            "dainty_chat_messages_total",
            "Room lines and whispers sent, by channel and the filter's verdict",
            &[("channel", channel.as_str()), ("verdict", rated.verdict.as_str())],
            1,
        );
        rated
    }

    pub fn record(&self, line: Line) {
        if let Some(out) = &self.out
            && out.try_send(Outgoing::Line(line)).is_err()
        {
            tracing::warn!("chat: dashboard queue full, a chat line was not sent");
        }
    }

    pub fn report(&self, report: Report) -> bool {
        self.out
            .as_ref()
            .is_some_and(|out| out.try_send(Outgoing::Report(report)).is_ok())
    }
}

type Https = hyper_rustls::HttpsConnector<HttpConnector>;

struct Dashboard {
    settings: DashboardSettings,
    client: Client<Https, Full<Bytes>>,
}

impl Dashboard {
    fn new(settings: DashboardSettings) -> Self {
        // aws-lc-rs by name, as content's client does: another provider compiled in leaves rustls no default
        let builder = hyper_rustls::HttpsConnectorBuilder::new();
        let builder = match builder.with_provider_and_webpki_roots(rustls::crypto::aws_lc_rs::default_provider()) {
            Ok(builder) => builder,
            Err(_) => hyper_rustls::HttpsConnectorBuilder::new().with_webpki_roots(),
        };
        let connector = builder.https_or_http().enable_http1().build();
        Self {
            settings,
            client: Client::builder(TokioExecutor::new()).build(connector),
        }
    }

    async fn call(
        &self,
        method: &str,
        path: &str,
        body: Option<Vec<u8>>,
        etag: Option<&str>,
    ) -> Result<(u16, Option<String>, Bytes), String> {
        let url = format!("{}{path}", self.settings.url.trim_end_matches('/'));
        let mut request = Request::builder()
            .method(method)
            .uri(url)
            .header("authorization", format!("Bearer {}", self.settings.key));
        if body.is_some() {
            request = request.header("content-type", "application/json");
        }
        if let Some(etag) = etag {
            request = request.header("if-none-match", etag);
        }
        let request = request
            .body(Full::new(Bytes::from(body.unwrap_or_default())))
            .map_err(|e| e.to_string())?;
        let work = async {
            let response = self.client.request(request).await.map_err(|e| e.to_string())?;
            let status = response.status().as_u16();
            let etag = response
                .headers()
                .get("etag")
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned);
            let body = Limited::new(response.into_body(), 4 << 20)
                .collect()
                .await
                .map_err(|e| e.to_string())?
                .to_bytes();
            Ok((status, etag, body))
        };
        tokio::time::timeout(TIMEOUT, work)
            .await
            .map_err(|_| "timed out".to_owned())?
    }
}

/// The last good filter stays while the dashboard is unreachable.
async fn poll_filter(dashboard: Arc<Dashboard>, filter: Arc<RwLock<Arc<Filter>>>) {
    let mut etag: Option<String> = None;
    let mut failing = false;
    loop {
        match dashboard
            .call("GET", "/api/ingest/chat-filter", None, etag.as_deref())
            .await
        {
            Ok((304, ..)) => failing = false,
            Ok((200, tag, body)) => match serde_json::from_slice::<FilterDocument>(&body) {
                Ok(doc) => {
                    let compiled = Filter::compile(Some(doc.version), &doc.terms);
                    let skipped = doc.terms.len() - compiled.terms.len();
                    tracing::info!(
                        "chat: filter version {} ({} terms{})",
                        doc.version,
                        compiled.terms.len(),
                        if skipped > 0 {
                            format!(", {skipped} skipped")
                        } else {
                            String::new()
                        }
                    );
                    *filter.write().unwrap_or_else(PoisonError::into_inner) = Arc::new(compiled);
                    etag = tag;
                    failing = false;
                }
                Err(error) => tracing::warn!("chat: the dashboard's filter did not parse ({error})"),
            },
            Ok((status, ..)) => {
                if !failing {
                    tracing::warn!("chat: the dashboard answered {status} for the filter");
                }
                failing = true;
            }
            Err(error) => {
                if !failing {
                    tracing::warn!("chat: the dashboard is unreachable for the filter ({error})");
                }
                failing = true;
            }
        }
        tokio::time::sleep(POLL).await;
    }
}

/// Sends lines in batches of up to 500 and reports one by one, retrying with the same ids (the dashboard
/// skips ids it has) until they go.
async fn ship(dashboard: Arc<Dashboard>, mut queue: mpsc::Receiver<Outgoing>) {
    let mut lines: Vec<Line> = Vec::new();
    let mut reports: Vec<Report> = Vec::new();
    let mut backoff = Duration::from_secs(1);
    loop {
        if lines.is_empty() && reports.is_empty() {
            match queue.recv().await {
                Some(item) => take(item, &mut lines, &mut reports),
                None => return,
            }
        }
        // Let a burst gather into one batch
        tokio::time::sleep(Duration::from_millis(250)).await;
        while lines.len() < QUEUE
            && let Ok(item) = queue.try_recv()
        {
            take(item, &mut lines, &mut reports);
        }
        let mut ok = true;
        while let Some(report) = reports.first() {
            match post(&dashboard, "/api/ingest/reports", report).await {
                Ok(()) => {
                    reports.remove(0);
                }
                Err(error) => {
                    tracing::warn!("chat: sending a report to the dashboard failed ({error})");
                    ok = false;
                    break;
                }
            }
        }
        while ok && !lines.is_empty() {
            let n = lines.len().min(BATCH);
            #[derive(Serialize)]
            struct Batch<'a> {
                messages: &'a [Line],
            }
            match post(&dashboard, "/api/ingest/chat", &Batch { messages: &lines[..n] }).await {
                Ok(()) => {
                    lines.drain(..n);
                }
                Err(error) => {
                    tracing::warn!("chat: sending {n} lines to the dashboard failed ({error})");
                    ok = false;
                }
            }
        }
        if ok {
            backoff = Duration::from_secs(1);
        } else {
            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(Duration::from_secs(60));
        }
    }
}

fn take(item: Outgoing, lines: &mut Vec<Line>, reports: &mut Vec<Report>) {
    match item {
        Outgoing::Line(line) => lines.push(line),
        Outgoing::Report(report) => reports.push(report),
    }
}

async fn post(dashboard: &Dashboard, path: &str, body: &impl Serialize) -> Result<(), String> {
    let body = serde_json::to_vec(body).map_err(|e| e.to_string())?;
    let answer = dashboard.call("POST", path, Some(body), None).await;
    let result = match &answer {
        Ok((200..=299, ..)) => "ok",
        Ok((400 | 413 | 422, ..)) => "refused",
        _ => "failed",
    };
    service_kit::metrics::count(
        "dainty_chat_dashboard_posts_total",
        "Posts to the staff dashboard, by path and result",
        &[("path", path), ("result", result)],
        1,
    );
    match answer? {
        (200..=299, ..) => Ok(()),
        // A malformed item is refused for good; retrying it would hold up everything behind it
        (400 | 413 | 422, ..) => {
            tracing::warn!("chat: the dashboard refused what was sent to {path}");
            Ok(())
        }
        (status, ..) => Err(format!("status {status}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn term(id: i64, pattern: &str, match_type: MatchType, action: Action) -> Term {
        Term {
            id,
            pattern: pattern.to_owned(),
            match_type,
            action,
            channels: None,
        }
    }

    fn filter(terms: &[Term]) -> Filter {
        Filter::compile(Some(1), terms)
    }

    #[test]
    fn words_catch_look_alikes_and_spacing_but_not_longer_words() {
        let f = filter(&[term(1, "idiot", MatchType::Word, Action::Flag)]);
        for caught in ["idiot", "you IDIOT!", "id1ot", "i.d.i.o.t", "i d i 0 t", "1d10t"] {
            assert_eq!(f.rate(caught, Channel::City).verdict, Verdict::Flagged, "{caught}");
        }
        for missed in ["idiots", "anidiot", "idiot2", "hello"] {
            assert_eq!(f.rate(missed, Channel::City).verdict, Verdict::Allowed, "{missed}");
        }
    }

    #[test]
    fn a_word_after_a_rejected_match_is_still_found() {
        let f = filter(&[term(1, "ass", MatchType::Word, Action::Mask)]);
        assert_eq!(f.rate("class ass", Channel::City).shown, "class ***");
        assert_eq!(f.rate("élan ass", Channel::City).shown, "élan ***");
    }

    #[test]
    fn substrings_match_anywhere() {
        let f = filter(&[term(1, "dumb", MatchType::Substring, Action::Flag)]);
        assert_eq!(f.rate("dumbest", Channel::City).verdict, Verdict::Flagged);
    }

    #[test]
    fn masking_keeps_spaces_and_flags_unmasked() {
        let f = filter(&[
            term(1, "idiot", MatchType::Word, Action::Mask),
            term(2, "noob", MatchType::Word, Action::Flag),
        ]);
        let rated = f.rate("noob i d i o t", Channel::City);
        assert_eq!(rated.verdict, Verdict::Masked);
        assert_eq!(rated.shown, "noob * * * * *");
    }

    #[test]
    fn the_worst_action_wins() {
        let f = filter(&[
            term(1, "bad", MatchType::Word, Action::Flag),
            term(2, "worse", MatchType::Word, Action::Block),
        ]);
        assert_eq!(f.rate("bad and worse", Channel::City).verdict, Verdict::Blocked);
    }

    #[test]
    fn regex_terms_and_bad_ones() {
        let f = filter(&[
            term(1, r"buy\s+gold", MatchType::Regex, Action::Block),
            term(2, r"(?<=x)y", MatchType::Regex, Action::Block),
            term(3, "  ", MatchType::Word, Action::Block),
        ]);
        assert_eq!(f.terms.len(), 1);
        assert_eq!(f.rate("BUY   GOLD here", Channel::City).verdict, Verdict::Blocked);
    }

    #[test]
    fn channels_scope_a_term() {
        let mut t = term(1, "spoiler", MatchType::Word, Action::Mask);
        t.channels = Some(vec!["city".to_owned()]);
        let f = filter(&[t]);
        assert_eq!(f.rate("spoiler", Channel::City).verdict, Verdict::Masked);
        assert_eq!(f.rate("spoiler", Channel::Party).verdict, Verdict::Allowed);
    }

    #[test]
    fn the_dashboards_filter_document_parses() {
        let doc: FilterDocument = serde_json::from_str(
            r#"{"version":12,"terms":[{"id":3,"pattern":"idiot","matchType":"word","action":"mask","channels":null}]}"#,
        )
        .unwrap();
        assert_eq!(doc.version, 12);
        assert_eq!(Filter::compile(Some(12), &doc.terms).terms.len(), 1);
    }

    #[test]
    fn report_commands() {
        assert_eq!(parse_report("hello"), None);
        assert_eq!(parse_report("/reports x"), None);
        assert_eq!(parse_report("/report"), Some(Err(())));
        assert_eq!(
            parse_report("/report Grevel Harassment kept following me"),
            Some(Ok(("Grevel".to_owned(), "harassment", "kept following me".to_owned())))
        );
        assert_eq!(
            parse_report("/report Grevel kept following me"),
            Some(Ok(("Grevel".to_owned(), "other", "kept following me".to_owned())))
        );
    }

    #[test]
    fn lines_serialise_as_the_dashboard_reads_them() {
        let line = Line {
            id: "rm-1".to_owned(),
            at: 1,
            channel: Channel::City.as_str(),
            room_id: Some("city".to_owned()),
            sender: Person {
                user_id: "u".to_owned(),
                name: "Moonwick".to_owned(),
            },
            recipient: None,
            body: "hi".to_owned(),
            verdict: Verdict::Masked,
        };
        assert_eq!(
            serde_json::to_string(&line).unwrap(),
            r#"{"id":"rm-1","at":1,"channel":"city","roomId":"city","sender":{"userId":"u","name":"Moonwick"},"recipient":null,"body":"hi","verdict":"masked"}"#
        );
    }

    #[test]
    fn settings_need_a_url_and_a_long_key() {
        let good = DashboardSettings {
            url: "https://dash.example".to_owned(),
            key: "k".repeat(32),
        };
        assert!(good.problems().is_empty());
        assert!(!format!("{good:?}").contains("kkkk"));
        let bad = DashboardSettings {
            url: "dash.example".to_owned(),
            key: "short".to_owned(),
        };
        assert_eq!(bad.problems().len(), 2);
    }
}
