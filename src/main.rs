use blackhole_check::{hooks, init};
use clap::{Parser, Subcommand};
use std::{
    io::{self, Write},
    path::PathBuf,
    process::ExitCode,
    time::Duration,
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
        /// Write reported findings as tab-separated rule, path and line records.
        #[arg(long)]
        findings: Option<PathBuf>,
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
    /// Opens a draft pull request of forgeries and reports which ones the blackhole check stopped.
    Canary {
        /// Seconds to wait for the blackhole check.
        #[arg(long, default_value_t = 1200)]
        timeout: u64,
    },
}

fn main() -> ExitCode {
    let mut out = io::stdout().lock();
    let result = match Args::parse().command {
        Action::Init => match init::run(&mut out) {
            Ok(code) => return ExitCode::from(code),
            Err(e) => Err(e),
        },
        Action::Hook { event } => return hooks::run(event),
        Action::Canary { timeout } => {
            match blackhole_check::canary::run(Duration::from_secs(timeout), &mut out) {
                Ok(code) => return ExitCode::from(code),
                Err(e) => Err(e),
            }
        }
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
            findings,
        } => blackhole_check::ci::run_with_findings(
            &base,
            &head,
            approvals.as_deref(),
            today,
            coverage_dir.as_deref(),
            findings.as_deref(),
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
