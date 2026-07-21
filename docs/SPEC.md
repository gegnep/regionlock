# regionlock

Linux-first CLI (later TUI) server picker for Steam Datagram Relay games.
Deadlock is primary. CS2 and Dota 2 use the same mechanism.
The tool blocks Valve relay POPs at the firewall. Matchmaking then skips them.

**Design north star: user experience over implementation convenience.** Use clean command
grammar and a declarative staged workflow. Provide first-class JSON for third-party tooling.
Keep the privilege surface minimal.

## Mechanism

Valve publishes SDR topology per app:

```
https://api.steampowered.com/ISteamApps/GetSDRConfig/v1/?appid=<APPID>
```

- Deadlock uses `1422450`; CS2 uses `730`; Dota 2 uses `570`. All apps use the same
  schema. Multi-game support only switches the appid. The `--game` flag overrides the
  configured default.
- Response fields include `revision` (cache key), `pops` (map of POP code → `desc`,
  `geo` [lon, lat], `relays` [{`ipv4`, `port_range`}]), and `typical_pings` (inter-POP
  latency matrix). Use `typical_pings` for estimated latency before real probing. The
  response also contains SDR crypto fields (`certs`, `relay_public_key`, `revoked_keys`,
  `p2p_share_ip`), which we ignore. Parse tolerantly. Do NOT `deny_unknown_fields`.
- Some POPs have no relays (currently `eat`, `fsn`, `hel`). Exclude them from the
  blocklist UI, but keep them for the ping matrix. The feed is IPv4-only today. Do not
  build v6 plumbing yet.
- ~30 POPs have 2–14 relays each, for ~150 IPs total. Port ranges vary per relay.
  Deliberately block **all UDP to relay IPs, ignoring ports**. Relays are dedicated Valve
  boxes. This avoids per-range rule complexity. Document this choice.
- In README, state this honest limitation: blocking relays *biases* SDR routing. It does
  not guarantee routing. SDR can re-route. Actual gameservers are not in this feed.

## Architecture

Cargo workspace:

```
regionlock-core/    # lib: feed fetch/parse/cache, region aliases, desired-state
                    # model, nft ruleset codegen, plan/diff, ping, escalation
regionlock/         # bin: clap CLI over core (v1 deliverable)
regionlock-apply/   # tiny privileged applier (see privilege model)
# later: regionlock-tui/ (ratatui) — another consumer of core, no core changes
```

Core has zero UI dependencies. Every CLI capability must be callable through a library
function that returns typed results. The TUI and third-party JSON consumers depend on this.

## CLI grammar (v1)

Use a declarative workflow: **mutations edit desired state; `apply` reconciles.** Mutations
never require privileges. Follow an `nh os switch`-style flow: show everything, then
authenticate once.

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
- Support `-a/--apply` on mutations for one-shot use. Use staged mode by default. The
  config key (`apply_mode = "staged" | "auto"`) can switch the mode. Build both paths.
- Keep region aliases (`na`, `nae`, `naw`, `sa`, `eu`, `euw`, `eue`, `asia`, `apac`,
  `india`, `jp`, `kr`, `oce`, `me`, `af`) in a static table in core. Expose them through
  `list --regions --json` so wrappers never hardcode them.

## Privilege model

Run everything as the user. Escalate **only** when applying rules.

- The `regionlock-apply` component is the only privileged component. It reads a plan from
  **stdin or a file, never env**. pkexec sanitizes env. It validates the plan and refuses
  to touch anything except `table inet regionlock`. It must not accept raw nft rulesets
  from the caller. It constructs and validates the ruleset from the structured plan.
  This is a security boundary. Keep it tiny and auditable.
- Implement escalation as a trait with these backends: `pkexec` (polkit; ship
  `org.pengeg.regionlock.policy` with a human-readable action message), `sudo`, `doas`,
  and `run0`. Auto-detect the backend, with a config override. pkexec needs an auth
  agent. Spawn `pkttyagent` as a fallback, then fall back to sudo.
- Reading nft state also requires root. Unprivileged `status` reads `applied.json`, which
  the applier writes on success. `status --verify` escalates and computes a diff against
  the live table. It exits with code 2 on drift.

## Firewall backend

Use nftables natively. Generate a full ruleset. Apply it atomically with `nft -f -`.
Shelling out to `nft` is the maintainable choice. Do not take on netlink crates.

- Own `table inet regionlock` exclusively. Replace the full table on each apply. Cleanup
  always runs `nft delete table inet regionlock`. This design is crash-safe, and `reset`
  works after an unclean exit. Detect an orphaned table on startup and offer cleanup.
- Create one named set per POP (`set pop_fra`). Per-POP changes then update sets.
  `nft -j list table inet regionlock` yields machine-readable state for `--verify`.
- Use this rule shape: drop outbound UDP where daddr ∈ relay-IP sets. Do not match ports.
  See the mechanism notes.
- Keep the iptables fallback out of scope for v1. Design the backend as a trait so it can
  be added later, but do not implement it yet.

## XDG layout

- `~/.config/regionlock/config.toml` stores user intent: default game, per-game desired
  blocklist, presets, `apply_mode`, escalator preference, and ping method. Keep desired
  state in *config*. This makes the state declarative and dotfile-able. A future
  home-manager module (`programs.regionlock`) follows from this layout.
- `~/.cache/regionlock/` stores SDR feeds per appid, keyed on `revision`. It powers
  `--offline`.
- `~/.local/state/regionlock/applied.json` stores the state actually in the firewall.
  The applier writes it at apply time.
- Resolve config in this order: `--config`, `$REGIONLOCK_CONFIG`, user XDG, then
  `/etc/regionlock/config.toml`.

## systemd

Make systemd persistence opt-in.
Use a session-scoped interactive default. Offer cleanup on exit and detect orphans.
Enable persistence with a system oneshot unit:

```ini
[Unit]
After=network-online.target nftables.service
[Service]
Type=oneshot
RemainAfterExit=yes
ExecStart=regionlock apply --yes --offline
ExecStop=regionlock reset --yes
```

- The unit runs as root. It needs no escalation.
- `enable-persist` snapshots the user's desired state into `/etc/regionlock/`. Boot-time
  apply is then self-contained and does not depend on a user homedir.
- Use `--offline` at boot. Apply from cache and do not race the network.
- Optionally use `regionlock-refresh.timer` to keep the cache fresh.

## JSON contract

Treat this contract as a public API.

- `--json` applies to every read command. Every payload carries `"schema_version": 1`.
  Breaking changes bump the version. Document the schema in `docs/json-api.md`.
- `plan --json` returns a structured diff (`to_block`, `to_unblock`, per-game context)
  **plus** the rendered nft ruleset as a string.
- `ping --json` returns NDJSON, with one object per result as results arrive.
- When `--json` is active, write structured errors to stderr. Use exit codes 0 for ok,
  1 for error, and 2 for drift/verify-mismatch. Document all exit codes.
- Human output uses colored tables when stdout is a tty. It respects `NO_COLOR` and stays
  plain when piped.

## Ping

Firewall operations already require escalation. Ping should work without privileges where
possible. Use ICMP echo by default (`surge-ping` or similar). It needs `CAP_NET_RAW`.
If unprivileged, fall back to `typical_pings` estimates and label them as estimates.
A UDP probe against relay ports remains unverified. The SDR ping wire format is not
confirmed public or stable. If attempted, prototype it behind a flag. Do not make it the
default without empirical verification.

## Polish requirements

These requirements are not optional.

- Provide `clap_complete` shell completions (bash/zsh/fish/nu) and a `clap_mangen` man
  page. Generate both at build time and install them through packaging.
- Make `--dry-run` on `apply` print the exact ruleset for trust and debugging.
- Give helpful errors. If `nft` is missing, say so and state what to install. If
  escalation fails, identify the backend tried.

## Conventions

- Use Rust, edition 2024. Keep `cargo clippy -- -D warnings` and `cargo fmt` clean at
  all times. Test feed parsing with a fixture of a real response. Test region alias
  resolution and plan/diff logic. Test nft codegen with a golden file for the rendered
  ruleset. Test applier plan validation. It must reject anything touching other tables.
- Suggested dependencies: `clap` (+derive, complete, mangen), `serde`/`serde_json`, and
  `toml`. Use `ureq`, or use `reqwest` if async is otherwise justified. Prefer sync
  because this is not an async-shaped problem. Use `directories` or `etcetera` for XDG.
  Split `anyhow`/`thiserror`: use thiserror in core and anyhow in bins.
- Use conventional commits. Keep `regionlock-apply` dependency-minimal.
- Set the license to GPL-3.0. It matches the ecosystem, and prior art is GPL. Do not
  copy code from the prior-art repos. Use mechanism knowledge only.

## Prior art

Use this section for reference. Do not vendor these projects.

- https://github.com/FN-FAL113/server-picker-x: C#/Avalonia, cross-platform GUI
- https://github.com/shibne/DeadlockServerPicker-linux: Python TUI with iptables/nftables.
  The project validates dedicated chain and cleanup-on-exit ideas.

## Out of scope for v1

Out-of-scope features for v1 are TUI (v2, builds on core), iptables backend, Windows/macOS,
IPv6, and GUI. Do not speculatively build them beyond the trait seams already specified.

## Open questions

Resolve these questions with the user. Do not guess.

- Session-scoped cleanup UX for the CLI: Which event triggers cleanup? No long-lived
  process runs in CLI mode. Likely, rules persist until `reset` or reboot unless persist
  is enabled. Confirm this behavior.
- Define the exact confirmation UX for `apply`, including the prompt style and what the plan
  render looks like.
- Decide whether preset semantics are per-game or global.
