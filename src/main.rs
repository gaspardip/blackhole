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
    Check { paths: Vec<PathBuf> },
}

fn main() -> ExitCode {
    let Action::Check { paths } = Args::parse().command;
    match blackhole_check::check(&paths, &mut io::stdout().lock()) {
        Ok(found) => ExitCode::from(u8::from(found)),
        Err(e) => {
            eprintln!("blackhole: {e}");
            ExitCode::from(2)
        }
    }
}
