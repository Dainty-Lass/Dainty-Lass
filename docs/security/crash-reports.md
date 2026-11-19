<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Security checklist: crash reports and launcher logs (RX3)

The checklist for players' crash reports: the launcher side (`crates/dainty-launcher-kit`, `platform/crash.rs`,
`controller/crash.rs`, the session logs in `platform/log.rs`), the upload route on the content server
(`crates/dainty-content`, `crash.rs` and `app.rs`), and the copy the staff dashboard keeps (`ui/apps/web`,
`api/ingest/crash-reports`). `SECURITY_REVIEW.md` indexes it.

How to use it: before the feature reaches players, copy this list into the release notes' review, check each item
in the code (not from memory), link the code or test that shows it, and sign at the end with the date and the commit
reviewed. An item that does not hold is a bug to fix first, not a box to leave open.

## What it is and where the boundary is

When the game exits, the launcher looks for a crash folder the game wrote since it started it (`Archon/Saved/Crashes`
beside the game, and the `Saved/Crashes` beside its config). Each one is acted on once, per the player's choice in
Preferences: never (the default), ask, or always. A report is the crash folder's text files and minidump plus the end
of this session's launcher log, packed into one gzip bundle and sent to the server the crash was played on.

- **In, on the content server:** `POST /content/v1/crash-reports` from anyone who reaches the gateway, with an
  account key header and a body the player's PC wrote. Everything in it is untrusted: names, sizes, header values,
  file contents. It is the only content route that reads a request body.
- **Out, from the launcher:** the bundle, with the account key, to the joined server's endpoint only (the pinned
  certificate in public mode, a Tailscale or loopback host in private mode), the same rule as every keyed request.
- **Out, from the content server:** a JSON copy to the staff dashboard's ingest API (`CONTENT_DASHBOARD_URL`) with
  the ingest key (`CONTENT_DASHBOARD_INGEST_KEY`), when the host set `dashboard_url`.
- **Files:** the launcher's `<userData>/logs` and `crash-reports.json` (the ids already offered); the server's
  `.workspace/crash-reports/<id>/` (`CONTENT_CRASH_DIR`), one folder per report with `meta.json` and the files.

## Checklist

### Off unless the player chooses

- [ ] `crashReports` is absent from `settings.json` unless the player picked ask or always; absent reads as never,
      and never sends nothing and asks nothing (`platform/settings.rs`, `controller/stored.rs`, `crash::decide`).
- [ ] A crash is acted on once: its folder name goes into `crash-reports.json` the first time it is seen, before
      anything is asked or sent, so neither a restart nor a second exit offers it again (`Ledger`, tests
      `each_crash_is_acted_on_once`, `the_ledger_keeps_the_newest_ids`).
- [ ] Only folders written since this launcher started the game count (5 s slack), so an old crash folder is never
      offered (`new_crashes`, `only_new_crash_folders_count`). A Host page dump's exit is never looked at.

### The account key and other secrets never leave in a report

- [ ] Every text file goes through the launcher log's redaction (`DLK_`/`RRK_` keys, `-AUTH_PASSWORD=`, the user API
      key header, every registered secret) and then the key itself is replaced (`redact_text`). UE writes the
      command line, which holds the key, into its log and `CrashContext.runtime-xml`.
- [ ] The minidump keeps its layout: the key's UTF-8 and UTF-16 LE bytes are overwritten with `*` (`scrub`). Other
      memory in it is the game's own; the dump goes only to the server the key was made for, which already holds
      that key.
- [ ] Test: `the_key_never_leaves_in_a_bundle` (UTF-8 and UTF-16 in the dump, the key in a log line and in the
      `-AUTH_PASSWORD=` argument, the API key header in the launcher log).
- [ ] The report goes to the server the crash was played on; when the player joined another one since, it is not
      sent (`upload_crash`). The key header is sent only where `key_headers` allows it.

### The launcher's logs

- [ ] Every line is redacted as it is written (`log::write`); the session log in a report was redacted when written
      and again when packed.
- [ ] One log per launcher session: at start the last session's `launcher.log` (and its overflow) is kept as
      `launcher-<UTC time>-*.log`, and only the newest 10 sessions are kept (`start_session`, `rotate_sessions`,
      test `sessions_rotate_and_keep_the_newest`). Within a session the 2 MiB rotation still applies.
- [ ] Preferences' Open logs folder opens `<userData>/logs` only (`open_logs_folder`).

### The upload route

- [ ] Off unless `CONTENT_CRASH_DIR` is set: without it the path is an unknown route (404). The host kit always sets
      it to `.workspace/crash-reports` (`hostconfig.rs`).
- [ ] Checked in the order that reads the least first: method (405, `POST` only), declared length over 128 KiB
      (413, body not read), content type `application/vnd.dainty.crash+gzip` (415), the account key against the
      backend (401, 503 while it can't answer), the rate limit (429 with `retry-after`), then the body read through a
      128 KiB limit (413), then the parser (400 `bad_report`) (`app.rs`, test
      `crash_reports_are_keyed_capped_and_rate_limited`).
- [ ] The gateway's own `GATEWAY_MAX_BODY_BYTES` (128 KiB default) caps the body before it reaches content; the
      launcher packs reports under 120 KiB, cutting text from the start and leaving the dump out until it fits
      (`a_large_crash_is_cut_to_fit`).
- [ ] Rate limit: 3 uploads per account and 60 in all per sliding hour, counted once the key checks out, refused
      or not, so a malformed body still costs an attempt (`RateLimit`, `the_rate_limit_slides`).
- [ ] Unpacking stops at 8 MiB: a small upload can't unpack into a large one
      (`a_small_upload_cannot_unpack_past_the_cap`).
- [ ] The parser accepts nothing it doesn't know: the `DLCR1` line; a header line under 2 KiB holding only
      `crashId` (required), `gameVersion`, `launcherVersion`, `exitCode`, each a plain string of at most 128 bytes;
      at most 16 files, each a plain name of at most 80 bytes (letters, digits, `-`, `_`, `.`, not starting with a
      dot) ending `.log`, `.xml`, `.runtime-xml`, `.txt`, `.json` or `.dmp`, names unique ignoring case; frames that
      end exactly at the end (`crash::parse_frame`, `bad_bundles_are_refused`, fuzz target `content_crash_report`).
- [ ] The report's folder name is the server's own (`<UTC time>-<8 hex>`), never anything the player sent; file
      names are written inside it only after the parser accepted them.
- [ ] Disk: at most 200 reports and 256 MiB together; the oldest go first (`crash::prune`,
      `saved_reports_are_pruned_oldest_first`).
- [ ] The log line names the account and the saved id, never the key or any file's content.

### The dashboard copy

- [ ] Sent in the background after the reply; a failure is logged and dropped, the report stays on the server
      (`client::post_crash_report`). The ingest key is never logged and is hidden from `Config`'s `Debug`.
- [ ] Text files are copied up to their last 256 KiB; the minidump never leaves the server (its name, size and
      SHA-256 do).
- [ ] The dashboard parses the body strictly (`ui/apps/web/src/lib/crash-reports/parse.ts`), keeps the newest 500
      reports within 256 MB, and shows them only to staff with `reports:read`. A report's text is shown as text,
      never rendered as HTML.

## Sign-off

Reviewed at commit `________` on `________` by `________`.
