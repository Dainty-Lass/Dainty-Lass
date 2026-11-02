# Security: the QoS pong listener

The 1.14.7 client measures each region by fetching every `regionUrls` entry of `GET /candidate/regions` over plain
HTTP and expects `<!DOCTYPE html><html><body>pong</body></html>`. Node (`server.ts`, `config/listenerPlan.ts`) answers
that on a second listener at `QOS_PORT` (80 by default), on every interface (**RU5**).

## 1. What the module opens

- `crates/isles-core/src/areas/matchmaking/qos.rs` binds **127.0.0.1 only**, on the manifest's `qos` TCP port (61010
  by default; the controller moves a clash, DD4). It announces the address as `qos` in `isles.listening`, with the
  public port `QOS_PORT` asked for as `qos_public_port`.
- Every method and path answers `200 text/html` with the pong. The handler reads nothing from the request: no path,
  header or body is parsed beyond what hyper does for any listener, so there is no fuzz target of its own.
- `QOS_PORT=0` or `off` starts nothing. A value that is neither a port nor `off` is logged and starts nothing. A bind
  failure is logged and changes nothing else.
- The backend's own `GET /QoS` route answers the same body through the gateway's `/` route, as Node's
  `matchmaking.ts` does.

## 2. What it does not open, and the decision it needs

AGENTS.md: the gateway is the only listener off loopback, with DD24 the one written exception. The module therefore
never binds `QOS_PORT` publicly, and nothing yet forwards a public port to the loopback side: the module registrar has
routes and UDP ports, but no plain TCP port the gateway would publish.

**Under DaintyLass no server listener is needed.** The host kit writes `QOS_PORT=0` and
`QOS_TARGET_URL=http://127.0.0.1:61100/QoS`, so `regionUrls` points the client at the launcher's relay, which answers
the pong on the player's own machine (and on 127.0.0.1:80 as well). The measured latency is then the loopback's, not
the server's: every region looks equally close, which is what the single-region server wants.

Publishing the pong from the server (for a client without the launcher, or a real latency figure) needs one of:

1. a written exception like DD24 for a public pong port;
2. the gateway publishing the module's `qos` port, which needs a registrar entry for a public TCP port in DaintyLass's
   `module-api`;
3. `QOS_TARGET_URL` pointing at the gateway's own `/QoS` route on its public port, which needs no new listener.

Option 3 is the smallest: the route already exists and the gateway already rate-limits it.

## 3. Invariants

- The loopback side never binds anything but 127.0.0.1 (`the_loopback_listener_answers_pong_on_every_path`, and the
  module sweep in `testing.rs` checks the announced `qos` address is loopback and answers the pong).
- The body and `Content-Type` match Node byte for byte.
- A busy port or a bad `QOS_PORT` never stops the module.
