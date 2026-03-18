mod commands;
mod filter;
mod output;
mod resolve;
mod tokens;
mod ts;
mod walker;

use clap::Parser;
use commands::Cli;

fn main() {
    let cli = Cli::parse();
    if let Err(e) = commands::run(cli) {
        // Check if this is a broken pipe (piped to head, less, etc.)
        if let Some(io_err) = e.downcast_ref::<std::io::Error>() {
            if io_err.kind() == std::io::ErrorKind::BrokenPipe {
                std::process::exit(0);
            }
        }
        eprintln!("Error: {:#}", e);
        std::process::exit(1);
    }
}
