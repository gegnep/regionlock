# regionlock

Linux CLI that biases Steam Datagram Relay (SDR) matchmaking away from
chosen relay regions. Deadlock is the primary target; CS2 and Dota 2 work
through the same mechanism. regionlock blocks Valve relay POPs at the
firewall so matchmaking skips them.

## Honest limitation

Blocking relays *biases* SDR routing. It does not guarantee it. SDR can
re-route through relays you did not block, and the actual game servers are
not in the SDR feed. Treat regionlock as a strong nudge, not a hard region
lock.

## How it works

Valve publishes SDR topology per game. regionlock fetches it, resolves the
POPs you want blocked (by code or region alias), and generates an nftables
ruleset that drops outbound UDP to those relay IPs. It blocks all UDP to a
relay IP and ignores ports: relays are dedicated Valve boxes, so this avoids
per-range rule complexity.

## Model

Mutations edit desired state and never need privileges. `apply` reconciles
that state into the firewall and escalates once, at that moment only.

```
regionlock block eu fra        # edit desired state
regionlock plan                # preview the diff and the exact ruleset
regionlock apply               # confirm, escalate once, apply atomically
regionlock status --verify     # compare the live table to what was applied
regionlock teardown            # remove the firewall table (leaves state)
```

- `list [--ping]` shows POPs, regions, and latency (measured or estimated).
- `block` / `unblock` / `allow` / `reset` edit per-game desired state.
- `preset save|load|list|rm` stores per-game blocklists.
- `game [deadlock|cs2|dota2]` sets the default game; `--game` overrides once.
- Every read command takes `--json`. See [docs/json-api.md](docs/json-api.md).

## Privilege model

Only `regionlock-apply` runs as root. It reads a typed operation from stdin,
validates it, and touches nothing except `table inet regionlock`. It cannot
be handed raw nftables text; it builds the ruleset itself. Escalation uses
pkexec, sudo, doas, or run0 (auto-detected, configurable).

## Configuration

`~/.config/regionlock/config.toml` holds the default game, per-game desired
state, presets, apply mode, and escalator preference. Desired state lives in
config on purpose: declarative, dotfile-able, and ready for a home-manager
module.

## Requirements

- Linux with nftables.
- `ping` (iputils) for live latency; without it, regionlock shows feed
  estimates labeled as estimates.

## Scope

v1 is a Linux CLI. A TUI, an iptables backend, IPv6, and Windows/macOS are
out of scope. The core library is UI-free so a future TUI reuses it whole.

## License

GPL-3.0-only. Prior art is GPL; regionlock uses mechanism knowledge only and
copies no code from it.
