# regionlock

[![CI](https://github.com/gegnep/regionlock/actions/workflows/ci.yml/badge.svg)](https://github.com/gegnep/regionlock/actions/workflows/ci.yml)

> **Disclaimer:** A variety of LLMs assisted in building this project. I
> oversaw and made all major design and implementation decisions.

Linux CLI that biases Steam Datagram Relay (SDR) matchmaking away from
chosen relay regions. Deadlock is the primary target. CS2 and Dota 2 use the
same mechanism. regionlock blocks Valve relay POPs at the firewall, so
matchmaking skips them.

## Honest limitation

Blocking relays *biases* SDR routing. It does not guarantee it. SDR can
re-route through relays you did not block. The game servers are not in the
SDR feed either. Treat regionlock as a strong nudge. It is not a hard region
lock.

## Mechanism

Valve publishes SDR topology per game. regionlock fetches it. It resolves the
POPs you want blocked, by code or region alias. It then generates an nftables
ruleset that drops outbound UDP to those relay IPs. The rule blocks all UDP
to a relay IP and ignores ports. Relays are dedicated Valve boxes, so this
avoids per-range rule complexity.

## Model

Mutations edit desired state and never need privileges. `apply` reconciles
that state into the firewall. It escalates once, only at that moment.

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
- `game [deadlock|cs2|dota2]` sets the default game. `--game` overrides once.
- `ping` probes relay latency live. `--json` emits NDJSON as results arrive.
- `enable-persist` / `disable-persist` manage boot-time persistence.
- Every read command takes `--json`. See [docs/json-api.md](docs/json-api.md).

## Install

### Flake input (NixOS)

Add regionlock to your system flake. Import the module.

```nix
{
  inputs.regionlock = {
    url = "github:gegnep/regionlock";        # or git+file:///path for local dev
    inputs.nixpkgs.follows = "nixpkgs";
  };

  # in your NixOS configuration:
  imports = [ inputs.regionlock.nixosModules.regionlock ];

  programs.regionlock = {
    enable = true;
    # Optional: apply a blocklist at boot from a declarative config.
    persist = true;
    settings = {
      default_game = "deadlock";
      games.deadlock.desired = [ "fra" "ams" "waw" ];
    };
  };
}
```

The module installs the CLI and the polkit action. The polkit action makes
pkexec show a regionlock-specific prompt and cache the authorization for a
session. When `persist` is set, the module also defines a store-managed boot
service. The applier recognizes that service as module-managed and skips
`systemctl` itself.

Prefer `inputs.nixpkgs.follows = "nixpkgs"`. The applier's runtime deps
(nftables, systemd) then match your system. This needs a nixpkgs with rustc
1.85 or newer, for edition 2024.

`overlays.default` exposes `pkgs.regionlock` if you would rather not use the
module.

### Arch Linux (AUR)

```
paru -S regionlock   # or your AUR helper of choice
```

### From source (any FHS distro)

```
make
sudo make install PREFIX=/usr
sudo systemctl daemon-reload
```

Install to `PREFIX=/usr`, not the default `/usr/local`. systemd does not read
units from `/usr/local/lib`, so persistence needs `/usr`. The install stages
the binaries, the polkit action, the systemd units, shell completions, and
the man page. Runtime deps: nftables, systemd, iputils, polkit.

### Standalone build

```
nix build github:gegnep/regionlock#regionlock   # or cargo build --release
```

## Configuration

`~/.config/regionlock/config.toml` holds user intent: the default game,
per-game desired state, presets, apply mode, escalator preference, and
`home_pop` for latency estimates. Desired state lives in config on purpose.
This keeps it declarative and dotfile-able. It is also the source for the
NixOS module's `settings`. Config resolution order: `--config`,
`$REGIONLOCK_CONFIG`, `~/.config/regionlock`, then `/etc/regionlock`.

## Privilege model

Only `regionlock-apply` runs as root. It reads a typed operation from stdin.
It validates the operation. It touches nothing except `table inet
regionlock`. It never accepts raw nftables text. It builds the ruleset
itself. Escalation uses pkexec, sudo, doas, or run0, auto-detected and
configurable. A boot service already runs as root, so it invokes the applier
directly without an escalator.

## Requirements

- Linux with nftables.
- `ping` (iputils) for live latency. Without it, regionlock shows feed
  estimates and labels them as estimates.

## Scope

v1 is a Linux CLI. A TUI is the planned v2. The core library carries no UI
dependencies, so the TUI reuses it whole and needs no core changes. An
iptables backend, IPv6, and Windows/macOS stay out of scope.

## License

GPL-3.0-only. Prior art is GPL. regionlock uses mechanism knowledge only and
copies no code from it.
