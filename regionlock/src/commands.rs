//! Command implementations: the M1 command set wired to regionlock-core.
//!
//! Commands that land at later milestones answer "not yet wired" instead
//! of touching plumbing.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use clap::CommandFactory;
use clap_complete::{Shell, generate as write_completions};
use clap_complete_nushell::Nushell;
use regionlock_core::backend::{FirewallBackend, NftBackend};
use regionlock_core::config::{ApplyMode, Config};
use regionlock_core::feed::{self, Pop, SdrFeed};
use regionlock_core::payload::{
    DeltaPayload, ListPayload, PlanPayload, PopInfo, RegionInfo, RegionsPayload, StatusPayload,
};
use regionlock_core::plan::{AppliedState, PlanDiff, RulesetSpec};
use regionlock_core::regions::{self, Classification, Region};
use regionlock_core::state::{Delta, DesiredState};
use regionlock_core::{Error, Game, SCHEMA_VERSION};
use serde_json::json;

use crate::cli::{Cli, Command, GenerateCommand, PresetCommand};
use crate::output::{Cell, Style, render_table};

/// A command failure. Core errors own their exit code and JSON kind.
/// Not-yet-wired commands are CLI-level: "not_implemented" is not in core's
/// frozen kind list, so the CLI composes that stderr object itself.
pub enum Failure {
    Core(Error),
    NotWired {
        what: &'static str,
        milestone: &'static str,
    },
    /// Packaging-facing `generate` failure: plain stderr, exit 1. No JSON
    /// shape; the hidden command carries no --json flag.
    Usage {
        message: String,
    },
}

impl From<Error> for Failure {
    fn from(err: Error) -> Self {
        Failure::Core(err)
    }
}

impl Failure {
    fn not_wired(what: &'static str, milestone: &'static str) -> Self {
        Failure::NotWired { what, milestone }
    }

    /// Print the error to stderr (a JSON object when --json is active, a
    /// human message otherwise) and return the process exit code.
    pub fn report(&self, json: bool) -> i32 {
        match self {
            Failure::Core(err) => {
                if json {
                    let payload = err.to_payload();
                    eprintln!(
                        "{}",
                        serde_json::to_string(&payload).expect("ErrorPayload serializes")
                    );
                } else {
                    eprintln!("error: {err}");
                }
                err.exit_code()
            }
            Failure::NotWired { what, milestone } => {
                let message = format!("not yet wired: {what} lands at {milestone}");
                if json {
                    eprintln!(
                        "{}",
                        json!({
                            "schema_version": SCHEMA_VERSION,
                            "error": message,
                            "exit_code": 1,
                        })
                    );
                } else {
                    eprintln!("{message}");
                }
                1
            }
            Failure::Usage { message } => {
                eprintln!("{message}");
                1
            }
        }
    }
}

/// Whether the invocation asked for --json; decides the stderr error shape.
pub fn json_requested(command: &Command) -> bool {
    match command {
        Command::List { json, .. }
        | Command::Block { json, .. }
        | Command::Unblock { json, .. }
        | Command::Allow { json, .. }
        | Command::Reset { json, .. }
        | Command::Teardown { json, .. }
        | Command::Plan { json }
        | Command::Apply { json, .. }
        | Command::Status { json, .. }
        | Command::Game { json, .. }
        | Command::Ping { json }
        | Command::EnablePersist { json, .. }
        | Command::DisablePersist { json, .. } => *json,
        Command::Preset(PresetCommand::List { json }) => *json,
        Command::Preset(_) => false,
        Command::Generate { .. } => false,
    }
}

/// Shared plumbing threaded through every wired command: the resolved
/// config (and its path, so mutations save back to the same file), the
/// active game, and the terminal styling decision.
struct Ctx {
    config: Config,
    config_path: PathBuf,
    game: Game,
    style: Style,
}

pub fn run(cli: &Cli) -> Result<(), Failure> {
    // Not-yet-wired commands answer before any config or feed plumbing.
    // generate renders from the grammar alone: no config, cache, or network.
    match &cli.command {
        Command::Generate { what } => return cmd_generate(what),
        Command::Ping { .. } => return Err(Failure::not_wired("ping", "M4")),
        Command::EnablePersist { .. } => return Err(Failure::not_wired("enable-persist", "M5")),
        Command::DisablePersist { .. } => return Err(Failure::not_wired("disable-persist", "M5")),
        _ => {}
    }

    let config_path = Config::resolve_path(cli.config.as_deref())?;
    let config = Config::load(&config_path)?;
    let game = regionlock_core::game::resolve(cli.game, config.default_game);
    let mut ctx = Ctx {
        config,
        config_path,
        game,
        style: Style::detect(),
    };

    match &cli.command {
        Command::List {
            ping,
            regions: show_regions,
            json,
        } => cmd_list(&ctx, *ping, *show_regions, *json),
        Command::Block {
            selectors,
            apply,
            json,
        } => cmd_selectors(&mut ctx, Mutation::Block, selectors, *apply, *json),
        Command::Unblock {
            selectors,
            apply,
            json,
        } => cmd_selectors(&mut ctx, Mutation::Unblock, selectors, *apply, *json),
        Command::Allow {
            selectors,
            apply,
            json,
        } => cmd_selectors(&mut ctx, Mutation::Allow, selectors, *apply, *json),
        Command::Reset { apply, json } => {
            let delta = ctx.config.desired_mut(ctx.game).reset();
            finish_mutation(&ctx, delta, *apply, *json)
        }
        Command::Preset(subcommand) => cmd_preset(&mut ctx, subcommand),
        Command::Game { set, json } => cmd_game(&mut ctx, *set, *json),
        Command::Plan { json } => cmd_plan(&ctx, *json),
        Command::Status { verify, json } => cmd_status(&ctx, *verify, *json),
        Command::Apply {
            yes,
            offline,
            dry_run,
            verbose,
            json,
        } => cmd_apply(&ctx, *yes, *offline, *dry_run, *verbose, *json),
        Command::Teardown { yes, json } => cmd_teardown(&ctx, *yes, *json),
        Command::Ping { .. }
        | Command::EnablePersist { .. }
        | Command::DisablePersist { .. }
        | Command::Generate { .. } => unreachable!("early-return commands return above"),
    }
}

/// Q2-resolved confirmation UX: summary diff table with a y/N prompt; [v]
/// expands the full ruleset; --verbose shows it immediately; --yes skips.
/// Returns false when the user declines.
fn confirm(prompt_target: &str, ruleset: &str, verbose: bool) -> Result<bool, Failure> {
    use std::io::{BufRead, IsTerminal};
    if verbose {
        print!("{ruleset}");
    }
    if !std::io::stdin().is_terminal() {
        // Non-interactive without --yes: refuse rather than hang or assume.
        eprintln!("stdin is not a terminal; use --yes for unattended {prompt_target}");
        return Ok(false);
    }
    loop {
        print!("{prompt_target}? [y/N{}] ", if verbose { "" } else { "/v" });
        use std::io::Write as _;
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        if std::io::stdin().lock().read_line(&mut line).is_err() {
            return Ok(false);
        }
        match line.trim() {
            "y" | "Y" | "yes" => return Ok(true),
            "v" | "V" if !verbose => print!("{ruleset}"),
            _ => return Ok(false),
        }
    }
}

fn cmd_apply(
    ctx: &Ctx,
    yes: bool,
    offline: bool,
    dry_run: bool,
    verbose: bool,
    json: bool,
) -> Result<(), Failure> {
    let feed = if offline {
        feed::cache::load_latest(ctx.game)?.ok_or(Error::NoCachedFeed { game: ctx.game })?
    } else {
        load_feed(ctx.game)?
    };
    let (spec, missing) = RulesetSpec::build(&ctx.config, ctx.game, &feed);
    let applied = AppliedState::read()?;
    let diff = PlanDiff::compute(&spec, applied.as_ref());
    let ruleset = NftBackend.render(&spec);

    if dry_run {
        // Trust + debugging: the exact ruleset, no escalation, no mutation.
        print!("{ruleset}");
        return Ok(());
    }
    if diff.is_empty() {
        if json {
            println!(
                "{}",
                json!({ "schema_version": SCHEMA_VERSION, "applied": false, "reason": "nothing to do" })
            );
        } else {
            println!("nothing to do");
        }
        return Ok(());
    }

    if !yes {
        println!("  game: {} (rev {})", ctx.game.name(), spec.revision);
        print_plan_line("  block", &diff.to_block);
        print_plan_line("  unblock", &diff.to_unblock);
        print_plan_line("  update", &diff.to_update);
        if !missing.is_empty() {
            println!("  note: absent from feed: {}", missing.join(", "));
        }
        println!(
            "  {} POPs blocked after apply, {} IPs total.",
            spec.pops.len(),
            spec.pops.values().map(Vec::len).sum::<usize>()
        );
        if !confirm("Apply", &ruleset, verbose)? {
            eprintln!("aborted");
            return Err(Failure::Usage {
                message: "apply aborted".into(),
            });
        }
    }

    let operation = regionlock_core::ops::Operation::replace_from_spec(&spec);
    let reply = regionlock_core::escalate::run_applier(ctx.config.escalator, &operation)?;
    match reply {
        regionlock_core::ops::Reply::Applied { journal } => {
            if json {
                println!(
                    "{}",
                    json!({ "schema_version": SCHEMA_VERSION, "applied": true, "journal": journal })
                );
            } else {
                println!(
                    "applied: {} POPs blocked ({} IPs)",
                    journal.pops.len(),
                    journal.pops.values().map(Vec::len).sum::<usize>()
                );
            }
            Ok(())
        }
        regionlock_core::ops::Reply::Refused { reason } => {
            Err(Error::ApplierRefused { reason }.into())
        }
        other => Err(Failure::Usage {
            message: format!("unexpected applier reply: {other:?}"),
        }),
    }
}

fn cmd_teardown(ctx: &Ctx, yes: bool, json: bool) -> Result<(), Failure> {
    if !yes {
        println!("teardown deletes `table inet regionlock` from the firewall.");
        println!("Desired state and any boot snapshot stay untouched.");
        if !confirm("Teardown", "", true)? {
            eprintln!("aborted");
            return Err(Failure::Usage {
                message: "teardown aborted".into(),
            });
        }
    }
    let operation = regionlock_core::ops::Operation::DeleteTable {
        ops_version: regionlock_core::ops::OPS_VERSION,
    };
    let reply = regionlock_core::escalate::run_applier(ctx.config.escalator, &operation)?;
    match reply {
        regionlock_core::ops::Reply::Deleted { existed } => {
            if json {
                println!(
                    "{}",
                    json!({ "schema_version": SCHEMA_VERSION, "deleted": true, "existed": existed })
                );
            } else if existed {
                println!("table inet regionlock removed");
            } else {
                println!("no table to remove");
            }
            Ok(())
        }
        regionlock_core::ops::Reply::Refused { reason } => {
            Err(Error::ApplierRefused { reason }.into())
        }
        other => Err(Failure::Usage {
            message: format!("unexpected applier reply: {other:?}"),
        }),
    }
}

/// Hidden packaging command: completions and the man page render from the
/// live grammar, so packaging output never drifts from cli.rs.
fn cmd_generate(what: &GenerateCommand) -> Result<(), Failure> {
    let mut cmd = Cli::command();
    // Locked handle: completions/man are one large write; skip per-line
    // stdout lock churn.
    let mut stdout = std::io::stdout().lock();
    match what {
        GenerateCommand::Completions { shell } => {
            match shell.as_str() {
                "bash" => write_completions(Shell::Bash, &mut cmd, "regionlock", &mut stdout),
                "zsh" => write_completions(Shell::Zsh, &mut cmd, "regionlock", &mut stdout),
                "fish" => write_completions(Shell::Fish, &mut cmd, "regionlock", &mut stdout),
                "nu" => write_completions(Nushell, &mut cmd, "regionlock", &mut stdout),
                other => {
                    return Err(Failure::Usage {
                        message: format!(
                            "unknown shell {other:?}; supported shells: bash, zsh, fish, nu"
                        ),
                    });
                }
            }
            Ok(())
        }
        GenerateCommand::Man => clap_mangen::Man::new(cmd)
            .render(&mut stdout)
            .map_err(|err| Failure::Usage {
                message: format!("failed to render man page: {err}"),
            }),
    }
}

/// Cache-first feed loading: serve the newest cached revision, and only
/// touch the network (via `feed::acquire`, which re-caches and itself
/// falls back to cache on fetch failure) when no cache entry exists.
fn load_feed(game: Game) -> regionlock_core::Result<SdrFeed> {
    if let Some(feed) = feed::cache::load_latest(game)? {
        return Ok(feed);
    }
    feed::acquire(game, false)
}

fn cmd_list(ctx: &Ctx, ping: bool, show_regions: bool, json: bool) -> Result<(), Failure> {
    if ping {
        return Err(Failure::not_wired("list --ping", "M4"));
    }
    if show_regions {
        return print_regions(json, &ctx.style);
    }
    let feed = load_feed(ctx.game)?;
    let desired = ctx.config.desired(ctx.game);
    if json {
        let pops = feed
            .pops
            .iter()
            .map(|(code, pop)| PopInfo {
                code: code.clone(),
                desc: pop.desc.clone(),
                regions: region_names(code),
                blockable: is_blockable(pop),
                relay_count: pop.relays.as_ref().map_or(0, Vec::len),
                tier: pop.tier,
                blocked: desired.blocked.contains(code),
                ping: None,
            })
            .collect();
        let payload = ListPayload {
            schema_version: SCHEMA_VERSION,
            game: ctx.game.name().to_string(),
            revision: feed.revision,
            pops,
        };
        println!(
            "{}",
            serde_json::to_string(&payload).expect("ListPayload serializes")
        );
        return Ok(());
    }
    let rows: Vec<Vec<Cell>> = feed
        .pops
        .iter()
        .map(|(code, pop)| {
            let marker = if !is_blockable(pop) {
                Cell::plain("unblockable")
            } else if desired.blocked.contains(code) {
                Cell::red("blocked")
            } else {
                Cell::green("ok")
            };
            vec![
                Cell::plain(code.clone()),
                Cell::plain(pop.desc.clone().unwrap_or_else(|| "-".to_string())),
                Cell::plain(regions_label(code)),
                Cell::plain(
                    pop.tier
                        .map_or_else(|| "-".to_string(), |tier| tier.to_string()),
                ),
                Cell::plain(pop.relays.as_ref().map_or(0, Vec::len).to_string()),
                marker,
            ]
        })
        .collect();
    println!(
        "{}",
        render_table(
            &["CODE", "DESC", "REGIONS", "TIER", "RELAYS", "BLOCKED"],
            &rows,
            &ctx.style
        )
    );
    Ok(())
}

fn cmd_plan(ctx: &Ctx, json: bool) -> Result<(), Failure> {
    let feed = load_feed(ctx.game)?;
    let (spec, missing_from_feed) = RulesetSpec::build(&ctx.config, ctx.game, &feed);
    let applied = AppliedState::read()?;
    let diff = PlanDiff::compute(&spec, applied.as_ref());
    let ruleset = NftBackend.render(&spec);

    if json {
        let payload = PlanPayload {
            schema_version: SCHEMA_VERSION,
            game: ctx.game.name().to_string(),
            revision: spec.revision,
            diff,
            missing_from_feed,
            ruleset,
        };
        println!(
            "{}",
            serde_json::to_string(&payload).expect("PlanPayload serializes")
        );
        return Ok(());
    }

    if diff.is_empty() {
        println!("nothing to do");
    } else {
        print_plan_line("to block", &diff.to_block);
        print_plan_line("to unblock", &diff.to_unblock);
        print_plan_line("to update", &diff.to_update);
    }
    if !missing_from_feed.is_empty() {
        println!(
            "note: desired POPs absent from feed revision {}: {}",
            spec.revision,
            missing_from_feed.join(", ")
        );
    }
    print!("{ruleset}");
    Ok(())
}

fn print_plan_line(label: &str, codes: &[String]) {
    if codes.is_empty() {
        println!("{label}: 0");
    } else {
        println!("{label}: {} ({})", codes.len(), codes.join(", "));
    }
}

fn cmd_status(ctx: &Ctx, verify: bool, json: bool) -> Result<(), Failure> {
    if verify {
        return cmd_status_verify(ctx, json);
    }

    let applied = AppliedState::read()?;
    if json {
        let payload = StatusPayload {
            schema_version: SCHEMA_VERSION,
            applied,
        };
        println!(
            "{}",
            serde_json::to_string(&payload).expect("StatusPayload serializes")
        );
        return Ok(());
    }

    match applied {
        Some(applied) => {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |duration| duration.as_secs());
            let age = now.saturating_sub(applied.applied_at);
            println!(
                "applied: game={} revision={} POPs={} age={}s",
                applied.game,
                applied.revision,
                applied.pops.len(),
                age
            );
        }
        None => println!("nothing applied"),
    }
    Ok(())
}

/// `status --verify`: escalate an Inspect and diff the live table against
/// the journal. Exit 2 (Error::Drift) on mismatch (SPEC exit codes).
fn cmd_status_verify(ctx: &Ctx, json: bool) -> Result<(), Failure> {
    let operation = regionlock_core::ops::Operation::Inspect {
        ops_version: regionlock_core::ops::OPS_VERSION,
    };
    let reply = regionlock_core::escalate::run_applier(ctx.config.escalator, &operation)?;
    let regionlock_core::ops::Reply::Inspected {
        live,
        journal,
        reconciled_pending,
    } = reply
    else {
        if let regionlock_core::ops::Reply::Refused { reason } = reply {
            return Err(Error::ApplierRefused { reason }.into());
        }
        return Err(Failure::Usage {
            message: "unexpected applier reply to inspect".into(),
        });
    };

    let journal_pops = journal.as_ref().map(|state| &state.pops);
    let in_sync = match (&live, journal_pops) {
        (None, None) => true,
        (Some(live_pops), Some(journal_pops)) => live_pops == journal_pops,
        _ => false,
    };

    if json {
        println!(
            "{}",
            json!({
                "schema_version": SCHEMA_VERSION,
                "verified": in_sync,
                "reconciled_pending": reconciled_pending,
                "live_pops": live.as_ref().map(|pops| pops.len()),
                "journal": journal,
            })
        );
    } else if in_sync {
        println!(
            "verified: firewall matches the journal ({})",
            match &live {
                Some(pops) => format!("{} POPs", pops.len()),
                None => "nothing applied".to_string(),
            }
        );
    }
    if in_sync {
        Ok(())
    } else {
        Err(Error::Drift.into())
    }
}

/// The region alias table (human or RegionsPayload), so wrappers never
/// hardcode the aliases. Built from the complete static CLASSIFICATION
/// table, not the active feed: the alias map is game-independent and this
/// keeps `list --regions` free of config, cache, and network access.
fn print_regions(json: bool, style: &Style) -> Result<(), Failure> {
    let table: Vec<(Region, Vec<String>)> = Region::ALL
        .into_iter()
        .map(|region| {
            let mut pops: Vec<String> = regions::CLASSIFICATION
                .iter()
                .filter(|(_, rs)| rs.contains(&region))
                .map(|(code, _)| (*code).to_string())
                .collect();
            pops.sort();
            (region, pops)
        })
        .collect();
    if json {
        let payload = RegionsPayload {
            schema_version: SCHEMA_VERSION,
            regions: table
                .into_iter()
                .map(|(alias, pops)| RegionInfo { alias, pops })
                .collect(),
        };
        println!(
            "{}",
            serde_json::to_string(&payload).expect("RegionsPayload serializes")
        );
        return Ok(());
    }
    let rows: Vec<Vec<Cell>> = table
        .iter()
        .map(|(region, pops)| vec![Cell::plain(region.name()), Cell::plain(pops.join(", "))])
        .collect();
    println!("{}", render_table(&["ALIAS", "POPS"], &rows, style));
    Ok(())
}

enum Mutation {
    Block,
    Unblock,
    Allow,
}

/// block / unblock / allow: parse and expand selectors against the feed,
/// mutate desired state, then run the shared mutation tail.
fn cmd_selectors(
    ctx: &mut Ctx,
    mutation: Mutation,
    selectors: &[String],
    apply: bool,
    json: bool,
) -> Result<(), Failure> {
    let feed = load_feed(ctx.game)?;
    let known_pops: Vec<&str> = feed.pops.keys().map(String::as_str).collect();
    let mut pops: Vec<String> = Vec::new();
    for input in selectors {
        let selector = regions::parse_selector(input, &known_pops)?;
        pops.extend(regions::expand(&selector, &feed));
    }
    pops.sort();
    pops.dedup();
    let delta = {
        let desired = ctx.config.desired_mut(ctx.game);
        match mutation {
            Mutation::Block => desired.block(&pops),
            Mutation::Unblock => desired.unblock(&pops),
            Mutation::Allow => {
                let all_blockable: Vec<String> = feed
                    .blockable_pops()
                    .map(|(code, _)| code.to_string())
                    .collect();
                desired.allow(&pops, &all_blockable)
            }
        }
    };
    finish_mutation(ctx, delta, apply, json)
}

/// Shared mutation tail: persist desired state, print the delta (staged
/// mode adds the apply hint), then honor -a/--apply / apply_mode = "auto"
/// by chaining into `apply`, which lands at M2-M3. The config save runs
/// first so state persists even on that not-yet-wired exit.
fn finish_mutation(ctx: &Ctx, delta: Delta, apply: bool, json: bool) -> Result<(), Failure> {
    ctx.config.save(&ctx.config_path)?;
    let staged = !apply && ctx.config.apply_mode != ApplyMode::Auto;
    if json {
        let payload = DeltaPayload {
            schema_version: SCHEMA_VERSION,
            game: ctx.game.name().to_string(),
            now_blocked: delta.now_blocked,
            now_unblocked: delta.now_unblocked,
            blocked_total: ctx.config.desired(ctx.game).blocked.len(),
            staged,
        };
        println!(
            "{}",
            serde_json::to_string(&payload).expect("DeltaPayload serializes")
        );
    } else {
        print_delta(&delta, &ctx.style);
    }
    if staged {
        if !json {
            println!("run `regionlock apply` to reconcile");
        }
        Ok(())
    } else {
        // One-shot (-a) or apply_mode = "auto": reconcile now. The Q2
        // confirmation still applies; --yes exists on `apply` for scripts.
        cmd_apply(ctx, false, false, false, false, json)
    }
}

fn print_delta(delta: &Delta, style: &Style) {
    if delta.now_blocked.is_empty() && delta.now_unblocked.is_empty() {
        println!("no changes");
        return;
    }
    if !delta.now_blocked.is_empty() {
        println!("now blocked: {}", style.red(&delta.now_blocked.join(", ")));
    }
    if !delta.now_unblocked.is_empty() {
        println!(
            "now unblocked: {}",
            style.green(&delta.now_unblocked.join(", "))
        );
    }
}

fn cmd_game(ctx: &mut Ctx, set: Option<Game>, json: bool) -> Result<(), Failure> {
    if let Some(game) = set {
        ctx.config.default_game = game;
        ctx.config.save(&ctx.config_path)?;
    }
    let default_game = ctx.config.default_game.name();
    if json {
        println!(
            "{}",
            json!({
                "schema_version": SCHEMA_VERSION,
                "default_game": default_game,
            })
        );
    } else if set.is_some() {
        println!("default game set to {default_game}");
    } else {
        println!("{default_game}");
    }
    Ok(())
}

fn cmd_preset(ctx: &mut Ctx, subcommand: &PresetCommand) -> Result<(), Failure> {
    let game = ctx.game;
    match subcommand {
        PresetCommand::Save { name } => {
            let game_config = ctx.config.games.entry(game).or_default();
            let blocked = game_config.desired.blocked.len();
            game_config
                .presets
                .insert(name.clone(), game_config.desired.clone());
            ctx.config.save(&ctx.config_path)?;
            println!("saved preset {name:?} for {game} ({blocked} blocked)");
            Ok(())
        }
        PresetCommand::Load { name } => {
            let game_config = ctx.config.games.entry(game).or_default();
            let Some(preset) = game_config.presets.get(name).cloned() else {
                return Err(Error::UnknownPreset {
                    name: name.clone(),
                    game,
                }
                .into());
            };
            let previous = std::mem::replace(&mut game_config.desired, preset);
            let delta = Delta {
                now_blocked: game_config
                    .desired
                    .blocked
                    .difference(&previous.blocked)
                    .cloned()
                    .collect(),
                now_unblocked: previous
                    .blocked
                    .difference(&game_config.desired.blocked)
                    .cloned()
                    .collect(),
            };
            ctx.config.save(&ctx.config_path)?;
            print_delta(&delta, &ctx.style);
            println!("run `regionlock apply` to reconcile");
            Ok(())
        }
        PresetCommand::List { json } => {
            let presets: Vec<(&String, &DesiredState)> = ctx
                .config
                .games
                .get(&game)
                .map(|game_config| game_config.presets.iter().collect())
                .unwrap_or_default();
            if *json {
                let entries: Vec<serde_json::Value> = presets
                    .iter()
                    .map(|(name, state)| json!({"name": name, "blocked": state.blocked.len()}))
                    .collect();
                println!(
                    "{}",
                    json!({
                        "schema_version": SCHEMA_VERSION,
                        "presets": entries,
                    })
                );
                return Ok(());
            }
            let rows: Vec<Vec<Cell>> = presets
                .iter()
                .map(|(name, state)| {
                    vec![
                        Cell::plain(name.to_string()),
                        Cell::plain(state.blocked.len().to_string()),
                    ]
                })
                .collect();
            println!("{}", render_table(&["NAME", "BLOCKED"], &rows, &ctx.style));
            Ok(())
        }
        PresetCommand::Rm { name } => {
            let game_config = ctx.config.games.entry(game).or_default();
            if game_config.presets.remove(name).is_none() {
                return Err(Error::UnknownPreset {
                    name: name.clone(),
                    game,
                }
                .into());
            }
            ctx.config.save(&ctx.config_path)?;
            println!("removed preset {name:?} for {game}");
            Ok(())
        }
    }
}

fn is_blockable(pop: &Pop) -> bool {
    pop.relays.as_ref().is_some_and(|relays| !relays.is_empty())
}

fn region_names(code: &str) -> Vec<Region> {
    match regions::classify(code) {
        Classification::Regions(regions) => regions.to_vec(),
        Classification::Unclassified => Vec::new(),
    }
}

/// Comma-joined region alias names; unclassified POPs are labeled visibly.
fn regions_label(code: &str) -> String {
    match regions::classify(code) {
        Classification::Regions(regions) => regions
            .iter()
            .map(|region| region.name())
            .collect::<Vec<_>>()
            .join(", "),
        Classification::Unclassified => "unclassified".to_string(),
    }
}
