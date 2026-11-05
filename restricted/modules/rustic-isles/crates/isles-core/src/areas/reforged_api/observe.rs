//! The request log and request metrics `app.ts` installs before any router: `[REQ]`/`[RES]` lines with the URL
//! sanitised (`security/logRedaction.ts`), the `rustic_isles_http_*` series. `http::dispatch` calls
//! [`begin`] and [`Observation::finish`].

use std::sync::OnceLock;
use std::time::Instant;

use axum::extract::Request;
use regex::Regex;

use super::metrics;

fn patterns() -> &'static [Regex; 3] {
    static PATTERNS: OnceLock<[Regex; 3]> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        // The patterns are constants that compile; an empty fallback keeps a typo from taking requests down
        let make = |source: &str| Regex::new(source).unwrap_or_else(|_| Regex::new("$^").unwrap_or_else(|_| unreachable!()));
        [
            make(r"(?i)(/sessions/kill/)[^/?#]+"),
            make(r"eyJ[A-Za-z0-9_-]{5,}\.[A-Za-z0-9_-]{5,}\.[A-Za-z0-9_-]*"),
            make(
                r"(?i)([?&](?:token|access_token|refresh_token|code|exchange_code|authorization|api_key|apikey|signature)=)[^&#]*",
            ),
        ]
    })
}

/// `sanitizeUrlForLog`: bearer-like values in the path and query never reach the log.
pub fn sanitize_url_for_log(url: &str) -> String {
    let [kill, jwt, query] = patterns();
    let url = kill.replace_all(url, "${1}[REDACTED]");
    let url = jwt.replace_all(&url, "[REDACTED-JWT]");
    query.replace_all(&url, "${1}[REDACTED]").into_owned()
}

/// `app.ts`'s `metricClass`.
pub fn metric_class(path: &str) -> &'static str {
    if path.starts_with("/admin/") {
        "admin"
    } else if path.starts_with("/health/") {
        "health"
    } else if path.starts_with("/launcher/") {
        "launcher"
    } else {
        "game"
    }
}

/// The player-host lease and Guard evidence routes: logged only when refused.
fn high_frequency(path: &str) -> bool {
    let Some(rest) = path.strip_prefix("/mystic/v1/hunts/") else {
        return false;
    };
    matches!(rest.split_once('/'), Some((id, "lease" | "event-batches")) if !id.is_empty())
}

pub struct Observation {
    started: Instant,
    class: &'static str,
    method: String,
    request_id: String,
    url: String,
    request_line: String,
    quiet_when_ok: bool,
}

/// Counts the request and logs `[REQ]`; the `/__origin/<host>` prefix is read here only for the log.
pub fn begin(request: &Request, request_id: &str) -> Observation {
    let target = request.uri().path_and_query().map_or("/", |pq| pq.as_str());
    let (origin, target) = match target.strip_prefix("/__origin/") {
        Some(rest) => {
            let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
            let after = &rest[end..];
            let after = if after.is_empty() {
                "/".to_owned()
            } else {
                after.to_owned()
            };
            (Some(rest[..end].to_lowercase()), after)
        }
        None => (None, target.to_owned()),
    };
    let path = target.split(['?', '#']).next().unwrap_or("/");
    let class = metric_class(path);
    let method = request.method().as_str().to_owned();
    metrics::increment(
        "rustic_isles_http_requests_total",
        1.0,
        &[("class", class), ("method", &method)],
    );
    metrics::adjust("rustic_isles_http_inflight", 1.0, &[]);
    let headers = request.headers();
    let gs_key = if headers.contains_key("x-rustic-isles-gameserver-apikey") {
        "Y"
    } else {
        "N"
    };
    let bearer = if headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.to_lowercase().starts_with("bearer "))
    {
        "Y"
    } else {
        "N"
    };
    let host = headers.get("host").and_then(|v| v.to_str().ok()).unwrap_or("?");
    let url = sanitize_url_for_log(&target);
    let request_line = format!(
        "[REQ] requestId={request_id} {method} {host}{url}{} gsKey={gs_key} bearer={bearer}",
        origin.map_or_else(String::new, |o| format!(" origin={o}"))
    );
    let quiet_when_ok = high_frequency(path);
    if !quiet_when_ok {
        tracing::info!("{request_line}");
    }
    Observation {
        started: Instant::now(),
        class,
        method,
        request_id: request_id.to_owned(),
        url,
        request_line,
        quiet_when_ok,
    }
}

impl Observation {
    /// Counts the answer and logs `[RES]`.
    pub fn finish(self, status: u16) {
        let elapsed = self.started.elapsed().as_millis();
        metrics::adjust("rustic_isles_http_inflight", -1.0, &[]);
        let class_of_status = format!("{}xx", status / 100);
        metrics::increment(
            "rustic_isles_http_responses_total",
            1.0,
            &[("class", self.class), ("status", &class_of_status)],
        );
        metrics::observe(
            "rustic_isles_http_request_duration_ms",
            elapsed as f64,
            &[("class", self.class)],
        );
        if self.quiet_when_ok && status < 400 {
            return;
        }
        if self.quiet_when_ok {
            tracing::info!("{}", self.request_line);
        }
        tracing::info!(
            "[RES] requestId={} {} {} \u{2192} {status} ({elapsed}ms)",
            self.request_id,
            self.method,
            self.url
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // security/logRedaction.test.ts
    #[test]
    fn urls_are_logged_without_their_secrets() {
        assert_eq!(
            sanitize_url_for_log("/notification/id_currency_s19_coin?token=abc.def&x=1"),
            "/notification/id_currency_s19_coin?token=[REDACTED]&x=1"
        );
        assert_eq!(
            sanitize_url_for_log("/oauth?CODE=secret&state=ok&access_token=another"),
            "/oauth?CODE=[REDACTED]&state=ok&access_token=[REDACTED]"
        );
        assert_eq!(
            sanitize_url_for_log("/balance?currency=id_currency_s19_coin"),
            "/balance?currency=id_currency_s19_coin"
        );
        assert_eq!(
            sanitize_url_for_log("/account/api/oauth/sessions/kill/eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJ4In0.sig_value"),
            "/account/api/oauth/sessions/kill/[REDACTED]"
        );
        assert_eq!(
            sanitize_url_for_log("/x/eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJ4In0.sigvalue/y"),
            "/x/[REDACTED-JWT]/y"
        );
    }

    #[test]
    fn requests_are_classed_for_the_metrics() {
        assert_eq!(metric_class("/admin/v1/me"), "admin");
        assert_eq!(metric_class("/health/live"), "health");
        assert_eq!(metric_class("/launcher/v1/updates/x"), "launcher");
        assert_eq!(metric_class("/character"), "game");
        assert!(high_frequency("/mystic/v1/hunts/abc/lease") && !high_frequency("/mystic/v1/hunts/abc/end"));
    }
}
