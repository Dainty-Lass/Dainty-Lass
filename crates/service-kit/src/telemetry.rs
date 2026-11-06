// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Unified `tracing` + `tracing-subscriber` logging pipeline (`docs/app-structure-plan.md` §4):
//! - [`RedactingVisitor`] and [`redact_secrets`] scrub `RRK_` keys, operator secrets, and auth tokens.
//! - [`TableStdoutLayer`] writes Docker-like lines (`{service:<14} | {time} {LEVEL} {target}: {line}`, colored, or
//!   JSON; [`crate::console`]) atomically to `stdout` (or a custom writer).
//! - [`GelfLayer`] ships every record to Graylog in GELF ([`crate::gelf`]).
//! - [`ChannelLogLayer`] (`LauncherGuiLayer`) streams structured [`HostLogEntry`] records to the GUI.
//! - [`TuiRingBufferLayer`] buffers structured [`HostLogEntry`] records for the console `--tui` pane.
//! - [`FileLogLayer`] appends redacted timestamped lines to a log file.
//! - [`LogFilterReloadHandle`] adjusts per-service log levels at runtime via `reload::Layer<Targets>`.

use std::collections::VecDeque;
use std::fmt;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use regex::Regex;
use serde::{Deserialize, Serialize};
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Record as SpanRecord};
use tracing::{Event, Id, Level, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::filter::{LevelFilter, Targets};
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::reload;
use tracing_subscriber::util::SubscriberInitExt;

use crate::console::{ColorMode, ConsoleStyle, LogFormat, Record, render};
use crate::gelf::GelfSink;

static TRACING_BRIDGE_ACTIVE: AtomicBool = AtomicBool::new(false);
static NEXT_LOG_SEQ: AtomicU64 = AtomicU64::new(1);

/// Whether the unified `tracing` subscriber is active for `linelog` bridging.
pub fn tracing_bridge_active() -> bool {
    TRACING_BRIDGE_ACTIVE.load(Ordering::Relaxed)
}

/// Enables or disables forwarding of `service_kit::linelog` events into `tracing`.
pub fn set_tracing_bridge_active(active: bool) {
    TRACING_BRIDGE_ACTIVE.store(active, Ordering::Relaxed);
}

fn redact_patterns() -> &'static [(Regex, &'static str)] {
    static PATTERNS: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        let compile = |pat: &str| match Regex::new(pat) {
            Ok(re) => re,
            Err(_) => unreachable!(),
        };
        vec![
            (compile(r"RRK_[A-Za-z0-9_-]*"), "[REDACTED]"),
            (compile(r"(?i)(-AUTH_PASSWORD=)\S+"), "${1}[REDACTED]"),
            (
                compile(
                    r#"(?i)((?:x-reforged-user-api-key|x-paradox-operator-key|x-paradox-operator-signature|x-allowlist-key|authorization)\s*[:=]\s*"?)(?:Bearer\s+)?[^\s",}]+"#,
                ),
                "${1}[REDACTED]",
            ),
            (
                compile(r"(?i)([?&](?:code|token|key|secret|password)=)[^&\s#]+"),
                "${1}[REDACTED]",
            ),
        ]
    })
}

/// Scrubs `RRK_` account keys, sensitive headers, and query/CLI secrets from a log string.
pub fn redact_secrets(input: &str) -> String {
    let mut out = input.to_owned();
    for (re, replacement) in redact_patterns() {
        if re.is_match(&out) {
            out = re.replace_all(&out, *replacement).into_owned();
        }
    }
    out
}

/// Structured log entry delivered to the GUI Status Page (`LauncherGuiLayer`) or `--tui` ring buffer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostLogEntry {
    /// Order of arrival, rising across the process.
    pub seq: u64,
    /// When it was logged, in Unix milliseconds.
    pub timestamp_ms: i64,
    /// Service the record belongs to.
    pub service: String,
    /// Level name.
    pub level: String,
    /// The message with its fields, redacted.
    pub message: String,
}

#[derive(Debug, Clone)]
struct SpanServiceName(String);

/// Extracts the `service` attribute from `component` spans.
#[derive(Default)]
struct ServiceFieldVisitor {
    service: Option<String>,
}

impl Visit for ServiceFieldVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "service" {
            self.service = Some(value.to_owned());
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if field.name() == "service" {
            self.service = Some(format!("{value:?}").trim_matches('"').to_owned());
        }
    }
}

/// Extracts and redacts the message and structured fields of a `tracing::Event`.
#[derive(Default)]
pub struct RedactingVisitor {
    /// The message, redacted.
    pub message: Option<String>,
    /// A `service` field, which names the record's service instead of its span's.
    pub service_override: Option<String>,
    /// Other fields, redacted, sensitive keys replaced whole.
    pub fields: Vec<(String, String)>,
}

impl RedactingVisitor {
    fn is_sensitive_key(name: &str) -> bool {
        let lower = name.to_ascii_lowercase();
        lower.contains("secret")
            || lower.contains("password")
            || lower.contains("token")
            || lower == "key"
            || lower.ends_with("_key")
            || lower == "authorization"
    }

    fn push_field(&mut self, name: &str, raw_value: &str) {
        if name == "message" {
            self.message = Some(redact_secrets(raw_value));
            return;
        }
        if name == "service" {
            self.service_override = Some(raw_value.to_owned());
            return;
        }
        let redacted = if Self::is_sensitive_key(name) {
            "[REDACTED]".to_owned()
        } else {
            redact_secrets(raw_value)
        };
        self.fields.push((name.to_owned(), redacted));
    }

    /// The message, then the fields as `name=value`.
    pub fn format_line(&self) -> String {
        let mut line = self.message.clone().unwrap_or_default();
        for (k, v) in &self.fields {
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(k);
            line.push('=');
            line.push_str(v);
        }
        line
    }
}

impl Visit for RedactingVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.push_field(field.name(), value);
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        let formatted = format!("{value:?}");
        let trimmed = formatted
            .strip_prefix('"')
            .and_then(|s| s.strip_suffix('"'))
            .unwrap_or(&formatted);
        self.push_field(field.name(), trimmed);
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.push_field(field.name(), &value.to_string());
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.push_field(field.name(), &value.to_string());
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.push_field(field.name(), if value { "true" } else { "false" });
    }
}

fn resolve_service<S>(event: &Event<'_>, ctx: &Context<'_, S>, visitor: &RedactingVisitor) -> String
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    if let Some(svc) = &visitor.service_override {
        return svc.clone();
    }
    if let Some(scope) = ctx.event_scope(event) {
        for span in scope.from_root() {
            if let Some(svc) = span.extensions().get::<SpanServiceName>() {
                return svc.0.clone();
            }
        }
    }
    let target = event.metadata().target();
    target
        .split("::")
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or("server")
        .replace('_', "-")
}

fn record_span_service<S>(attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>)
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    let mut visitor = ServiceFieldVisitor::default();
    attrs.record(&mut visitor);
    if let Some(service) = visitor.service
        && let Some(span) = ctx.span(id)
    {
        span.extensions_mut().replace(SpanServiceName(service));
    }
}

fn update_span_service<S>(id: &Id, values: &SpanRecord<'_>, ctx: Context<'_, S>)
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    let mut visitor = ServiceFieldVisitor::default();
    values.record(&mut visitor);
    if let Some(service) = visitor.service
        && let Some(span) = ctx.span(id)
    {
        span.extensions_mut().insert(SpanServiceName(service));
    }
}

/// The writer the console layer takes a line's bytes to.
pub type MakeWriter = Box<dyn Fn() -> Box<dyn Write> + Send + Sync>;

/// Writes each record as [`crate::console::render`] lays it out, every line of it in one write.
pub struct TableStdoutLayer<W = MakeWriter> {
    make_writer: W,
    style: ConsoleStyle,
}

impl TableStdoutLayer<MakeWriter> {
    /// `stdout` in plain text, colored when [`ColorMode::Auto`] says so.
    pub fn stdout() -> Self {
        Self::stdout_styled(LogFormat::Text, ColorMode::Auto)
    }

    /// `stdout` in `format`; color only in text. Colors go through `anstream`, which turns them on in a legacy
    /// Windows console.
    pub fn stdout_styled(format: LogFormat, color: ColorMode) -> Self {
        use std::io::IsTerminal;
        let color =
            format == LogFormat::Text && color.resolve(std::io::stdout().is_terminal(), |n| std::env::var(n).ok());
        let make: MakeWriter = if color {
            Box::new(|| Box::new(anstream::AutoStream::always(std::io::stdout())))
        } else {
            Box::new(|| Box::new(std::io::stdout()))
        };
        Self {
            make_writer: make,
            style: ConsoleStyle { format, color },
        }
    }
}

impl<W> TableStdoutLayer<W> {
    /// Plain text to `make_writer`.
    pub fn new(make_writer: W) -> Self {
        Self::styled(make_writer, ConsoleStyle::default())
    }

    /// Writes to `make_writer` in `style`.
    pub fn styled(make_writer: W, style: ConsoleStyle) -> Self {
        Self { make_writer, style }
    }
}

/// The record a layer writes: redacted, with its service and the module that logged it.
struct Taken {
    service: String,
    level: &'static str,
    target: String,
    visitor: RedactingVisitor,
}

impl Taken {
    fn of<S>(event: &Event<'_>, ctx: &Context<'_, S>) -> Self
    where
        S: Subscriber + for<'a> LookupSpan<'a>,
    {
        let mut visitor = RedactingVisitor::default();
        event.record(&mut visitor);
        let service = resolve_service(event, ctx, &visitor);
        Self {
            service,
            level: level_str(event.metadata().level()),
            target: event.metadata().target().to_owned(),
            visitor,
        }
    }

    fn record(&self, timestamp_ms: i64) -> Record<'_> {
        Record {
            timestamp_ms,
            level: self.level,
            service: &self.service,
            target: &self.target,
            message: self.visitor.message.as_deref().unwrap_or(""),
            fields: &self.visitor.fields,
        }
    }

    /// Every rendered line, each ended.
    fn text(&self, style: ConsoleStyle) -> String {
        let mut text = String::new();
        for line in render(&self.record(chrono::Utc::now().timestamp_millis()), style) {
            text.push_str(&line);
            text.push('\n');
        }
        text
    }
}

impl<S, W, O> Layer<S> for TableStdoutLayer<W>
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    W: Fn() -> O + 'static,
    O: Write,
{
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        record_span_service(attrs, id, ctx);
    }

    fn on_record(&self, id: &Id, values: &SpanRecord<'_>, ctx: Context<'_, S>) {
        update_span_service(id, values, ctx);
    }

    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        let text = Taken::of(event, &ctx).text(self.style);
        let mut out = (self.make_writer)();
        let _ = out.write_all(text.as_bytes());
    }
}

/// Ships every record to a GELF collector (Graylog), through [`GelfSink`]'s queue.
#[derive(Clone)]
pub struct GelfLayer {
    sink: GelfSink,
}

impl GelfLayer {
    /// Ships to `sink`.
    pub fn new(sink: GelfSink) -> Self {
        Self { sink }
    }
}

impl<S> Layer<S> for GelfLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        record_span_service(attrs, id, ctx);
    }

    fn on_record(&self, id: &Id, values: &SpanRecord<'_>, ctx: Context<'_, S>) {
        update_span_service(id, values, ctx);
    }

    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        let taken = Taken::of(event, &ctx);
        self.sink.send(&taken.record(chrono::Utc::now().timestamp_millis()));
    }
}

/// Streams structured [`HostLogEntry`] items over an unbounded channel (`LauncherGuiLayer`).
#[derive(Clone)]
pub struct ChannelLogLayer {
    tx: tokio::sync::mpsc::UnboundedSender<HostLogEntry>,
}

/// The layer the launcher's GUI reads logs through.
pub type LauncherGuiLayer = ChannelLogLayer;

impl ChannelLogLayer {
    /// Sends every record to `tx`.
    pub fn new(tx: tokio::sync::mpsc::UnboundedSender<HostLogEntry>) -> Self {
        Self { tx }
    }
}

fn level_str(level: &Level) -> &'static str {
    match *level {
        Level::TRACE => "trace",
        Level::DEBUG => "debug",
        Level::INFO => "info",
        Level::WARN => "warn",
        Level::ERROR => "error",
    }
}

impl<S> Layer<S> for ChannelLogLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        record_span_service(attrs, id, ctx);
    }

    fn on_record(&self, id: &Id, values: &SpanRecord<'_>, ctx: Context<'_, S>) {
        update_span_service(id, values, ctx);
    }

    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        let mut visitor = RedactingVisitor::default();
        event.record(&mut visitor);
        let service = resolve_service(event, &ctx, &visitor);
        let entry = HostLogEntry {
            seq: NEXT_LOG_SEQ.fetch_add(1, Ordering::Relaxed),
            timestamp_ms: chrono::Utc::now().timestamp_millis(),
            service,
            level: level_str(event.metadata().level()).to_owned(),
            message: visitor.format_line(),
        };
        let _ = self.tx.send(entry);
    }
}

/// Bounded ring buffer of [`HostLogEntry`] records for `server-console --tui`.
#[derive(Clone)]
pub struct TuiRingBufferLayer {
    entries: Arc<Mutex<VecDeque<HostLogEntry>>>,
    max_entries: usize,
}

impl TuiRingBufferLayer {
    /// Keeps the last `max_entries` records (at least one).
    pub fn new(max_entries: usize) -> Self {
        Self {
            entries: Arc::new(Mutex::new(VecDeque::with_capacity(max_entries))),
            max_entries: max_entries.max(1),
        }
    }

    /// The records kept, filtered by service and by level name.
    pub fn snapshot(&self, service_filter: Option<&str>, level_filter: Option<&str>) -> Vec<HostLogEntry> {
        let guard = self.entries.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        guard
            .iter()
            .filter(|e| service_filter.is_none_or(|s| e.service == s))
            .filter(|e| level_filter.is_none_or(|l| e.level.eq_ignore_ascii_case(l)))
            .cloned()
            .collect()
    }

    /// The records kept, filtered by service.
    pub fn entries(&self, service_filter: Option<&str>) -> Vec<HostLogEntry> {
        self.snapshot(service_filter, None)
    }
}

impl<S> Layer<S> for TuiRingBufferLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        record_span_service(attrs, id, ctx);
    }

    fn on_record(&self, id: &Id, values: &SpanRecord<'_>, ctx: Context<'_, S>) {
        update_span_service(id, values, ctx);
    }

    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        let mut visitor = RedactingVisitor::default();
        event.record(&mut visitor);
        let service = resolve_service(event, &ctx, &visitor);
        let entry = HostLogEntry {
            seq: NEXT_LOG_SEQ.fetch_add(1, Ordering::Relaxed),
            timestamp_ms: chrono::Utc::now().timestamp_millis(),
            service,
            level: level_str(event.metadata().level()).to_owned(),
            message: visitor.format_line(),
        };
        let mut guard = self.entries.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.push_back(entry);
        while guard.len() > self.max_entries {
            guard.pop_front();
        }
    }
}

/// Writes redacted timestamped log lines to a file (`logs/launcher.log` or `.workspace/logs/server.log`).
#[derive(Clone)]
pub struct FileLogLayer {
    file: Arc<Mutex<Option<std::fs::File>>>,
}

impl FileLogLayer {
    /// Appends to `path`, creating its folder; when it can't be opened, nothing is written.
    pub fn open(path: &std::path::Path) -> Self {
        let file = path.parent().and_then(|dir| {
            std::fs::create_dir_all(dir).ok()?;
            std::fs::OpenOptions::new().create(true).append(true).open(path).ok()
        });
        Self {
            file: Arc::new(Mutex::new(file)),
        }
    }
}

impl<S> Layer<S> for FileLogLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        record_span_service(attrs, id, ctx);
    }

    fn on_record(&self, id: &Id, values: &SpanRecord<'_>, ctx: Context<'_, S>) {
        update_span_service(id, values, ctx);
    }

    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        let mut guard = self.file.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(file) = guard.as_mut() else {
            return;
        };
        let _ = file.write_all(Taken::of(event, &ctx).text(ConsoleStyle::default()).as_bytes());
    }
}

type ReloadExporter = dyn Fn(&str, LevelFilter) -> Result<(), String> + Send + Sync;

/// Reads a level name: `trace`, `debug`, `info`, `warn` or `warning`, `error`, `off`.
pub fn parse_level_filter(raw: &str) -> Option<LevelFilter> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "trace" => Some(LevelFilter::TRACE),
        "debug" => Some(LevelFilter::DEBUG),
        "info" => Some(LevelFilter::INFO),
        "warn" | "warning" => Some(LevelFilter::WARN),
        "error" => Some(LevelFilter::ERROR),
        "off" => Some(LevelFilter::OFF),
        _ => None,
    }
}

/// Handle for dynamically reloading per-service log levels at runtime.
#[derive(Clone)]
pub struct LogFilterReloadHandle {
    default_level: LevelFilter,
    overrides: Arc<Mutex<std::collections::HashMap<String, LevelFilter>>>,
    apply: Arc<ReloadExporter>,
}

impl LogFilterReloadHandle {
    /// A handle that records levels but changes no filter.
    pub fn noop(default_level: LevelFilter) -> Self {
        Self {
            default_level,
            overrides: Arc::new(Mutex::new(std::collections::HashMap::new())),
            apply: Arc::new(|_, _| Ok(())),
        }
    }

    /// Sets `service`'s level and applies it to the running filter.
    pub fn set_service_level(&self, service: &str, level: LevelFilter) -> Result<(), String> {
        {
            let mut map = self.overrides.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            map.insert(service.to_owned(), level);
        }
        (self.apply)(service, level)
    }

    /// As [`LogFilterReloadHandle::set_service_level`], from a level name.
    pub fn set_service_level_str(&self, service: &str, level_str: &str) -> Result<(), String> {
        let level = parse_level_filter(level_str).ok_or_else(|| {
            format!("unknown log level `{level_str}` (expected trace, debug, info, warn, error, or off)")
        })?;
        self.set_service_level(service, level)
    }

    /// `service`'s level: its override, or the default.
    pub fn service_level(&self, service: &str) -> LevelFilter {
        self.overrides
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(service)
            .copied()
            .unwrap_or(self.default_level)
    }
}

/// The reload handle over `reload_handle`'s `Targets`: per-service levels, by service name and by crate.
fn reload_handle_for<S: 'static>(
    default_level: LevelFilter,
    reload_handle: reload::Handle<Targets, S>,
) -> LogFilterReloadHandle {
    let overrides: Arc<Mutex<std::collections::HashMap<String, LevelFilter>>> =
        Arc::new(Mutex::new(std::collections::HashMap::new()));
    let overrides_clone = overrides.clone();
    LogFilterReloadHandle {
        default_level,
        overrides,
        apply: Arc::new(move |_service, _level| {
            let map = overrides_clone
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone();
            reload_handle
                .modify(move |targets| {
                    let mut next = Targets::new().with_default(default_level);
                    for (svc, lvl) in &map {
                        let crate_target = svc.replace('-', "_");
                        next = next.with_target(svc.clone(), *lvl).with_target(crate_target, *lvl);
                    }
                    *targets = next;
                })
                .map_err(|e| e.to_string())
        }),
    }
}

/// How the console reads, and the GELF collector the log goes to besides (`[logs]`).
#[derive(Clone, Default)]
pub struct LogOutputs {
    /// Text or JSON.
    pub format: LogFormat,
    /// When to color.
    pub color: ColorMode,
    /// GELF collector to ship to as well, if any.
    pub gelf: Option<GelfSink>,
}

/// Installs the global `TableStdoutLayer` subscriber in plain text, colored on a terminal, with runtime reload support.
pub fn init_table_stdout_subscriber(default_level: LevelFilter) -> LogFilterReloadHandle {
    init_console_subscriber(default_level, LogOutputs::default())
}

/// Installs the global `TableStdoutLayer` subscriber as `outputs` says, and its GELF shipping.
pub fn init_console_subscriber(default_level: LevelFilter, outputs: LogOutputs) -> LogFilterReloadHandle {
    let targets = Targets::new().with_default(default_level);
    let (filter_layer, reload_handle) = reload::Layer::new(targets);
    let _ = tracing_log::LogTracer::init();
    let installed = tracing_subscriber::registry()
        .with(filter_layer)
        .with(TableStdoutLayer::stdout_styled(outputs.format, outputs.color))
        .with(outputs.gelf.map(GelfLayer::new))
        .try_init();
    if installed.is_ok() {
        set_tracing_bridge_active(true);
    }
    reload_handle_for(default_level, reload_handle)
}

/// Installs the global `TuiRingBufferLayer` subscriber for `server-console --tui`.
pub fn init_tui_subscriber(
    default_level: LevelFilter,
    max_entries: usize,
    log_file: Option<PathBuf>,
) -> (TuiRingBufferLayer, LogFilterReloadHandle) {
    init_tui_subscriber_with(default_level, max_entries, log_file, None)
}

/// The same, shipping to a GELF collector too.
pub fn init_tui_subscriber_with(
    default_level: LevelFilter,
    max_entries: usize,
    log_file: Option<PathBuf>,
    gelf: Option<GelfSink>,
) -> (TuiRingBufferLayer, LogFilterReloadHandle) {
    let targets = Targets::new().with_default(default_level);
    let (filter_layer, reload_handle) = reload::Layer::new(targets);
    let ring = TuiRingBufferLayer::new(max_entries);
    let file_layer = log_file.as_deref().map(FileLogLayer::open);
    let _ = tracing_log::LogTracer::init();
    let installed = tracing_subscriber::registry()
        .with(filter_layer)
        .with(ring.clone())
        .with(file_layer)
        .with(gelf.map(GelfLayer::new))
        .try_init();
    if installed.is_ok() {
        set_tracing_bridge_active(true);
    }
    (ring, reload_handle_for(default_level, reload_handle))
}

/// Installs the global `ChannelLogLayer` (`LauncherGuiLayer`) subscriber for `server-gui*`.
pub fn init_gui_channel_subscriber(
    default_level: LevelFilter,
) -> (
    ChannelLogLayer,
    tokio::sync::mpsc::UnboundedReceiver<HostLogEntry>,
    LogFilterReloadHandle,
) {
    let targets = Targets::new().with_default(default_level);
    let (filter_layer, reload_handle) = reload::Layer::new(targets);
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let layer = ChannelLogLayer::new(tx);
    let _ = tracing_log::LogTracer::init();
    let installed = tracing_subscriber::registry()
        .with(filter_layer)
        .with(layer.clone())
        .try_init();
    if installed.is_ok() {
        set_tracing_bridge_active(true);
    }
    let handle = reload_handle_for(default_level, reload_handle);
    (layer, rx, handle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Default)]
    struct TestBuf(Arc<Mutex<Vec<u8>>>);

    impl Write for TestBuf {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn table_stdout_layer_formats_service_span_and_redacts_secrets() {
        let buf = TestBuf::default();
        let writer = buf.clone();
        let subscriber = tracing_subscriber::registry().with(TableStdoutLayer::new(move || writer.clone()));
        tracing::subscriber::with_default(subscriber, || {
            let span = tracing::info_span!("component", service = "gateway");
            let _enter = span.enter();
            tracing::info!(
                api_key = "RRK_0123456789abcdef0123456789abcdef0123456789abcdef",
                port = 443,
                "listening on public port with token RRK_secret123"
            );
        });
        let text = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
        let (prefix, rest) = text.split_once(" | ").unwrap();
        assert_eq!(prefix, "gateway       ");
        let (_time, rest) = rest.split_once(' ').unwrap();
        assert_eq!(
            rest,
            "INFO  service_kit::telemetry::tests: listening on public port with token [REDACTED] api_key=[REDACTED] \
             port=443\n"
        );
    }

    #[test]
    fn channel_and_tui_layers_capture_structured_entries() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let ring = TuiRingBufferLayer::new(10);
        let subscriber = tracing_subscriber::registry()
            .with(ChannelLogLayer::new(tx))
            .with(ring.clone());
        tracing::subscriber::with_default(subscriber, || {
            let span = tracing::info_span!("component", service = "backend:reforged");
            let _enter = span.enter();
            tracing::warn!("player session started -AUTH_PASSWORD=RRK_abc");
        });
        let entry = rx.try_recv().unwrap();
        assert_eq!(entry.service, "backend:reforged");
        assert_eq!(entry.level, "warn");
        assert_eq!(entry.message, "player session started -AUTH_PASSWORD=[REDACTED]");
        let buffered = ring.snapshot(Some("backend:reforged"), Some("warn"));
        assert_eq!(buffered.len(), 1);
        assert_eq!(buffered[0].message, entry.message);
    }
}
