//! regionlock CLI: parses the frozen grammar (cli.rs), runs commands
//! against regionlock-core, and owns process-level concerns: human/JSON
//! output shapes, exit codes, and terminal styling.

mod cli;
mod commands;
mod output;

use clap::Parser;

fn main() {
    let cli = cli::Cli::parse();
    if let Err(failure) = commands::run(&cli) {
        let code = failure.report(commands::json_requested(&cli.command));
        std::process::exit(code);
    }
}
