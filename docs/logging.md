<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Logging

`dainty-host start` and `serve` log every in-process service, and every process they supervise, through one
`tracing` subscriber (`crates/dainty-service-kit/src/telemetry.rs`). Every record is redacted first: account keys, secrets,
tokens and auth headers never reach the console, the file or a collector.

## The console

The text form keeps Docker Compose's prefix and adds a fixed header (`crates/dainty-service-kit/src/console.rs`):

```text
gateway        | 2026-11-06T12:00:00.123Z INFO  gateway::server: listening port=443 mode=public
isles-1-14-7   | 2026-11-06T12:00:01.004Z WARN  rustic_isles_director::gameservers: hub restarting attempt=2
backend        | 2026-11-06T12:00:01.210Z INFO  process: {"level":30,"msg":"…"}
```

| Part | What it is |
| --- | --- |
| `gateway` | The service or module: a `component` span's `service`, else the logging crate's name |
| `2026-11-06T12:00:00.123Z` | UTC, milliseconds, as `docker logs --timestamps` |
| `INFO` | `TRACE`, `DEBUG`, `INFO`, `WARN` or `ERROR`, padded to five |
| `gateway::server` | The part of the service that logged it: the Rust module path. `process` is a supervised program's own line |
| `listening` | The message; a message of several lines gives several lines, each with the whole header |
| `port=443` | The record's fields, logfmt: quoted and escaped when a value holds a space, quote, `=` or control character |

A parser splits on the first ` | `, then on spaces for the time and level, then on the first `: ` for the module:
`^(\S+)\s+\| (\S+) (\S+)\s+(\S+): (.*)$`.

With color, the prefix takes a color per service (the same one every run), the level its own (errors red), and the
time, module and field names are dimmed. Only those parts are wrapped, so stripping the escapes gives the plain line.

```toml
[logs]
format = "text"   # or "json": {"time","level","service","target","message","fields"} one object a line
color = "auto"    # "always", "never"
```

`auto` colors on a terminal unless `NO_COLOR` is set; `CLICOLOR_FORCE` or `FORCE_COLOR` turn it on anywhere, which is
what `docker compose up` without `-t` needs. On Windows the colors work in the legacy console too (`anstream`). `start`
under the launcher's control writes to a pipe, so it gets plain lines unless `always` is set. The `--tui` dashboard and
`.workspace/logs/server.log` keep plain text.

The machine lines `start` prints itself (`DAINTY_SERVER_READY`, `say`'s `- ...` lines) are unchanged.

## Shipping to Graylog (GELF)

Graylog is the open-source alternative to Splunk this ships to, in GELF 1.1 (`crates/dainty-service-kit/src/gelf.rs`).
Logstash's `gelf` input, Fluentd, Vector and Docker's own `gelf` log driver read the same messages, so the ELK or
OpenSearch stack works through any of them.

```toml
[logs]
gelf_url = "https://graylog.example.org:12201/gelf"   # Graylog's GELF HTTP input
# gelf_url = "tcp://graylog:12201"                     # GELF TCP, NUL-delimited
```

| Scheme | Transport | Allowed to |
| --- | --- | --- |
| `https://` | GELF HTTP, one message a POST on a kept connection | anywhere |
| `http://` | the same, plain text | this machine, a private or link-local address, or a name with no dot (a Docker service) |
| `tcp://host:port` | GELF TCP | the same as `http://` |

Each record becomes one message: `short_message` (its first line, 250 characters at most), `full_message` when there
is more, `level` in syslog's numbers (3 error, 4 warn, 6 info, 7 debug and trace), and the fields `_service`,
`_target` (the module), `_level_name`, `_server` (the server's name), `_node` (its `[network]` node) and one `_<name>`
for each of the record's fields. In Graylog, `_service:gateway AND level:<=4` is the gateway's warnings and errors.

Shipping never slows a service: records queue for one thread (10 000 at most; the rest are dropped and counted) and
are sent in order. When the collector is down, records are dropped until it is back, and `start` says so once on
stderr, never through the log itself.

## From the dashboard

A controller's page on the staff dashboard has a Logs card for the owner's key: format, color and the GELF URL. It
calls the admin API's `logSettings` and `setLogSettings` (`crates/dainty-admin/src/host_settings.rs`), which read and write
`host.toml`'s `[logs]` with the same checks as the file, so a plain-text URL to a public host is refused there too.
The log is set up once at start, so a change takes effect at the next start; until then the card says a restart is
needed. Each change is audited on the dashboard and can be undone from its activity log. A server started with
`--config` pointing at another file still has its `host.toml` edited.

## Not covered yet

- `dainty-host <service>` run alone (the per-service Docker containers) still logs pino-style JSON
  (`crates/dainty-service-kit/src/log.rs`), not this format, and ships nothing. Docker's own `gelf` log driver ships those.
- GELF over UDP, and an auth header for a collector behind a proxy.
