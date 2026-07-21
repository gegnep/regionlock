//! SDR feed model and cache.
//!
//! Parse tolerantly: the live feed carries fields we ignore (`certs`,
//! `relay_public_key`, `revoked_keys`, `p2p_share_ip`, `success`) and
//! per-POP fields beyond the spec (`partners`, `aliases`). Never
//! `deny_unknown_fields`. Relay-less POPs omit the `relays` key entirely.

use std::collections::BTreeMap;
use std::net::Ipv4Addr;

use serde::{Deserialize, Serialize};

use crate::{Error, Game, Result};

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
    use std::fs;
    use std::io;
    use std::path::{Path, PathBuf};

    /// Cache directory (~/.cache/regionlock), honoring XDG overrides.
    pub fn dir() -> Result<PathBuf> {
        use etcetera::base_strategy::{BaseStrategy, choose_base_strategy};
        let strategy = choose_base_strategy().map_err(|e| Error::CacheDirUnavailable {
            reason: e.to_string(),
        })?;
        let dir = strategy.cache_dir().join("regionlock");
        fs::create_dir_all(&dir).map_err(|source| Error::Io {
            path: dir.clone(),
            source,
        })?;
        Ok(dir)
    }

    /// Store a raw feed body keyed on (appid, revision). Atomic write.
    pub fn store(game: Game, revision: u64, body: &[u8]) -> Result<PathBuf> {
        store_in(&dir()?, game, revision, body)
    }

    /// [`store`] against an explicit base directory. Test seam: keeps the
    /// process environment untouched (`std::env::set_var` is unsafe in
    /// edition 2024).
    pub fn store_in(dir: &Path, game: Game, revision: u64, body: &[u8]) -> Result<PathBuf> {
        fs::create_dir_all(dir).map_err(|source| Error::Io {
            path: dir.to_path_buf(),
            source,
        })?;
        // pid + counter keeps concurrent writers (two CLI invocations, or
        // parallel tests) off each other's temp file; rename stays atomic.
        static TMP_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let seq = TMP_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let name = format!("{}-{revision}.json", game.appid());
        let tmp = dir.join(format!(".{name}.{}.{seq}.tmp", std::process::id()));
        let path = dir.join(name);
        fs::write(&tmp, body).map_err(|source| Error::Io {
            path: tmp.clone(),
            source,
        })?;
        fs::rename(&tmp, &path)
            .inspect_err(|_| {
                let _ = fs::remove_file(&tmp);
            })
            .map_err(|source| Error::Io {
                path: path.clone(),
                source,
            })?;
        Ok(path)
    }

    /// Newest cached feed for the game, if any.
    pub fn load_latest(game: Game) -> Result<Option<SdrFeed>> {
        load_latest_in(&dir()?, game)
    }

    /// [`load_latest`] against an explicit base directory. Files that do not
    /// match `<appid>-<revision>.json` are ignored.
    pub fn load_latest_in(dir: &Path, game: Game) -> Result<Option<SdrFeed>> {
        let prefix = format!("{}-", game.appid());
        let entries = match fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(source) => {
                return Err(Error::Io {
                    path: dir.to_path_buf(),
                    source,
                });
            }
        };
        let mut newest: Option<(u64, PathBuf)> = None;
        for entry in entries {
            let entry = entry.map_err(|source| Error::Io {
                path: dir.to_path_buf(),
                source,
            })?;
            if !entry.file_type().is_ok_and(|t| t.is_file()) {
                continue;
            }
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            let Some(revision) = name
                .strip_prefix(&prefix)
                .and_then(|rest| rest.strip_suffix(".json"))
                .and_then(|rev| rev.parse::<u64>().ok())
            else {
                continue;
            };
            if newest.as_ref().is_none_or(|(r, _)| revision > *r) {
                newest = Some((revision, entry.path()));
            }
        }
        let Some((_, path)) = newest else {
            return Ok(None);
        };
        let body = fs::read(&path).map_err(|source| Error::Io {
            path: path.clone(),
            source,
        })?;
        Ok(Some(SdrFeed::parse(&body)?))
    }
}

/// Fetch the live feed, store it in the cache, and return it.
/// With `offline`, never touch the network: serve from cache or fail with
/// [`crate::Error::NoCachedFeed`].
#[cfg(feature = "fetch")]
pub fn acquire(game: Game, offline: bool) -> Result<SdrFeed> {
    if offline {
        return cache::load_latest(game)?.ok_or(Error::NoCachedFeed { game });
    }
    match fetch_body(game) {
        Ok(body) => {
            let feed = SdrFeed::parse(&body)?;
            cache::store(game, feed.revision, &body)?;
            Ok(feed)
        }
        Err(reason) => cache::load_latest(game)?.ok_or(Error::FeedFetch {
            appid: game.appid(),
            reason,
        }),
    }
}

/// GET the live feed body; any network/HTTP failure becomes the reason
/// string reported by [`Error::FeedFetch`] when no cache can cover it.
#[cfg(feature = "fetch")]
fn fetch_body(game: Game) -> std::result::Result<Vec<u8>, String> {
    let response = ureq::get(feed_url(game))
        .call()
        .map_err(|e| e.to_string())?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("HTTP {status}"));
    }
    response
        .into_body()
        .read_to_vec()
        .map_err(|e| e.to_string())
}

pub fn feed_url(game: Game) -> String {
    format!(
        "https://api.steampowered.com/ISteamApps/GetSDRConfig/v1/?appid={}",
        game.appid()
    )
}
