# tokio-console

A diagnostic build of the server that serves [tokio-console](https://github.com/tokio-rs/console): every task's
busy and idle time, poll counts, wakes and the resources it waits on, live. Release builds leave it out.

## Building

The `tokio-console` feature needs tokio's unstable cfg; without it the build stops with a `compile_error!`. Use a
target dir of its own, or the cfg change rebuilds the normal one each way. On Windows keep `crt-static`, since
`RUSTFLAGS` replaces `.cargo/config.toml`'s:

```powershell
$env:RUSTFLAGS = "--cfg tokio_unstable -C target-feature=+crt-static"
$env:CARGO_TARGET_DIR = "target/console"
cargo build -p dainty-lass --features tokio-console
```

In `restricted/`, the same with `-p dainty-server --features tokio-console`. `dainty-host`, `dainty-hostkit` and
`dainty-service-kit` take the feature too.

## Watching

Run the server as usual, then in another terminal:

```powershell
cargo install --locked tokio-console
tokio-console
```

The layer listens on `127.0.0.1:6669`. `TOKIO_CONSOLE_BIND` moves it, and the other `TOKIO_CONSOLE_*` variables
console-subscriber reads (retention, publish interval) apply. Keep it on loopback: the console has no auth.

## Per module

The tasks a module starts through `dainty_service_kit::task::spawn_named` carry a name in the console's Name
column, and the rest show their spawn location, whose path names the crate:

| Name | Task |
| --- | --- |
| a service's name (`gateway`, `content`, `admin`...) | the controller's task running that service |
| `rustic-isles <server>` | one of Rustic Isles' listeners (backend, director) |
| `rustic-isles watch` | the game-server watcher |
| `<module> restart`, `<module> event` | the registry restarting a module, or handing it an event |
| `module watch`, `remote sync`, `tunnel` | the registry's child watcher, remote management, the node tunnel |

A connection's task is spawned by axum, so it shows axum's location rather than its module's. Sort by the
Location column, or look at the listener task that owns it. A new long-lived task in a module should go through
`spawn_named` (it is `tokio::spawn` in a normal build, still counted on `/metrics`).

## On /metrics, in every build

The console is for a debugging session; the admin API's `/metrics` keeps per-module totals in every build, from
[tokio-metrics](https://docs.rs/tokio-metrics) task monitors. Each `spawn_named` task, each request a
`task::monitored` router handles (admin, chat, Rustic Isles' services, the QoS listener) and each gateway and
content connection is counted under its module's name:

| Metric | What |
| --- | --- |
| `dainty_task_started_total{task}` | futures a module ran |
| `dainty_task_alive{task}` | those not yet finished |
| `dainty_task_polls_total{task}`, `dainty_task_poll_seconds_total{task}` | polls, and the time they took: the module's CPU on the runtime |
| `dainty_task_slow_polls_total{task}` | polls over 50 µs, each one blocking a worker |
| `dainty_task_scheduled_seconds_total{task}` | time woken tasks waited for a worker |
| `dainty_runtime_workers`, `dainty_runtime_alive_tasks`, `dainty_runtime_global_queue_depth`, `dainty_runtime_busy_seconds_total` | the runtime as a whole |

`rate(dainty_task_poll_seconds_total[1m])` is each module's share of the runtime; a rising
`dainty_task_slow_polls_total` names the module blocking it.

The log's level filter sits on each log output instead of the whole subscriber, so tokio's TRACE task spans reach
the console layer without turning on TRACE logging.
