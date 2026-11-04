# Security

Report a security problem privately to the maintainer on `git.silentspud.com` (a private issue on
`silentspud/dainty-lass`, or a direct message), never in a public issue. Say which part is affected (the
launcher, the gateway, the content server, the admin API, the host kit, a patch to Rustic Isles), how to
reproduce it, and what an attacker gains. A problem in Mystic Paradox's or Dauntless Revived's own code that
our changes did not cause belongs to that project; we help pass it on.

In scope, among other things: an account key reaching anything but its own server and certificate; a player
reaching a host-only route or header through the gateway; the launcher installing or running a runtime the
player did not accept; plain HTTP leaving the tailnet; a secret or key in a log; path traversal in the content
server or the host kit.

The trust boundaries are in [docs/architecture.md](https://git.silentspud.com/silentspud/dainty-lass/src/branch/main/docs/architecture.md), and each boundary component has a
checklist under [docs/security/](docs/security). The game runtime is native code that each host builds and
each player chooses to accept: a malicious host can harm the players who accept their runtime, and the
launcher says so before a player does.
