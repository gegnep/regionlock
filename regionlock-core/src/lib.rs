//! regionlock-core: everything the regionlock CLI (and later TUI) can do,
//! as typed library functions.
//!
//! Layering rules (enforced at review):
//! - This crate has zero UI dependencies. No terminal, color, or tty code.
//! - JSON payload *shapes* live here (public API, shared by all frontends);
//!   rendering (tables, color, NDJSON framing) lives in the binaries.
//! - Every fallible function returns [`Error`], which owns the process
//!   exit-code mapping.

pub mod config;
pub mod error;
pub mod feed;
pub mod game;
pub mod payload;
pub mod regions;
pub mod state;

pub use error::Error;
pub use game::Game;

/// Version stamped into every JSON payload. Breaking payload changes bump it.
pub const SCHEMA_VERSION: u32 = 1;

pub type Result<T> = std::result::Result<T, Error>;
