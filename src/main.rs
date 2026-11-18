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
    Ci {
        #[arg(long)]
        base: String,
        #[arg(long)]
        head: String,
        #[arg(long)]
        approvals: Option<PathBuf>,
        #[arg(long)]
        today: Option<blackhole_check::objections::Date>,
    },
    Object {
        #[arg(long)]
        renew: bool,
        rule: String,
        path: String,
        reason: Option<String>,
    },
    Check {
        paths: Vec<PathBuf>,
    },
}

fn main() -> ExitCode {
    let mut out = io::stdout().lock();
    let result = match Args::parse().command {
        Action::Check { paths } => blackhole_check::check(&paths, &mut out),
        Action::Object {
            rule,
            path,
            reason,
            renew,
        } => blackhole_check::object(rule, path, reason.as_deref(), renew),
        Action::Ci {
            base,
            head,
            approvals,
            today,
        } => blackhole_check::ci::run(&base, &head, approvals.as_deref(), today, &mut out),
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
