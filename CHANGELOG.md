# Changelog

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
regionlock uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [1.0.2] - 2026-07-24

### Fixed

- `regionlock-apply` no longer masks nft's error output. When nft exited
  before draining stdin, the applier reported "Broken pipe" instead of
  nft's stderr. A shared BrokenPipe-tolerant write helper
  (`regionlock_core::child_io`) now serves all child stdin writers, with
  a regression test.
- The child stdin writers fail fast again when stdin is not piped,
  instead of silently skipping the write.
- The applier contract now states the early-exit invariant. The stdout
  reply is the sole authoritative outcome.

## [1.0.1] - 2026-07-23

### Fixed

- Flaky applier test (`unsupported_version_is_refused`) that failed the build
  with a BrokenPipe under some schedulers. The applier checks root before it
  reads stdin, so an unprivileged run exits without consuming the operation.
  The writer then hits a BrokenPipe. The test and the CLI's applier path now
  tolerate it and trust the stdout reply. This fixes intermittent
  `nix flake check` and package build failures.

## [1.0.0] - 2026-07-21

First release.

### Added

- Server picker for Deadlock, CS2, and Dota 2 over the Steam Datagram Relay
  feed. `--game` overrides the configured default.
- Declarative workflow. `block`, `unblock`, `allow`, and `reset` edit desired
  state. `apply` reconciles it into an nftables ruleset and escalates once.
- Region aliases and a static POP-to-region table, exposed via
  `list --regions`.
- Per-game presets: `preset save|load|list|rm`.
- `plan`, `status`, and `status --verify`. `--verify` exits with code 2 on
  drift.
- `teardown` removes the firewall table without touching desired state.
- Live latency probing: `ping` and `list --ping`. Feed estimates fill in when
  probing is unavailable, labeled as estimates.
- Privileged applier `regionlock-apply` with a typed operation schema, a
  root-owned lock, and a two-phase crash-safe journal. It builds the ruleset
  itself and never accepts raw nftables text.
- Escalation via pkexec (with pkttyagent), sudo, doas, or run0. Direct exec
  when already root, so boot units need no auth agent.
- Boot persistence: `enable-persist`, `disable-persist`, and a systemd
  oneshot.
- JSON on every read command (`"schema_version": 1`), NDJSON for `ping`.
  Documented in [docs/json-api.md](docs/json-api.md).
- Shell completions (bash, zsh, fish, nu) and a man page, generated at build
  time.
- Nix flake: package, NixOS module (`programs.regionlock`), and overlay.

[1.0.2]: https://github.com/gegnep/regionlock/releases/tag/v1.0.2
[1.0.1]: https://github.com/gegnep/regionlock/releases/tag/v1.0.1
[1.0.0]: https://github.com/gegnep/regionlock/releases/tag/v1.0.0
