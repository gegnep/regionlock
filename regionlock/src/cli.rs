//! The complete v1 command grammar, frozen at M1 (SPEC + resolved decisions).
//! Commands may answer "not yet wired" until their milestone lands, but the
//! surface itself does not change shape after this file is reviewed.

use clap::{Parser, Subcommand};
use regionlock_core::Game;

#[derive(Debug, Parser)]
#[command(name = "regionlock", version, about, max_term_width = 100)]
pub struct Cli {
    /// Override the configured default game for this invocation.
    #[arg(long, global = true, value_name = "GAME")]
    pub game: Option<Game>,

    /// Path to config.toml (overrides $REGIONLOCK_CONFIG and XDG lookup).
    #[arg(long, global = true, value_name = "PATH")]
    pub config: Option<std::path::PathBuf>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// List POPs: code, description, regions, ping, blocked state.
    List {
        /// Probe live latency (ICMP; falls back to labeled estimates).
        #[arg(long)]
        ping: bool,
        /// Print the region alias table instead of POPs.
        #[arg(long)]
        regions: bool,
        #[arg(long)]
        json: bool,
    },
    /// Block POPs or regions (edits desired state).
    Block {
        /// POP codes or region aliases, e.g. `fra ams` or `eu`.
        #[arg(required = true)]
        selectors: Vec<String>,
        /// Apply immediately after staging (one-shot mode).
        #[arg(short = 'a', long)]
        apply: bool,
        #[arg(long)]
        json: bool,
    },
    /// Unblock POPs or regions (edits desired state).
    Unblock {
        #[arg(required = true)]
        selectors: Vec<String>,
        #[arg(short = 'a', long)]
        apply: bool,
        #[arg(long)]
        json: bool,
    },
    /// Exclusive allow: block everything except these POPs/regions.
    Allow {
        #[arg(required = true)]
        selectors: Vec<String>,
        #[arg(short = 'a', long)]
        apply: bool,
        #[arg(long)]
        json: bool,
    },
    /// Clear desired state. Never touches the firewall (see teardown).
    Reset {
        #[arg(short = 'a', long)]
        apply: bool,
        #[arg(long)]
        json: bool,
    },
    /// Delete `table inet regionlock` from the firewall (privileged).
    /// Leaves desired state and the boot snapshot alone.
    Teardown {
        /// Skip the confirmation prompt (systemd ExecStop uses this).
        #[arg(long)]
        yes: bool,
        #[arg(long)]
        json: bool,
    },
    /// Show the diff between desired and applied state plus the ruleset.
    Plan {
        #[arg(long)]
        json: bool,
    },
    /// Reconcile: plan, confirm, escalate once, apply atomically.
    Apply {
        /// Skip the confirmation prompt.
        #[arg(long)]
        yes: bool,
        /// Use only cached/snapshot feed data; never touch the network.
        #[arg(long)]
        offline: bool,
        /// Boot mode: resolve config and feed from /etc/regionlock ONLY
        /// (never user config, cache, or network; overrides --config).
        /// regionlock.service uses this so a stray root homedir can never
        /// shadow the boot snapshot.
        #[arg(long)]
        system: bool,
        /// Print the exact ruleset and exit. No escalation, no mutation.
        #[arg(long)]
        dry_run: bool,
        /// Show the full nft ruleset in the confirmation prompt.
        #[arg(short, long)]
        verbose: bool,
        #[arg(long)]
        json: bool,
    },
    /// Show applied state from the journal; --verify diffs the live table.
    Status {
        /// Escalate and compare the journal against the real nft table.
        /// Exit code 2 on drift.
        #[arg(long)]
        verify: bool,
        #[arg(long)]
        json: bool,
    },
    /// Save, load, list, or remove per-game presets.
    #[command(subcommand)]
    Preset(PresetCommand),
    /// Show or set the default game.
    Game {
        /// New default; prints the current default when omitted.
        set: Option<Game>,
        #[arg(long)]
        json: bool,
    },
    /// Probe relay latency live. --json emits NDJSON as results arrive.
    Ping {
        #[arg(long)]
        json: bool,
    },
    /// Install persistence: snapshot state to /etc/regionlock and enable
    /// the systemd unit (privileged, idempotent).
    EnablePersist {
        /// Skip the confirmation prompt.
        #[arg(long)]
        yes: bool,
        /// Fetch a fresh feed before snapshotting instead of cache-first.
        #[arg(long)]
        refresh: bool,
        #[arg(long)]
        json: bool,
    },
    /// Remove persistence (privileged, idempotent).
    DisablePersist {
        /// Skip the confirmation prompt.
        #[arg(long)]
        yes: bool,
        #[arg(long)]
        json: bool,
    },
    /// Packaging helpers: emit shell completions or the man page. Hidden
    /// from --help; packaging invokes this at package-build time.
    #[command(hide = true)]
    Generate {
        #[command(subcommand)]
        what: GenerateCommand,
    },
}

#[derive(Debug, Subcommand)]
pub enum GenerateCommand {
    /// Write the completion script for the given shell to stdout.
    Completions {
        /// One of: bash, zsh, fish, nu.
        shell: String,
    },
    /// Write the roff man page for the top-level command to stdout.
    Man,
}

#[derive(Debug, Subcommand)]
pub enum PresetCommand {
    Save {
        name: String,
    },
    Load {
        name: String,
    },
    List {
        #[arg(long)]
        json: bool,
    },
    Rm {
        name: String,
    },
}
