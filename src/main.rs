use clap::{Parser, Subcommand};
use std::{io, path::PathBuf, process::ExitCode};

#[derive(Parser)]
#[command(name = "blackhole", version)]
struct Args {
    #[command(subcommand)]
    command: Action,
}

#[derive(Subcommand)]
enum Action {
    Ledger,
    Check { paths: Vec<PathBuf> },
}

fn main() -> ExitCode {
    let mut out = io::stdout().lock();
    let result = match Args::parse().command {
        Action::Check { paths } => blackhole_check::check(&paths, &mut out),
        Action::Ledger => blackhole_check::regenerate(&mut out),
    };
    match result {
        Ok(found) => ExitCode::from(u8::from(found)),
        Err(e) => {
            eprintln!("blackhole: {e}");
            ExitCode::from(2)
        }
    }
}
