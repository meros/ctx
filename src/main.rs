mod commands;
mod filter;
mod output;
mod resolve;
mod tokens;
mod ts;
mod walker;

use anyhow::Result;
use clap::Parser;
use commands::Cli;

fn main() -> Result<()> {
    let cli = Cli::parse();
    commands::run(cli)
}
