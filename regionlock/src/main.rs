mod cli;

use clap::Parser;

fn main() -> anyhow::Result<()> {
    let cli = cli::Cli::parse();
    // M1e wires commands to regionlock-core; until then every command is
    // an explicit stub so the grammar is testable.
    anyhow::bail!("not yet wired: {:?}", cli.command)
}
