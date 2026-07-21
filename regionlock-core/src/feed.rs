//! SDR feed model and cache.
//!
//! Parse tolerantly: the live feed carries fields we ignore (`certs`,
//! `relay_public_key`, `revoked_keys`, `p2p_share_ip`, `success`) and
//! per-POP fields beyond the spec (`partners`, `aliases`). Never
//! `deny_unknown_fields`. Relay-less POPs omit the `relays` key entirely.

use std::collections::BTreeMap;
use std::net::Ipv4Addr;

use serde::{Deserialize, Serialize};

use crate::{Game, Result};

/// A parsed GetSDRConfig response for one game.
#[derive(Debug, Clone, Deserialize)]
pub struct SdrFeed {
    /// Cache key. All games currently share one global revision.
    pub revision: u64,
    /// POP code (e.g. "fra") → POP. BTreeMap: deterministic iteration order
    /// feeds deterministic codegen and stable JSON output.
    pub pops: BTreeMap<String, Pop>,
    /// Sparse inter-POP latency estimates. NOT a full matrix: ~20% of pairs.
    #[serde(default)]
    pub typical_pings: Vec<TypicalPing>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Pop {
    #[serde(default)]
    pub desc: Option<String>,
    /// [lon, lat] — note the order; Frankfurt is [8.68, 50.12].
    #[serde(default)]
    pub geo: Option<[f64; 2]>,
    /// Omitted (not empty) for relay-less POPs. Relay-less POPs are excluded
    /// from blocklist resolution but kept for ping estimates.
    #[serde(default)]
    pub relays: Option<Vec<Relay>>,
    /// Valve's own tiering (0/1/2). Exposed in list --json (user decision Q5).
    #[serde(default)]
    pub tier: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Relay {
    pub ipv4: Ipv4Addr,
    /// [min, max]; deliberately unused for blocking (we drop all UDP per IP).
    #[serde(default)]
    pub port_range: Option<[u16; 2]>,
}

/// One `[from, to, ms]` triple from the feed's `typical_pings` array.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypicalPing(pub String, pub String, pub i64);

impl SdrFeed {
    /// Parse a raw GetSDRConfig response body.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        Ok(serde_json::from_slice(bytes)?)
    }

    /// POPs that can be blocked: those with at least one relay.
    pub fn blockable_pops(&self) -> impl Iterator<Item = (&str, &Pop)> {
        self.pops
            .iter()
            .filter(|(_, p)| p.relays.as_ref().is_some_and(|r| !r.is_empty()))
            .map(|(code, p)| (code.as_str(), p))
    }

    /// Relay IPs for one POP; empty for relay-less POPs.
    pub fn relay_ips(&self, pop: &str) -> Vec<Ipv4Addr> {
        self.pops
            .get(pop)
            .and_then(|p| p.relays.as_ref())
            .map(|relays| relays.iter().map(|r| r.ipv4).collect())
            .unwrap_or_default()
    }

    /// Estimated latency between two POPs, if the sparse table has the pair
    /// (either direction). `None` renders as "unknown" (user decision Q4).
    pub fn estimate_ms(&self, from: &str, to: &str) -> Option<i64> {
        self.typical_pings
            .iter()
            .find(|TypicalPing(a, b, _)| (a == from && b == to) || (a == to && b == from))
            .map(|TypicalPing(_, _, ms)| *ms)
    }
}

/// Feed acquisition and the on-disk cache under ~/.cache/regionlock/,
/// files named `<appid>-<revision>.json`, powering --offline.
pub mod cache {
    use super::*;
    use std::path::PathBuf;

    /// Cache directory (~/.cache/regionlock), honoring XDG overrides.
    pub fn dir() -> Result<PathBuf> {
        todo!("M1a: etcetera cache dir + create_dir_all")
    }

    /// Store a raw feed body keyed on (appid, revision). Atomic write.
    pub fn store(game: Game, revision: u64, body: &[u8]) -> Result<PathBuf> {
        let _ = (game, revision, body);
        todo!("M1a")
    }

    /// Newest cached feed for the game, if any.
    pub fn load_latest(game: Game) -> Result<Option<SdrFeed>> {
        let _ = game;
        todo!("M1a")
    }
}

/// Fetch the live feed, store it in the cache, and return it.
/// With `offline`, never touch the network: serve from cache or fail with
/// [`crate::Error::NoCachedFeed`].
#[cfg(feature = "fetch")]
pub fn acquire(game: Game, offline: bool) -> Result<SdrFeed> {
    let _ = (game, offline);
    todo!("M1a: ureq GET, cache::store, cache fallback on network failure")
}

pub fn feed_url(game: Game) -> String {
    format!(
        "https://api.steampowered.com/ISteamApps/GetSDRConfig/v1/?appid={}",
        game.appid()
    )
}
