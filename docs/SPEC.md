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
  `geo` [lon, lat], `relays` [{`ipv4`, `port_range`}]), and `typical_pings`. The
  response also contains SDR crypto fields (`certs`, `relay_public_key`, `revoked_keys`,
  `p2p_share_ip`), which we ignore. Parse tolerantly. Do NOT `deny_unknown_fields`.
  POPs also carry `tier`, `partners`, and sometimes `aliases`; expose `tier` in
  `list --json`.
- `typical_pings` is a sparse list of `[from, to, ms]` triples, not a full matrix
  (verified live: 105 entries for Deadlock's 32 POPs). Use it for estimated latency
  before real probing. Label estimates as estimates. Show "unknown" for missing pairs;
  do not synthesize values.
- Some POPs have no relays; those omit the `relays` key entirely. For Deadlock the
  relay-less POPs are `eat`, `fsn`, `hel`; Dota 2 has 32 (partner `m*` codes). Exclude
  them from the blocklist UI, but keep them for ping estimates. The feed is IPv4-only
  today. Do not build v6 plumbing yet.
- Scale differs per game (verified live): Deadlock 32 POPs / 141 IPs, CS2 48 / 210,
  Dota 2 61 / 141. Relay counts run 2–14 per POP. Port ranges vary per relay.
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
regionlock teardown [--yes]               # delete the nft table only (privileged)
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
- `reset` and `teardown` are distinct. `reset` edits desired state and needs no
  privileges. `teardown` deletes `table inet regionlock` and touches neither desired
  state nor the boot snapshot. systemd `ExecStop` uses `teardown --yes`.

## Privilege model

Run everything as the user. Escalate **only** when applying rules.

- The `regionlock-apply` component is the only privileged component. It reads a typed,
  versioned operation from **stdin or a file, never env**. pkexec sanitizes env. The
  operation schema (ReplaceRuleset, DeleteTable, Inspect, EnablePersist, DisablePersist)
  cannot express raw nft syntax or a table name. The applier validates POP codes, IPv4
  addresses, and counts, then constructs the ruleset itself for `table inet regionlock`.
  This is a security boundary. Keep it tiny and auditable.
- All applier operations serialize on a root-owned 0600 flock at `/run/regionlock/lock`.
  A world-readable lock would let any user block privileged operations.
- Implement escalation as a trait with these backends: `pkexec` (polkit; ship
  `org.pengeg.regionlock.policy` with a human-readable action message), `sudo`, `doas`,
  and `run0`. Auto-detect the backend, with a config override. pkexec needs an auth
  agent. Spawn `pkttyagent` as a fallback, then fall back to sudo.
- Reading nft state also requires root. The applier writes the applied-state journal to
  the fixed path `/run/regionlock/applied.json` (root-owned dir, 0644 file, atomic
  tmp+rename, never a caller-supplied path). Unprivileged `status` reads that path.
  `status --verify` escalates and computes a diff against the live table. It exits with
  code 2 on drift.
- The journal write is two-phase: pending intent record, nft apply, commit rename. The
  next applier invocation reconciles a leftover pending record against the live table.
  `/run` clears on reboot, which matches the ruleset lifetime.

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
- `/run/regionlock/applied.json` stores the state actually in the firewall. The applier
  writes it at apply time (see privilege model). No `~/.local/state` use in v1.
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
ExecStop=regionlock teardown --yes
```

- The unit runs as root. It needs no escalation.
- `enable-persist` snapshots the user's desired state AND the pinned feed data
  (revision plus per-POP relay IPs for the selected game) into `/etc/regionlock/`.
  Boot-time apply is then self-contained and does not depend on a user homedir or cache.
- Unit files are static packaging artifacts. Nothing writes unit content at runtime.
  `enable-persist`/`disable-persist` are idempotent applier transactions (snapshot write
  plus `systemctl enable`/`disable`), with prior-state compensation on partial failure.
  On NixOS (unit path in `/nix/store`) they skip the systemctl step and defer to the
  module.
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

## Resolved decisions

The user resolved these on 2026-07-21. They are binding for v1.

- Cleanup: rules persist until `reset`/`teardown` or reboot. On startup, detect an
  orphaned table and offer cleanup interactively.
- `apply` confirmation: colored summary diff table (POPs, IP counts, game, revision)
  with a y/N prompt. The full nft ruleset renders behind `[v]`/`--verbose` and always
  via `--dry-run`.
- Presets are per-game.
- Ping estimate gaps show "unknown"; no synthesized values.
- `list --json` includes the feed `tier` field.
- The firewall-only cleanup verb is `regionlock teardown [--yes]`.
