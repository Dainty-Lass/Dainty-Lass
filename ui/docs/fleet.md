# Running many controllers from one dashboard

> **Deprecated (D1, module-sharing plan MS8).** Dainty Lass's own `dainty-remote` replaces the agent: each
> install reports to a remote controller over mTLS, and a dashboard beside that controller reads the whole
> fleet from its admin API (`installs`, on the Controllers page). The agent keeps working until it is
> removed, but gains nothing new, game servers included.

One dashboard can manage any number of controllers: the hosts that run a Dainty Lass server and its game
servers. It runs on one of them (the **local** controller, which it calls directly, as before). Each of
the others runs the **agent** (`apps/agent`), which connects to the dashboard.

```
            ┌──────────────── dashboard host ────────────────┐
staff ─TLS─▶│ dashboard ──loopback──▶ admin API, backend      │
            │     ▲  ▲                                        │
            └─────┼──┼────────────────────────────────────────┘
       outbound   │  │   outbound
    ┌─────────────┘  └──────────────┐
┌── controller ──────────┐   ┌── controller ──────────┐
│ agent ─loopback─▶ game │   │ agent ─loopback─▶ game │   … as many as you run
└────────────────────────┘   └────────────────────────┘
```

## Why it is built this way

- **The agent dials out; nothing dials in.** A controller opens no port for the dashboard. The admin API
  and backend stay on loopback, as dainty-lass's architecture requires. The agent holds one event stream
  open to the dashboard, commands come down it, and answers go back as POSTs.
- **Each controller's secrets stay on it.** The agent keeps that host's admin API key and manager-operator
  secret and signs its own backend calls. The dashboard never holds them.
- **The agent runs only allowlisted calls.** The dashboard sends the _name_ of a GraphQL operation, never a
  query, and backend calls must match a fixed set of routes ([`ops.ts`](../packages/fleet/src/ops.ts)).
  Anything else is refused on the host. A dashboard that is compromised can still moderate, but cannot
  reach anything else on the controller.
- **Status is pushed, not pulled.** Each controller reports every 15 s, and pages read the last report. A
  page view costs the same with three controllers as with three hundred, and a slow controller never slows
  a page. Commands (a player lookup, a ban) are the only calls that wait on a controller, and they time out
  after 20 s.

## The agent protocol

[`protocol.ts`](../packages/fleet/src/protocol.ts) has the types. Every call but pairing carries the
agent's token.

| Call                                    | Carries                                                                          |
| --------------------------------------- | -------------------------------------------------------------------------------- |
| `POST /api/agent/v1/pair`               | the pairing code, once; answers the token                                        |
| `GET /api/agent/v1/stream`              | an event stream of commands (`command`) and keep-alives (`ping`)                 |
| `POST /api/agent/v1/result`             | a command's answer                                                               |
| `POST /api/agent/v1/status`             | the status every 15 s: services, server, hunts, players, and the metrics summary |
| `/api/agent/v1/ingest/*`, `chat-filter` | the ingest API, relayed and tagged with the controller                           |

### Metrics

With each status the agent reads its host's `/metrics` (the admin service, beside its GraphQL, on the same
key: dainty-lass `docs/admin-api.md`) and sends a summary of it as `metrics`
([`metrics.ts`](../packages/fleet/src/metrics.ts)): each component's phase, uptime and restarts; CPU,
memory and uptime of the controller process and every process it started; players on each game server;
requests and the latency histogram per service and route; chat lines by verdict. The dashboard makes the
same summary for its own controller on the same clock. Counters are sent as they are, never as rates: the
dashboard keeps an hour of summaries per controller in memory (`lib/fleet/metrics.ts`) and works out
request rates, 5xx share, p50 and p95 over the last 5 minutes, and a point a minute for the charts. A
restart of the dashboard starts that hour again; the last summary is kept with the status.

The field is optional, so the protocol version stays 1: an agent that predates it reports
`capabilities` without `"metrics"` and its controller shows "No metrics yet", and an older dashboard
ignores the field. The dashboard checks and caps the summary before keeping it, and stamps it with its
own clock, as it does the status.

## Adding a controller

1. **Controllers → Add controller.** Give it a name and, optionally, a region. You get a pairing code,
   valid for 30 minutes and usable once. It is stored only as a hash, so it is shown only this once.
2. On that controller, from a dainty-dash checkout, write `agent.env`:

   ```bash
   DAINTY_AGENT_DASHBOARD_URL=https://dashboard.example.com
   DAINTY_AGENT_PAIRING_CODE=ABCD-EFGH
   DAINTY_AGENT_ADMIN_API_KEY=...        # DAINTY_ADMIN_API_KEY from that host's secrets.env
   DAINTY_AGENT_OPERATOR_SECRET=...      # RUSTIC_ISLES_MANAGER_OPERATOR_SECRET from secrets.env
   DAINTY_AGENT_INGEST_KEY=...           # optional: 32+ characters, for relaying chat and reports
   ```

3. Start it, and keep it running as a service:

   ```bash
   bun --env-file=agent.env apps/agent/src/main.ts
   ```

   Or build one binary with `bun run --filter @dainty/agent build`, which writes `apps/agent/dist/dainty-agent`.

The agent exchanges the code for a token, saves it to `DAINTY_AGENT_TOKEN_FILE` (default
`./data/agent-token`, mode 0600), and uses it from then on. Remove the code from the env file once it has
paired. **Re-pair** on the controller's page issues a new code and revokes the old token, for a reinstalled
host or a token you think has leaked. **Disable** stops a controller's token working without forgetting it.

The dashboard must be on HTTPS for agents across the internet: the token goes with every request, and the
agent refuses a plain-HTTP dashboard that is not on loopback unless `DAINTY_AGENT_ALLOW_HTTP=on`.

### Agent settings

| Variable                       | Default                          | What                                             |
| ------------------------------ | -------------------------------- | ------------------------------------------------ |
| `DAINTY_AGENT_DASHBOARD_URL`   | required                         | The dashboard's public origin                    |
| `DAINTY_AGENT_PAIRING_CODE`    |                                  | Only for the first start, or after a re-pair     |
| `DAINTY_AGENT_TOKEN_FILE`      | `./data/agent-token`             | Where the token is kept                          |
| `DAINTY_AGENT_ADMIN_API_URL`   | `http://127.0.0.1:41003/graphql` | This host's admin service                        |
| `DAINTY_AGENT_ADMIN_API_KEY`   |                                  | Its key                                          |
| `DAINTY_AGENT_BACKEND_URL`     | `http://127.0.0.1:41000`         | This host's backend                              |
| `DAINTY_AGENT_OPERATOR_SECRET` |                                  | Signs `/admin/v1` calls                          |
| `DAINTY_AGENT_INGEST_PORT`     | `61009`                          | Loopback port of the ingest relay                |
| `DAINTY_AGENT_INGEST_KEY`      |                                  | Turns the relay on                               |
| `DAINTY_AGENT_ALLOW_HTTP`      |                                  | `on` to allow a non-loopback `http://` dashboard |

An admin API or backend URL on loopback that names a port dainty-lass used before 2026-11-10 (61000 to 61006) is read
as the port it moved to (61003 as 41003), with a warning to update the setting; the dashboard does the same with
`DASHBOARD_ADMIN_API_URL` and `DASHBOARD_BACKEND_URL`. A host that moved a port in `[ports]` sets the URL to match.

## Chat and reports from a controller

The agent serves the same ingest API as the dashboard (`/api/ingest/chat`, `/api/ingest/reports`,
`/api/ingest/chat-filter`, [integration.md](integration.md)) on `127.0.0.1:61009`, and forwards it tagged
with its controller. So a controller's backend is configured the same way whether the dashboard runs on
that host or not; only the URL and key differ. Every controller gets the same chat filter.

## Using it

- **The controller picker** at the top of the rail scopes the whole dashboard: the overview, players,
  reports, chat and audit show either one controller or all of them. With one controller there is no picker.
- **Overview** across the fleet: players online, controllers up, hunts running out of capacity, the
  controllers that are down first, players over time, and every live hunt with its controller.
- **Controllers** lists them all, with what is down first, and filters by state, region and name. A
  controller's page has its services, a week of player counts, its hunts, its performance, and every
  change made on it.
- **Performance**, on the overview and each controller's page: requests, CPU and memory over the last
  hour; for one controller its components (phase, uptime, restarts, and CPU and memory where a process
  has the component's name), its game servers (players of the cap) and the processes it started, and
  for the fleet a row per controller. Below, requests by upstream (rate, 5xx, p50, p95) and, for
  moderators and up, chat lines by verdict. The metrics name no game server's process, so the
  processes are listed beside the game servers rather than matched to them.
- **Players** across the fleet asks each controller and merges the answers, with a controller column. Each
  controller has its own accounts, so a player page is always for one controller (`?c=` in the URL).
- **Ban on every controller**: admins get a switch in the ban dialog that bans the name on every reachable
  controller where it exists. Each ban is its own audit event and can be undone on its own; the result
  says where it worked, where the name does not exist, and which controllers could not be reached.
- **Undo** sends the inverse to the controller the original change was made on.

## Limits

- The dashboard is one process. The command registry and the list of connected agents live in its memory,
  so it does not run as several replicas behind a load balancer.
- Player search across the fleet asks each controller for one page of results, so with many controllers
  it shows page N of each rather than one global ordering.
- A controller whose agent is offline can't be moderated, but everything already recorded about it (chat,
  reports, history) can still be read.
