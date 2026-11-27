use blackhole_check::{hooks, init};
use clap::{Parser, Subcommand};
use std::{
    io::{self, Write},
    path::PathBuf,
    process::ExitCode,
};

#[derive(Parser)]
#[command(name = "blackhole", version)]
struct Args {
    #[command(subcommand)]
    command: Action,
}

#[derive(Subcommand)]
enum Action {
    Init,
    Hook {
        event: hooks::Event,
    },
    Ledger,
    Ci {
        #[arg(long)]
        base: String,
        #[arg(long)]
        head: String,
        #[arg(long)]
        approvals: Option<PathBuf>,
        /// Trusted external directory containing <head SHA>/<base-configured LCOV path>.
        #[arg(long)]
        coverage_dir: Option<PathBuf>,
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
        Action::Init => init::run(&mut out).map(|()| false),
        Action::Hook { event } => return hooks::run(event),
        Action::Check { paths } => blackhole_check::check(&paths, &mut out),
        Action::Object {
            rule,
            path,
            reason,
            renew,
        } => blackhole_check::object(rule, path, reason.as_deref(), renew).map(|()| false),
        Action::Ci {
            base,
            head,
            approvals,
            today,
            coverage_dir,
        } => blackhole_check::ci::run_with_coverage(
            &base,
            &head,
            approvals.as_deref(),
            today,
            coverage_dir.as_deref(),
            &mut out,
        ),
        Action::Ledger => blackhole_check::regenerate(&mut out),
    };
    match result {
        Ok(found) => ExitCode::from(u8::from(found)),
        Err(e) => {
            let _ = writeln!(io::stderr().lock(), "blackhole: {e}");
            ExitCode::from(2)
        }
    }
}
