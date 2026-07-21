use std::path::PathBuf;

/// Process exit codes are part of the public contract (SPEC: JSON contract).
pub const EXIT_OK: i32 = 0;
pub const EXIT_ERROR: i32 = 1;
pub const EXIT_DRIFT: i32 = 2;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("failed to fetch SDR feed for appid {appid}: {reason}")]
    FeedFetch { appid: u32, reason: String },

    #[error("failed to parse SDR feed: {0}")]
    FeedParse(#[from] serde_json::Error),

    #[error("no cached feed for {game} and --offline was requested")]
    NoCachedFeed { game: crate::Game },

    /// The XDG cache directory could not be determined (no home directory).
    /// Separate from `Io`: etcetera's HomeDirError carries no path and is
    /// not an io::Error.
    #[error("could not determine the XDG cache directory: {reason}")]
    CacheDirUnavailable { reason: String },

    #[error("unknown POP or region selector {selector:?}")]
    UnknownSelector { selector: String },

    #[error("config error at {path}: {reason}")]
    Config { path: PathBuf, reason: String },

    #[error("no preset named {name:?} for {game}")]
    UnknownPreset { name: String, game: crate::Game },

    #[error("io error on {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// Live firewall state does not match the applied-state journal.
    /// Maps to exit code 2; not an error in the usual sense.
    #[error("firewall state drifted from the applied-state journal")]
    Drift,
}

impl Error {
    /// The exit code this error maps to. Documented in docs/json-api.md.
    pub fn exit_code(&self) -> i32 {
        match self {
            Error::Drift => EXIT_DRIFT,
            _ => EXIT_ERROR,
        }
    }

    /// Structured form for stderr when --json is active.
    pub fn to_payload(&self) -> crate::payload::ErrorPayload {
        crate::payload::ErrorPayload {
            schema_version: crate::SCHEMA_VERSION,
            error: self.to_string(),
            kind: self.kind(),
            exit_code: self.exit_code(),
        }
    }

    /// Stable machine-readable discriminant. Part of the JSON contract:
    /// renaming a kind is a breaking change.
    pub fn kind(&self) -> &'static str {
        match self {
            Error::FeedFetch { .. } => "feed_fetch",
            Error::FeedParse(_) => "feed_parse",
            Error::NoCachedFeed { .. } => "no_cached_feed",
            Error::CacheDirUnavailable { .. } => "cache_dir_unavailable",
            Error::UnknownSelector { .. } => "unknown_selector",
            Error::Config { .. } => "config",
            Error::UnknownPreset { .. } => "unknown_preset",
            Error::Io { .. } => "io",
            Error::Drift => "drift",
        }
    }
}
