# regionlock

Linux-first CLI (later TUI) server picker for Steam Datagram Relay games — Deadlock
primary, CS2 and Dota 2 supported via the same mechanism. Blocks Valve relay POPs at
the firewall so matchmaking skips them.

**Design north star: user experience over implementation convenience.** Clean command
grammar, declarative staged workflow, first-class JSON for third-party tooling,
minimal privilege surface.

## How it works (mechanism)

Valve publishes SDR topology per app:

```
https://api.steampowered.com/ISteamApps/GetSDRConfig/v1/?appid=<APPID>
```

- Deadlock `1422450`, CS2 `730`, Dota 2 `570`. Same schema for all — multi-game
  support is only an appid switch. `--game` flag overrides the configured default.
- Response: `revision` (cache key), `pops` (map of POP code → `desc`, `geo`
  [lon, lat], `relays` [{`ipv4`, `port_range`}]), `typical_pings` (inter-POP latency
  matrix — use for estimated latency before real probing), plus SDR crypto fields
  (`certs`, `relay_public_key`, `revoked_keys`, `p2p_share_ip`) which we ignore.
  Parse tolerantly; do NOT `deny_unknown_fields`.
- Some POPs have no relays (currently `eat`, `fsn`, `hel`) — exclude from blocklist
  UI, keep for the ping matrix. Feed is IPv4-only today; don't build v6 plumbing yet.
- ~30 POPs, 2–14 relays each, ~150 IPs total. Port ranges vary per relay; we
  deliberately block **all UDP to relay IPs, ignoring ports** — relays are dedicated
  Valve boxes, and this avoids per-range rule complexity. Document this choice.
- Honest limitation (put in README): blocking relays *biases* SDR routing, it does
  not guarantee it. SDR can re-route, and actual gameservers are not in this feed.

## Architecture

Cargo workspace:

```
regionlock-core/    # lib: feed fetch/parse/cache, region aliases, desired-state
                    # model, nft ruleset codegen, plan/diff, ping, escalation
regionlock/         # bin: clap CLI over core (v1 deliverable)
regionlock-apply/   # tiny privileged applier (see privilege model)
# later: regionlock-tui/ (ratatui) — another consumer of core, no core changes
```

Core has zero UI dependencies. Everything the CLI can do must be callable as a
library function returning typed results — the TUI and third-party JSON consumers
depend on this.

## CLI grammar (v1)

Declarative: **mutations edit desired state; `apply` reconciles.** Mutations never
require privileges. `nh os switch`-style flow: show everything, then one auth.

```
regionlock list [--ping] [--json]         # POPs: code, desc, region, ping, blocked?
regionlock block <pop|region>...          # e.g. `block fra ams`, `block eu`
regionlock unblock <pop|region>...
regionlock allow <pop|region>...          # exclusive: block everything else
regionlock reset                          # clear desired state
regionlock plan [--json]                  # diff desired vs applied + rendered nft
regionlock apply [--yes] [--offline]      # plan → confirm → escalate once → atomic apply
regionlock status [--json] [--verify]     # applied state; --verify escalates to diff real table
regionlock preset save|load|list|rm <name>
regionlock game [deadlock|cs2|dota2]      # get/set default game
regionlock ping [--json]                  # live probe; --json emits NDJSON stream
regionlock enable-persist / disable-persist   # systemd unit management (privileged)
```

- Mutations print the resulting delta and hint `regionlock apply`.
- `-a/--apply` on mutations for one-shot use. Staged is the default; a config key
  (`apply_mode = "staged" | "auto"`) can flip it. Build both paths.
- Region aliases (`na`, `nae`, `naw`, `sa`, `eu`, `euw`, `eue`, `asia`, `apac`,
  `india`, `jp`, `kr`, `oce`, `me`, `af`) live as a static table in core, exposed
  via `list --regions --json` so wrappers never hardcode them.

## Privilege model

Run as the user for everything; escalate **only** at the moment rules are applied.

- `regionlock-apply` is the only privileged component. It reads a plan from
  **stdin or a file — never env** (pkexec sanitizes env), validates it, and refuses
  to touch anything except `table inet regionlock`. It must not accept raw nft
  rulesets from the caller; it constructs/validates the ruleset itself from the
  structured plan. This is a security boundary — keep it tiny and auditable.
- Escalation is a trait with backends: `pkexec` (polkit, ship
  `org.pengeg.regionlock.policy` with a human-readable action message), `sudo`,
  `doas`, `run0`. Auto-detect with config override. pkexec needs an auth agent:
  spawn `pkttyagent` as fallback, then fall back to sudo.
- Reading nft state is also root-only. Unprivileged `status` therefore reads
  `applied.json` (written by the applier on success); `status --verify` escalates
  and diffs against the live table, exit code 2 on drift.

## Firewall backend

nftables-native. Generate a full ruleset and apply atomically via `nft -f -`
(shelling out to `nft` is the maintainable choice; don't take on netlink crates).

- Own `table inet regionlock` exclusively. Full-table replace per apply; cleanup is
  always `nft delete table inet regionlock` — crash-safe by construction, `reset`
  works even after an unclean exit. Detect an orphaned table on startup and offer
  cleanup.
- One named set per POP (`set pop_fra`) so per-POP changes are set updates and
  `nft -j list table inet regionlock` yields machine-readable state for `--verify`.
- Rule shape: drop outbound UDP where daddr ∈ relay-IP sets. No port matching (see
  mechanism notes).
- iptables fallback: out of scope for v1. Design the backend as a trait so it can
  be added, but do not implement it yet.

## XDG layout

- `~/.config/regionlock/config.toml` — user intent: default game, per-game desired
  blocklist, presets, `apply_mode`, escalator preference, ping method. Desired
  state lives in *config* deliberately: declarative, dotfile-able, and a future
  home-manager module (`programs.regionlock`) falls out of it.
- `~/.cache/regionlock/` — SDR feeds per appid, keyed on `revision`. Powers
  `--offline`.
- `~/.local/state/regionlock/applied.json` — what's actually in the firewall,
  written by the applier at apply time.
- Config resolution: `--config` > `$REGIONLOCK_CONFIG` > user XDG >
  `/etc/regionlock/config.toml`.

## systemd (persistence, opt-in)

Interactive default is session-scoped (offer cleanup on exit / orphan detection).
Persistence via a system oneshot unit:

```ini
[Unit]
After=network-online.target nftables.service
[Service]
Type=oneshot
RemainAfterExit=yes
ExecStart=regionlock apply --yes --offline
ExecStop=regionlock reset --yes
```

- Runs as root → no escalation needed inside the unit.
- `enable-persist` snapshots the user's desired state into `/etc/regionlock/` so
  boot-time apply is self-contained (no dependency on a user homedir).
- `--offline` at boot: apply from cache, don't race the network.
- Optional `regionlock-refresh.timer` for cache freshness.

## JSON contract — treat as a public API

- `--json` on every read command. Every payload carries `"schema_version": 1`.
  Breaking changes bump it. Document the schema in `docs/json-api.md`.
- `plan --json`: structured diff (`to_block`, `to_unblock`, per-game context)
  **plus** the rendered nft ruleset as a string.
- `ping --json`: NDJSON, one object per result as results arrive.
- Errors: structured JSON on stderr when `--json` is active. Exit codes: 0 ok,
  1 error, 2 drift/verify-mismatch. Document all of them.
- Human output: colored tables when stdout is a tty, respects `NO_COLOR`, plain
  when piped.

## Ping

Firewall ops already require escalation, but ping should work unprivileged where
possible. Default: ICMP echo (`surge-ping` or similar — needs `CAP_NET_RAW`, so
degrade gracefully: if unprivileged, fall back to `typical_pings` estimates and
label them as estimates). A UDP probe against relay ports is unverified — the SDR
ping wire format is not confirmed public/stable. Prototype it behind a flag if
attempted; do not make it the default without empirical verification.

## Polish requirements (not optional)

- `clap_complete` shell completions (bash/zsh/fish/nu) + `clap_mangen` man page,
  generated at build time, installed by packaging.
- `--dry-run` on `apply` printing the exact ruleset (trust + debugging).
- Helpful errors: if `nft` is missing, say so and what to install; if escalation
  fails, say which backend was tried.

## Conventions

- Rust, edition 2024. `cargo clippy -- -D warnings` and `cargo fmt` clean at all
  times. Tests for: feed parsing (fixture of a real response), region alias
  resolution, plan/diff logic, nft codegen (golden-file the rendered ruleset),
  applier plan validation (must reject anything touching other tables).
- Suggested deps: `clap` (+derive, complete, mangen), `serde`/`serde_json`,
  `toml`, `ureq` (or `reqwest` if async is otherwise justified — prefer sync,
  this is not an async-shaped problem), `directories` or `etcetera` for XDG,
  `anyhow`/`thiserror` split (thiserror in core, anyhow in bins).
- Conventional commits. Keep `regionlock-apply` dependency-minimal.
- License: GPL-3.0 (matches the ecosystem; prior art is GPL — do not copy code
  from the prior-art repos, mechanism knowledge only).

## Prior art (reference, do not vendor)

- https://github.com/FN-FAL113/server-picker-x — C#/Avalonia, cross-platform GUI
- https://github.com/shibne/DeadlockServerPicker-linux — Python TUI, iptables/nftables,
  dedicated chain + cleanup-on-exit ideas validated there

## Out of scope for v1

TUI (v2, builds on core), iptables backend, Windows/macOS, IPv6, GUI. Do not
speculatively build for these beyond the trait seams already specified.

## Open questions (resolve with the user, don't guess)

- Session-scoped cleanup UX for the CLI: cleanup-on-what-exactly? (No long-lived
  process in CLI mode — likely "rules persist until `reset` or reboot unless
  persist is enabled"; confirm.)
- Exact confirmation UX for `apply` (prompt style, what the plan render looks like).
- Preset semantics: per-game or global?
