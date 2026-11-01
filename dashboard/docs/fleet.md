# Running many controllers from one dashboard

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

## Adding a controller

1. **Controllers → Add controller.** Give it a name and, optionally, a region. You get a pairing code,
   valid for 30 minutes and usable once. It is stored only as a hash, so it is shown only this once.
2. On that controller, from a dainty-dashboard checkout, write `agent.env`:

   ```bash
   DAINTY_AGENT_DASHBOARD_URL=https://dashboard.example.com
   DAINTY_AGENT_PAIRING_CODE=ABCD-EFGH
   DAINTY_AGENT_ADMIN_API_KEY=...        # REFORGED_ADMIN_API_KEY from that host's secrets.env
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
| `DAINTY_AGENT_ADMIN_API_URL`   | `http://127.0.0.1:61003/graphql` | This host's admin service                        |
| `DAINTY_AGENT_ADMIN_API_KEY`   |                                  | Its key                                          |
| `DAINTY_AGENT_BACKEND_URL`     | `http://127.0.0.1:61000`         | This host's backend                              |
| `DAINTY_AGENT_OPERATOR_SECRET` |                                  | Signs `/admin/v1` calls                          |
| `DAINTY_AGENT_INGEST_PORT`     | `61009`                          | Loopback port of the ingest relay                |
| `DAINTY_AGENT_INGEST_KEY`      |                                  | Turns the relay on                               |
| `DAINTY_AGENT_ALLOW_HTTP`      |                                  | `on` to allow a non-loopback `http://` dashboard |

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
  controller's page has its services, a week of player counts, its hunts, and every change made on it.
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
