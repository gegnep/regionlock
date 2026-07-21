# JSON API

`regionlock` emits machine-readable JSON on every read command under `--json`.
Treat this as a public API. Every payload carries `"schema_version": 1`.
Breaking changes bump the version.

## Conventions

- `--json` selects JSON on stdout for reads and structured errors on stderr.
- Human output uses colored tables on a tty, respects `NO_COLOR`, and stays
  plain when piped.
- Exit codes: `0` success, `1` error, `2` drift or verify-mismatch.
- Field names are stable. Removing or renaming a field bumps
  `schema_version`. New optional fields may be added within a version.

## Exit codes

| Code | Meaning |
|------|---------|
| 0 | Success. |
| 1 | Error (bad selector, config error, escalation failure, applier refusal). |
| 2 | Drift: `status --verify` found the live table diverged from the journal. |

## Errors (`stderr`)

When `--json` is active, failures print one JSON object to stderr:

```json
{ "schema_version": 1, "error": "unknown POP or region selector \"zzz\"", "kind": "unknown_selector", "exit_code": 1 }
```

`kind` is a stable discriminant: `feed_fetch`, `feed_parse`, `no_cached_feed`,
`cache_dir_unavailable`, `unknown_selector`, `config`, `unknown_preset`,
`journal_parse`, `escalation`, `applier_refused`, `io`, `drift`.

Not-yet-implemented commands emit a CLI-composed object WITHOUT a `kind`
field (the milestone stubs are not core errors):

```json
{ "schema_version": 1, "error": "not yet wired: enable-persist lands at M5", "exit_code": 1 }
```

## `list --json`

```json
{
  "schema_version": 1,
  "game": "deadlock",
  "revision": 1784582254,
  "pops": [
    {
      "code": "fra",
      "desc": "Frankfurt (Germany)",
      "regions": ["eu", "euw"],
      "blockable": true,
      "relay_count": 6,
      "tier": 1,
      "blocked": true,
      "ping": null
    }
  ]
}
```

- `regions`: alias names; empty for unclassified POPs.
- `blockable`: false for relay-less POPs (shown, never blocked).
- `tier`: Valve's tier field, passed through; may be null.
- `ping`: null unless `--ping` was given. See PingValue below.

## `list --regions --json`

The static alias table, game-independent, so wrappers never hardcode it.

```json
{
  "schema_version": 1,
  "regions": [
    { "alias": "eu", "pops": ["ams", "fra", "par", "waw", "..."] }
  ]
}
```

## PingValue

The `ping` field (in `list --ping` and `ping`) is tagged by source:

```json
{ "source": "measured", "ms": 12 }
{ "source": "estimate", "ms": 34 }
{ "source": "unknown" }
```

`measured` is a live ICMP probe. `estimate` comes from the feed's
`typical_pings` and is labeled as such. `unknown` means no data; values are
never synthesized.

## `ping --json`

NDJSON: one object per line as each result arrives.

```
{"schema_version":1,"pop":"fra","ping":{"source":"measured","ms":12}}
{"schema_version":1,"pop":"ams","ping":{"source":"measured","ms":8}}
```

## Mutations (`block`/`unblock`/`allow`/`reset`) `--json`

```json
{
  "schema_version": 1,
  "game": "deadlock",
  "now_blocked": ["ams", "fra"],
  "now_unblocked": [],
  "blocked_total": 2,
  "staged": true
}
```

`staged` is true when the change was written to desired state but not applied
(the default). With `-a`/`--apply` or `apply_mode = "auto"`, the mutation
reconciles immediately and `staged` is false.

## `plan --json`

```json
{
  "schema_version": 1,
  "game": "deadlock",
  "revision": 1784582254,
  "diff": {
    "to_block": ["fra"],
    "to_unblock": [],
    "to_update": [],
    "unchanged": []
  },
  "missing_from_feed": [],
  "ruleset": "table inet regionlock\n..."
}
```

- `to_update`: POPs whose relay IPs changed since the last apply.
- `missing_from_feed`: desired POPs absent from the current feed revision
  (informational, not an error).
- `ruleset`: the exact nftables ruleset `apply` would submit.

## `status --json`

```json
{ "schema_version": 1, "applied": null }
```

`applied` is null when nothing has been applied since boot. Otherwise it is
the journal record: `schema_version`, `game`, `revision`, `pops` (code → IPs),
and `applied_at` (Unix seconds).

## `status --verify --json`

Escalates, inspects the live table, and diffs it against the journal.

```json
{
  "schema_version": 1,
  "verified": true,
  "reconciled_pending": false,
  "live_pops": 12,
  "journal": { "schema_version": 1, "game": "deadlock", "...": "..." }
}
```

`verified` false means drift; the command also exits 2. `reconciled_pending`
true means a crashed apply or delete left a pending record that this run
reconciled.
